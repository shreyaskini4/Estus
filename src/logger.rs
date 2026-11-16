use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

pub use crate::process_util::create_no_window_command;

static LOG_MUTEX: Mutex<()> = Mutex::new(());

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

/// Writes a timestamped log entry into `estus.log`, rotating at 1 MB to `estus.log.1`.
pub fn log(msg: &str) {
    write_log_to_file(&get_log_file_path(), msg, MAX_LOG_SIZE_BYTES);
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

/// Internal log writer supporting custom path and limit for testing and production.
pub fn write_log_to_file(log_path: &std::path::Path, msg: &str, max_bytes: u64) {
    let _guard = LOG_MUTEX.lock().unwrap_or_else(|e| e.into_inner());

    if let Some(parent) = log_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    // Check file size for 1 MB rotation
    if let Ok(metadata) = fs::metadata(log_path)
        && metadata.len() >= max_bytes {
            let backup_path = log_path.with_file_name("estus.log.1");
            if backup_path.exists() {
                let _ = fs::remove_file(&backup_path);
            }
            let _ = fs::rename(log_path, &backup_path);
        }

    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
        let now = chrono::Local::now().to_rfc3339();
        let _ = writeln!(file, "[{now}] {msg}");
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

    log(&msg);
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

    log(&msg);
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

    log(&msg);
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
    fn test_log_rotation() {
        let temp_dir = std::env::temp_dir().join(format!("estus_log_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let log_file = temp_dir.join("estus.log");
        let backup_file = temp_dir.join("estus.log.1");

        // Clean up previous runs
        let _ = fs::remove_file(&log_file);
        let _ = fs::remove_file(&backup_file);

        // Write entries under small limit (e.g. 200 bytes)
        let limit = 200u64;
        write_log_to_file(&log_file, "First entry", limit);
        assert!(log_file.exists());
        assert!(!backup_file.exists());

        // Fill past limit
        let big_msg = "A".repeat(300);
        write_log_to_file(&log_file, &big_msg, limit);

        // Next write should trigger rotation
        write_log_to_file(&log_file, "Entry after rotation", limit);

        assert!(log_file.exists(), "estus.log should exist after rotation");
        assert!(backup_file.exists(), "estus.log.1 should exist as backup");

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
