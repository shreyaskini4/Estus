use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Global thread-safe cache for the resolved smartctl executable path.
static CACHED_SMARTCTL_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmartSnapshot {
    pub ts: i64,
    pub data_units_written: u64,
    pub data_units_read: u64,
    pub percent_used: u8,
    pub available_spare: u8,
    pub power_on_hours: u64,
    pub power_cycles: u64,
}

impl SmartSnapshot {
    pub fn host_bytes_written(&self) -> u64 {
        self.data_units_written.saturating_mul(512_000)
    }

    pub fn host_bytes_read(&self) -> u64 {
        self.data_units_read.saturating_mul(512_000)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmartError {
    SmartctlNotFound(String),
    PermissionDenied(String),
    DeviceNotFound(String),
    SpawnError(String),
    ParseError(String),
    CommandFailed(String),
}

impl std::fmt::Display for SmartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SmartctlNotFound(msg) => write!(
                f,
                "smartctl not found: {msg}. Install smartmontools (`winget install smartmontools`) or specify the path."
            ),
            Self::PermissionDenied(msg) => write!(
                f,
                "Permission denied accessing NVMe SMART data: {msg}. Please run Estus as Administrator."
            ),
            Self::DeviceNotFound(msg) => write!(
                f,
                "NVMe device not found: {msg}."
            ),
            Self::SpawnError(msg) => write!(
                f,
                "Failed to spawn smartctl process: {msg}"
            ),
            Self::ParseError(msg) => write!(f, "Failed to parse smartctl output: {msg}"),
            Self::CommandFailed(msg) => write!(f, "smartctl command failed: {msg}"),
        }
    }
}

impl std::error::Error for SmartError {}

/// Resolves the smartctl path in the following order:
/// 1. User-configured path from settings (if valid file)
/// 2. `where smartctl` / PATH lookup
/// 3. `C:\Program Files\smartmontools\bin\smartctl.exe`
/// 4. `C:\Program Files (x86)\smartmontools\bin\smartctl.exe`
pub fn resolve_smartctl_path(custom_path: Option<&str>) -> Option<PathBuf> {
    // 1. User-configured path
    if let Some(custom) = custom_path {
        let trimmed = custom.trim();
        if !trimmed.is_empty() {
            let p = PathBuf::from(trimmed);
            if p.is_file() {
                return Some(p);
            }
        }
    }

    // 2. PATH lookup
    if let Some(p) = find_in_path("smartctl") {
        return Some(p);
    }
    #[cfg(windows)]
    if let Some(p) = find_with_where_cmd("smartctl") {
        return Some(p);
    }

    // 3. C:\Program Files\smartmontools\bin\smartctl.exe
    let p3 = PathBuf::from(r"C:\Program Files\smartmontools\bin\smartctl.exe");
    if p3.is_file() {
        return Some(p3);
    }

    // 4. C:\Program Files (x86)\smartmontools\bin\smartctl.exe
    let p4 = PathBuf::from(r"C:\Program Files (x86)\smartmontools\bin\smartctl.exe");
    if p4.is_file() {
        return Some(p4);
    }

    None
}

/// Retrieves the cached smartctl path or resolves and caches it.
pub fn get_or_resolve_smartctl_path(custom_path: Option<&str>) -> Option<PathBuf> {
    if let Ok(guard) = CACHED_SMARTCTL_PATH.lock()
        && let Some(ref cached) = *guard {
            // If no custom path is given or custom matches cached, verify file still exists
            if (custom_path.is_none() || custom_path.map(Path::new) == Some(cached.as_path()))
                && cached.is_file() {
                    return Some(cached.clone());
                }
        }

    let resolved = resolve_smartctl_path(custom_path);
    if let Ok(mut guard) = CACHED_SMARTCTL_PATH.lock() {
        *guard = resolved.clone();
    }
    resolved
}

/// Explicitly sets or updates the cached smartctl path.
pub fn set_cached_smartctl_path(path: Option<PathBuf>) {
    if let Ok(mut guard) = CACHED_SMARTCTL_PATH.lock() {
        *guard = path;
    }
}

/// Clears the cached smartctl path.
pub fn clear_cached_smartctl_path() {
    set_cached_smartctl_path(None);
}

fn find_in_path(binary_name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        #[cfg(windows)]
        {
            let exe = dir.join(format!("{binary_name}.exe"));
            if exe.is_file() {
                return Some(exe);
            }
        }
        let direct = dir.join(binary_name);
        if direct.is_file() {
            return Some(direct);
        }
    }
    None
}

#[cfg(windows)]
fn find_with_where_cmd(binary_name: &str) -> Option<PathBuf> {
    let mut cmd = std::process::Command::new("where.exe");
    cmd.arg(binary_name);
    cmd.creation_flags(CREATE_NO_WINDOW);
    if let Ok(output) = cmd.output()
        && output.status.success() {
            let out_str = String::from_utf8_lossy(&output.stdout);
            for line in out_str.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    let p = PathBuf::from(trimmed);
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
        }
    None
}

#[derive(Debug, Deserialize)]
struct NvmeHealthInfo {
    data_units_written: u64,
    data_units_read: u64,
    #[serde(alias = "percent_used")]
    percentage_used: u8,
    available_spare: u8,
    power_on_hours: u64,
    power_cycles: u64,
}

pub fn is_permission_denied_str(s: &str) -> bool {
    let lower = s.to_lowercase();
    lower.contains("permission denied")
        || lower.contains("access is denied")
        || lower.contains("access denied")
        || lower.contains("operation not permitted")
        || lower.contains("administrator")
        || lower.contains("admin privileges")
        || lower.contains("elevation required")
        || lower.contains("privilege")
}

pub fn is_device_not_found_str(s: &str) -> bool {
    let lower = s.to_lowercase();
    lower.contains("open device failed")
        || lower.contains("unable to open")
        || lower.contains("cannot open")
        || lower.contains("no such device")
        || lower.contains("device not found")
        || lower.contains("failed: \\\\.\\")
        || lower.contains("no nvme device")
        || lower.contains("does not exist")
}

/// Inspects `smartctl.messages` in parsed JSON for permission or device errors.
fn check_smartctl_messages(val: &serde_json::Value) -> Option<SmartError> {
    let messages = val.get("smartctl").and_then(|s| s.get("messages")).and_then(|m| m.as_array())?;
    for msg in messages {
        if let Some(msg_str) = msg.get("string").and_then(|s| s.as_str()) {
            if is_permission_denied_str(msg_str) {
                return Some(SmartError::PermissionDenied(msg_str.to_string()));
            }
            if is_device_not_found_str(msg_str) {
                return Some(SmartError::DeviceNotFound(msg_str.to_string()));
            }
        }
    }
    None
}

/// Parses smartctl JSON output.
/// Does not rely on exit status. Always parses stdout as JSON first.
/// If `smartctl.messages` in the JSON contains a permission or device error,
/// returns `PermissionDenied` or `DeviceNotFound` with that message.
pub fn parse_smartctl_output(json_str: &str, ts: i64) -> Result<SmartSnapshot, SmartError> {
    let trimmed = json_str.trim();
    if trimmed.is_empty() {
        return Err(SmartError::ParseError("Empty smartctl output".to_string()));
    }

    let val: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(e) => {
            if is_permission_denied_str(trimmed) {
                return Err(SmartError::PermissionDenied(trimmed.to_string()));
            }
            if is_device_not_found_str(trimmed) {
                return Err(SmartError::DeviceNotFound(trimmed.to_string()));
            }
            return Err(SmartError::ParseError(format!("Invalid JSON: {e}")));
        }
    };

    // 1. Inspect messages in JSON for explicit permission or device errors
    if let Some(err) = check_smartctl_messages(&val) {
        return Err(err);
    }

    // 2. Case A: Root JSON contains "nvme_smart_health_information_log"
    if let Some(health_val) = val.get("nvme_smart_health_information_log") {
        return match serde_json::from_value::<NvmeHealthInfo>(health_val.clone()) {
            Ok(health) => Ok(SmartSnapshot {
                ts,
                data_units_written: health.data_units_written,
                data_units_read: health.data_units_read,
                percent_used: health.percentage_used,
                available_spare: health.available_spare,
                power_on_hours: health.power_on_hours,
                power_cycles: health.power_cycles,
            }),
            Err(e) => Err(SmartError::ParseError(format!(
                "Invalid 'nvme_smart_health_information_log': {e}"
            ))),
        };
    }

    // 3. Case B: The JSON is directly the health information object
    if let Ok(health) = serde_json::from_value::<NvmeHealthInfo>(val.clone()) {
        return Ok(SmartSnapshot {
            ts,
            data_units_written: health.data_units_written,
            data_units_read: health.data_units_read,
            percent_used: health.percentage_used,
            available_spare: health.available_spare,
            power_on_hours: health.power_on_hours,
            power_cycles: health.power_cycles,
        });
    }

    // 4. Case C: If no health log, check if any messages exist in smartctl
    if let Some(messages) = val.get("smartctl").and_then(|s| s.get("messages")).and_then(|m| m.as_array())
        && let Some(first_msg) = messages.iter().find_map(|m| m.get("string").and_then(|s| s.as_str())) {
            if is_permission_denied_str(first_msg) {
                return Err(SmartError::PermissionDenied(first_msg.to_string()));
            }
            if is_device_not_found_str(first_msg) {
                return Err(SmartError::DeviceNotFound(first_msg.to_string()));
            }
            return Err(SmartError::CommandFailed(first_msg.to_string()));
        }

    if is_permission_denied_str(trimmed) {
        return Err(SmartError::PermissionDenied(trimmed.to_string()));
    }

    Err(SmartError::ParseError(
        "Missing 'nvme_smart_health_information_log' in smartctl JSON output".to_string(),
    ))
}

/// Runs `smartctl --scan -j` and returns the first NVMe device name found (e.g. "/dev/sda").
pub fn scan_for_nvme_device(smartctl_exe: &Path) -> Result<Option<String>, SmartError> {
    let mut cmd = std::process::Command::new(smartctl_exe);
    cmd.args(["--scan", "-j"]);

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let output = match cmd.output() {
        Ok(out) => out,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(SmartError::SmartctlNotFound(format!(
                "Executable not found at {}",
                smartctl_exe.display()
            )));
        }
        Err(err) => {
            return Err(SmartError::SpawnError(format!(
                "Failed to spawn {}: {} (OS code: {:?})",
                smartctl_exe.display(),
                err,
                err.raw_os_error()
            )));
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&stdout)
        && let Some(devices) = val.get("devices").and_then(|d| d.as_array()) {
            for dev in devices {
                let dev_type = dev.get("type").and_then(|t| t.as_str()).unwrap_or("");
                let protocol = dev.get("protocol").and_then(|p| p.as_str()).unwrap_or("");
                let name = dev.get("name").and_then(|n| n.as_str()).unwrap_or("");

                if (dev_type.eq_ignore_ascii_case("nvme")
                    || protocol.eq_ignore_ascii_case("nvme")
                    || name.to_lowercase().contains("nvme"))
                    && !name.is_empty() {
                        return Ok(Some(name.to_string()));
                    }
            }
        }

    Ok(None)
}

/// Logs detection failures with path tried, device, raw output, and error details.
pub fn log_detection_failure(
    path_tried: &str,
    device_tried: &str,
    stdout: &str,
    stderr: &str,
    err: &SmartError,
) {
    eprintln!(
        "[smartctl detection failure] Path: '{path_tried}', Device: '{device_tried}', Error: {err}"
    );

    let log_path = match directories::ProjectDirs::from("com", "estus", "Estus") {
        Some(proj) => {
            let dir = proj.data_local_dir();
            let _ = std::fs::create_dir_all(dir);
            dir.join("detection.log")
        }
        None => PathBuf::from("detection.log"),
    };

    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let now = chrono::Local::now().to_rfc3339();
        let _ = writeln!(
            file,
            "[{now}] DETECTION FAILURE\n  Path: {path_tried}\n  Device: {device_tried}\n  Error: {err}\n  Stdout:\n{stdout}\n  Stderr:\n{stderr}\n----------------------------------------\n"
        );
    }
}

/// Executes smartctl query for a specific device, parsing stdout as JSON.
fn query_device(smartctl_exe: &Path, device: &str) -> Result<SmartSnapshot, SmartError> {
    let mut cmd = std::process::Command::new(smartctl_exe);
    cmd.args(["-j", "-a", device]);

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let output = match cmd.output() {
        Ok(out) => out,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(SmartError::SmartctlNotFound(format!(
                "Executable not found at {}",
                smartctl_exe.display()
            )));
        }
        Err(err) => {
            return Err(SmartError::SpawnError(format!(
                "Failed to spawn {}: {} (OS code: {:?})",
                smartctl_exe.display(),
                err,
                err.raw_os_error()
            )));
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let ts = chrono::Utc::now().timestamp();

    // Do not rely on exit status. Always try to parse stdout as JSON first.
    if !stdout.trim().is_empty() {
        match parse_smartctl_output(&stdout, ts) {
            Ok(snapshot) => return Ok(snapshot),
            Err(err) => {
                log_detection_failure(
                    &smartctl_exe.display().to_string(),
                    device,
                    &stdout,
                    &stderr,
                    &err,
                );
                return Err(err);
            }
        }
    }

    // Stdout was empty
    let err = if is_permission_denied_str(&stderr) {
        SmartError::PermissionDenied(stderr.trim().to_string())
    } else if is_device_not_found_str(&stderr) {
        SmartError::DeviceNotFound(stderr.trim().to_string())
    } else if !stderr.trim().is_empty() {
        SmartError::CommandFailed(stderr.trim().to_string())
    } else {
        SmartError::ParseError("smartctl returned empty output".to_string())
    };

    log_detection_failure(
        &smartctl_exe.display().to_string(),
        device,
        &stdout,
        &stderr,
        &err,
    );
    Err(err)
}

/// Runs smartctl with custom path and device configuration.
/// If device is None and "/dev/nvme0" fails, automatically scans for the first NVMe device.
/// Returns `(SmartSnapshot, resolved_path, active_device)`.
pub fn run_smartctl_with_config(
    custom_path: Option<&str>,
    device: Option<&str>,
) -> Result<(SmartSnapshot, String, String), SmartError> {
    let smartctl_exe = match get_or_resolve_smartctl_path(custom_path) {
        Some(p) => p,
        None => {
            let err = SmartError::SmartctlNotFound(
                "smartctl was not found in settings, PATH, C:\\Program Files\\smartmontools\\bin, or C:\\Program Files (x86)\\smartmontools\\bin".to_string(),
            );
            log_detection_failure("None", device.unwrap_or("auto"), "", "", &err);
            return Err(err);
        }
    };

    let resolved_path_str = smartctl_exe.display().to_string();

    // If an explicit device was requested, query it directly
    if let Some(dev) = device {
        let snapshot = query_device(&smartctl_exe, dev)?;
        return Ok((snapshot, resolved_path_str, dev.to_string()));
    }

    // Default flow: Try "/dev/nvme0" first
    let default_dev = "/dev/nvme0";
    match query_device(&smartctl_exe, default_dev) {
        Ok(snapshot) => Ok((snapshot, resolved_path_str, default_dev.to_string())),
        Err(err1) => {
            // Requirement 4: If /dev/nvme0 fails, run `smartctl --scan -j`, and use the first NVMe device found.
            eprintln!("/dev/nvme0 query failed: {err1}. Scanning for NVMe devices with `smartctl --scan -j`...");
            match scan_for_nvme_device(&smartctl_exe) {
                Ok(Some(scanned_dev)) if scanned_dev != default_dev => {
                    eprintln!("Found candidate NVMe device from scan: {scanned_dev}. Querying...");
                    match query_device(&smartctl_exe, &scanned_dev) {
                        Ok(snapshot) => Ok((snapshot, resolved_path_str, scanned_dev)),
                        Err(err2) => {
                            log_detection_failure(
                                &resolved_path_str,
                                &scanned_dev,
                                "",
                                "",
                                &err2,
                            );
                            Err(err2)
                        }
                    }
                }
                _ => Err(err1),
            }
        }
    }
}

/// Convenience function matching the original signature.
pub fn run_smartctl(device: Option<&str>) -> Result<SmartSnapshot, SmartError> {
    run_smartctl_with_config(None, device).map(|(s, _, _)| s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_nvme_json() {
        let json = r#"{
          "json_format_version": [1, 0],
          "smartctl": {
            "version": [7, 3],
            "exit_status": 0
          },
          "nvme_smart_health_information_log": {
            "critical_warning": 0,
            "temperature": 35,
            "available_spare": 98,
            "available_spare_threshold": 10,
            "percentage_used": 2,
            "data_units_read": 500,
            "data_units_written": 1000,
            "host_reads": 10000,
            "host_writes": 20000,
            "controller_busy_time": 50,
            "power_cycles": 20,
            "power_on_hours": 50
          }
        }"#;

        let snapshot = parse_smartctl_output(json, 12345).unwrap();
        assert_eq!(snapshot.ts, 12345);
        assert_eq!(snapshot.data_units_written, 1000);
        assert_eq!(snapshot.data_units_read, 500);
        assert_eq!(snapshot.percent_used, 2);
        assert_eq!(snapshot.available_spare, 98);
        assert_eq!(snapshot.power_on_hours, 50);
        assert_eq!(snapshot.power_cycles, 20);
        assert_eq!(snapshot.host_bytes_written(), 512_000_000);
    }

    #[test]
    fn test_parse_with_percent_used_alias() {
        let json = r#"{
            "nvme_smart_health_information_log": {
                "data_units_written": 200,
                "data_units_read": 100,
                "percent_used": 15,
                "available_spare": 85,
                "power_on_hours": 300,
                "power_cycles": 50
            }
        }"#;

        let snapshot = parse_smartctl_output(json, 999).unwrap();
        assert_eq!(snapshot.percent_used, 15);
        assert_eq!(snapshot.available_spare, 85);
    }

    #[test]
    fn test_parse_permission_denied_json() {
        let json = r#"{
          "smartctl": {
            "version": [7, 3],
            "messages": [
              {
                "string": "Smartctl open device: /dev/nvme0 failed: Permission denied",
                "severity": "error"
              }
            ],
            "exit_status": 2
          }
        }"#;

        let res = parse_smartctl_output(json, 100);
        assert!(matches!(res, Err(SmartError::PermissionDenied(_))));
    }

    #[test]
    fn test_parse_access_denied_windows_json() {
        let json = r#"{
          "smartctl": {
            "version": [7, 4],
            "messages": [
              {
                "string": "Smartctl open device: /dev/nvme0 failed: Access is denied.",
                "severity": "error"
              }
            ],
            "exit_status": 2
          }
        }"#;

        let res = parse_smartctl_output(json, 100);
        assert!(matches!(res, Err(SmartError::PermissionDenied(_))));
    }

    #[test]
    fn test_parse_device_not_found_json() {
        let json = r#"{
          "smartctl": {
            "version": [7, 5],
            "messages": [
              {
                "string": "Smartctl open device: /dev/nvme0 failed: No such device",
                "severity": "error"
              }
            ],
            "exit_status": 2
          }
        }"#;

        let res = parse_smartctl_output(json, 100);
        assert!(matches!(res, Err(SmartError::DeviceNotFound(_))));
    }

    #[test]
    fn test_parse_scan_json() {
        let json = r#"{
          "devices": [
            {
              "name": "/dev/sda",
              "info_name": "/dev/sda",
              "type": "nvme",
              "protocol": "NVMe"
            }
          ]
        }"#;

        let val: serde_json::Value = serde_json::from_str(json).unwrap();
        let devices = val.get("devices").unwrap().as_array().unwrap();
        assert_eq!(devices.len(), 1);
        let name = devices[0].get("name").unwrap().as_str().unwrap();
        assert_eq!(name, "/dev/sda");
    }

    #[test]
    fn test_error_display() {
        let not_found = SmartError::SmartctlNotFound("not found".to_string());
        assert!(not_found.to_string().contains("winget install smartmontools"));

        let perm = SmartError::PermissionDenied("access denied".to_string());
        assert!(perm.to_string().contains("Administrator"));

        let dev_err = SmartError::DeviceNotFound("no device".to_string());
        assert!(dev_err.to_string().contains("no device"));

        let spawn_err = SmartError::SpawnError("permission error".to_string());
        assert!(spawn_err.to_string().contains("permission error"));

        let parse_err = SmartError::ParseError("invalid token".to_string());
        assert_eq!(parse_err.to_string(), "Failed to parse smartctl output: invalid token");

        let cmd_err = SmartError::CommandFailed("exit code 1".to_string());
        assert_eq!(cmd_err.to_string(), "smartctl command failed: exit code 1");
    }

    #[test]
    fn test_resolve_smartctl_path_order() {
        // User custom path that does not exist should fall through
        let p = resolve_smartctl_path(Some("C:\\non_existent_folder\\smartctl.exe"));
        // On this test machine, C:\Program Files\smartmontools\bin\smartctl.exe exists!
        if Path::new(r"C:\Program Files\smartmontools\bin\smartctl.exe").is_file() {
            assert_eq!(
                p,
                Some(PathBuf::from(r"C:\Program Files\smartmontools\bin\smartctl.exe"))
            );
        }
    }

    #[test]
    fn test_live_detection_and_fallback() {
        if Path::new(r"C:\Program Files\smartmontools\bin\smartctl.exe").is_file() {
            let res = run_smartctl_with_config(None, None);
            println!("Live smartctl test result: {res:?}");
            // Whether it succeeds or reports permission denied if un-elevated, it must NOT report SmartctlNotFound!
            assert!(!matches!(res, Err(SmartError::SmartctlNotFound(_))));
        }
    }
}
