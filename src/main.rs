#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::{
    env,
    io::IsTerminal,
    io::{BufReader, Cursor},
    sync::mpsc::{self, RecvTimeoutError, Sender},
    thread::{self, JoinHandle},
    time::Duration,
};

use chrono::{
    DateTime, Duration as ChronoDuration, Local, LocalResult, NaiveDateTime, TimeZone, Timelike,
    Utc,
};
use image::ImageFormat::Png as PngFormat;
use log::{debug, error, info, warn};
use rodio::{Decoder, DeviceSinkBuilder, Player};
use rust_embed::RustEmbed;
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{AboutMetadata, CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};
use winit::{
    application::ApplicationHandler,
    event::StartCause,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
};

#[derive(RustEmbed)]
#[folder = "assets/"]
struct Asset;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Watch {
    First,
    Middle,
    Morning,
    Forenoon,
    Afternoon,
    FirstDog,
    LastDog,
}

impl Watch {
    fn display_string(&self) -> &'static str {
        match self {
            Watch::First => "first",
            Watch::Middle => "middle",
            Watch::Morning => "morning",
            Watch::Forenoon => "forenoon",
            Watch::Afternoon => "afternoon",
            Watch::FirstDog => "first dog",
            Watch::LastDog => "last dog",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ClockState {
    watch: Watch,
    bells: u32,
}

impl ClockState {
    fn tooltip(self) -> String {
        let bell_word = if self.bells == 1 { "bell" } else { "bells" };
        format!(
            "{} {} of the {} watch",
            bell_count_text(self.bells),
            bell_word,
            self.watch.display_string()
        )
    }
}

fn bell_count_text(count: u32) -> &'static str {
    match count {
        1 => "One",
        2 => "Two",
        3 => "Three",
        4 => "Four",
        5 => "Five",
        6 => "Six",
        7 => "Seven",
        8 => "Eight",
        _ => unreachable!(),
    }
}

fn watch_and_bells_for_time(dt: DateTime<Local>) -> ClockState {
    let hour = dt.hour();
    let watch = match hour {
        20..=23 => Watch::First,
        0..=3 => Watch::Middle,
        4..=7 => Watch::Morning,
        8..=11 => Watch::Forenoon,
        12..=15 => Watch::Afternoon,
        16..=17 => Watch::FirstDog,
        18..=19 => Watch::LastDog,
        _ => unreachable!(),
    };

    let mut bells = hour % 4 * 2;

    if dt.minute() >= 30 {
        bells += 1;
    }
    if bells == 0 {
        bells = 8;
    }
    if watch == Watch::LastDog && bells >= 5 && bells != 8 {
        bells -= 4;
    }

    ClockState { watch, bells }
}

const MAX_BELL_LATENESS: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BoundaryStatus {
    Fresh,
    BeforeBoundary,
    Stale,
    InvalidLocalBoundary,
}

fn is_local_half_hour_boundary(boundary: DateTime<Utc>) -> bool {
    let local = boundary.with_timezone(&Local);
    is_unique_half_hour_boundary_with(boundary, local, |naive| Local.from_local_datetime(naive))
}

fn is_unique_half_hour_boundary_with<Tz, F>(
    boundary: DateTime<Utc>,
    local: DateTime<Tz>,
    resolve: F,
) -> bool
where
    Tz: TimeZone,
    F: FnOnce(&NaiveDateTime) -> LocalResult<DateTime<Tz>>,
{
    if !((local.minute() == 0 || local.minute() == 30)
        && local.second() == 0
        && local.nanosecond() == 0)
    {
        return false;
    }

    // An ambiguous local civil time is not a unique boundary under the
    // current timezone. Reject it rather than guessing which occurrence was
    // intended after a DST or timezone change.
    resolve(&local.naive_local())
        .single()
        .is_some_and(|resolved| resolved.with_timezone(&Utc) == boundary)
}

fn boundary_status(boundary: DateTime<Utc>, now: DateTime<Utc>) -> BoundaryStatus {
    if !is_local_half_hour_boundary(boundary) {
        return BoundaryStatus::InvalidLocalBoundary;
    }

    if now < boundary {
        return BoundaryStatus::BeforeBoundary;
    }

    match (now - boundary).to_std() {
        Ok(lateness) if lateness <= MAX_BELL_LATENESS => BoundaryStatus::Fresh,
        _ => BoundaryStatus::Stale,
    }
}

fn warn_boundary_rejection(
    stage: &str,
    boundary: DateTime<Utc>,
    now: DateTime<Utc>,
    status: BoundaryStatus,
) {
    match status {
        BoundaryStatus::Fresh => (),
        BoundaryStatus::BeforeBoundary => warn!(
            "Skipping {stage} boundary before intended instant: expected={boundary}, now={now}"
        ),
        BoundaryStatus::Stale => {
            warn!("Skipping stale {stage} boundary: expected={boundary}, now={now}")
        }
        BoundaryStatus::InvalidLocalBoundary => warn!(
            "Skipping {stage} boundary invalidated by wall-clock/timezone change: expected={boundary}, now={now}"
        ),
    }
}

fn claim_boundary(last_boundary: &mut Option<DateTime<Utc>>, boundary: DateTime<Utc>) -> bool {
    if last_boundary.is_some_and(|last| boundary <= last) {
        return false;
    }

    *last_boundary = Some(boundary);
    true
}

fn next_half_hour_naive(now: NaiveDateTime) -> Option<NaiveDateTime> {
    let truncated = now.with_second(0).and_then(|dt| dt.with_nanosecond(0))?;

    Some(if truncated.minute() < 30 {
        truncated.with_minute(30)?
    } else {
        (truncated + ChronoDuration::hours(1)).with_minute(0)?
    })
}

fn next_half_hour<Tz>(now: DateTime<Tz>) -> Option<DateTime<Tz>>
where
    Tz: TimeZone,
{
    next_half_hour_with(&now, |candidate| {
        now.timezone().from_local_datetime(candidate)
    })
}

fn next_half_hour_with<Tz, F>(now: &DateTime<Tz>, mut resolve: F) -> Option<DateTime<Tz>>
where
    Tz: TimeZone,
    F: FnMut(&NaiveDateTime) -> LocalResult<DateTime<Tz>>,
{
    let mut candidate = next_half_hour_naive(now.naive_local())?;

    // A local half-hour that is ambiguous or does not exist is not an
    // appropriate scheduling instant. Skip it and continue to the next one.
    for _ in 0..96 {
        if let Some(candidate) = resolve(&candidate).single()
            && is_after(&candidate, now)
        {
            return Some(candidate);
        }

        candidate = candidate.checked_add_signed(ChronoDuration::minutes(30))?;
    }

    None
}

fn is_after<Tz: TimeZone>(candidate: &DateTime<Tz>, now: &DateTime<Tz>) -> bool {
    candidate > now
}

fn future_display_candidate<Tz: TimeZone>(
    now: &DateTime<Tz>,
    result: LocalResult<DateTime<Tz>>,
) -> Option<DateTime<Tz>> {
    match result {
        LocalResult::Single(candidate) if is_after(&candidate, now) => Some(candidate),
        LocalResult::Ambiguous(first, second) => [first, second]
            .into_iter()
            .filter(|candidate| is_after(candidate, now))
            .min(),
        LocalResult::Single(_) | LocalResult::None => None,
    }
}

fn next_display_wake_with<Tz, F>(now: &DateTime<Tz>, mut resolve: F) -> Option<DateTime<Tz>>
where
    Tz: TimeZone,
    F: FnMut(&NaiveDateTime) -> LocalResult<DateTime<Tz>>,
{
    let mut candidate = next_half_hour_naive(now.naive_local())?;

    // Display synchronisation may use either concrete occurrence of an
    // ambiguous local time. Audible authority continues to require `single()`
    // through `next_half_hour()` and `boundary_status()`.
    for _ in 0..96 {
        if let Some(candidate) = future_display_candidate(now, resolve(&candidate)) {
            return Some(candidate);
        }

        candidate = candidate.checked_add_signed(ChronoDuration::minutes(30))?;
    }

    None
}

fn next_display_wake<Tz>(now: DateTime<Tz>) -> Option<DateTime<Tz>>
where
    Tz: TimeZone,
{
    next_display_wake_with(&now, |candidate| {
        now.timezone().from_local_datetime(candidate)
    })
}

fn duration_until(now: DateTime<Utc>, boundary: DateTime<Utc>) -> Duration {
    (boundary - now).to_std().unwrap_or(Duration::ZERO)
}

fn audio_start_authorised(boundary: DateTime<Utc>, state: ClockState, now: DateTime<Utc>) -> bool {
    let status = boundary_status(boundary, now);
    if status != BoundaryStatus::Fresh {
        warn_boundary_rejection("audio-start authorisation", boundary, now, status);
        return false;
    }

    let boundary_state = watch_and_bells_for_time(boundary.with_timezone(&Local));
    if boundary_state != state {
        warn!(
            "Skipping audio-start authorisation after clock state changed: expected={boundary}, now={now}"
        );
        return false;
    }

    true
}

#[derive(Debug)]
enum UserEvent {
    Tray(tray_icon::TrayIconEvent),
    Menu(tray_icon::menu::MenuEvent),
    Scheduler(SchedulerEvent),
}

#[derive(Debug)]
enum SchedulerEvent {
    Sync,
    Boundary {
        boundary: DateTime<Utc>,
        state: ClockState,
    },
}

enum SchedulerCommand {
    Quit,
}

struct App {
    event_proxy: EventLoopProxy<UserEvent>,
    icon: Icon,
    tray_menu: Menu,
    status_i: MenuItem,
    mute_i: CheckMenuItem,
    quit_i: MenuItem,
    tray_icon: Option<TrayIcon>,
    muted: bool,
    current_state: ClockState,
    last_consumed_boundary: Option<DateTime<Utc>>,
    scheduler_tx: Option<Sender<SchedulerCommand>>,
    scheduler_handle: Option<JoinHandle<()>>,
}

impl App {
    fn new(event_proxy: EventLoopProxy<UserEvent>) -> Self {
        let icon_png = Asset::get("icon.png").expect("missing icon.png in embedded assets");
        let icon_rgba = image::load_from_memory_with_format(&icon_png.data, PngFormat)
            .expect("invalid embedded icon.png")
            .into_rgba8();
        let icon = Icon::from_rgba(icon_rgba.into_vec(), 100, 100).expect("failed to load icon");

        let current_state = watch_and_bells_for_time(Local::now());
        let status_i = MenuItem::new(current_state.tooltip(), false, None);
        let mute_i = CheckMenuItem::new("Mute", true, false, None);
        let quit_i = MenuItem::new("Quit", true, None);
        let tray_menu = Menu::with_items(&[
            &PredefinedMenuItem::about(
                None,
                Some(AboutMetadata {
                    name: Some("Watch Bells".to_string()),
                    version: Some(env!("CARGO_PKG_VERSION").to_string()),
                    ..Default::default()
                }),
            ),
            &PredefinedMenuItem::separator(),
            &status_i,
            &PredefinedMenuItem::separator(),
            &mute_i,
            &PredefinedMenuItem::separator(),
            &quit_i,
        ])
        .expect("failed to build tray menu");

        info!("Starting: {}", current_state.tooltip());

        Self {
            event_proxy,
            icon,
            tray_menu,
            status_i,
            mute_i,
            quit_i,
            tray_icon: None,
            muted: false,
            current_state,
            last_consumed_boundary: None,
            scheduler_tx: None,
            scheduler_handle: None,
        }
    }

    fn ensure_tray_icon(&mut self) {
        if self.tray_icon.is_some() {
            return;
        }

        let tray_icon = TrayIconBuilder::new()
            .with_menu(Box::new(self.tray_menu.clone()))
            .with_tooltip(self.current_state.tooltip())
            .with_icon(self.icon.clone())
            .build()
            .expect("failed to create tray icon");

        if let Err(err) = tray_icon.set_visible(true) {
            error!("Error initialising tray icon: {err:?}");
        }

        self.tray_icon = Some(tray_icon);
    }

    fn ensure_scheduler(&mut self) {
        if self.scheduler_tx.is_some() {
            return;
        }

        let (scheduler_tx, scheduler_rx) = mpsc::channel();
        let event_proxy = self.event_proxy.clone();

        let handle = thread::spawn(move || {
            if event_proxy
                .send_event(UserEvent::Scheduler(SchedulerEvent::Sync))
                .is_err()
            {
                return;
            }

            let mut last_scheduled_boundary = None;

            loop {
                let now = Utc::now();
                let local_now = now.with_timezone(&Local);
                let Some(next_sync_local) = next_display_wake(local_now) else {
                    warn!("Unable to calculate the next display synchronisation wake; retrying");
                    match scheduler_rx.recv_timeout(Duration::from_secs(60)) {
                        Ok(SchedulerCommand::Quit) | Err(RecvTimeoutError::Disconnected) => break,
                        Err(RecvTimeoutError::Timeout) => continue,
                    }
                };
                let Some(next_audible_boundary_local) = next_half_hour(local_now) else {
                    warn!("Unable to calculate the next local half-hour boundary; retrying");
                    match scheduler_rx.recv_timeout(Duration::from_secs(60)) {
                        Ok(SchedulerCommand::Quit) | Err(RecvTimeoutError::Disconnected) => break,
                        Err(RecvTimeoutError::Timeout) => continue,
                    }
                };
                let next_sync = next_sync_local.with_timezone(&Utc);
                let next_audible_boundary = next_audible_boundary_local.with_timezone(&Utc);
                let timeout = duration_until(now, next_sync);

                match scheduler_rx.recv_timeout(timeout) {
                    Ok(SchedulerCommand::Quit) => break,
                    Err(RecvTimeoutError::Timeout) => {
                        let now = Utc::now();
                        if event_proxy
                            .send_event(UserEvent::Scheduler(SchedulerEvent::Sync))
                            .is_err()
                        {
                            break;
                        }

                        let status = boundary_status(next_audible_boundary, now);
                        if status != BoundaryStatus::Fresh {
                            // A display-only wake before the next unique boundary is
                            // expected around a DST fallback. Other non-fresh states
                            // represent a rejected or stale audible authorisation.
                            if !(next_sync != next_audible_boundary
                                && status == BoundaryStatus::BeforeBoundary)
                            {
                                warn_boundary_rejection(
                                    "scheduler",
                                    next_audible_boundary,
                                    now,
                                    status,
                                );
                            }
                            continue;
                        }

                        if !claim_boundary(&mut last_scheduled_boundary, next_audible_boundary) {
                            warn!(
                                "Suppressing duplicate or non-monotonic scheduler boundary: boundary={next_audible_boundary}"
                            );
                            continue;
                        }

                        let boundary_state =
                            watch_and_bells_for_time(next_audible_boundary.with_timezone(&Local));
                        if event_proxy
                            .send_event(UserEvent::Scheduler(SchedulerEvent::Boundary {
                                boundary: next_audible_boundary,
                                state: boundary_state,
                            }))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        });

        self.scheduler_tx = Some(scheduler_tx);
        self.scheduler_handle = Some(handle);
    }

    fn stop_scheduler(&mut self) {
        if let Some(tx) = self.scheduler_tx.take() {
            let _ = tx.send(SchedulerCommand::Quit);
        }

        if let Some(handle) = self.scheduler_handle.take()
            && let Err(err) = handle.join()
        {
            error!("Scheduler thread join failed: {err:?}");
        }
    }

    fn apply_clock_state(&mut self, state: ClockState) {
        self.current_state = state;
        let tooltip = state.tooltip();
        self.status_i.set_text(&tooltip);

        if let Some(tray_icon) = self.tray_icon.as_ref()
            && let Err(err) = tray_icon.set_tooltip(Some(tooltip))
        {
            error!("Error setting tooltip: {err:?}");
        }
    }

    fn handle_scheduler_event(&mut self, event: SchedulerEvent) {
        match event {
            SchedulerEvent::Sync => self.apply_clock_state(watch_and_bells_for_time(Local::now())),
            SchedulerEvent::Boundary { boundary, state } => {
                let now = Utc::now();
                let current_state = watch_and_bells_for_time(now.with_timezone(&Local));
                self.apply_clock_state(current_state);

                let status = boundary_status(boundary, now);
                if status != BoundaryStatus::Fresh {
                    warn_boundary_rejection("event-loop", boundary, now, status);
                    return;
                }

                let boundary_state = watch_and_bells_for_time(boundary.with_timezone(&Local));
                if state != current_state || state != boundary_state {
                    warn!(
                        "Skipping event-loop boundary after clock state changed: expected={boundary}, now={now}"
                    );
                    return;
                }

                if !claim_boundary(&mut self.last_consumed_boundary, boundary) {
                    warn!(
                        "Suppressing duplicate or non-monotonic boundary event: boundary={boundary}"
                    );
                    return;
                }

                if self.muted {
                    info!("Muted: skipping {} bells", state.bells);
                } else {
                    ring_bells(boundary, state);
                }
            }
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.stop_scheduler();
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        _event: winit::event::WindowEvent,
    ) {
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        if cause == StartCause::Init {
            // Tray icon creation is deferred until the event loop starts, per tray-icon docs.
            self.ensure_tray_icon();
            self.ensure_scheduler();

            #[cfg(target_os = "macos")]
            {
                use objc2_core_foundation::CFRunLoop;

                if let Some(rl) = CFRunLoop::main() {
                    rl.wake_up();
                }
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Tray(event) => {
                // Only log meaningful tray events; filter out noisy MouseMove/MouseLeave
                match event {
                    TrayIconEvent::Click { .. } => debug!("Tray clicked"),
                    TrayIconEvent::DoubleClick { .. } => debug!("Tray double-clicked"),
                    _ => {} // Ignore MouseMove, MouseLeave, etc.
                }
            }
            UserEvent::Menu(event) => {
                if event.id == self.mute_i.id() {
                    self.muted = !self.muted;
                    self.mute_i.set_checked(self.muted);
                    info!("Muted: {}", self.muted);
                }

                if event.id == self.quit_i.id() {
                    info!("Quit selected");
                    self.stop_scheduler();
                    self.tray_icon = None;
                    event_loop.exit();
                }
            }
            UserEvent::Scheduler(event) => self.handle_scheduler_event(event),
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
    }
}

fn ring_bells(boundary: DateTime<Utc>, state: ClockState) {
    thread::spawn(move || {
        if let Err(err) = play_bells_audio(boundary, state) {
            error!("Audio playback failed: {err}");
        }
    });
}

fn append_embedded_wav(player: &Player, asset_name: &str) -> Result<(), String> {
    let asset =
        Asset::get(asset_name).ok_or_else(|| format!("missing {asset_name} in embedded assets"))?;
    let reader = BufReader::new(Cursor::new(asset.data.into_owned()));
    let source =
        Decoder::try_from(reader).map_err(|err| format!("failed to decode {asset_name}: {err}"))?;
    player.append(source);
    Ok(())
}

fn play_bells_audio(boundary: DateTime<Utc>, state: ClockState) -> Result<(), String> {
    if !audio_start_authorised(boundary, state, Utc::now()) {
        return Ok(());
    }

    let sink_handle = DeviceSinkBuilder::open_default_sink()
        .map_err(|err| format!("failed to open default audio output: {err}"))?;

    // Opening an output device does not begin the sequence. Revalidate after
    // opening it and immediately before appending the first chime.
    if !audio_start_authorised(boundary, state, Utc::now()) {
        return Ok(());
    }

    info!("Ringing {} bells", state.bells);

    let bells = state.bells;
    let pairs = bells / 2;
    for _ in 0..pairs {
        let player = Player::connect_new(sink_handle.mixer());
        append_embedded_wav(&player, "chime_twice.wav")?;
        player.sleep_until_end();
    }
    if !bells.is_multiple_of(2) {
        let player = Player::connect_new(sink_handle.mixer());
        append_embedded_wav(&player, "chime_once.wav")?;
        player.sleep_until_end();
    }

    Ok(())
}

fn should_enable_logging() -> bool {
    if env::var_os("WATCH_BELLS_LOG").is_some() {
        return true;
    }

    std::io::stderr().is_terminal()
}

fn main() {
    if should_enable_logging() {
        simple_logger::init().expect("failed to initialize logger");
    }

    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .expect("failed to create event loop");

    let proxy = event_loop.create_proxy();
    TrayIconEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Tray(event));
    }));

    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::Menu(event));
    }));

    let mut app = App::new(event_loop.create_proxy());
    if let Err(err) = event_loop.run_app(&mut app) {
        error!("Application error: {err:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, FixedOffset};

    fn local_test_boundary() -> DateTime<Utc> {
        Local::now()
            .with_hour(12)
            .and_then(|dt| dt.with_minute(0))
            .and_then(|dt| dt.with_second(0))
            .and_then(|dt| dt.with_nanosecond(0))
            .expect("failed to construct local test boundary")
            .with_timezone(&Utc)
    }

    fn fixed_time(hour: u32, minute: u32, second: u32) -> DateTime<FixedOffset> {
        FixedOffset::east_opt(0)
            .expect("failed to construct test offset")
            .with_ymd_and_hms(2026, 1, 2, hour, minute, second)
            .single()
            .expect("failed to construct fixed test time")
    }

    fn fixed_offset(hours: i32) -> FixedOffset {
        FixedOffset::east_opt(hours * 60 * 60).expect("failed to construct test offset")
    }

    fn fallback_resolution(candidate: &NaiveDateTime) -> LocalResult<DateTime<FixedOffset>> {
        let daylight = fixed_offset(-4);
        let standard = fixed_offset(-5);

        match (candidate.hour(), candidate.minute()) {
            (1, 0) | (1, 30) => LocalResult::Ambiguous(
                daylight
                    .from_local_datetime(candidate)
                    .single()
                    .expect("failed to construct daylight fallback occurrence"),
                standard
                    .from_local_datetime(candidate)
                    .single()
                    .expect("failed to construct standard fallback occurrence"),
            ),
            _ if candidate.hour() >= 2 => LocalResult::Single(
                standard
                    .from_local_datetime(candidate)
                    .single()
                    .expect("failed to construct standard time"),
            ),
            _ => LocalResult::Single(
                daylight
                    .from_local_datetime(candidate)
                    .single()
                    .expect("failed to construct daylight time"),
            ),
        }
    }

    fn spring_gap_resolution(candidate: &NaiveDateTime) -> LocalResult<DateTime<FixedOffset>> {
        let standard = fixed_offset(-5);
        let daylight = fixed_offset(-4);

        match candidate.hour() {
            2 => LocalResult::None,
            3.. => LocalResult::Single(
                daylight
                    .from_local_datetime(candidate)
                    .single()
                    .expect("failed to construct daylight time"),
            ),
            _ => LocalResult::Single(
                standard
                    .from_local_datetime(candidate)
                    .single()
                    .expect("failed to construct standard time"),
            ),
        }
    }

    fn next_boundary_utc(now: DateTime<Utc>) -> DateTime<Utc> {
        next_half_hour(now.with_timezone(&Local))
            .expect("failed to calculate next test boundary")
            .with_timezone(&Utc)
    }

    #[test]
    fn test_first_watch() {
        // First Watch: 8pm-midnight (20:00-23:59)
        let dt = Local::now()
            .with_hour(20)
            .and_then(|d| d.with_minute(0))
            .unwrap();
        let state = watch_and_bells_for_time(dt);
        assert_eq!(state.watch, Watch::First);
        assert_eq!(state.bells, 8); // Top of watch

        let dt = Local::now()
            .with_hour(22)
            .and_then(|d| d.with_minute(30))
            .unwrap();
        let state = watch_and_bells_for_time(dt);
        assert_eq!(state.watch, Watch::First);
        assert_eq!(state.bells, 5); // 22 % 4 = 2, 2*2 = 4, +1 at 30min = 5
    }

    #[test]
    fn test_middle_watch() {
        // Middle Watch: midnight-4am (00:00-03:59)
        let dt = Local::now()
            .with_hour(0)
            .and_then(|d| d.with_minute(0))
            .unwrap();
        let state = watch_and_bells_for_time(dt);
        assert_eq!(state.watch, Watch::Middle);
        assert_eq!(state.bells, 8); // Top of watch

        let dt = Local::now()
            .with_hour(2)
            .and_then(|d| d.with_minute(30))
            .unwrap();
        let state = watch_and_bells_for_time(dt);
        assert_eq!(state.watch, Watch::Middle);
        assert_eq!(state.bells, 5); // 2 % 4 = 2, 2*2 = 4, +1 at 30min = 5
    }

    #[test]
    fn test_morning_watch() {
        // Morning Watch: 4am-8am (04:00-07:59)
        let dt = Local::now()
            .with_hour(4)
            .and_then(|d| d.with_minute(0))
            .unwrap();
        let state = watch_and_bells_for_time(dt);
        assert_eq!(state.watch, Watch::Morning);
        assert_eq!(state.bells, 8); // Top of watch

        let dt = Local::now()
            .with_hour(6)
            .and_then(|d| d.with_minute(30))
            .unwrap();
        let state = watch_and_bells_for_time(dt);
        assert_eq!(state.watch, Watch::Morning);
        assert_eq!(state.bells, 5); // 6 % 4 = 2, 2*2 = 4, +1 at 30min = 5
    }

    #[test]
    fn test_forenoon_watch() {
        // Forenoon Watch: 8am-midday (08:00-11:59)
        let dt = Local::now()
            .with_hour(8)
            .and_then(|d| d.with_minute(0))
            .unwrap();
        let state = watch_and_bells_for_time(dt);
        assert_eq!(state.watch, Watch::Forenoon);
        assert_eq!(state.bells, 8); // Top of watch

        let dt = Local::now()
            .with_hour(10)
            .and_then(|d| d.with_minute(30))
            .unwrap();
        let state = watch_and_bells_for_time(dt);
        assert_eq!(state.watch, Watch::Forenoon);
        assert_eq!(state.bells, 5); // 10 % 4 = 2, 2*2 = 4, +1 at 30min = 5
    }

    #[test]
    fn test_afternoon_watch() {
        // Afternoon Watch: midday-4pm (12:00-15:59)
        let dt = Local::now()
            .with_hour(12)
            .and_then(|d| d.with_minute(0))
            .unwrap();
        let state = watch_and_bells_for_time(dt);
        assert_eq!(state.watch, Watch::Afternoon);
        assert_eq!(state.bells, 8); // Top of watch

        let dt = Local::now()
            .with_hour(14)
            .and_then(|d| d.with_minute(30))
            .unwrap();
        let state = watch_and_bells_for_time(dt);
        assert_eq!(state.watch, Watch::Afternoon);
        assert_eq!(state.bells, 5); // 14 % 4 = 2, 2*2 = 4, +1 at 30min = 5
    }

    #[test]
    fn test_first_dog_watch() {
        // First Dog Watch: 4pm-6pm (16:00-17:59)
        let dt = Local::now()
            .with_hour(16)
            .and_then(|d| d.with_minute(0))
            .unwrap();
        let state = watch_and_bells_for_time(dt);
        assert_eq!(state.watch, Watch::FirstDog);
        assert_eq!(state.bells, 8); // Top of watch

        let dt = Local::now()
            .with_hour(16)
            .and_then(|d| d.with_minute(30))
            .unwrap();
        let state = watch_and_bells_for_time(dt);
        assert_eq!(state.watch, Watch::FirstDog);
        assert_eq!(state.bells, 1); // First half-hour of first dog watch is 1 bell
    }

    #[test]
    fn test_last_dog_watch() {
        // Last Dog Watch: 6pm-8pm (18:00-19:59)
        // Special: rings 1-2-3-4 then 1-2-3-8 (never 5 bells due to Nore mutiny)
        let dt = Local::now()
            .with_hour(18)
            .and_then(|d| d.with_minute(0))
            .unwrap();
        let state = watch_and_bells_for_time(dt);
        assert_eq!(state.watch, Watch::LastDog);
        assert_eq!(state.bells, 4); // First hour of last dog watch is 4 bells

        let dt = Local::now()
            .with_hour(18)
            .and_then(|d| d.with_minute(30))
            .unwrap();
        let state = watch_and_bells_for_time(dt);
        assert_eq!(state.watch, Watch::LastDog);
        // 18 % 4 = 2, 2*2 = 4, +1 at 30min = 5, but LastDog and bells > 4 so -4 → 1
        assert_eq!(state.bells, 1);

        let dt = Local::now()
            .with_hour(19)
            .and_then(|d| d.with_minute(30))
            .unwrap();
        let state = watch_and_bells_for_time(dt);
        assert_eq!(state.watch, Watch::LastDog);
        // 19 % 4 = 3, 3*2 = 6, +1 at 30min = 7, but LastDog and bells > 4 so -4 → 3
        assert_eq!(state.bells, 3);
    }

    #[test]
    fn test_bell_count_text() {
        assert_eq!(bell_count_text(1), "One");
        assert_eq!(bell_count_text(2), "Two");
        assert_eq!(bell_count_text(3), "Three");
        assert_eq!(bell_count_text(4), "Four");
        assert_eq!(bell_count_text(5), "Five");
        assert_eq!(bell_count_text(6), "Six");
        assert_eq!(bell_count_text(7), "Seven");
        assert_eq!(bell_count_text(8), "Eight");
    }

    #[test]
    fn test_clock_state_tooltip() {
        let state = ClockState {
            watch: Watch::Morning,
            bells: 4,
        };
        let tooltip = state.tooltip();
        assert_eq!(tooltip, "Four bells of the morning watch");
    }

    #[test]
    fn test_clock_state_tooltip_singular() {
        let state = ClockState {
            watch: Watch::FirstDog,
            bells: 1,
        };
        let tooltip = state.tooltip();
        assert_eq!(tooltip, "One bell of the first dog watch");
    }

    #[test]
    fn exact_boundary_is_fresh() {
        let boundary = local_test_boundary();
        assert_eq!(boundary_status(boundary, boundary), BoundaryStatus::Fresh);
    }

    #[test]
    fn milliseconds_late_boundary_is_fresh() {
        let boundary = local_test_boundary();
        let now = boundary + ChronoDuration::milliseconds(50);
        assert_eq!(boundary_status(boundary, now), BoundaryStatus::Fresh);
    }

    #[test]
    fn boundary_just_inside_grace_period_is_fresh() {
        let boundary = local_test_boundary();
        let now = boundary + ChronoDuration::milliseconds(4_999);
        assert_eq!(boundary_status(boundary, now), BoundaryStatus::Fresh);
    }

    #[test]
    fn boundary_at_grace_period_limit_is_still_fresh() {
        let boundary = local_test_boundary();
        let now = boundary + ChronoDuration::seconds(5);
        assert_eq!(boundary_status(boundary, now), BoundaryStatus::Fresh);
    }

    #[test]
    fn boundary_just_outside_grace_period_is_stale() {
        let boundary = local_test_boundary();
        let now = boundary + ChronoDuration::milliseconds(5_001);
        assert_eq!(boundary_status(boundary, now), BoundaryStatus::Stale);
    }

    #[test]
    fn boundary_before_intended_instant_is_rejected() {
        let boundary = local_test_boundary();
        let now = boundary - ChronoDuration::milliseconds(1);
        assert_eq!(
            boundary_status(boundary, now),
            BoundaryStatus::BeforeBoundary
        );
    }

    #[test]
    fn stale_scheduler_wake_is_rejected_and_resynchronises() {
        let boundary = local_test_boundary();
        let now = boundary + ChronoDuration::seconds(20);
        assert_eq!(boundary_status(boundary, now), BoundaryStatus::Stale);

        let next = next_boundary_utc(now);
        assert!(next > now);
    }

    #[test]
    fn stale_event_loop_delivery_is_rejected() {
        let boundary = local_test_boundary();
        let queued_until = boundary + ChronoDuration::minutes(42);
        assert_eq!(
            boundary_status(boundary, queued_until),
            BoundaryStatus::Stale
        );
    }

    #[test]
    fn stale_audio_start_is_rejected() {
        let boundary = local_test_boundary();
        let audio_thread_now = boundary + ChronoDuration::seconds(6);
        let state = watch_and_bells_for_time(boundary.with_timezone(&Local));
        assert!(!audio_start_authorised(boundary, state, audio_thread_now));
    }

    #[test]
    fn forward_clock_jump_skips_old_boundary_and_uses_future_boundary() {
        let boundary = local_test_boundary();
        let now = boundary + ChronoDuration::hours(2);
        assert_eq!(boundary_status(boundary, now), BoundaryStatus::Stale);

        let next = next_boundary_utc(now);
        assert!(next > now);
        assert_ne!(next, boundary);
    }

    #[test]
    fn backward_clock_jump_rejects_old_authorisation() {
        let boundary = local_test_boundary();
        let now = boundary - ChronoDuration::seconds(1);
        assert_eq!(
            boundary_status(boundary, now),
            BoundaryStatus::BeforeBoundary
        );

        let resynchronised = next_boundary_utc(now);
        assert_eq!(resynchronised, boundary);
        assert!(resynchronised > now);
    }

    #[test]
    fn invalid_local_boundary_is_rejected_conservatively() {
        let boundary = local_test_boundary() + ChronoDuration::minutes(1);
        let now = boundary + ChronoDuration::seconds(1);
        assert_eq!(
            boundary_status(boundary, now),
            BoundaryStatus::InvalidLocalBoundary
        );
    }

    #[test]
    fn duplicate_boundary_cannot_be_claimed_twice() {
        let boundary = local_test_boundary();
        let mut last_boundary = None;

        assert!(claim_boundary(&mut last_boundary, boundary));
        assert!(!claim_boundary(&mut last_boundary, boundary));
        assert!(!claim_boundary(
            &mut last_boundary,
            boundary - ChronoDuration::minutes(30)
        ));
    }

    #[test]
    fn distinct_consecutive_boundaries_can_each_be_claimed() {
        let first = local_test_boundary();
        let second = first + ChronoDuration::minutes(30);
        let mut last_boundary = None;

        assert_eq!(boundary_status(first, first), BoundaryStatus::Fresh);
        assert!(claim_boundary(&mut last_boundary, first));
        assert_eq!(boundary_status(second, second), BoundaryStatus::Fresh);
        assert!(claim_boundary(&mut last_boundary, second));
    }

    #[test]
    fn stale_boundary_does_not_suppress_next_legitimate_boundary() {
        let stale = local_test_boundary();
        let next = stale + ChronoDuration::minutes(30);
        let mut last_boundary = None;

        assert_eq!(
            boundary_status(stale, stale + ChronoDuration::seconds(20)),
            BoundaryStatus::Stale
        );
        assert_eq!(boundary_status(next, next), BoundaryStatus::Fresh);
        assert!(claim_boundary(&mut last_boundary, next));
    }

    #[test]
    fn startup_shortly_after_boundary_schedules_only_a_future_boundary() {
        let boundary = local_test_boundary();
        let startup = boundary + ChronoDuration::seconds(3);
        assert_eq!(boundary_status(boundary, startup), BoundaryStatus::Fresh);

        let next = next_boundary_utc(startup);
        assert!(next > startup);
        assert_ne!(next, boundary);
    }

    #[test]
    fn startup_away_from_boundary_schedules_only_a_future_boundary() {
        let previous = local_test_boundary();
        let startup = previous + ChronoDuration::minutes(17);

        let next = next_boundary_utc(startup);
        assert!(next > startup);
        assert_ne!(next, previous);
    }

    #[test]
    fn next_half_hour_handles_representative_local_times() {
        let cases = [
            (fixed_time(0, 0, 0), 0, 30, 2),
            (fixed_time(10, 0, 0), 10, 30, 2),
            (fixed_time(10, 29, 59), 10, 30, 2),
            (fixed_time(10, 30, 0), 11, 0, 2),
            (fixed_time(10, 59, 30), 11, 0, 2),
            (fixed_time(23, 59, 59), 0, 0, 3),
        ];

        for (now, expected_hour, expected_minute, expected_day) in cases {
            let next = next_half_hour(now).expect("failed to find next half-hour");
            assert!(next > now);
            assert_eq!(next.hour(), expected_hour);
            assert_eq!(next.minute(), expected_minute);
            assert_eq!(next.day(), expected_day);
            assert_eq!(next.second(), 0);
            assert_eq!(next.nanosecond(), 0);
        }
    }

    #[test]
    fn explicit_dst_gap_skips_nonexistent_half_hours_for_scheduling() {
        let standard = fixed_offset(-5);
        let before_gap = standard
            .with_ymd_and_hms(2026, 3, 8, 1, 59, 0)
            .single()
            .expect("failed to construct time before DST gap");

        let next = next_half_hour_with(&before_gap, spring_gap_resolution)
            .expect("failed to skip explicit DST gap");
        assert_eq!(next.hour(), 3);
        assert_eq!(next.minute(), 0);
        assert_eq!(next.offset().local_minus_utc(), -4 * 60 * 60);
    }

    #[test]
    fn explicit_dst_fallback_display_wake_progresses_through_repeated_hour() {
        let daylight = fixed_offset(-4);
        let standard = fixed_offset(-5);
        let before_fallback = daylight
            .with_ymd_and_hms(2026, 11, 1, 0, 30, 0)
            .single()
            .expect("failed to construct time before DST fallback");

        let next_audible = next_half_hour_with(&before_fallback, fallback_resolution)
            .expect("failed to find unique post-fallback boundary");
        assert_eq!((next_audible.hour(), next_audible.minute()), (2, 0));
        assert_eq!(next_audible.offset().local_minus_utc(), -5 * 60 * 60);

        let first_hour = next_display_wake_with(&before_fallback, fallback_resolution)
            .expect("failed to find first fallback display wake");
        assert_eq!((first_hour.hour(), first_hour.minute()), (1, 0));
        assert_eq!(first_hour.offset().local_minus_utc(), -4 * 60 * 60);

        let first_half_hour = next_display_wake_with(
            &(first_hour + ChronoDuration::seconds(1)),
            fallback_resolution,
        )
        .expect("failed to find second fallback display wake");
        assert_eq!((first_half_hour.hour(), first_half_hour.minute()), (1, 30));
        assert_eq!(first_half_hour.offset().local_minus_utc(), -4 * 60 * 60);

        let after_repeated_hour = next_display_wake_with(
            &(first_half_hour + ChronoDuration::seconds(1)),
            fallback_resolution,
        )
        .expect("failed to find post-fallback display wake");
        assert_eq!(
            (after_repeated_hour.hour(), after_repeated_hour.minute()),
            (2, 0)
        );
        assert_eq!(after_repeated_hour.offset().local_minus_utc(), -5 * 60 * 60);

        let second_occurrence = standard
            .with_ymd_and_hms(2026, 11, 1, 1, 0, 0)
            .single()
            .expect("failed to construct repeated-hour occurrence");
        let second_half_hour = next_display_wake_with(
            &(second_occurrence + ChronoDuration::seconds(1)),
            fallback_resolution,
        )
        .expect("failed to find repeated-hour display wake");
        assert_eq!(
            (second_half_hour.hour(), second_half_hour.minute()),
            (1, 30)
        );
        assert_eq!(second_half_hour.offset().local_minus_utc(), -5 * 60 * 60);
    }

    #[test]
    fn explicit_dst_fallback_boundaries_remain_unauthorised() {
        let daylight = fixed_offset(-4);
        let standard = fixed_offset(-5);

        for local in [
            daylight
                .with_ymd_and_hms(2026, 11, 1, 1, 0, 0)
                .single()
                .expect("failed to construct daylight 01:00"),
            standard
                .with_ymd_and_hms(2026, 11, 1, 1, 0, 0)
                .single()
                .expect("failed to construct standard 01:00"),
            daylight
                .with_ymd_and_hms(2026, 11, 1, 1, 30, 0)
                .single()
                .expect("failed to construct daylight 01:30"),
            standard
                .with_ymd_and_hms(2026, 11, 1, 1, 30, 0)
                .single()
                .expect("failed to construct standard 01:30"),
        ] {
            assert!(!is_unique_half_hour_boundary_with(
                local.with_timezone(&Utc),
                local,
                fallback_resolution,
            ));
        }

        let unique = standard
            .with_ymd_and_hms(2026, 11, 1, 2, 0, 0)
            .single()
            .expect("failed to construct unique post-fallback boundary");
        assert!(is_unique_half_hour_boundary_with(
            unique.with_timezone(&Utc),
            unique,
            fallback_resolution,
        ));
    }

    #[test]
    fn explicit_dst_gap_does_not_produce_a_display_candidate_for_missing_time() {
        let standard = fixed_offset(-5);
        let before_gap = standard
            .with_ymd_and_hms(2026, 3, 8, 1, 59, 0)
            .single()
            .expect("failed to construct time before DST gap");
        let nonexistent = NaiveDateTime::new(
            before_gap.date_naive(),
            chrono::NaiveTime::from_hms_opt(2, 0, 0).expect("failed to construct gap time"),
        );

        assert!(
            future_display_candidate(&before_gap, spring_gap_resolution(&nonexistent),).is_none()
        );
    }
}
