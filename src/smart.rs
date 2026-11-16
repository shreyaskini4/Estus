use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub use crate::process_util::{create_no_window_command, CREATE_NO_WINDOW};

/// Global thread-safe cache for the resolved smartctl executable path.
static CACHED_SMARTCTL_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Global storage for the last raw output captured from smartctl execution.
static LAST_RAW_OUTPUT: Mutex<Option<String>> = Mutex::new(None);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriveInfo {
    pub id: String,
    pub serial: String,
    pub model: String,
    pub capacity_bytes: u64,
    pub protocol: String,
    pub device_path: String,
    pub nickname: Option<String>,
    pub enabled: bool,
}

pub fn make_drive_id(serial: &str, model: &str) -> String {
    let s = serial.trim();
    let m = model.trim();
    if s.is_empty() && m.is_empty() {
        "unknown_drive".to_string()
    } else if s.is_empty() {
        m.replace(' ', "_")
    } else if m.is_empty() {
        s.replace(' ', "_")
    } else {
        format!("{}_{}", s, m).replace(' ', "_")
    }
}

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
                "Permission denied accessing SMART data: {msg}. Please run Estus as Administrator."
            ),
            Self::DeviceNotFound(msg) => write!(
                f,
                "Storage device not found: {msg}."
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

/// Sets the last captured raw output from smartctl execution.
pub fn set_last_raw_output(output: String) {
    if let Ok(mut guard) = LAST_RAW_OUTPUT.lock() {
        *guard = Some(output);
    }
}

/// Retrieves the last captured raw output from smartctl execution.
pub fn get_last_raw_output() -> Option<String> {
    if let Ok(guard) = LAST_RAW_OUTPUT.lock() {
        guard.clone()
    } else {
        None
    }
}

/// Retrieves the version string from a candidate executable by running `<candidate> --version`.
pub fn get_smartctl_version(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }

    let mut cmd = create_no_window_command(path);
    cmd.arg("--version");

    let output = cmd.output().ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    extract_version_from_stdout(&stdout)
}

fn extract_version_from_stdout(stdout: &str) -> Option<String> {
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("smartctl ") {
            let ver = trimmed.split('[').next().unwrap_or(trimmed).trim();
            return Some(ver.to_string());
        }
    }
    stdout.lines().find(|l| !l.trim().is_empty()).map(|l| l.trim().to_string())
}

/// Verifies a candidate executable by running `<candidate> --version`, logging the attempt.
fn verify_candidate(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }

    let mut cmd = create_no_window_command(path);
    cmd.arg("--version");

    match cmd.output() {
        Ok(output) => {
            let code = output.status.code();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            crate::logger::log_verification_attempt(
                &path.display().to_string(),
                code,
                &stdout,
                &stderr,
                None,
            );
            if !stdout.trim().is_empty() {
                extract_version_from_stdout(&stdout)
            } else {
                None
            }
        }
        Err(err) => {
            crate::logger::log_verification_attempt(
                &path.display().to_string(),
                None,
                "",
                "",
                Some(&err),
            );
            None
        }
    }
}

#[cfg(windows)]
fn add_candidate_dir(candidates: &mut Vec<PathBuf>, dir_str: &str) {
    let trimmed = dir_str.trim().trim_matches('"');
    if trimmed.is_empty() {
        return;
    }
    let p = PathBuf::from(trimmed);
    if p.is_file() {
        candidates.push(p);
    } else {
        candidates.push(p.join("bin").join("smartctl.exe"));
        candidates.push(p.join("smartctl.exe"));
    }
}

#[cfg(windows)]
fn check_registry_key_values(key: &windows_registry::Key, candidates: &mut Vec<PathBuf>) {
    for val_name in &["InstallLocation", "Path", "Install_Dir", "InstallDir", ""] {
        if let Ok(val) = key.get_string(val_name) {
            add_candidate_dir(candidates, &val);
        }
    }
}

/// Searches Windows Registry for smartmontools install directories.
#[cfg(windows)]
fn find_in_registry() -> Vec<PathBuf> {
    use windows_registry::LOCAL_MACHINE;
    let mut candidates = Vec::new();

    // 1. HKLM\SOFTWARE\smartmontools
    if let Ok(key) = LOCAL_MACHINE.open(r"SOFTWARE\smartmontools") {
        check_registry_key_values(&key, &mut candidates);
    }

    // 2. HKLM\SOFTWARE\WOW6432Node\smartmontools
    if let Ok(key) = LOCAL_MACHINE.open(r"SOFTWARE\WOW6432Node\smartmontools") {
        check_registry_key_values(&key, &mut candidates);
    }

    // 3. Uninstall keys: HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall
    for uninstall_root in &[
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
        r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
    ] {
        if let Ok(root_key) = LOCAL_MACHINE.open(uninstall_root)
            && let Ok(subkeys) = root_key.keys() {
                for subkey_name in subkeys {
                    let lower = subkey_name.to_lowercase();
                    if (lower.contains("smartmontools") || lower.contains("smartctl"))
                        && let Ok(sub) = root_key.open(&subkey_name) {
                            check_registry_key_values(&sub, &mut candidates);
                            if let Ok(loc) = sub.get_string("InstallLocation") {
                                add_candidate_dir(&mut candidates, &loc);
                            }
                        }
                }
            }
    }

    candidates
}

#[cfg(not(windows))]
fn find_in_registry() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(windows)]
fn find_with_where_cmd(binary_name: &str) -> Vec<PathBuf> {
    let mut results = Vec::new();
    let mut cmd = create_no_window_command("where.exe");
    cmd.arg(binary_name);

    if let Ok(output) = cmd.output()
        && output.status.success() {
            let out_str = String::from_utf8_lossy(&output.stdout);
            for line in out_str.lines() {
                let trimmed = line.trim().trim_matches('"');
                if !trimmed.is_empty() {
                    results.push(PathBuf::from(trimmed));
                }
            }
        }
    results
}

fn check_and_accept(
    p: PathBuf,
    checked_paths: &mut std::collections::HashSet<String>,
    tried: &mut Vec<String>,
) -> Option<PathBuf> {
    let canonical_key = p.to_string_lossy().to_lowercase();
    if checked_paths.contains(&canonical_key) {
        return None;
    }
    checked_paths.insert(canonical_key);
    tried.push(p.display().to_string());

    if let Some(ver) = verify_candidate(&p) {
        crate::logger::log(&format!(
            "Candidate verified successfully: \"{}\" (version: {})",
            p.display(),
            ver
        ));
        Some(p)
    } else {
        None
    }
}

/// Resolves the smartctl path in the designated order and verifies each candidate.
pub fn resolve_smartctl(custom_path: Option<&str>) -> Option<PathBuf> {
    if custom_path.is_none()
        && let Ok(guard) = CACHED_SMARTCTL_PATH.lock()
            && let Some(ref cached) = *guard
                && cached.is_file() {
                    return Some(cached.clone());
                }

    let mut tried = Vec::new();
    let mut checked_paths = std::collections::HashSet::new();

    // 1. User-configured path
    if let Some(custom) = custom_path {
        let trimmed = custom.trim().trim_matches('"');
        if !trimmed.is_empty() {
            let p = PathBuf::from(trimmed);
            if p.is_file() {
                if let Some(valid) = check_and_accept(p, &mut checked_paths, &mut tried) {
                    set_cached_smartctl_path(Some(valid.clone()));
                    return Some(valid);
                }
            } else {
                tried.push(format!("{trimmed} (configured file does not exist)"));
            }
        }
    }

    // 2. PATH lookup
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            #[cfg(windows)]
            {
                let exe = dir.join("smartctl.exe");
                if exe.is_file()
                    && let Some(valid) = check_and_accept(exe, &mut checked_paths, &mut tried) {
                        set_cached_smartctl_path(Some(valid.clone()));
                        return Some(valid);
                    }
            }
            let direct = dir.join("smartctl");
            if direct.is_file()
                && let Some(valid) = check_and_accept(direct, &mut checked_paths, &mut tried) {
                    set_cached_smartctl_path(Some(valid.clone()));
                    return Some(valid);
                }
        }
    }

    #[cfg(windows)]
    {
        for p in find_with_where_cmd("smartctl") {
            if p.is_file()
                && let Some(valid) = check_and_accept(p, &mut checked_paths, &mut tried) {
                    set_cached_smartctl_path(Some(valid.clone()));
                    return Some(valid);
                }
        }
    }

    // 3. C:\Program Files\smartmontools\bin\smartctl.exe
    let p3 = PathBuf::from(r"C:\Program Files\smartmontools\bin\smartctl.exe");
    if p3.is_file()
        && let Some(valid) = check_and_accept(p3, &mut checked_paths, &mut tried) {
            set_cached_smartctl_path(Some(valid.clone()));
            return Some(valid);
        }

    // 4. C:\Program Files (x86)\smartmontools\bin\smartctl.exe
    let p4 = PathBuf::from(r"C:\Program Files (x86)\smartmontools\bin\smartctl.exe");
    if p4.is_file()
        && let Some(valid) = check_and_accept(p4, &mut checked_paths, &mut tried) {
            set_cached_smartctl_path(Some(valid.clone()));
            return Some(valid);
        }

    // 5. Windows Registry search
    for reg_path in find_in_registry() {
        if reg_path.is_file()
            && let Some(valid) = check_and_accept(reg_path, &mut checked_paths, &mut tried) {
                set_cached_smartctl_path(Some(valid.clone()));
                return Some(valid);
            }
    }

    crate::logger::log(&format!(
        "smartctl detection failed after checking candidates: {tried:?}"
    ));
    None
}

pub fn resolve_smartctl_path(custom_path: Option<&str>) -> Option<PathBuf> {
    resolve_smartctl(custom_path)
}

pub fn get_or_resolve_smartctl_path(custom_path: Option<&str>) -> Option<PathBuf> {
    resolve_smartctl(custom_path)
}

pub fn set_cached_smartctl_path(path: Option<PathBuf>) {
    if let Ok(mut guard) = CACHED_SMARTCTL_PATH.lock() {
        *guard = path;
    }
}

pub fn clear_cached_smartctl_path() {
    set_cached_smartctl_path(None);
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
        || lower.contains("elevation")
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

/// Parses SATA SMART attributes table from smartctl JSON.
pub fn parse_sata_smart_attributes(
    val: &serde_json::Value,
    ts: i64,
) -> Result<SmartSnapshot, SmartError> {
    let table = val
        .get("ata_smart_attributes")
        .and_then(|a| a.get("table"))
        .and_then(|t| t.as_array());

    let mut data_units_written = 0u64;
    let mut data_units_read = 0u64;
    let mut percent_used = 0u8;
    let mut available_spare = 100u8;
    let mut power_on_hours = 0u64;
    let mut power_cycles = 0u64;

    if let Some(poh) = val.get("power_on_time").and_then(|p| p.get("hours")).and_then(|h| h.as_u64()) {
        power_on_hours = poh;
    }
    if let Some(pcc) = val.get("power_cycle_count").and_then(|c| c.as_u64()) {
        power_cycles = pcc;
    }

    if let Some(attrs) = table {
        for attr in attrs {
            let id = attr.get("id").and_then(|i| i.as_u64()).unwrap_or(0);
            let name = attr.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let norm_val = attr.get("value").and_then(|v| v.as_u64()).unwrap_or(0);
            let raw_val = attr
                .get("raw")
                .and_then(|r| r.get("value").and_then(|v| v.as_u64()))
                .unwrap_or(0);

            match id {
                9 => {
                    if power_on_hours == 0 {
                        power_on_hours = raw_val;
                    }
                }
                12 => {
                    if power_cycles == 0 {
                        power_cycles = raw_val;
                    }
                }
                177 | 231 | 202 | 169 => {
                    let name_lower = name.to_lowercase();
                    if name_lower.contains("percentage_used") || name_lower.contains("percent_used") {
                        percent_used = raw_val.min(100) as u8;
                    } else if norm_val > 0 && norm_val <= 100 {
                        percent_used = 100u64.saturating_sub(norm_val).min(100) as u8;
                        available_spare = norm_val.min(100) as u8;
                    } else if raw_val > 0 && raw_val <= 100 && name_lower.contains("life") {
                        percent_used = 100u64.saturating_sub(raw_val).min(100) as u8;
                        available_spare = raw_val.min(100) as u8;
                    }
                }
                232 => {
                    if norm_val > 0 && norm_val <= 100 {
                        available_spare = norm_val as u8;
                    }
                }
                241 => {
                    // Total_LBAs_Written: 1 LBA = 512 bytes; 512,000 bytes = 1 data unit
                    data_units_written = raw_val.saturating_mul(512) / 512_000;
                }
                242 => {
                    data_units_read = raw_val.saturating_mul(512) / 512_000;
                }
                _ => {
                    let name_lower = name.to_lowercase();
                    if (name_lower.contains("wear") || name_lower.contains("life_left"))
                        && norm_val > 0
                        && norm_val <= 100
                    {
                        percent_used = 100u64.saturating_sub(norm_val).min(100) as u8;
                    } else if name_lower.contains("lba") && name_lower.contains("written") {
                        data_units_written = raw_val.saturating_mul(512) / 512_000;
                    }
                }
            }
        }
    }

    Ok(SmartSnapshot {
        ts,
        data_units_written,
        data_units_read,
        percent_used,
        available_spare,
        power_on_hours,
        power_cycles,
    })
}

/// Extracts DriveInfo metadata from smartctl JSON output.
pub fn extract_drive_info(val: &serde_json::Value, device_path: &str) -> DriveInfo {
    let model = val
        .get("model_name")
        .and_then(|m| m.as_str())
        .or_else(|| val.get("model_family").and_then(|m| m.as_str()))
        .or_else(|| val.get("device").and_then(|d| d.get("model_name")).and_then(|m| m.as_str()))
        .unwrap_or("Unknown Model")
        .trim()
        .to_string();

    let serial = val
        .get("serial_number")
        .and_then(|s| s.as_str())
        .or_else(|| val.get("device").and_then(|d| d.get("serial_number")).and_then(|s| s.as_str()))
        .unwrap_or("Unknown Serial")
        .trim()
        .to_string();

    let capacity_bytes = val
        .get("user_capacity")
        .and_then(|c| c.get("bytes"))
        .and_then(|b| b.as_u64())
        .unwrap_or(0);

    let protocol = val
        .get("device")
        .and_then(|d| d.get("protocol"))
        .and_then(|p| p.as_str())
        .or_else(|| val.get("device").and_then(|d| d.get("type")).and_then(|t| t.as_str()))
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            if val.get("nvme_smart_health_information_log").is_some() {
                "NVMe".to_string()
            } else {
                "ATA".to_string()
            }
        });

    let id = make_drive_id(&serial, &model);

    DriveInfo {
        id,
        serial,
        model,
        capacity_bytes,
        protocol,
        device_path: device_path.to_string(),
        nickname: None,
        enabled: true,
    }
}

/// Parses smartctl JSON output to produce DriveInfo and SmartSnapshot.
pub fn parse_smartctl_output_full(
    json_str: &str,
    device_path: &str,
    ts: i64,
) -> Result<(DriveInfo, SmartSnapshot), SmartError> {
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

    if let Some(err) = check_smartctl_messages(&val) {
        return Err(err);
    }

    let drive_info = extract_drive_info(&val, device_path);

    // 1. Root JSON contains "nvme_smart_health_information_log"
    if let Some(health_val) = val.get("nvme_smart_health_information_log") {
        return match serde_json::from_value::<NvmeHealthInfo>(health_val.clone()) {
            Ok(health) => Ok((
                drive_info,
                SmartSnapshot {
                    ts,
                    data_units_written: health.data_units_written,
                    data_units_read: health.data_units_read,
                    percent_used: health.percentage_used,
                    available_spare: health.available_spare,
                    power_on_hours: health.power_on_hours,
                    power_cycles: health.power_cycles,
                },
            )),
            Err(e) => Err(SmartError::ParseError(format!(
                "Invalid 'nvme_smart_health_information_log': {e}"
            ))),
        };
    }

    // 2. The JSON is directly the health info object
    if let Ok(health) = serde_json::from_value::<NvmeHealthInfo>(val.clone()) {
        return Ok((
            drive_info,
            SmartSnapshot {
                ts,
                data_units_written: health.data_units_written,
                data_units_read: health.data_units_read,
                percent_used: health.percentage_used,
                available_spare: health.available_spare,
                power_on_hours: health.power_on_hours,
                power_cycles: health.power_cycles,
            },
        ));
    }

    // 3. SATA ATA attributes table
    if val.get("ata_smart_attributes").is_some() {
        let snap = parse_sata_smart_attributes(&val, ts)?;
        return Ok((drive_info, snap));
    }

    // 4. Any messages
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
        "Missing 'nvme_smart_health_information_log' or 'ata_smart_attributes' in smartctl JSON output".to_string(),
    ))
}

/// Convenience parser matching the original signature for backward compatibility.
pub fn parse_smartctl_output(json_str: &str, ts: i64) -> Result<SmartSnapshot, SmartError> {
    parse_smartctl_output_full(json_str, "/dev/nvme0", ts).map(|(_, s)| s)
}

/// Runs `smartctl --scan -j` and returns the list of all detected device paths.
pub fn discover_drives(smartctl_exe: &Path) -> Result<Vec<String>, SmartError> {
    let mut cmd = create_no_window_command(smartctl_exe);
    let args = ["--scan", "-j"];
    cmd.args(args);

    let output = match cmd.output() {
        Ok(out) => out,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            crate::logger::log_smartctl_execution(
                &smartctl_exe.display().to_string(),
                &args,
                None,
                "",
                "",
                Some(&err),
            );
            return Err(SmartError::SmartctlNotFound(format!(
                "Executable not found at {}",
                smartctl_exe.display()
            )));
        }
        Err(err) => {
            crate::logger::log_smartctl_execution(
                &smartctl_exe.display().to_string(),
                &args,
                None,
                "",
                "",
                Some(&err),
            );
            return Err(SmartError::SpawnError(format!(
                "Failed to spawn {}: {}",
                smartctl_exe.display(),
                err
            )));
        }
    };

    let exit_code = output.status.code();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    crate::logger::log_smartctl_execution(
        &smartctl_exe.display().to_string(),
        &args,
        exit_code,
        &stdout,
        &stderr,
        None,
    );

    let mut devices = Vec::new();
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&stdout)
        && let Some(dev_list) = val.get("devices").and_then(|d| d.as_array()) {
            for dev in dev_list {
                if let Some(name) = dev.get("name").and_then(|n| n.as_str()) {
                    let trimmed = name.trim();
                    if !trimmed.is_empty() && !devices.contains(&trimmed.to_string()) {
                        devices.push(trimmed.to_string());
                    }
                }
            }
        }

    if devices.is_empty() {
        for candidate in &["/dev/sda", "/dev/sdb", "/dev/nvme0", "/dev/nvme1"] {
            devices.push(candidate.to_string());
        }
    }

    Ok(devices)
}

/// Retained for backward compatibility: returns the first NVMe device from scan.
pub fn scan_for_nvme_device(smartctl_exe: &Path) -> Result<Option<String>, SmartError> {
    let devices = discover_drives(smartctl_exe)?;
    for dev in devices {
        if dev.to_lowercase().contains("nvme") {
            return Ok(Some(dev));
        }
    }
    Ok(None)
}

/// Executes smartctl query for a specific device, returning both DriveInfo and SmartSnapshot.
pub fn query_device(
    smartctl_exe: &Path,
    device: &str,
) -> Result<(DriveInfo, SmartSnapshot), SmartError> {
    let mut cmd = create_no_window_command(smartctl_exe);
    let args = ["-j", "-a", device];
    cmd.args(args);

    let output = match cmd.output() {
        Ok(out) => out,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            let err_msg = format!("Executable not found at {}", smartctl_exe.display());
            crate::logger::log_smartctl_execution(
                &smartctl_exe.display().to_string(),
                &args,
                None,
                "",
                "",
                Some(&err),
            );
            return Err(SmartError::SmartctlNotFound(err_msg));
        }
        Err(err) => {
            let err_msg = format!(
                "Failed to spawn {}: {} (OS code: {:?})",
                smartctl_exe.display(),
                err,
                err.raw_os_error()
            );
            crate::logger::log_smartctl_execution(
                &smartctl_exe.display().to_string(),
                &args,
                None,
                "",
                "",
                Some(&err),
            );
            return Err(SmartError::SpawnError(err_msg));
        }
    };

    let exit_code = output.status.code();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let ts = chrono::Utc::now().timestamp();

    crate::logger::log_smartctl_execution(
        &smartctl_exe.display().to_string(),
        &args,
        exit_code,
        &stdout,
        &stderr,
        None,
    );

    if !stdout.trim().is_empty() {
        set_last_raw_output(stdout.to_string());
    } else if !stderr.trim().is_empty() {
        set_last_raw_output(stderr.to_string());
    }

    if !stdout.trim().is_empty() {
        match parse_smartctl_output_full(&stdout, device, ts) {
            Ok(res) => return Ok(res),
            Err(err) => {
                crate::logger::log_detection_failure(
                    &smartctl_exe.display().to_string(),
                    device,
                    &stdout,
                    &stderr,
                    &err.to_string(),
                );
                return Err(err);
            }
        }
    }

    let err = if is_permission_denied_str(&stderr) {
        SmartError::PermissionDenied(stderr.trim().to_string())
    } else if is_device_not_found_str(&stderr) {
        SmartError::DeviceNotFound(stderr.trim().to_string())
    } else if !stderr.trim().is_empty() {
        SmartError::CommandFailed(stderr.trim().to_string())
    } else {
        SmartError::ParseError("smartctl returned empty output".to_string())
    };

    crate::logger::log_detection_failure(
        &smartctl_exe.display().to_string(),
        device,
        &stdout,
        &stderr,
        &err.to_string(),
    );
    Err(err)
}

/// Discovers all storage devices via `smartctl --scan -j` and queries each one.
/// Unplugged or unreadable devices are logged and skipped without crashing.
pub fn discover_and_query_all_drives(
    custom_path: Option<&str>,
) -> Result<Vec<(DriveInfo, SmartSnapshot)>, SmartError> {
    let smartctl_exe = match resolve_smartctl(custom_path) {
        Some(p) => p,
        None => {
            let err = SmartError::SmartctlNotFound(
                "smartctl was not found in settings, PATH, C:\\Program Files\\smartmontools\\bin, C:\\Program Files (x86)\\smartmontools\\bin, or Windows Registry".to_string(),
            );
            crate::logger::log_detection_failure("None", "all", "", "", &err.to_string());
            return Err(err);
        }
    };

    let devices = discover_drives(&smartctl_exe)?;
    let mut results = Vec::new();
    let mut last_err = None;

    for dev in &devices {
        match query_device(&smartctl_exe, dev) {
            Ok((drive_info, snapshot)) => {
                crate::logger::log(&format!(
                    "Discovered drive: ID=\"{}\", model=\"{}\", path=\"{}\"",
                    drive_info.id, drive_info.model, drive_info.device_path
                ));
                results.push((drive_info, snapshot));
            }
            Err(err) => {
                crate::logger::log(&format!(
                    "Gracefully skipping device \"{dev}\" after query failed: {err}"
                ));
                last_err = Some(err);
            }
        }
    }

    if results.is_empty()
        && let Some(err) = last_err {
            return Err(err);
        }

    Ok(results)
}

/// Legacy entry point for backward compatibility.
pub fn run_smartctl_with_config(
    custom_path: Option<&str>,
    device: Option<&str>,
) -> Result<(SmartSnapshot, String, String), SmartError> {
    let smartctl_exe = match resolve_smartctl(custom_path) {
        Some(p) => p,
        None => {
            let err = SmartError::SmartctlNotFound(
                "smartctl was not found in settings, PATH, C:\\Program Files\\smartmontools\\bin, C:\\Program Files (x86)\\smartmontools\\bin, or Windows Registry".to_string(),
            );
            crate::logger::log_detection_failure("None", device.unwrap_or("auto"), "", "", &err.to_string());
            return Err(err);
        }
    };

    let resolved_path_str = smartctl_exe.display().to_string();

    if let Some(dev) = device {
        let (_, snapshot) = query_device(&smartctl_exe, dev)?;
        return Ok((snapshot, resolved_path_str, dev.to_string()));
    }

    match discover_and_query_all_drives(custom_path) {
        Ok(drives) if !drives.is_empty() => {
            let (info, snap) = drives.into_iter().next().unwrap();
            Ok((snap, resolved_path_str, info.device_path))
        }
        _ => {
            let default_dev = "/dev/nvme0";
            match query_device(&smartctl_exe, default_dev) {
                Ok((_, snapshot)) => Ok((snapshot, resolved_path_str, default_dev.to_string())),
                Err(err) => Err(err),
            }
        }
    }
}

pub fn run_smartctl(device: Option<&str>) -> Result<SmartSnapshot, SmartError> {
    run_smartctl_with_config(None, device).map(|(s, _, _)| s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_drive_id() {
        assert_eq!(
            make_drive_id("S67MNF1W478385", "SAMSUNG MZAL4512HBLU-00BL2"),
            "S67MNF1W478385_SAMSUNG_MZAL4512HBLU-00BL2"
        );
        assert_eq!(
            make_drive_id("24294A12E065", "CT500P3PSSD8"),
            "24294A12E065_CT500P3PSSD8"
        );
        assert_eq!(make_drive_id("", "Model Only"), "Model_Only");
        assert_eq!(make_drive_id("SerialOnly", ""), "SerialOnly");
        assert_eq!(make_drive_id("", ""), "unknown_drive");
    }

    #[test]
    fn test_parse_valid_nvme_json() {
        let json = r#"{
          "json_format_version": [1, 0],
          "smartctl": {
            "version": [7, 3],
            "exit_status": 0
          },
          "model_name": "Samsung 980 PRO 1TB",
          "serial_number": "S5GXNF0R123456",
          "user_capacity": {
            "bytes": 1000204886016
          },
          "device": {
            "name": "/dev/nvme0",
            "protocol": "NVMe"
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

        let (drive_info, snapshot) = parse_smartctl_output_full(json, "/dev/nvme0", 12345).unwrap();
        assert_eq!(drive_info.id, "S5GXNF0R123456_Samsung_980_PRO_1TB");
        assert_eq!(drive_info.model, "Samsung 980 PRO 1TB");
        assert_eq!(drive_info.serial, "S5GXNF0R123456");
        assert_eq!(drive_info.capacity_bytes, 1000204886016);
        assert_eq!(drive_info.protocol, "NVMe");
        assert_eq!(drive_info.device_path, "/dev/nvme0");
        assert!(drive_info.enabled);

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
    fn test_parse_valid_sata_json() {
        let json = r#"{
          "model_name": "Crucial CT500MX500SSD1",
          "serial_number": "1842E1689ABC",
          "user_capacity": {
            "bytes": 500107862016
          },
          "device": {
            "name": "/dev/sda",
            "protocol": "ATA"
          },
          "ata_smart_attributes": {
            "table": [
              {
                "id": 9,
                "name": "Power_On_Hours",
                "raw": { "value": 2400 }
              },
              {
                "id": 12,
                "name": "Power_Cycle_Count",
                "raw": { "value": 350 }
              },
              {
                "id": 177,
                "name": "Wear_Leveling_Count",
                "value": 96,
                "raw": { "value": 25 }
              },
              {
                "id": 232,
                "name": "Available_Reservd_Blocks",
                "value": 98,
                "raw": { "value": 98 }
              },
              {
                "id": 241,
                "name": "Total_LBAs_Written",
                "raw": { "value": 20000000 }
              },
              {
                "id": 242,
                "name": "Total_LBAs_Read",
                "raw": { "value": 15000000 }
              }
            ]
          }
        }"#;

        let (drive_info, snapshot) = parse_smartctl_output_full(json, "/dev/sda", 55555).unwrap();
        assert_eq!(drive_info.id, "1842E1689ABC_Crucial_CT500MX500SSD1");
        assert_eq!(drive_info.model, "Crucial CT500MX500SSD1");
        assert_eq!(drive_info.capacity_bytes, 500107862016);
        assert_eq!(drive_info.protocol, "ATA");

        assert_eq!(snapshot.ts, 55555);
        // 20_000_000 * 512 / 512_000 = 20_000 data units
        assert_eq!(snapshot.data_units_written, 20_000);
        // 15_000_000 * 512 / 512_000 = 15_000 data units
        assert_eq!(snapshot.data_units_read, 15_000);
        // Wear: 100 - 96 = 4%
        assert_eq!(snapshot.percent_used, 4);
        assert_eq!(snapshot.available_spare, 98);
        assert_eq!(snapshot.power_on_hours, 2400);
        assert_eq!(snapshot.power_cycles, 350);
        assert_eq!(snapshot.host_bytes_written(), 10_240_000_000);
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
            },
            {
              "name": "/dev/sdb",
              "info_name": "/dev/sdb",
              "type": "ata",
              "protocol": "ATA"
            }
          ]
        }"#;

        let val: serde_json::Value = serde_json::from_str(json).unwrap();
        let devices = val.get("devices").unwrap().as_array().unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].get("name").unwrap().as_str().unwrap(), "/dev/sda");
        assert_eq!(devices[1].get("name").unwrap().as_str().unwrap(), "/dev/sdb");
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
    fn test_resolve_smartctl_order() {
        clear_cached_smartctl_path();
        let p = resolve_smartctl(Some("C:\\non_existent_folder\\smartctl.exe"));
        if Path::new(r"C:\Program Files\smartmontools\bin\smartctl.exe").is_file() {
            assert_eq!(
                p,
                Some(PathBuf::from(r"C:\Program Files\smartmontools\bin\smartctl.exe"))
            );
        }
    }

    #[test]
    fn test_get_smartctl_version_extraction() {
        let sample = "smartctl 7.5 2025-04-30 r5714 [x86_64-w64-mingw32-w11-b26300] (AppVeyor)\nCopyright (C) 2002-25";
        let ver = extract_version_from_stdout(sample);
        assert_eq!(ver, Some("smartctl 7.5 2025-04-30 r5714".to_string()));
    }

    #[test]
    fn test_live_detection_and_fallback() {
        if Path::new(r"C:\Program Files\smartmontools\bin\smartctl.exe").is_file() {
            let res = run_smartctl_with_config(None, None);
            crate::logger::log(&format!("Live smartctl test result: {res:?}"));
            assert!(!matches!(res, Err(SmartError::SmartctlNotFound(_))));
        }
    }
}
