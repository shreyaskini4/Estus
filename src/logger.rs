use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

pub use crate::process_util::create_no_window_command;

static LOG_MUTEX: Mutex<()> = Mutex::new(());

/// Log level enumeration defining the severity/detail of log entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::Debug => write!(f, "DEBUG"),
            LogLevel::Info => write!(f, "INFO"),
            LogLevel::Warn => write!(f, "WARN"),
            LogLevel::Error => write!(f, "ERROR"),
        }
    }
}

// In release builds, default log level is Info (so debug lines are ignored unless verbose logging is enabled).
// In debug builds, debug lines can be logged by default.
static VERBOSE_LOGGING: AtomicBool = AtomicBool::new(cfg!(debug_assertions));

/// Enables or disables verbose debug logging.
pub fn set_verbose_logging(verbose: bool) {
    VERBOSE_LOGGING.store(verbose, Ordering::SeqCst);
}

/// Returns whether verbose debug logging is currently enabled.
pub fn is_verbose_logging() -> bool {
    VERBOSE_LOGGING.load(Ordering::SeqCst)
}

/// Determines whether an entry at `level` should be logged given the active configuration.
pub fn should_log(level: LogLevel) -> bool {
    match level {
        LogLevel::Debug => is_verbose_logging(),
        LogLevel::Info | LogLevel::Warn | LogLevel::Error => true,
    }
}

/// Maximum size in bytes before log file rotation (1 MB = 1,000,000 bytes).
pub const MAX_LOG_SIZE_BYTES: u64 = 1_000_000;

/// Returns the application data directory where log files are stored.
pub fn get_log_dir() -> PathBuf {
    if let Some(proj) = directories::ProjectDirs::from("com", "estus", "Estus") {
        proj.data_local_dir().to_path_buf()
    } else {
        PathBuf::from(".")
    }
}

/// Returns the primary log file path (`<data dir>/estus.log`).
pub fn get_log_file_path() -> PathBuf {
    get_log_dir().join("estus.log")
}

/// Helper function to truncate a string to at most `max_chars` characters safely on UTF-8 boundaries.
pub fn truncate_chars(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

/// Writes a timestamped log entry at `Info` level for backwards compatibility.
pub fn log(msg: &str) {
    log_info(msg);
}

/// Writes a timestamped debug log entry (`LogLevel::Debug`).
pub fn log_debug(msg: &str) {
    log_with_level(LogLevel::Debug, msg);
}

/// Writes a timestamped info log entry (`LogLevel::Info`).
pub fn log_info(msg: &str) {
    log_with_level(LogLevel::Info, msg);
}

/// Writes a timestamped warning log entry (`LogLevel::Warn`).
pub fn log_warn(msg: &str) {
    log_with_level(LogLevel::Warn, msg);
}

/// Writes a timestamped error log entry (`LogLevel::Error`).
pub fn log_error(msg: &str) {
    log_with_level(LogLevel::Error, msg);
}

/// Writes a timestamped log entry at the specified log level to the primary log file.
pub fn log_with_level(level: LogLevel, msg: &str) {
    write_log_with_level_to_file(&get_log_file_path(), level, msg, MAX_LOG_SIZE_BYTES);
}

/// Formats a panic hook info into a descriptive string suitable for logging.
pub fn format_panic_info(panic_info: &std::panic::PanicHookInfo) -> String {
    let location = panic_info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "unknown location".to_string());

    let payload = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "Unknown panic payload".to_string()
    };

    format!("PANIC occurred at [{location}]: {payload}\nDetails: {panic_info}")
}

/// Rotates log files maintaining at most 2 old backup files:
/// When `estus.log` reaches limit:
/// - If `estus.log.2` exists, remove it.
/// - If `estus.log.1` exists, rename `estus.log.1` to `estus.log.2`.
/// - Rename `estus.log` to `estus.log.1`.
fn rotate_log_files(log_path: &std::path::Path) {
    let file_name = log_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("estus.log");
    let backup_1 = log_path.with_file_name(format!("{file_name}.1"));
    let backup_2 = log_path.with_file_name(format!("{file_name}.2"));

    if backup_2.exists() {
        let _ = fs::remove_file(&backup_2);
    }
    if backup_1.exists() {
        let _ = fs::rename(&backup_1, &backup_2);
    }
    let _ = fs::rename(log_path, &backup_1);
}

/// Internal log writer supporting custom path and limit for testing and production.
pub fn write_log_to_file(log_path: &std::path::Path, msg: &str, max_bytes: u64) {
    write_log_with_level_to_file(log_path, LogLevel::Info, msg, max_bytes);
}

/// Internal log writer supporting custom path, level, and limit for testing and production.
pub fn write_log_with_level_to_file(
    log_path: &std::path::Path,
    level: LogLevel,
    msg: &str,
    max_bytes: u64,
) {
    if !should_log(level) {
        return;
    }

    let _guard = LOG_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(parent) = log_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    // Check file size for 2-file rotation
    if let Ok(metadata) = fs::metadata(log_path)
        && metadata.len() >= max_bytes
    {
        rotate_log_files(log_path);
    }

    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
        let now = chrono::Local::now().to_rfc3339();
        let _ = writeln!(file, "[{now}] [{level}] {msg}");
    }
}

/// Logs a candidate verification attempt (`<candidate> --version`).
pub fn log_verification_attempt(
    candidate: &str,
    exit_code: Option<i32>,
    stdout: &str,
    stderr: &str,
    spawn_error: Option<&std::io::Error>,
) {
    let stdout_trunc = truncate_chars(stdout, 500);
    let stderr_trunc = truncate_chars(stderr, 500);

    let msg = match spawn_error {
        Some(err) => {
            format!(
                "VERIFY CANDIDATE: \"{candidate}\" -> Spawn failed: {err} (raw OS code: {:?})",
                err.raw_os_error()
            )
        }
        None => {
            let code_str = exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "none".to_string());
            format!(
                "VERIFY CANDIDATE: \"{candidate}\" -> Exit code: {code_str}\n  stdout[:500]: {stdout_trunc:?}\n  stderr[:500]: {stderr_trunc:?}"
            )
        }
    };

    log_debug(&msg);
}

/// Logs an execution query of smartctl (e.g. querying device or scanning).
pub fn log_smartctl_execution(
    path_tried: &str,
    args: &[&str],
    exit_code: Option<i32>,
    stdout: &str,
    stderr: &str,
    spawn_error: Option<&std::io::Error>,
) {
    let stdout_trunc = truncate_chars(stdout, 500);
    let stderr_trunc = truncate_chars(stderr, 500);

    let msg = match spawn_error {
        Some(err) => {
            format!(
                "EXECUTE: \"{path_tried}\" args: {args:?} -> Spawn failed: {err} (raw OS code: {:?})",
                err.raw_os_error()
            )
        }
        None => {
            let code_str = exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "none".to_string());
            format!(
                "EXECUTE: \"{path_tried}\" args: {args:?} -> Exit code: {code_str}\n  stdout[:500]: {stdout_trunc:?}\n  stderr[:500]: {stderr_trunc:?}"
            )
        }
    };

    log_debug(&msg);
}

/// Logs detection failures with path, device, output, and error.
pub fn log_detection_failure(
    path_tried: &str,
    device_tried: &str,
    stdout: &str,
    stderr: &str,
    err_display: &str,
) {
    let stdout_trunc = truncate_chars(stdout, 500);
    let stderr_trunc = truncate_chars(stderr, 500);

    let msg = format!(
        "DETECTION FAILURE: path: \"{path_tried}\", device: \"{device_tried}\", error: {err_display}\n  stdout[:500]: {stdout_trunc:?}\n  stderr[:500]: {stderr_trunc:?}"
    );

    log_warn(&msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_chars() {
        assert_eq!(truncate_chars("hello world", 5), "hello");
        assert_eq!(truncate_chars("hello", 10), "hello");
        assert_eq!(truncate_chars("🦀🦀🦀🦀", 2), "🦀🦀");
    }

    #[test]
    fn test_log_rotation_2_files() {
        let temp_dir = std::env::temp_dir().join(format!("estus_log_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let log_file = temp_dir.join("estus.log");
        let backup_file_1 = temp_dir.join("estus.log.1");
        let backup_file_2 = temp_dir.join("estus.log.2");

        // Clean up previous runs
        let _ = fs::remove_file(&log_file);
        let _ = fs::remove_file(&backup_file_1);
        let _ = fs::remove_file(&backup_file_2);

        let limit = 200u64;

        // 1. Initial write under limit
        write_log_to_file(&log_file, "First entry", limit);
        assert!(log_file.exists());
        assert!(!backup_file_1.exists());
        assert!(!backup_file_2.exists());

        // Fill past limit
        let big_msg = "A".repeat(300);
        write_log_to_file(&log_file, &big_msg, limit);

        // 2. Next write should trigger first rotation -> estus.log.1 created
        write_log_to_file(&log_file, "Entry after first rotation", limit);
        assert!(log_file.exists(), "estus.log should exist");
        assert!(backup_file_1.exists(), "estus.log.1 should exist");
        assert!(!backup_file_2.exists(), "estus.log.2 should not yet exist");

        // Verify estus.log.1 contains the previous content
        let content_1 = fs::read_to_string(&backup_file_1).unwrap();
        assert!(content_1.contains(&big_msg));

        // Fill current estus.log past limit again
        let big_msg_2 = "B".repeat(300);
        write_log_to_file(&log_file, &big_msg_2, limit);

        // 3. Next write should trigger second rotation:
        // estus.log.1 -> estus.log.2, estus.log -> estus.log.1
        write_log_to_file(&log_file, "Entry after second rotation", limit);
        assert!(log_file.exists(), "estus.log should exist");
        assert!(backup_file_1.exists(), "estus.log.1 should exist");
        assert!(backup_file_2.exists(), "estus.log.2 should exist");

        let content_2 = fs::read_to_string(&backup_file_2).unwrap();
        assert!(content_2.contains(&big_msg), "estus.log.2 should have oldest content");
        let content_1_after = fs::read_to_string(&backup_file_1).unwrap();
        assert!(content_1_after.contains(&big_msg_2), "estus.log.1 should have intermediate content");

        // Fill current estus.log past limit third time
        let big_msg_3 = "C".repeat(300);
        write_log_to_file(&log_file, &big_msg_3, limit);

        // 4. Next write should trigger third rotation:
        // old estus.log.2 removed, estus.log.1 -> estus.log.2, estus.log -> estus.log.1
        write_log_to_file(&log_file, "Entry after third rotation", limit);
        assert!(log_file.exists());
        assert!(backup_file_1.exists());
        assert!(backup_file_2.exists());

        let content_2_third = fs::read_to_string(&backup_file_2).unwrap();
        assert!(content_2_third.contains(&big_msg_2), "estus.log.2 should have received previous estus.log.1");
        assert!(!content_2_third.contains(&big_msg), "original oldest entry should have been pruned");

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_log_levels_filtering() {
        let temp_dir = std::env::temp_dir().join(format!("estus_log_levels_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let log_file = temp_dir.join("test_levels.log");
        let _ = fs::remove_file(&log_file);

        let limit = 10_000u64;

        // Test with verbose logging disabled
        set_verbose_logging(false);
        assert!(!is_verbose_logging());

        write_log_with_level_to_file(&log_file, LogLevel::Debug, "debug message hidden", limit);
        write_log_with_level_to_file(&log_file, LogLevel::Info, "info message visible", limit);
        write_log_with_level_to_file(&log_file, LogLevel::Warn, "warn message visible", limit);
        write_log_with_level_to_file(&log_file, LogLevel::Error, "error message visible", limit);

        let contents = fs::read_to_string(&log_file).unwrap();
        assert!(!contents.contains("debug message hidden"));
        assert!(contents.contains("[INFO] info message visible"));
        assert!(contents.contains("[WARN] warn message visible"));
        assert!(contents.contains("[ERROR] error message visible"));

        // Now enable verbose logging
        set_verbose_logging(true);
        assert!(is_verbose_logging());

        write_log_with_level_to_file(&log_file, LogLevel::Debug, "debug message now visible", limit);
        let contents2 = fs::read_to_string(&log_file).unwrap();
        assert!(contents2.contains("[DEBUG] debug message now visible"));

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_panic_formatting() {
        let prev_hook = std::panic::take_hook();
        let captured = std::sync::Arc::new(Mutex::new(None));
        let captured_clone = std::sync::Arc::clone(&captured);

        std::panic::set_hook(Box::new(move |info| {
            let formatted = format_panic_info(info);
            *captured_clone.lock().unwrap() = Some(formatted);
        }));

        let _ = std::panic::catch_unwind(|| {
            panic!("test panic for logger verification");
        });

        std::panic::set_hook(prev_hook);

        let msg = captured.lock().unwrap().take().expect("panic hook did not run");
        assert!(msg.contains("test panic for logger verification"));
        assert!(msg.contains("PANIC occurred at"));
    }
}
