use std::{
    io::{BufReader, Cursor},
    sync::mpsc::{self, RecvTimeoutError, Sender},
    thread::{self, JoinHandle},
    time::Duration as StdDuration,
};

use chrono::{DateTime, Duration as ChronoDuration, Local, Timelike};
use image::ImageFormat::Png as PngFormat;
use log::{debug, error, info};
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

fn next_half_hour(now: DateTime<Local>) -> DateTime<Local> {
    let truncated = now
        .with_second(0)
        .and_then(|dt| dt.with_nanosecond(0))
        .expect("failed to truncate current time");

    if truncated.minute() < 30 {
        truncated
            .with_minute(30)
            .expect("failed to align to half-hour")
    } else {
        (truncated + ChronoDuration::hours(1))
            .with_minute(0)
            .expect("failed to align to top of hour")
    }
}

fn duration_until(dt: DateTime<Local>) -> StdDuration {
    let now = Local::now();
    match (dt - now).to_std() {
        Ok(duration) => duration,
        Err(_) => StdDuration::ZERO,
    }
}

#[derive(Debug)]
enum UserEvent {
    Tray(tray_icon::TrayIconEvent),
    Menu(tray_icon::menu::MenuEvent),
    Scheduler(SchedulerEvent),
}

#[derive(Debug)]
enum SchedulerEvent {
    Sync(ClockState),
    Boundary(ClockState),
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
            let initial_state = watch_and_bells_for_time(Local::now());
            if event_proxy
                .send_event(UserEvent::Scheduler(SchedulerEvent::Sync(initial_state)))
                .is_err()
            {
                return;
            }

            loop {
                let next_boundary = next_half_hour(Local::now());
                let timeout = duration_until(next_boundary);

                match scheduler_rx.recv_timeout(timeout) {
                    Ok(SchedulerCommand::Quit) => break,
                    Err(RecvTimeoutError::Timeout) => {
                        let boundary_state = watch_and_bells_for_time(Local::now());
                        if event_proxy
                            .send_event(UserEvent::Scheduler(SchedulerEvent::Boundary(
                                boundary_state,
                            )))
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
            SchedulerEvent::Sync(state) => self.apply_clock_state(state),
            SchedulerEvent::Boundary(state) => {
                self.apply_clock_state(state);

                if self.muted {
                    info!("Muted: skipping {} bells", state.bells);
                } else {
                    ring_bells(state.bells);
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

fn ring_bells(bells: u32) {
    info!("Ringing {} bells", bells);
    thread::spawn(move || {
        if let Err(err) = play_bells_audio(bells) {
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

fn play_bells_audio(bells: u32) -> Result<(), String> {
    let sink_handle = DeviceSinkBuilder::open_default_sink()
        .map_err(|err| format!("failed to open default audio output: {err}"))?;
    let player = Player::connect_new(sink_handle.mixer());

    let pairs = bells / 2;
    for _ in 0..pairs {
        append_embedded_wav(&player, "chime_twice.wav")?;
    }
    if !bells.is_multiple_of(2) {
        append_embedded_wav(&player, "chime_once.wav")?;
    }

    player.sleep_until_end();
    Ok(())
}

fn main() {
    simple_logger::init().expect("failed to initialize logger");

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
    fn test_next_half_hour_at_top() {
        let dt = Local::now()
            .with_hour(10)
            .and_then(|d| d.with_minute(0))
            .and_then(|d| d.with_second(0))
            .unwrap();
        let next = next_half_hour(dt);
        assert_eq!(next.minute(), 30);
    }

    #[test]
    fn test_next_half_hour_at_half() {
        let dt = Local::now()
            .with_hour(10)
            .and_then(|d| d.with_minute(30))
            .and_then(|d| d.with_second(0))
            .unwrap();
        let next = next_half_hour(dt);
        assert_eq!(next.hour(), 11);
        assert_eq!(next.minute(), 0);
    }

    #[test]
    fn test_next_half_hour_at_59_minutes() {
        let dt = Local::now()
            .with_hour(10)
            .and_then(|d| d.with_minute(59))
            .and_then(|d| d.with_second(30))
            .unwrap();
        let next = next_half_hour(dt);
        // Should round down to hour:00 (already past :30), then add 1 hour → 11:00
        assert_eq!(next.hour(), 11);
        assert_eq!(next.minute(), 0);
    }
}
