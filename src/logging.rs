use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{self, IsTerminal, Write},
    panic::{self, PanicHookInfo},
    path::{Path, PathBuf},
    sync::Mutex,
};

use chrono::{Local, SecondsFormat};
use log::{Level, LevelFilter, Log, Metadata, Record, error};

const APP_NAME: &str = "Watch Bells";
const LINUX_APP_NAME: &str = "watch-bells";
const LOG_FILE_NAME: &str = "watch-bells.log";
const OLD_LOG_FILE_NAME: &str = "watch-bells.old.log";
const MAX_LOG_SIZE: u64 = 512 * 1024;
const PERSISTENT_LEVEL: LevelFilter = LevelFilter::Info;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Platform {
    Macos,
    Windows,
    Linux,
    Unsupported,
}

struct LogFile {
    file: File,
    path: PathBuf,
}

struct WatchBellsLogger {
    file: Mutex<Option<LogFile>>,
    stderr_level: LevelFilter,
}

impl WatchBellsLogger {
    fn new(stderr_level: LevelFilter) -> Self {
        let file = match open_log_file() {
            Some(file) => Some(file),
            None => {
                report_stderr(
                    stderr_level,
                    "Watch Bells could not initialise persistent logging",
                );
                None
            }
        };

        Self {
            file: Mutex::new(file),
            stderr_level,
        }
    }

    fn write_persistent(&self, line: &str) {
        let failure = {
            let Ok(mut state) = self.file.lock() else {
                return;
            };

            match write_persistent_record(&mut state, line) {
                Ok(()) => None,
                Err(error) => {
                    state.take();
                    Some(error)
                }
            }
        };

        if let Some(message) = failure {
            report_stderr(self.stderr_level, &message);
        }
    }
}

impl Log for WatchBellsLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        level_enabled(metadata.level(), PERSISTENT_LEVEL)
            || level_enabled(metadata.level(), self.stderr_level)
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let line = format_log_line(record);

        if level_enabled(record.level(), PERSISTENT_LEVEL) {
            self.write_persistent(&line);
        }

        if level_enabled(record.level(), self.stderr_level) {
            let mut stderr = io::stderr().lock();
            let _ = stderr.write_all(line.as_bytes());
            let _ = stderr.flush();
        }
    }

    fn flush(&self) {
        if let Ok(mut state) = self.file.lock()
            && let Some(log_file) = state.as_mut()
        {
            let _ = log_file.file.flush();
        }

        if self.stderr_level != LevelFilter::Off {
            let _ = io::stderr().lock().flush();
        }
    }
}

pub fn initialise() {
    let stderr_level = stderr_level();
    let max_level = std::cmp::max(PERSISTENT_LEVEL, stderr_level);

    if log::set_boxed_logger(Box::new(WatchBellsLogger::new(stderr_level))).is_ok() {
        log::set_max_level(max_level);
    }

    install_panic_hook();
}

fn install_panic_hook() {
    let previous_hook = panic::take_hook();
    panic::set_hook(Box::new(move |panic_info| {
        let location = panic_info
            .location()
            .map(|location| {
                format!(
                    " at {}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            })
            .unwrap_or_default();
        error!("Panic{location}: {}", panic_message(panic_info));
        previous_hook(panic_info);
    }));
}

fn panic_message(panic_info: &PanicHookInfo<'_>) -> String {
    if let Some(message) = panic_info.payload().downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = panic_info.payload().downcast_ref::<String>() {
        return message.clone();
    }

    "non-string panic payload".to_string()
}

fn stderr_level() -> LevelFilter {
    if env::var_os("WATCH_BELLS_LOG").is_some() {
        LevelFilter::Debug
    } else if io::stderr().is_terminal() {
        LevelFilter::Info
    } else {
        LevelFilter::Off
    }
}

fn report_stderr(level: LevelFilter, message: &str) {
    if level == LevelFilter::Off {
        return;
    }

    let mut stderr = io::stderr().lock();
    let _ = writeln!(stderr, "{message}");
    let _ = stderr.flush();
}

fn level_enabled(level: Level, threshold: LevelFilter) -> bool {
    threshold != LevelFilter::Off && level.to_level_filter() <= threshold
}

fn format_log_line(record: &Record<'_>) -> String {
    let timestamp = Local::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    format_log_line_at(&timestamp, record.level(), record.args())
}

fn format_log_line_at(timestamp: &str, level: Level, message: impl std::fmt::Display) -> String {
    format!("{timestamp} {level} {message}\n")
}

fn current_platform() -> Platform {
    #[cfg(target_os = "macos")]
    {
        Platform::Macos
    }
    #[cfg(target_os = "windows")]
    {
        Platform::Windows
    }
    #[cfg(target_os = "linux")]
    {
        Platform::Linux
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Platform::Unsupported
    }
}

fn environment_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .filter(|path| usable_base_path(Some(path)).is_some())
}

fn log_directory() -> Option<PathBuf> {
    let home = environment_path("HOME");
    let local_app_data = environment_path("LOCALAPPDATA");
    let xdg_state_home = environment_path("XDG_STATE_HOME");

    log_directory_for(
        current_platform(),
        home.as_deref(),
        local_app_data.as_deref(),
        xdg_state_home.as_deref(),
    )
}

fn log_directory_for(
    platform: Platform,
    home: Option<&Path>,
    local_app_data: Option<&Path>,
    xdg_state_home: Option<&Path>,
) -> Option<PathBuf> {
    match platform {
        Platform::Macos => home.map(|home| home.join("Library").join("Logs").join(APP_NAME)),
        Platform::Windows => local_app_data.map(|path| path.join(APP_NAME)),
        Platform::Linux => xdg_state_home
            .map(|path| path.join(LINUX_APP_NAME))
            .or_else(|| home.map(|home| home.join(".local").join("state").join(LINUX_APP_NAME))),
        Platform::Unsupported => None,
    }
}

fn usable_base_path(path: Option<&Path>) -> Option<&Path> {
    path.filter(|path| !path.as_os_str().is_empty() && path.is_absolute())
}

fn open_log_file() -> Option<LogFile> {
    let directory = log_directory()?;
    open_log_file_at(&directory.join(LOG_FILE_NAME))
}

fn open_log_file_at(path: &Path) -> Option<LogFile> {
    let directory = path.parent()?;
    if fs::create_dir_all(directory).is_err() {
        return None;
    }

    rotate_log_if_needed(path);

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()?;
    Some(LogFile {
        file,
        path: path.to_path_buf(),
    })
}

fn write_persistent_record(state: &mut Option<LogFile>, line: &str) -> Result<(), String> {
    let path = {
        let Some(log_file) = state.as_mut() else {
            return Ok(());
        };

        log_file
            .file
            .write_all(line.as_bytes())
            .map_err(|error| format!("Watch Bells logfile write failed: {error}"))?;
        log_file
            .file
            .flush()
            .map_err(|error| format!("Watch Bells logfile flush failed: {error}"))?;
        let size = log_file
            .file
            .metadata()
            .map_err(|error| format!("Watch Bells logfile size check failed: {error}"))?
            .len();

        if !needs_rotation(size) {
            return Ok(());
        }

        log_file.path.clone()
    };

    drop(state.take());
    rotate_live_log(state, &path)
}

fn rotate_live_log(state: &mut Option<LogFile>, path: &Path) -> Result<(), String> {
    let old_path = path.with_file_name(OLD_LOG_FILE_NAME);
    remove_previous_log(&old_path)
        .map_err(|error| format!("Watch Bells previous logfile removal failed: {error}"))?;
    fs::rename(path, &old_path)
        .map_err(|error| format!("Watch Bells logfile rotation failed: {error}"))?;

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("Watch Bells logfile reopen failed: {error}"))?;
    *state = Some(LogFile {
        file,
        path: path.to_path_buf(),
    });
    Ok(())
}

fn remove_previous_log(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn rotate_log_if_needed(path: &Path) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };

    if !needs_rotation(metadata.len()) {
        return;
    }

    let old_path = path.with_file_name(OLD_LOG_FILE_NAME);
    let _ = fs::remove_file(&old_path);
    let _ = fs::rename(path, old_path);
}

fn needs_rotation(size: u64) -> bool {
    size > MAX_LOG_SIZE
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        APP_NAME, LINUX_APP_NAME, LOG_FILE_NAME, Level, LevelFilter, MAX_LOG_SIZE, Platform,
        format_log_line_at, level_enabled, log_directory_for, needs_rotation, open_log_file_at,
        rotate_log_if_needed, usable_base_path, write_persistent_record,
    };

    fn test_directory() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is before the Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "watch-bells-logging-{}-{unique}",
            std::process::id()
        ))
    }

    #[test]
    fn log_directory_uses_platform_conventions() {
        let home = Path::new("/home/tester");
        let local_app_data = Path::new("/local/app-data");
        let xdg_state_home = Path::new("/state");

        assert_eq!(
            log_directory_for(Platform::Macos, Some(home), None, None),
            Some(home.join("Library").join("Logs").join(APP_NAME))
        );
        assert_eq!(
            log_directory_for(Platform::Windows, None, Some(local_app_data), None),
            Some(local_app_data.join(APP_NAME))
        );
        assert_eq!(
            log_directory_for(Platform::Linux, Some(home), None, Some(xdg_state_home)),
            Some(xdg_state_home.join(LINUX_APP_NAME))
        );
        assert_eq!(
            log_directory_for(Platform::Linux, Some(home), None, None),
            Some(home.join(".local").join("state").join(LINUX_APP_NAME))
        );
    }

    #[test]
    fn log_directory_rejects_unusable_environment_paths() {
        let relative = Path::new("relative");
        let empty = Path::new("");

        assert!(usable_base_path(Some(relative)).is_none());
        assert!(usable_base_path(Some(empty)).is_none());
    }

    #[test]
    fn log_directory_requires_the_platform_base_directory() {
        assert_eq!(log_directory_for(Platform::Macos, None, None, None), None);
        assert_eq!(log_directory_for(Platform::Windows, None, None, None), None);
        assert_eq!(log_directory_for(Platform::Linux, None, None, None), None);
        assert_eq!(
            log_directory_for(Platform::Unsupported, None, None, None),
            None
        );
    }

    #[test]
    fn rotation_starts_only_above_the_size_limit() {
        assert!(!needs_rotation(MAX_LOG_SIZE));
        assert!(needs_rotation(MAX_LOG_SIZE + 1));
    }

    #[test]
    fn persistent_and_stderr_thresholds_are_independent() {
        assert!(level_enabled(Level::Error, LevelFilter::Info));
        assert!(level_enabled(Level::Warn, LevelFilter::Info));
        assert!(level_enabled(Level::Info, LevelFilter::Info));
        assert!(!level_enabled(Level::Debug, LevelFilter::Info));
        assert!(level_enabled(Level::Debug, LevelFilter::Debug));
        assert!(!level_enabled(Level::Info, LevelFilter::Off));
    }

    #[test]
    fn log_line_format_is_human_readable() {
        assert_eq!(
            format_log_line_at(
                "2026-08-23T12:31:04-07:00",
                Level::Warn,
                "Skipping stale scheduler boundary"
            ),
            "2026-08-23T12:31:04-07:00 WARN Skipping stale scheduler boundary\n"
        );
    }

    #[test]
    fn rotation_below_limit_leaves_both_generations_untouched() {
        let directory = test_directory();
        fs::create_dir_all(&directory).expect("failed to create test directory");

        let path = directory.join(LOG_FILE_NAME);
        let old_path = directory.join("watch-bells.old.log");
        fs::write(&path, vec![b'c'; MAX_LOG_SIZE as usize - 1])
            .expect("failed to create current test log");
        fs::write(&old_path, b"previous").expect("failed to create old test log");

        rotate_log_if_needed(&path);

        assert_eq!(
            fs::read(&path).expect("failed to read current log"),
            vec![b'c'; MAX_LOG_SIZE as usize - 1]
        );
        assert_eq!(
            fs::read(&old_path).expect("failed to read old log"),
            b"previous"
        );

        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn rotation_above_limit_replaces_previous_generation() {
        let directory = test_directory();
        fs::create_dir_all(&directory).expect("failed to create test directory");

        let path = directory.join(LOG_FILE_NAME);
        let old_path = directory.join("watch-bells.old.log");
        fs::write(&path, vec![b'c'; MAX_LOG_SIZE as usize + 1])
            .expect("failed to create current test log");
        fs::write(&old_path, b"previous").expect("failed to create old test log");

        rotate_log_if_needed(&path);

        assert!(!path.exists());
        assert_eq!(
            fs::read(&old_path).expect("failed to read rotated log"),
            vec![b'c'; MAX_LOG_SIZE as usize + 1]
        );

        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn live_rotation_replaces_previous_generation_after_a_record() {
        let directory = test_directory();
        fs::create_dir_all(&directory).expect("failed to create test directory");

        let path = directory.join(LOG_FILE_NAME);
        let old_path = directory.join("watch-bells.old.log");
        let current_generation = vec![b'c'; MAX_LOG_SIZE as usize - 1];
        fs::write(&path, &current_generation).expect("failed to create current test log");
        fs::write(&old_path, b"previous").expect("failed to create old test log");

        let mut state = Some(open_log_file_at(&path).expect("failed to open current test log"));
        write_persistent_record(&mut state, "live record\n").expect("live rotation should succeed");
        assert!(state.is_some());
        drop(state);

        let mut expected_old = current_generation;
        expected_old.extend_from_slice(b"live record\n");
        assert_eq!(
            fs::read(&old_path).expect("failed to read rotated log"),
            expected_old
        );
        assert_eq!(
            fs::read(&path).expect("failed to read new current log"),
            b""
        );

        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn unavailable_log_parent_is_best_effort() {
        let directory = test_directory();
        fs::create_dir_all(&directory).expect("failed to create test directory");

        let blocker = directory.join("not-a-directory");
        fs::write(&blocker, b"blocker").expect("failed to create blocking file");
        assert!(open_log_file_at(&blocker.join(LOG_FILE_NAME)).is_none());

        let _ = fs::remove_dir_all(directory);
    }
}
