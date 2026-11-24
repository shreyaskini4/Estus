//! Single-instance guard using a Windows named mutex and activation event.
//!
//! Ensures only one instance of Estus runs at a time. If an instance is already running,
//! the existing instance's window is brought to the foreground and the secondary process exits.

pub const DEFAULT_MUTEX_NAME: &str = "Local\\Estus_SingleInstance_Mutex_98a72b";
pub const DEFAULT_EVENT_NAME: &str = "Local\\Estus_SingleInstance_Event_98a72b";

#[cfg(windows)]
#[allow(clippy::upper_case_acronyms)]
mod win32 {
    use std::ffi::c_void;

    pub type HANDLE = *mut c_void;
    pub type HWND = *mut c_void;
    pub type BOOL = i32;

    pub const ERROR_ALREADY_EXISTS: u32 = 183;
    pub const SW_RESTORE: i32 = 9;
    pub const WAIT_OBJECT_0: u32 = 0;
    pub const EVENT_MODIFY_STATE: u32 = 0x0002;
    pub const SYNCHRONIZE: u32 = 0x0010_0000;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn CreateMutexW(
            lpMutexAttributes: *const c_void,
            bInitialOwner: BOOL,
            lpName: *const u16,
        ) -> HANDLE;

        pub fn GetLastError() -> u32;

        pub fn CloseHandle(hObject: HANDLE) -> BOOL;

        pub fn CreateEventW(
            lpEventAttributes: *const c_void,
            bManualReset: BOOL,
            bInitialState: BOOL,
            lpName: *const u16,
        ) -> HANDLE;

        pub fn OpenEventW(
            dwDesiredAccess: u32,
            bInheritHandle: BOOL,
            lpName: *const u16,
        ) -> HANDLE;

        pub fn SetEvent(hEvent: HANDLE) -> BOOL;

        pub fn WaitForSingleObject(hHandle: HANDLE, dwMilliseconds: u32) -> u32;
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        pub fn FindWindowW(lpClassName: *const u16, lpWindowName: *const u16) -> HWND;
        pub fn ShowWindow(hWnd: HWND, nCmdShow: i32) -> BOOL;
        pub fn SetForegroundWindow(hWnd: HWND) -> BOOL;
    }
}

/// Converts a UTF-8 string to a null-terminated UTF-16 wide string.
pub fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// RAII guard holding the single-instance named mutex and activation event handles.
#[derive(Debug)]
pub struct SingleInstanceGuard {
    #[cfg(windows)]
    mutex: win32::HANDLE,
    #[cfg(windows)]
    event: win32::HANDLE,
}

unsafe impl Send for SingleInstanceGuard {}
unsafe impl Sync for SingleInstanceGuard {}

impl SingleInstanceGuard {
    /// Waits up to `timeout_ms` for an activation event signaled by another instance.
    /// Returns `true` if an activation signal was received.
    pub fn wait_for_activation(&self, timeout_ms: u32) -> bool {
        #[cfg(windows)]
        {
            if self.event.is_null() {
                return false;
            }
            unsafe { win32::WaitForSingleObject(self.event, timeout_ms) == win32::WAIT_OBJECT_0 }
        }
        #[cfg(not(windows))]
        {
            let _ = timeout_ms;
            false
        }
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            if !self.mutex.is_null() {
                win32::CloseHandle(self.mutex);
                self.mutex = std::ptr::null_mut();
            }
            if !self.event.is_null() {
                win32::CloseHandle(self.event);
                self.event = std::ptr::null_mut();
            }
        }
    }
}

#[derive(Debug)]
pub enum SingleInstanceResult {
    Acquired(SingleInstanceGuard),
    AlreadyRunning,
}

/// Attempts to acquire the single-instance mutex and event.
pub fn try_acquire_single_instance(mutex_name: &str, event_name: &str) -> SingleInstanceResult {
    #[cfg(windows)]
    {
        let mutex_w = to_wide(mutex_name);
        let mutex = unsafe { win32::CreateMutexW(std::ptr::null(), 0, mutex_w.as_ptr()) };
        let err = unsafe { win32::GetLastError() };

        if err == win32::ERROR_ALREADY_EXISTS {
            if !mutex.is_null() {
                unsafe { win32::CloseHandle(mutex); }
            }
            return SingleInstanceResult::AlreadyRunning;
        }

        if mutex.is_null() {
            crate::logger::log_warn(&format!("Warning: CreateMutexW returned null, error={}", err));
        }

        let event_w = to_wide(event_name);
        let event = unsafe { win32::CreateEventW(std::ptr::null(), 0, 0, event_w.as_ptr()) };

        SingleInstanceResult::Acquired(SingleInstanceGuard { mutex, event })
    }

    #[cfg(not(windows))]
    {
        let _ = (mutex_name, event_name);
        SingleInstanceResult::Acquired(SingleInstanceGuard {})
    }
}

/// Signals the running instance to bring its window forward and activates its HWND.
pub fn activate_existing_instance(event_name: &str) {
    #[cfg(windows)]
    unsafe {
        // 1. Signal named event so the running instance unhides itself
        let event_w = to_wide(event_name);
        let h_event = win32::OpenEventW(win32::EVENT_MODIFY_STATE | win32::SYNCHRONIZE, 0, event_w.as_ptr());
        if !h_event.is_null() {
            win32::SetEvent(h_event);
            win32::CloseHandle(h_event);
        }

        // 2. Find existing window by title "Estus" and restore/focus it
        let title_w = to_wide("Estus");
        let hwnd = win32::FindWindowW(std::ptr::null(), title_w.as_ptr());
        if !hwnd.is_null() {
            win32::ShowWindow(hwnd, win32::SW_RESTORE);
            win32::SetForegroundWindow(hwnd);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = event_name;
    }
}

/// Acquires the single-instance guard. If another instance is already running,
/// brings it to foreground, logs the event, and exits the current process.
pub fn acquire_or_activate() -> SingleInstanceGuard {
    match try_acquire_single_instance(DEFAULT_MUTEX_NAME, DEFAULT_EVENT_NAME) {
        SingleInstanceResult::Acquired(guard) => guard,
        SingleInstanceResult::AlreadyRunning => {
            crate::logger::log_info("Another instance of Estus is already running. Bringing it forward and exiting.");
            activate_existing_instance(DEFAULT_EVENT_NAME);
            std::process::exit(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_wide() {
        let wide = to_wide("Estus");
        assert_eq!(wide, vec![69, 115, 116, 117, 115, 0]);
    }

    #[test]
    fn test_default_constants() {
        assert!(DEFAULT_MUTEX_NAME.starts_with("Local\\"));
        assert!(DEFAULT_EVENT_NAME.starts_with("Local\\"));
        assert_ne!(DEFAULT_MUTEX_NAME, DEFAULT_EVENT_NAME);
    }

    #[test]
    fn test_single_instance_acquisition_and_conflict() {
        let unique_mutex = format!("Local\\Estus_Test_Mutex_{}", std::process::id());
        let unique_event = format!("Local\\Estus_Test_Event_{}", std::process::id());

        let res1 = try_acquire_single_instance(&unique_mutex, &unique_event);
        assert!(matches!(res1, SingleInstanceResult::Acquired(_)));

        let res2 = try_acquire_single_instance(&unique_mutex, &unique_event);
        assert!(matches!(res2, SingleInstanceResult::AlreadyRunning));

        // When res1 is dropped, the mutex should become acquirable again
        drop(res1);

        let res3 = try_acquire_single_instance(&unique_mutex, &unique_event);
        assert!(matches!(res3, SingleInstanceResult::Acquired(_)));
    }

    #[test]
    fn test_event_signaling() {
        let unique_mutex = format!("Local\\Estus_EventTest_Mutex_{}", std::process::id());
        let unique_event = format!("Local\\Estus_EventTest_Event_{}", std::process::id());

        if let SingleInstanceResult::Acquired(guard) = try_acquire_single_instance(&unique_mutex, &unique_event) {
            // Initially event should not be signaled (wait with 10ms timeout returns false)
            assert!(!guard.wait_for_activation(10));

            // Signal existing instance
            activate_existing_instance(&unique_event);

            // Now wait_for_activation should return true
            assert!(guard.wait_for_activation(500));

            // Auto-reset: next wait should return false
            assert!(!guard.wait_for_activation(10));
        } else {
            panic!("Expected to acquire single instance guard");
        }
    }
}
