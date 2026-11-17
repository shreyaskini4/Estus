use anyhow::Result;

pub const TASK_NAME: &str = "Estus";

/// Escapes XML special characters for safe inclusion in Task Scheduler XML.
pub fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Unescapes XML special characters.
pub fn unescape_xml(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Normalizes an extracted executable path by trimming whitespace, surrounding quotes, and CLI flags.
pub fn clean_exe_path(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with('"')
        && let Some(end) = trimmed[1..].find('"') {
            return trimmed[1..1 + end].trim().to_string();
        }
    if let Some(idx) = trimmed.find(" --minimized") {
        return trimmed[..idx].trim_matches('"').trim().to_string();
    }
    trimmed.trim_matches('"').trim().to_string()
}

/// Extracts the target executable command path from Task Scheduler XML output.
pub fn extract_command_from_xml(xml: &str) -> Option<String> {
    if let Some(start) = xml.find("<Command>") {
        let after = &xml[start + "<Command>".len()..];
        if let Some(end) = after.find("</Command>") {
            let cmd = after[..end].trim();
            let unescaped = unescape_xml(cmd);
            let cleaned = clean_exe_path(&unescaped);
            if !cleaned.is_empty() {
                return Some(cleaned);
            }
        }
    }
    None
}

/// Extracts the target executable command from `schtasks /Query /FO LIST /V` output.
pub fn extract_command_from_list_output(output: &str) -> Option<String> {
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(val) = trimmed.strip_prefix("Task To Run:") {
            let cleaned = clean_exe_path(val.trim());
            if !cleaned.is_empty() {
                return Some(cleaned);
            }
        }
    }
    None
}

/// Encodes a UTF-8 string into UTF-16LE bytes with a Byte Order Mark (BOM: 0xFF, 0xFE).
/// Windows `schtasks /XML` specifically requires this encoding.
pub fn encode_utf16_le_bom(s: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(2 + s.len() * 2);
    bytes.push(0xFF);
    bytes.push(0xFE);
    for c in s.encode_utf16() {
        bytes.extend_from_slice(&c.to_le_bytes());
    }
    bytes
}

/// Generates the Task Scheduler XML definition for Estus autostart with highest available privileges.
pub fn generate_task_xml(exe_path: &str) -> String {
    let escaped_path = escape_xml(exe_path);
    format!(
r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Estus SSD Monitor Autostart</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{}</Command>
      <Arguments>--minimized</Arguments>
    </Exec>
  </Actions>
</Task>"#,
        escaped_path
    )
}

/// Helper to execute `schtasks` using `create_no_window_command` and log the command and result.
fn run_schtasks(args: &[&str]) -> std::io::Result<std::process::Output> {
    let mut cmd = crate::process_util::create_no_window_command("schtasks");
    cmd.args(args);
    let full_cmd_str = format!("schtasks {}", args.join(" "));
    crate::logger::log(&format!("Executing: {}", full_cmd_str));

    match cmd.output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            crate::logger::log(&format!(
                "Result for [{}]: status={}, stdout={}, stderr={}",
                full_cmd_str,
                output.status,
                stdout.trim(),
                stderr.trim()
            ));
            Ok(output)
        }
        Err(e) => {
            crate::logger::log(&format!(
                "Failed to execute [{}]: {}",
                full_cmd_str, e
            ));
            Err(e)
        }
    }
}

/// Parses the output of `schtasks /Query` to determine if the task exists and is enabled.
pub fn parse_is_enabled_from_query_output(exit_success: bool, stdout: &str) -> bool {
    if !exit_success {
        return false;
    }
    let lower = stdout.to_ascii_lowercase();
    !lower.contains("disabled")
}

/// Checks whether the Estus autostart scheduled task is currently registered and enabled.
pub fn is_enabled() -> bool {
    match run_schtasks(&["/Query", "/TN", TASK_NAME]) {
        Ok(output) => parse_is_enabled_from_query_output(
            output.status.success(),
            &String::from_utf8_lossy(&output.stdout),
        ),
        Err(_) => false,
    }
}

/// Creates or updates the Estus scheduled task by generating an XML definition and importing it via `schtasks /XML`.
pub fn create_task_from_xml(exe_path: &str) -> Result<()> {
    let xml = generate_task_xml(exe_path);
    let temp_dir = std::env::temp_dir();
    let random_suffix: u128 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp_xml_path = temp_dir.join(format!(
        "estus_autostart_{}_{}.xml",
        std::process::id(),
        random_suffix
    ));

    let utf16_bytes = encode_utf16_le_bom(&xml);
    std::fs::write(&temp_xml_path, utf16_bytes)?;

    let temp_path_str = temp_xml_path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid temp XML path"))?;

    let result = run_schtasks(&["/Create", "/TN", TASK_NAME, "/XML", temp_path_str, "/F"]);

    // Always clean up the temporary XML file
    let _ = std::fs::remove_file(&temp_xml_path);

    let output = result?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("schtasks /Create /XML failed: {}", stderr.trim());
    }

    Ok(())
}

/// Fallback method to create the Estus scheduled task using direct `schtasks /Create` parameters.
pub fn create_task_direct(exe_path: &str) -> Result<()> {
    let tr_arg = format!("\"{}\" --minimized", exe_path);
    let output = run_schtasks(&[
        "/Create",
        "/TN",
        TASK_NAME,
        "/TR",
        &tr_arg,
        "/SC",
        "ONLOGON",
        "/RL",
        "HIGHEST",
        "/F",
    ])?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Direct schtasks creation failed: {}", stderr.trim());
    }

    crate::logger::log("Successfully configured autostart task via direct schtasks command");
    Ok(())
}

/// Enables autostart by registering a scheduled task pointing to the current executable with `--minimized`.
/// Attempts XML import first for full setting fidelity (e.g. battery, execution time limit),
/// and falls back to direct `schtasks /Create` if XML import fails.
pub fn enable() -> Result<()> {
    let current_exe = std::env::current_exe()?;
    let exe_path = current_exe
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Current executable path is not valid UTF-8"))?;

    match create_task_from_xml(exe_path) {
        Ok(()) => {
            crate::logger::log("Successfully configured autostart task via XML");
            Ok(())
        }
        Err(e) => {
            crate::logger::log(&format!(
                "Failed to configure autostart task via XML: {}. Falling back to direct schtasks command.",
                e
            ));
            create_task_direct(exe_path)
        }
    }
}

/// Disables autostart by removing the Estus scheduled task via `schtasks /Delete`.
pub fn disable() -> Result<()> {
    let output = run_schtasks(&["/Delete", "/TN", TASK_NAME, "/F"])?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // If the task does not exist, consider it already disabled without error
        if stderr.contains("cannot find") || stderr.contains("not find") {
            return Ok(());
        }
        anyhow::bail!("Failed to disable autostart task: {}", stderr.trim());
    }
    Ok(())
}

/// Retrieves the configured executable path from the scheduled task, if any.
pub fn get_task_command() -> Option<String> {
    if let Ok(output) = run_schtasks(&["/Query", "/TN", TASK_NAME, "/XML"])
        && output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(cmd) = extract_command_from_xml(&stdout) {
                return Some(cmd);
            }
        }

    if let Ok(output) = run_schtasks(&["/Query", "/TN", TASK_NAME, "/FO", "LIST", "/V"])
        && output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(cmd) = extract_command_from_list_output(&stdout) {
                return Some(cmd);
            }
        }

    None
}

/// If the scheduled task is enabled but points to a different executable path than `std::env::current_exe()`,
/// updates/recreates the scheduled task to point to the current executable.
/// Returns `Ok(true)` if the task was updated, `Ok(false)` if no change was needed or task was disabled.
pub fn sync_task_if_path_changed() -> Result<bool> {
    if !is_enabled() {
        return Ok(false);
    }

    let current_exe = std::env::current_exe()?;
    let current_exe_str = current_exe
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Current executable path is not valid UTF-8"))?;

    let configured_cmd = match get_task_command() {
        Some(cmd) => cmd,
        None => {
            crate::logger::log(
                "Autostart task is enabled but executable command could not be parsed; recreating task",
            );
            enable()?;
            return Ok(true);
        }
    };

    let needs_update = {
        let cur_canonical = std::fs::canonicalize(&current_exe).ok();
        let cfg_canonical = std::fs::canonicalize(&configured_cmd).ok();
        match (cur_canonical, cfg_canonical) {
            (Some(p1), Some(p2)) => p1 != p2,
            _ => !current_exe_str.eq_ignore_ascii_case(&configured_cmd),
        }
    };

    if needs_update {
        crate::logger::log(&format!(
            "Autostart task executable path changed from '{}' to '{}'; updating task",
            configured_cmd, current_exe_str
        ));
        enable()?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_and_unescape_xml() {
        let input = r#"C:\Program Files & Tools\Estus <App> "2.0" 'Beta' & More\estus.exe"#;
        let escaped = escape_xml(input);
        assert!(!escaped.contains('<'));
        assert!(!escaped.contains('>'));
        assert!(!escaped.contains('&') || escaped.contains("&amp;"));
        assert!(escaped.contains("&amp;"));
        assert!(escaped.contains("&lt;"));
        assert!(escaped.contains("&gt;"));
        assert!(escaped.contains("&quot;"));
        assert!(escaped.contains("&apos;"));

        let unescaped = unescape_xml(&escaped);
        assert_eq!(input, unescaped);
    }

    #[test]
    fn test_generate_task_xml_structure() {
        let exe = r#"C:\Program Files\Estus\estus.exe"#;
        let xml = generate_task_xml(exe);

        // Required elements
        assert!(xml.contains("<RunLevel>HighestAvailable</RunLevel>"));
        assert!(xml.contains("<LogonTrigger>"));
        assert!(xml.contains("<Enabled>true</Enabled>"));
        assert!(xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(xml.contains("<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>"));
        assert!(xml.contains("<StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>"));
        assert!(xml.contains("<LogonType>InteractiveToken</LogonType>"));
        assert!(xml.contains("<Command>C:\\Program Files\\Estus\\estus.exe</Command>"));
        assert!(xml.contains("<Arguments>--minimized</Arguments>"));
    }

    #[test]
    fn test_generate_task_xml_with_escaped_characters() {
        let exe = r#"C:\Apps & Tools <X>\estus.exe"#;
        let xml = generate_task_xml(exe);
        assert!(xml.contains("<Command>C:\\Apps &amp; Tools &lt;X&gt;\\estus.exe</Command>"));
    }

    #[test]
    fn test_encode_utf16_le_bom() {
        let text = "Hello Estus!";
        let bytes = encode_utf16_le_bom(text);
        assert_eq!(bytes[0], 0xFF);
        assert_eq!(bytes[1], 0xFE);

        let (chunks, _) = bytes[2..].as_chunks::<2>();
        let u16_slice: Vec<u16> = chunks
            .iter()
            .map(|chunk| u16::from_le_bytes(*chunk))
            .collect();
        let decoded = String::from_utf16(&u16_slice).unwrap();
        assert_eq!(decoded, text);
    }

    #[test]
    fn test_clean_exe_path() {
        assert_eq!(
            clean_exe_path(r#""C:\Program Files\Estus\estus.exe""#),
            r#"C:\Program Files\Estus\estus.exe"#
        );
        assert_eq!(
            clean_exe_path(r#"C:\Estus\estus.exe --minimized"#),
            r#"C:\Estus\estus.exe"#
        );
        assert_eq!(
            clean_exe_path(r#""C:\Program Files\Estus\estus.exe" --minimized"#),
            r#"C:\Program Files\Estus\estus.exe"#
        );
        assert_eq!(
            clean_exe_path(r#"C:\Estus\estus.exe"#),
            r#"C:\Estus\estus.exe"#
        );
    }

    #[test]
    fn test_extract_command_from_xml() {
        let sample_xml = r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Actions Context="Author">
    <Exec>
      <Command>C:\Program Files &amp; Tools\Estus\estus.exe</Command>
      <Arguments>--minimized</Arguments>
    </Exec>
  </Actions>
</Task>"#;
        let cmd = extract_command_from_xml(sample_xml);
        assert_eq!(
            cmd,
            Some(r#"C:\Program Files & Tools\Estus\estus.exe"#.to_string())
        );
    }

    #[test]
    fn test_extract_command_from_list_output() {
        let list_output = r#"
Folder: \
HostName:                             PC
TaskName:                             \Estus
Task To Run:                          "C:\Program Files\Estus\estus.exe" --minimized
Scheduled Task State:                 Enabled
"#;
        let cmd = extract_command_from_list_output(list_output);
        assert_eq!(
            cmd,
            Some(r#"C:\Program Files\Estus\estus.exe"#.to_string())
        );
    }

    #[test]
    fn test_parse_is_enabled() {
        assert!(!parse_is_enabled_from_query_output(false, "any error"));
        assert!(!parse_is_enabled_from_query_output(
            true,
            "Folder: \\\nStatus: Disabled"
        ));
        assert!(parse_is_enabled_from_query_output(
            true,
            "Folder: \\\nStatus: Ready"
        ));
        assert!(parse_is_enabled_from_query_output(
            true,
            "Folder: \\\nStatus: Running"
        ));
    }
}
