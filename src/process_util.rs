#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Windows creation flag to prevent a console window from being created for child processes.
pub const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Spawns a `std::process::Command` configured with `CREATE_NO_WINDOW` on Windows
/// so that child processes (e.g. smartctl, where.exe) do not flash a console window.
pub fn create_no_window_command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_no_window_command() {
        let cmd = create_no_window_command("where.exe");
        let program = cmd.get_program();
        assert_eq!(program, "where.exe");
    }
}
