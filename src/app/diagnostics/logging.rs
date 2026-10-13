//! Per-user log storage, bounded rotation, and startup diagnostics.

use super::super::paths::migrate_legacy_directory;
use simplelog::{
    ColorChoice, CombinedLogger, ConfigBuilder, LevelFilter, SharedLogger, TermLogger,
    TerminalMode, WriteLogger,
};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;
const LOG_BACKUPS: usize = 3;
const MAX_CRASH_LOG_BYTES: u64 = 1024 * 1024;
const CRASH_LOG_BACKUPS: usize = 3;
const DUMP_BACKUPS: usize = 2;
const CRASH_SUMMARY_BYTES: u64 = 16 * 1024;

static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct StartupReport {
    pub log_dir: PathBuf,
    pub warnings: Vec<String>,
    pub previous_crash_summary: Option<String>,
}

impl StartupReport {
    fn warn(&mut self, message: impl Into<String>) {
        let message = message.into();
        report_to_stderr(&message);
        self.warnings.push(message);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(dead_code)]
enum Platform {
    Windows,
    MacOs,
    Linux,
    Other,
}

impl Platform {
    fn current() -> Self {
        #[cfg(target_os = "windows")]
        {
            Self::Windows
        }
        #[cfg(target_os = "macos")]
        {
            Self::MacOs
        }
        #[cfg(target_os = "linux")]
        {
            Self::Linux
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        {
            Self::Other
        }
    }
}

#[derive(Default)]
struct EnvironmentPaths {
    local_app_data: Option<PathBuf>,
    home: Option<PathBuf>,
    xdg_state_home: Option<PathBuf>,
}

fn environment_paths() -> EnvironmentPaths {
    EnvironmentPaths {
        local_app_data: env::var_os("LOCALAPPDATA").map(PathBuf::from),
        home: env::var_os("HOME").map(PathBuf::from),
        xdg_state_home: env::var_os("XDG_STATE_HOME").map(PathBuf::from),
    }
}

fn default_log_directory(platform: Platform, paths: &EnvironmentPaths) -> Option<PathBuf> {
    match platform {
        Platform::Windows => paths
            .local_app_data
            .as_ref()
            .filter(|base| !base.as_os_str().is_empty())
            .map(|base| base.join("StompRig").join("Logs")),
        Platform::MacOs => paths
            .home
            .as_ref()
            .filter(|home| home.is_absolute())
            .map(|home| home.join("Library").join("Logs").join("StompRig")),
        Platform::Linux => paths
            .xdg_state_home
            .as_ref()
            .filter(|path| path.is_absolute())
            .map(|base| base.join("stomprig"))
            .or_else(|| {
                paths
                    .home
                    .as_ref()
                    .filter(|home| home.is_absolute())
                    .map(|home| home.join(".local").join("state").join("stomprig"))
            }),
        Platform::Other => None,
    }
}

fn legacy_log_directory(platform: Platform, paths: &EnvironmentPaths) -> Option<PathBuf> {
    match platform {
        Platform::Windows => paths
            .local_app_data
            .as_ref()
            .filter(|base| !base.as_os_str().is_empty())
            .map(|base| base.join("DSP Rack").join("Logs")),
        Platform::MacOs => paths
            .home
            .as_ref()
            .filter(|home| home.is_absolute())
            .map(|home| home.join("Library").join("Logs").join("DSP Rack")),
        Platform::Linux => paths
            .xdg_state_home
            .as_ref()
            .filter(|path| path.is_absolute())
            .map(|base| base.join("dsp-rack"))
            .or_else(|| {
                paths
                    .home
                    .as_ref()
                    .filter(|home| home.is_absolute())
                    .map(|home| home.join(".local").join("state").join("dsp-rack"))
            }),
        Platform::Other => None,
    }
}

pub(super) fn init() -> StartupReport {
    let mut report = StartupReport::default();
    let paths = environment_paths();
    if Platform::current() == Platform::Linux
        && paths
            .xdg_state_home
            .as_ref()
            .is_some_and(|path| !path.is_absolute())
    {
        report.warn("XDG_STATE_HOME is not an absolute path; using ~/.local/state when available.");
    }

    let platform = Platform::current();
    let default_dir = match (
        default_log_directory(platform, &paths),
        legacy_log_directory(platform, &paths),
    ) {
        (Some(current), Some(legacy)) => Some(migrate_legacy_directory(&legacy, &current)),
        (current, _) => current,
    };
    let override_dir = env::var_os("STOMPRIG_LOG_DIR").map(PathBuf::from);
    let temp_dir = env::temp_dir().join("stomprig").join("logs");
    let (resolved_dir, path_warnings) =
        resolve_writable_log_directory(override_dir, default_dir, temp_dir);
    for warning in path_warnings {
        report.warn(warning);
    }
    let log_dir = LOG_DIR.get_or_init(|| resolved_dir.clone()).to_path_buf();
    report.log_dir = log_dir.clone();

    let active_run_marker = log_dir.join("active-session");
    let previous_run_was_active = active_run_marker.exists();
    report.previous_crash_summary = match previous_crash_summary(&log_dir) {
        Ok(summary) => summary,
        Err(error) => {
            report.warn(format!(
                "Could not read the previous crash summary from {}: {error}",
                log_dir.join("crash.log").display()
            ));
            None
        }
    };
    if report.previous_crash_summary.is_none() && previous_run_was_active {
        report.previous_crash_summary = Some(
            "StompRig did not shut down cleanly in the previous session. A native plug-in may have terminated the process."
                .to_owned(),
        );
    }
    if let Err(error) = fs::write(&active_run_marker, format!("pid={}\n", std::process::id())) {
        report.warn(format!(
            "Could not write the active-session marker {}: {error}; an unexpected exit may not be reported next time.",
            active_run_marker.display()
        ));
    }

    for (name, result) in [
        (
            "crash.log",
            rotate_artifact(&log_dir.join("crash.log"), CRASH_LOG_BACKUPS),
        ),
        ("crash.dmp", rotate_dump(&log_dir)),
    ] {
        if let Err(error) = result {
            report.warn(format!(
                "Could not rotate {name} in {}: {error}",
                log_dir.display()
            ));
        }
    }
    for (name, result) in [
        (
            "crash.log backups",
            trim_backups(
                &log_dir.join("crash.log"),
                CRASH_LOG_BACKUPS,
                MAX_CRASH_LOG_BYTES,
            ),
        ),
        (
            "crash.dmp backups",
            prune_backups(&log_dir.join("crash.dmp"), DUMP_BACKUPS),
        ),
    ] {
        if let Err(error) = result {
            report.warn(format!(
                "Could not enforce retention for {name} in {}: {error}",
                log_dir.display()
            ));
        }
    }

    let level = configured_log_level(&mut report);
    let mut config_builder = ConfigBuilder::new();
    config_builder
        .set_thread_level(LevelFilter::Error)
        .set_thread_mode(simplelog::ThreadLogMode::Both)
        .set_location_level(LevelFilter::Error)
        .set_time_format_rfc3339();
    if level == LevelFilter::Trace {
        // Full third-party UI trace logs can emit a record on every rendered frame.
        config_builder.add_filter_allow_str("stomprig::");
    }
    let config = config_builder.build();

    let rotating_file =
        match RotatingFile::open(log_dir.join("stomprig.log"), MAX_LOG_BYTES, LOG_BACKUPS) {
            Ok(file) => Some(file),
            Err(error) => {
                report.warn(format!(
                    "Could not create {}: {error}; continuing with console logging.",
                    log_dir.join("stomprig.log").display()
                ));
                None
            }
        };
    initialize_logger(level, config, rotating_file, &mut report);
    report
}

fn configured_log_level(report: &mut StartupReport) -> LevelFilter {
    match env::var("STOMPRIG_LOG") {
        Ok(value) => configured_log_level_for(&value, report),
        Err(env::VarError::NotPresent) => LevelFilter::Debug,
        Err(env::VarError::NotUnicode(_)) => {
            report.warn("STOMPRIG_LOG is not valid Unicode; using Debug.");
            LevelFilter::Debug
        }
    }
}

fn configured_log_level_for(value: &str, report: &mut StartupReport) -> LevelFilter {
    match parse_log_level(value) {
        Some(level) => level,
        None => {
            report.warn(format!(
                "Invalid STOMPRIG_LOG value {value:?}; expected Off, Error, Warn, Info, Debug, or Trace. Using Debug."
            ));
            LevelFilter::Debug
        }
    }
}

fn parse_log_level(value: &str) -> Option<LevelFilter> {
    value.parse::<LevelFilter>().ok()
}

fn initialize_logger(
    level: LevelFilter,
    config: simplelog::Config,
    rotating_file: Option<RotatingFile>,
    report: &mut StartupReport,
) {
    let console_logger = || {
        TermLogger::new(
            level,
            config.clone(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        )
    };
    let loggers: Vec<Box<dyn SharedLogger>> = match rotating_file {
        Some(file) => vec![
            console_logger(),
            WriteLogger::new(level, config.clone(), file),
        ],
        None => vec![console_logger()],
    };

    if let Err(error) = CombinedLogger::init(loggers) {
        report.warn(format!(
            "Could not initialize the combined logger: {error}; retrying with console logging."
        ));
        if let Err(console_error) = CombinedLogger::init(vec![console_logger()]) {
            report.warn(format!(
                "Could not initialize console logging: {console_error}. Startup will continue without the log crate logger."
            ));
        }
    }
}

fn resolve_writable_log_directory(
    override_dir: Option<PathBuf>,
    default_dir: Option<PathBuf>,
    fallback_dir: PathBuf,
) -> (PathBuf, Vec<String>) {
    let mut warnings = Vec::new();
    if let Some(override_dir) = override_dir {
        if override_dir.as_os_str().is_empty() {
            warnings
                .push("STOMPRIG_LOG_DIR is empty; trying the platform log directory.".to_owned());
        } else {
            match fs::create_dir_all(&override_dir) {
                Ok(()) => return (override_dir, warnings),
                Err(error) => warnings.push(format!(
                    "Could not create STOMPRIG_LOG_DIR {}: {error}; trying the platform log directory.",
                    override_dir.display()
                )),
            }
        }
    }

    if let Some(default_dir) = default_dir {
        match fs::create_dir_all(&default_dir) {
            Ok(()) => return (default_dir, warnings),
            Err(error) => warnings.push(format!(
                "Could not create the per-user log directory {}: {error}; trying temporary storage.",
                default_dir.display()
            )),
        }
    } else {
        warnings.push(
            "Could not resolve the platform per-user log directory; trying temporary storage."
                .to_owned(),
        );
    }

    match fs::create_dir_all(&fallback_dir) {
        Ok(()) => {
            warnings.push(format!(
                "Using temporary log storage at {}.",
                fallback_dir.display()
            ));
            (fallback_dir, warnings)
        }
        Err(error) => {
            warnings.push(format!(
                "Could not create temporary log storage {}: {error}; console logging will be used where available.",
                fallback_dir.display()
            ));
            (fallback_dir, warnings)
        }
    }
}

pub(super) fn log_dir() -> PathBuf {
    LOG_DIR
        .get()
        .cloned()
        .unwrap_or_else(|| env::temp_dir().join("stomprig").join("logs"))
}

pub(super) fn mark_clean_shutdown() {
    let marker = log_dir().join("active-session");
    if let Err(error) = fs::remove_file(&marker)
        && error.kind() != io::ErrorKind::NotFound
    {
        report_to_stderr(&format!(
            "Could not remove the active-session marker {}: {error}",
            marker.display()
        ));
    }
}

fn previous_crash_summary(directory: &Path) -> io::Result<Option<String>> {
    let crash_log = directory.join("crash.log");
    let mut file = match File::open(&crash_log) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return if directory.join("crash.dmp").exists() {
                Ok(Some(
                    "The previous session left a native crash dump (crash.dmp).".to_owned(),
                ))
            } else {
                Ok(None)
            };
        }
        Err(error) => return Err(error),
    };
    let length = file.metadata()?.len();
    let start = length.saturating_sub(CRASH_SUMMARY_BYTES);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::with_capacity((length - start) as usize);
    file.read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    let marker = text
        .lines()
        .rev()
        .find(|line| line.starts_with("PANIC on thread") || line.starts_with("NATIVE CRASH:"));
    let has_dump = directory.join("crash.dmp").exists();
    Ok(match marker {
        Some(line) if has_dump => Some(format!("{line} A crash dump was also captured.")),
        Some(line) => Some(line.to_owned()),
        None if has_dump => {
            Some("The previous session left a native crash dump (crash.dmp).".to_owned())
        }
        None => None,
    })
}

fn rotate_artifact(path: &Path, backup_count: usize) -> io::Result<()> {
    prune_backups(path, backup_count)?;
    if !path.exists() {
        return Ok(());
    }
    if backup_count == 0 {
        return fs::remove_file(path);
    }

    for index in (1..=backup_count).rev() {
        let source = if index == 1 {
            path.to_path_buf()
        } else {
            backup_path(path, index - 1)
        };
        let destination = backup_path(path, index);
        if !source.exists() {
            continue;
        }
        if destination.exists() {
            fs::remove_file(&destination)?;
        }
        fs::rename(source, destination)?;
    }
    Ok(())
}

fn prune_backups(path: &Path, backup_count: usize) -> io::Result<()> {
    let Some(directory) = path.parent() else {
        return Ok(());
    };
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "artifact".to_owned());
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        let Some(index) = file_name
            .strip_prefix(&format!("{name}."))
            .and_then(|suffix| suffix.parse::<usize>().ok())
        else {
            continue;
        };
        if index > backup_count {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn trim_backups(path: &Path, backup_count: usize, max_bytes: u64) -> io::Result<()> {
    for index in 1..=backup_count {
        let backup = backup_path(path, index);
        trim_file_to_tail(&backup, max_bytes)?;
    }
    Ok(())
}

fn trim_file_to_tail(path: &Path, max_bytes: u64) -> io::Result<()> {
    let mut file = match OpenOptions::new().read(true).write(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let size = file.metadata()?.len();
    if size <= max_bytes {
        return Ok(());
    }
    file.seek(SeekFrom::Start(size - max_bytes))?;
    let mut retained = Vec::with_capacity(max_bytes as usize);
    file.read_to_end(&mut retained)?;
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&retained)
}

fn backup_path(path: &Path, index: usize) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "artifact".to_owned());
    path.with_file_name(format!("{name}.{index}"))
}

pub(super) fn rotate_dump(directory: &Path) -> io::Result<()> {
    rotate_artifact(&directory.join("crash.dmp"), DUMP_BACKUPS)
}

pub(super) fn append_crash_log(directory: &Path, bytes: &[u8]) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    let path = directory.join("crash.log");
    let current_size = file_length_or_zero(&path)?;
    if current_size > 0 && current_size.saturating_add(bytes.len() as u64) > MAX_CRASH_LOG_BYTES {
        rotate_artifact(&path, CRASH_LOG_BACKUPS)?;
    }
    let current_size = file_length_or_zero(&path)?;
    let remaining = MAX_CRASH_LOG_BYTES.saturating_sub(current_size) as usize;
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(&bytes[..bytes.len().min(remaining)])?;
    file.sync_all()
}

fn file_length_or_zero(path: &Path) -> io::Result<u64> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.len()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(0),
        Err(error) => Err(error),
    }
}

pub(super) fn report_crash_write_failure(file_name: &str, error: &io::Error) {
    let message = format!("Could not write {file_name}: {error}");
    report_to_stderr(&message);
}

fn report_to_stderr(message: &str) {
    let mut stderr = io::stderr().lock();
    let _ = writeln!(stderr, "StompRig warning: {message}");
}

struct RotatingFile {
    path: PathBuf,
    file: Option<File>,
    size: u64,
    max_bytes: u64,
    backup_count: usize,
}

impl RotatingFile {
    fn open(path: PathBuf, max_bytes: u64, backup_count: usize) -> io::Result<Self> {
        if fs::metadata(&path).is_ok_and(|metadata| metadata.len() > 0) {
            rotate_artifact(&path, backup_count)?;
        }
        trim_backups(&path, backup_count, max_bytes)?;
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let size = file.metadata()?.len();
        Ok(Self {
            path,
            file: Some(file),
            size,
            max_bytes: max_bytes.max(1),
            backup_count,
        })
    }

    fn rotate(&mut self) -> io::Result<()> {
        if let Some(mut file) = self.file.take() {
            file.flush()?;
            drop(file);
        }
        rotate_artifact(&self.path, self.backup_count)?;
        self.file = Some(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?,
        );
        self.size = 0;
        Ok(())
    }
}

impl Write for RotatingFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        if self.size >= self.max_bytes
            || (self.size > 0
                && self.size.saturating_add(bytes.len() as u64) > self.max_bytes
                && bytes.len() as u64 <= self.max_bytes)
        {
            self.rotate()?;
        }

        let available = self.max_bytes.saturating_sub(self.size).max(1) as usize;
        let written = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("rotating log file is not open"))?
            .write(&bytes[..bytes.len().min(available)])?;
        self.size = self.size.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("rotating log file is not open"))?
            .flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_directory(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        env::temp_dir().join(format!(
            "stomprig-diagnostics-{name}-{}-{unique}",
            std::process::id()
        ))
    }

    fn write(path: &Path, contents: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[cfg(not(windows))]
    #[test]
    fn per_user_directories_follow_platform_conventions() {
        let paths = EnvironmentPaths {
            local_app_data: Some(PathBuf::from("C:/Users/test/AppData/Local")),
            home: Some(PathBuf::from("/Users/test")),
            xdg_state_home: Some(PathBuf::from("/state/test")),
        };
        assert_eq!(
            default_log_directory(Platform::Windows, &paths),
            Some(PathBuf::from("C:/Users/test/AppData/Local/StompRig/Logs"))
        );
        assert_eq!(
            default_log_directory(Platform::MacOs, &paths),
            Some(PathBuf::from("/Users/test/Library/Logs/StompRig"))
        );
        assert_eq!(
            default_log_directory(Platform::Linux, &paths),
            Some(PathBuf::from("/state/test/stomprig"))
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_per_user_directory_follows_platform_convention() {
        let paths = EnvironmentPaths {
            local_app_data: Some(PathBuf::from("C:/Users/test/AppData/Local")),
            ..EnvironmentPaths::default()
        };
        assert_eq!(
            default_log_directory(Platform::Windows, &paths),
            Some(PathBuf::from("C:/Users/test/AppData/Local/StompRig/Logs"))
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn linux_uses_home_state_directory_when_xdg_is_relative_or_missing() {
        let paths = EnvironmentPaths {
            home: Some(PathBuf::from("/home/test")),
            xdg_state_home: Some(PathBuf::from("relative")),
            ..EnvironmentPaths::default()
        };
        assert_eq!(
            default_log_directory(Platform::Linux, &paths),
            Some(PathBuf::from("/home/test/.local/state/stomprig"))
        );
    }

    #[test]
    fn empty_log_directory_override_is_rejected_before_default_fallback() {
        let root = test_directory("empty-override");
        let default_path = root.join("user-state").join("stomprig");

        let (chosen, warnings) = resolve_writable_log_directory(
            Some(PathBuf::new()),
            Some(default_path.clone()),
            root.join("temporary"),
        );

        assert_eq!(chosen, default_path);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("STOMPRIG_LOG_DIR is empty"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_override_falls_back_to_the_platform_directory() {
        let root = test_directory("override-fallback");
        let blocking_file = root.join("not-a-directory");
        write(&blocking_file, b"block");
        let override_path = blocking_file.join("logs");
        let default_path = root.join("user-state").join("stomprig");
        let fallback_path = root.join("temporary");

        let (chosen, warnings) = resolve_writable_log_directory(
            Some(override_path),
            Some(default_path.clone()),
            fallback_path,
        );

        assert_eq!(chosen, default_path);
        assert_eq!(warnings.len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn writable_override_takes_precedence_over_the_platform_directory() {
        let root = test_directory("override");
        let override_path = root.join("custom-logs");
        let default_path = root.join("user-state").join("stomprig");

        let (chosen, warnings) = resolve_writable_log_directory(
            Some(override_path.clone()),
            Some(default_path),
            root.join("temporary"),
        );

        assert_eq!(chosen, override_path);
        assert!(warnings.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unavailable_user_directory_falls_back_to_temporary_storage() {
        let root = test_directory("temporary-fallback");
        let blocking_file = root.join("not-a-directory");
        write(&blocking_file, b"block");
        let default_path = blocking_file.join("state");
        let fallback_path = root.join("temporary").join("logs");

        let (chosen, warnings) =
            resolve_writable_log_directory(None, Some(default_path), fallback_path.clone());

        assert_eq!(chosen, fallback_path);
        assert!(warnings.len() >= 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rotation_keeps_only_the_configured_number_of_backups() {
        let root = test_directory("retention");
        let active = root.join("crash.dmp");
        for index in 0..5 {
            write(&active, format!("dump-{index}").as_bytes());
            rotate_artifact(&active, DUMP_BACKUPS).unwrap();
        }

        assert!(!active.exists());
        assert_eq!(fs::read(backup_path(&active, 1)).unwrap(), b"dump-4");
        assert_eq!(fs::read(backup_path(&active, 2)).unwrap(), b"dump-3");
        assert!(!backup_path(&active, 3).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn active_log_rotates_at_startup_and_at_the_size_limit() {
        let root = test_directory("log-rotation");
        let path = root.join("stomprig.log");
        write(&path, b"previous session");
        let mut log_file = RotatingFile::open(path.clone(), 8, 2).unwrap();
        assert_eq!(fs::read(backup_path(&path, 1)).unwrap(), b" session");

        log_file.write_all(b"12345678").unwrap();
        log_file.write_all(b"next").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"next");
        assert_eq!(fs::read(backup_path(&path, 1)).unwrap(), b"12345678");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn crash_log_appends_stay_bounded_and_rotate_through_retention() {
        let root = test_directory("crash-retention");
        let chunk = vec![b'x'; MAX_CRASH_LOG_BYTES as usize];
        for _ in 0..5 {
            append_crash_log(&root, &chunk).unwrap();
        }

        let active = root.join("crash.log");
        assert_eq!(fs::metadata(&active).unwrap().len(), MAX_CRASH_LOG_BYTES);
        assert_eq!(
            fs::metadata(backup_path(&active, 1)).unwrap().len(),
            MAX_CRASH_LOG_BYTES
        );
        assert_eq!(
            fs::metadata(backup_path(&active, CRASH_LOG_BACKUPS))
                .unwrap()
                .len(),
            MAX_CRASH_LOG_BYTES
        );
        assert!(!backup_path(&active, CRASH_LOG_BACKUPS + 1).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn previous_crash_summary_is_read_before_the_log_is_rotated() {
        let root = test_directory("previous-crash");
        let path = root.join("crash.log");
        write(&path, b"PANIC on thread 'audio'\nstack trace\n");
        write(&root.join("crash.dmp"), b"dump");

        let summary = previous_crash_summary(&root).unwrap().unwrap();
        rotate_artifact(&path, CRASH_LOG_BACKUPS).unwrap();

        assert!(summary.contains("PANIC on thread 'audio'"));
        assert!(summary.contains("crash dump was also captured"));
        assert!(!path.exists());
        assert!(backup_path(&path, 1).exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_log_level_is_rejected_for_startup_reporting() {
        assert_eq!(parse_log_level("Trace"), Some(LevelFilter::Trace));
        assert_eq!(parse_log_level("chatty"), None);

        let mut report = StartupReport::default();
        assert_eq!(
            configured_log_level_for("chatty", &mut report),
            LevelFilter::Debug
        );
        assert_eq!(report.warnings.len(), 1);
        assert!(report.warnings[0].contains("Invalid STOMPRIG_LOG value"));
    }
}
