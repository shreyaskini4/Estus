use serde::{Deserialize, Serialize};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

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
    SmartctlNotFound,
    PermissionDenied,
    ParseError(String),
    CommandFailed(String),
}

impl std::fmt::Display for SmartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SmartctlNotFound => write!(
                f,
                "smartctl not found in PATH. Install smartmontools (`winget install smartmontools`) or add it to PATH."
            ),
            Self::PermissionDenied => write!(
                f,
                "Permission denied accessing NVMe SMART data. Please run SSD Watch as Administrator."
            ),
            Self::ParseError(msg) => write!(f, "Failed to parse smartctl output: {msg}"),
            Self::CommandFailed(msg) => write!(f, "smartctl command failed: {msg}"),
        }
    }
}

impl std::error::Error for SmartError {}

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

#[derive(Debug, Deserialize)]
struct SmartctlMeta {
    #[allow(dead_code)]
    exit_status: Option<i32>,
    messages: Option<Vec<SmartctlMessage>>,
}

#[derive(Debug, Deserialize)]
struct SmartctlMessage {
    string: Option<String>,
    #[allow(dead_code)]
    severity: Option<String>,
}

fn is_permission_denied_str(s: &str) -> bool {
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

pub fn parse_smartctl_output(json_str: &str, ts: i64) -> Result<SmartSnapshot, SmartError> {
    let trimmed = json_str.trim();
    if trimmed.is_empty() {
        return Err(SmartError::ParseError("Empty smartctl output".to_string()));
    }

    let val: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(e) => {
            if is_permission_denied_str(trimmed) {
                return Err(SmartError::PermissionDenied);
            }
            return Err(SmartError::ParseError(format!("Invalid JSON: {e}")));
        }
    };

    // Case 1: Root JSON contains "nvme_smart_health_information_log"
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

    // Case 2: The JSON is directly the health information object
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

    // Case 3: Check for error messages / permission denied
    if let Some(smartctl_val) = val.get("smartctl")
        && let Ok(meta) = serde_json::from_value::<SmartctlMeta>(smartctl_val.clone())
            && let Some(messages) = meta.messages {
                for msg in messages {
                    if let Some(ref str_val) = msg.string
                        && is_permission_denied_str(str_val) {
                            return Err(SmartError::PermissionDenied);
                        }
                }
            }

    if is_permission_denied_str(trimmed) {
        return Err(SmartError::PermissionDenied);
    }

    Err(SmartError::ParseError(
        "Missing 'nvme_smart_health_information_log' in smartctl JSON output".to_string(),
    ))
}

pub fn run_smartctl(device: Option<&str>) -> Result<SmartSnapshot, SmartError> {
    let dev = device.unwrap_or("/dev/nvme0");
    let mut cmd = std::process::Command::new("smartctl");
    cmd.args(["-j", "-a", dev]);

    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let output = match cmd.output() {
        Ok(out) => out,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(SmartError::SmartctlNotFound);
        }
        Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
            return Err(SmartError::PermissionDenied);
        }
        Err(err) => {
            return Err(SmartError::CommandFailed(format!(
                "Failed to spawn smartctl: {err}"
            )));
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let ts = chrono::Utc::now().timestamp();

    if is_permission_denied_str(&stderr) {
        return Err(SmartError::PermissionDenied);
    }

    if !stdout.trim().is_empty() {
        match parse_smartctl_output(&stdout, ts) {
            Ok(snapshot) => return Ok(snapshot),
            Err(SmartError::PermissionDenied) => return Err(SmartError::PermissionDenied),
            Err(err) => {
                let code = output.status.code().unwrap_or(-1);
                if code != 0 {
                    if is_permission_denied_str(&stdout) {
                        return Err(SmartError::PermissionDenied);
                    }
                    return Err(SmartError::CommandFailed(
                        format!("smartctl exited with code {code}: {stderr} {stdout}")
                            .trim()
                            .to_string(),
                    ));
                } else {
                    return Err(err);
                }
            }
        }
    }

    let code = output.status.code().unwrap_or(-1);
    if is_permission_denied_str(&stderr) || is_permission_denied_str(&stdout) {
        Err(SmartError::PermissionDenied)
    } else if code != 0 {
        Err(SmartError::CommandFailed(
            format!("smartctl exited with code {code}: {stderr}")
                .trim()
                .to_string(),
        ))
    } else {
        Err(SmartError::ParseError("smartctl returned empty output".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REALISTIC_NVME_JSON: &str = r#"{
      "json_format_version": [1, 0],
      "smartctl": {
        "version": [7, 3],
        "platform": "x86_64-w64-mingw32",
        "build_info": "smartctl 7.3 2022-02-28 r5338",
        "argv": ["smartctl", "-a", "-j", "/dev/nvme0"],
        "exit_status": 0
      },
      "device": {
        "name": "/dev/nvme0",
        "info_name": "/dev/nvme0",
        "type": "nvme",
        "protocol": "NVMe"
      },
      "model_name": "Samsung SSD 970 EVO Plus 1TB",
      "serial_number": "S4EWNF0M123456",
      "nvme_smart_health_information_log": {
        "critical_warning": 0,
        "temperature": 43,
        "available_spare": 100,
        "available_spare_threshold": 10,
        "percentage_used": 5,
        "data_units_read": 3264831,
        "data_units_written": 35345226,
        "host_reads": 38564427,
        "host_writes": 339789142,
        "controller_busy_time": 686,
        "power_cycles": 779,
        "power_on_hours": 1290,
        "unsafe_shutdowns": 9,
        "media_errors": 0,
        "num_err_log_entries": 979
      }
    }"#;

    #[test]
    fn test_parse_valid_nvme_json() {
        let ts = 1700000000;
        let snapshot = parse_smartctl_output(REALISTIC_NVME_JSON, ts)
            .expect("Should parse valid smartctl json");

        assert_eq!(snapshot.ts, ts);
        assert_eq!(snapshot.data_units_written, 35345226);
        assert_eq!(snapshot.data_units_read, 3264831);
        assert_eq!(snapshot.percent_used, 5);
        assert_eq!(snapshot.available_spare, 100);
        assert_eq!(snapshot.power_on_hours, 1290);
        assert_eq!(snapshot.power_cycles, 779);

        // Host bytes = data_units * 512_000
        assert_eq!(snapshot.host_bytes_written(), 35345226 * 512_000);
        assert_eq!(snapshot.host_bytes_read(), 3264831 * 512_000);
    }

    #[test]
    fn test_parse_direct_health_log() {
        let json = r#"{
            "data_units_written": 1000,
            "data_units_read": 500,
            "percentage_used": 2,
            "available_spare": 98,
            "power_on_hours": 50,
            "power_cycles": 20
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
        assert_eq!(res, Err(SmartError::PermissionDenied));
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
        assert_eq!(res, Err(SmartError::PermissionDenied));
    }

    #[test]
    fn test_parse_missing_health_log() {
        let json = r#"{
          "json_format_version": [1, 0],
          "smartctl": {
            "version": [7, 3],
            "exit_status": 0
          },
          "device": {
            "name": "/dev/sda",
            "type": "ata"
          }
        }"#;

        let res = parse_smartctl_output(json, 100);
        match res {
            Err(SmartError::ParseError(msg)) => {
                assert!(msg.contains("nvme_smart_health_information_log"));
            }
            other => panic!("Expected ParseError, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_malformed_json() {
        assert!(matches!(
            parse_smartctl_output("", 100),
            Err(SmartError::ParseError(_))
        ));

        assert!(matches!(
            parse_smartctl_output("{ not valid json", 100),
            Err(SmartError::ParseError(_))
        ));

        let bad_field_json = r#"{
          "nvme_smart_health_information_log": {
            "data_units_written": "not_a_number",
            "data_units_read": 100,
            "percentage_used": 1,
            "available_spare": 100,
            "power_on_hours": 10,
            "power_cycles": 10
          }
        }"#;
        assert!(matches!(
            parse_smartctl_output(bad_field_json, 100),
            Err(SmartError::ParseError(_))
        ));

        let missing_fields_json = r#"{
          "nvme_smart_health_information_log": {
            "data_units_written": 100
          }
        }"#;
        assert!(matches!(
            parse_smartctl_output(missing_fields_json, 100),
            Err(SmartError::ParseError(_))
        ));
    }

    #[test]
    fn test_error_display() {
        let not_found = SmartError::SmartctlNotFound;
        assert!(not_found.to_string().contains("winget install smartmontools"));

        let perm = SmartError::PermissionDenied;
        assert!(perm.to_string().contains("Administrator"));

        let parse_err = SmartError::ParseError("invalid token".to_string());
        assert_eq!(parse_err.to_string(), "Failed to parse smartctl output: invalid token");

        let cmd_err = SmartError::CommandFailed("exit code 1".to_string());
        assert_eq!(cmd_err.to_string(), "smartctl command failed: exit code 1");
    }

    #[test]
    fn test_run_smartctl_when_not_installed() {
        // Since smartctl is not in PATH in test environment, run_smartctl should report SmartctlNotFound
        let res = run_smartctl(Some("/dev/nvme0"));
        assert!(matches!(res, Err(SmartError::SmartctlNotFound)));
    }
}
