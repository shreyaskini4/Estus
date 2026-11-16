use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::smart::{SmartError, SmartSnapshot};

/// Application configuration and persistence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    pub alert_threshold_gb: f64,
    pub start_with_windows: bool,
    pub last_notified_date: Option<NaiveDate>,
    #[serde(default)]
    pub custom_smartctl_path: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            alert_threshold_gb: 100.0,
            start_with_windows: false,
            last_notified_date: None,
            custom_smartctl_path: None,
        }
    }
}

/// Returns the path to `settings.json` within the system config directory.
pub fn settings_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "estus", "Estus")
        .map(|dirs| dirs.config_dir().join("settings.json"))
}

impl Settings {
    /// Loads settings from standard `settings.json`. Returns default if missing or corrupted.
    pub fn load() -> Self {
        if let Some(path) = settings_path() {
            Self::load_from_path(&path)
        } else {
            Self::default()
        }
    }

    /// Loads settings from a specified file path. Returns default on error or nonexistence.
    pub fn load_from_path(path: &Path) -> Self {
        if !path.exists() {
            return Self::default();
        }

        match std::fs::read_to_string(path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Persists settings to standard `settings.json`.
    pub fn save(&self) -> Result<()> {
        let path = settings_path()
            .ok_or_else(|| anyhow::anyhow!("Unable to determine configuration directory"))?;
        self.save_to_path(&path)
    }

    /// Persists settings to a specific file path.
    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Applies Windows startup configuration using `auto_launch`.
    pub fn apply_auto_launch(&self) -> Result<()> {
        let current_exe = std::env::current_exe()?;
        let app_path = current_exe
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Invalid executable path"))?;

        let auto = auto_launch::AutoLaunchBuilder::new()
            .set_app_name("Estus")
            .set_app_path(app_path)
            .build()?;

        if self.start_with_windows {
            auto.enable()?;
        } else {
            auto.disable()?;
        }
        Ok(())
    }
}

/// Global shared state accessible by UI and background worker threads.
#[derive(Debug, Clone)]
pub struct SharedState {
    pub latest_smart: Option<SmartSnapshot>,
    pub smart_error: Option<SmartError>,
    pub daily_writes: Vec<(NaiveDate, u64)>,
    pub top_processes_today: Vec<(String, u64)>,
    pub top_processes_7d: Vec<(String, u64)>,
    pub top_processes_30d: Vec<(String, u64)>,
    pub settings: Settings,
    pub data_changed: bool,
    pub show_window: bool,
    pub should_quit: bool,
    pub resolved_smartctl_path: Option<String>,
    pub last_raw_error: Option<String>,
    pub active_device: Option<String>,
}

impl SharedState {
    pub fn new(settings: Settings) -> Self {
        Self {
            latest_smart: None,
            smart_error: None,
            daily_writes: Vec::new(),
            top_processes_today: Vec::new(),
            top_processes_7d: Vec::new(),
            top_processes_30d: Vec::new(),
            settings,
            data_changed: false,
            show_window: true,
            should_quit: false,
            resolved_smartctl_path: None,
            last_raw_error: None,
            active_device: None,
        }
    }
}

pub type AppState = Arc<Mutex<SharedState>>;

/// Creates a new thread-safe `AppState` wrapper around `SharedState`.
pub fn new_app_state(settings: Settings) -> AppState {
    Arc::new(Mutex::new(SharedState::new(settings)))
}

/// Triggers a Windows toast notification when daily write threshold is exceeded.
pub fn fire_notification(today_gb: f64, threshold_gb: f64) -> Result<()> {
    notify_rust::Notification::new()
        .appname("Estus")
        .summary("SSD Write Alert")
        .body(&format!(
            "Today's SSD writes ({:.1} GB) exceeded your daily threshold of {:.1} GB.",
            today_gb, threshold_gb
        ))
        .show()?;
    Ok(())
}

/// Checks if today's writes exceed the configured threshold, sending at most one notification per day.
pub fn check_and_send_alert(
    state: &AppState,
    today_bytes: u64,
    today_date: NaiveDate,
) -> Result<bool> {
    let mut state_guard = state
        .lock()
        .map_err(|e| anyhow::anyhow!("State mutex poisoned: {e}"))?;

    let threshold_gb = state_guard.settings.alert_threshold_gb;
    if crate::analytics::is_daily_threshold_exceeded(today_bytes, threshold_gb)
        && state_guard.settings.last_notified_date != Some(today_date) {
            let today_gb = today_bytes as f64 / crate::analytics::BYTES_PER_GB;
            if let Err(e) = fire_notification(today_gb, threshold_gb) {
                eprintln!("Failed to display notification: {e}");
            }
            state_guard.settings.last_notified_date = Some(today_date);
            if let Err(e) = state_guard.settings.save() {
                eprintln!("Failed to save settings with updated notification date: {e}");
            }
            return Ok(true);
        }
    Ok(false)
}

/// Initializes the system tray icon with "Show" and "Quit" menu entries.
pub fn init_tray() -> Result<(tray_icon::TrayIcon, tray_icon::menu::MenuId, tray_icon::menu::MenuId)> {
    let menu = tray_icon::menu::Menu::new();
    let show_item = tray_icon::menu::MenuItem::new("Show", true, None);
    let quit_item = tray_icon::menu::MenuItem::new("Quit", true, None);

    let show_id = show_item.id().clone();
    let quit_id = quit_item.id().clone();

    menu.append(&show_item)?;
    menu.append(&quit_item)?;

    // Construct a standard 16x16 icon in memory
    let width = 16u32;
    let height = 16u32;
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            if x == 0 || x == width - 1 || y == 0 || y == height - 1 {
                rgba.extend_from_slice(&[40, 40, 40, 255]); // border
            } else if (4..=11).contains(&y) && (3..=12).contains(&x) {
                rgba.extend_from_slice(&[41, 128, 185, 255]); // drive body
            } else {
                rgba.extend_from_slice(&[220, 220, 220, 255]); // background
            }
        }
    }
    let icon = tray_icon::Icon::from_rgba(rgba, width, height)
        .map_err(|e| anyhow::anyhow!("Failed to construct tray icon: {e}"))?;

    let tray = tray_icon::TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Estus")
        .with_icon(icon)
        .build()?;

    Ok((tray, show_id, quit_id))
}

/// Processes incoming tray icon and context menu events.
pub fn handle_tray_events(
    state: &AppState,
    show_id: &tray_icon::menu::MenuId,
    quit_id: &tray_icon::menu::MenuId,
) {
    while let Ok(event) = tray_icon::menu::MenuEvent::receiver().try_recv() {
        if event.id == *show_id {
            if let Ok(mut s) = state.lock() {
                s.show_window = true;
            }
        } else if event.id == *quit_id
            && let Ok(mut s) = state.lock() {
                s.should_quit = true;
            }
    }

    while let Ok(event) = tray_icon::TrayIconEvent::receiver().try_recv() {
        match event {
            tray_icon::TrayIconEvent::DoubleClick { .. } => {
                if let Ok(mut s) = state.lock() {
                    s.show_window = true;
                }
            }
            tray_icon::TrayIconEvent::Click {
                button: tray_icon::MouseButton::Left,
                button_state: tray_icon::MouseButtonState::Up,
                ..
            } => {
                if let Ok(mut s) = state.lock() {
                    s.show_window = true;
                }
            }
            _ => {}
        }
    }
}

/// Performs startup initialization: prunes old records and pre-loads recent historical metrics from DB.
pub fn initialize_database_and_state(state: &AppState) -> Result<()> {
    let conn = crate::db::init_db()?;

    // Prune records older than 90 days
    if let Err(e) = crate::db::prune_old_records(&conn, 90) {
        eprintln!("Warning: Failed to prune old database records: {e}");
    }

    // Pre-populate state from existing database entries
    let latest_snapshot = crate::db::latest_snapshot(&conn)?;
    let daily_writes = crate::db::daily_writes(&conn, 30)?;

    let now_ts = chrono::Utc::now().timestamp();
    let today = Local::now().date_naive();
    let today_start_ts = today
        .and_hms_opt(0, 0, 0)
        .and_then(|t| t.and_local_timezone(Local).single())
        .map(|dt| dt.timestamp())
        .unwrap_or(now_ts - 86_400);

    let top_today = crate::db::top_processes(&conn, today_start_ts, 10)?;
    let top_7d = crate::db::top_processes(&conn, now_ts - 7 * 86_400, 10)?;
    let top_30d = crate::db::top_processes(&conn, now_ts - 30 * 86_400, 10)?;

    if let Ok(mut s) = state.lock() {
        if latest_snapshot.is_some() {
            s.latest_smart = latest_snapshot;
        }
        s.daily_writes = daily_writes;
        s.top_processes_today = top_today;
        s.top_processes_7d = top_7d;
        s.top_processes_30d = top_30d;
        s.data_changed = true;
    }

    Ok(())
}

/// Spawns the SMART Poller thread. Runs immediately, then every 10 minutes.
pub fn spawn_smart_poller(state: AppState) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("estus-smart-poller".to_string())
        .spawn(move || {
            loop {
                // Poll SMART
                let custom_path = if let Ok(s) = state.lock() {
                    s.settings.custom_smartctl_path.clone()
                } else {
                    None
                };

                match crate::smart::run_smartctl_with_config(custom_path.as_deref(), None) {
                    Ok((snapshot, resolved_path, active_dev)) => {
                        if let Ok(conn) = crate::db::init_db() {
                            let _ = crate::db::insert_snapshot(&conn, &snapshot);
                        }
                        if let Ok(mut s) = state.lock() {
                            s.latest_smart = Some(snapshot);
                            s.smart_error = None;
                            s.resolved_smartctl_path = Some(resolved_path);
                            s.active_device = Some(active_dev);
                            s.last_raw_error = None;
                            s.data_changed = true;
                        }
                    }
                    Err(err) => {
                        let raw_err = err.to_string();
                        let resolved = crate::smart::get_or_resolve_smartctl_path(custom_path.as_deref())
                            .map(|p| p.display().to_string());
                        if let Ok(mut s) = state.lock() {
                            s.smart_error = Some(err);
                            s.resolved_smartctl_path = resolved;
                            s.last_raw_error = Some(raw_err);
                            s.data_changed = true;
                        }
                    }
                }

                // Sleep for 10 minutes (600s), checking should_quit every 1 second
                for _ in 0..600 {
                    if let Ok(s) = state.lock()
                        && s.should_quit {
                            return;
                        }
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        })
        .expect("Failed to spawn smart poller thread")
}

/// Spawns the Process Poller thread. Runs every 30 seconds.
pub fn spawn_procs_poller(state: AppState) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("estus-procs-poller".to_string())
        .spawn(move || {
            let mut collector = crate::procs::ProcCollector::new();

            loop {
                // Sleep for 30s in 1s increments, checking should_quit
                for _ in 0..30 {
                    if let Ok(s) = state.lock()
                        && s.should_quit {
                            return;
                        }
                    std::thread::sleep(Duration::from_secs(1));
                }

                if let Ok(s) = state.lock()
                    && s.should_quit {
                        return;
                    }

                // Collect process deltas and record to DB
                let deltas = collector.poll_deltas();
                let now_ts = chrono::Utc::now().timestamp();

                if let Ok(conn) = crate::db::init_db() {
                    if !deltas.is_empty() {
                        let _ = crate::db::insert_process_deltas(&conn, now_ts, &deltas);
                    }

                    // Recalculate daily writes and top processes
                    let daily = crate::db::daily_writes(&conn, 30).unwrap_or_default();
                    let today = Local::now().date_naive();
                    let today_start_ts = today
                        .and_hms_opt(0, 0, 0)
                        .and_then(|t| t.and_local_timezone(Local).single())
                        .map(|dt| dt.timestamp())
                        .unwrap_or(now_ts - 86_400);

                    let top_today =
                        crate::db::top_processes(&conn, today_start_ts, 10).unwrap_or_default();
                    let top_7d =
                        crate::db::top_processes(&conn, now_ts - 7 * 86_400, 10).unwrap_or_default();
                    let top_30d =
                        crate::db::top_processes(&conn, now_ts - 30 * 86_400, 10).unwrap_or_default();

                    // Calculate today's total written bytes
                    let today_bytes = daily
                        .iter()
                        .find(|(d, _)| *d == today)
                        .map(|(_, b)| *b)
                        .unwrap_or(0);

                    // Check notification threshold
                    let _ = check_and_send_alert(&state, today_bytes, today);

                    // Refresh state
                    if let Ok(mut s) = state.lock() {
                        s.daily_writes = daily;
                        s.top_processes_today = top_today;
                        s.top_processes_7d = top_7d;
                        s.top_processes_30d = top_30d;
                        s.data_changed = true;
                    }
                }
            }
        })
        .expect("Failed to spawn procs poller thread")
}

/// Spawns both poller threads and performs startup pruning / state initialization.
pub fn start_background_threads(
    state: AppState,
) -> (std::thread::JoinHandle<()>, std::thread::JoinHandle<()>) {
    let _ = initialize_database_and_state(&state);
    let smart_handle = spawn_smart_poller(Arc::clone(&state));
    let procs_handle = spawn_procs_poller(Arc::clone(&state));
    (smart_handle, procs_handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_default() {
        let s = Settings::default();
        assert!((s.alert_threshold_gb - 100.0).abs() < 1e-6);
        assert!(!s.start_with_windows);
        assert_eq!(s.last_notified_date, None);
    }

    #[test]
    fn test_settings_serialization() {
        let date = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        let s = Settings {
            alert_threshold_gb: 150.5,
            start_with_windows: true,
            last_notified_date: Some(date),
            custom_smartctl_path: None,
        };

        let json = serde_json::to_string(&s).expect("Serialization failed");
        let decoded: Settings = serde_json::from_str(&json).expect("Deserialization failed");

        assert_eq!(s, decoded);
    }

    #[test]
    fn test_settings_save_and_load_from_path() {
        let temp_dir = std::env::temp_dir().join(format!("estus_test_{}", std::process::id()));
        let file_path = temp_dir.join("test_settings.json");

        let date = NaiveDate::from_ymd_opt(2026, 5, 20).unwrap();
        let s = Settings {
            alert_threshold_gb: 75.0,
            start_with_windows: true,
            last_notified_date: Some(date),
            custom_smartctl_path: None,
        };

        s.save_to_path(&file_path).expect("Failed to save settings");
        assert!(file_path.exists());

        let loaded = Settings::load_from_path(&file_path);
        assert_eq!(s, loaded);

        // Cleanup
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_settings_load_corrupted_file() {
        let temp_dir = std::env::temp_dir().join(format!("estus_test_corrupt_{}", std::process::id()));
        let file_path = temp_dir.join("corrupt_settings.json");

        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::write(&file_path, "not valid json {{{").unwrap();

        let loaded = Settings::load_from_path(&file_path);
        assert_eq!(loaded, Settings::default());

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_settings_load_nonexistent_file() {
        let nonexistent = PathBuf::from("Z:\\nonexistent_dir_12345\\settings.json");
        let loaded = Settings::load_from_path(&nonexistent);
        assert_eq!(loaded, Settings::default());
    }

    #[test]
    fn test_shared_state_initialization() {
        let s = Settings {
            alert_threshold_gb: 80.0,
            start_with_windows: false,
            last_notified_date: None,
            custom_smartctl_path: None,
        };
        let state = SharedState::new(s);

        assert!(state.latest_smart.is_none());
        assert!(state.smart_error.is_none());
        assert!(state.daily_writes.is_empty());
        assert!(state.top_processes_today.is_empty());
        assert!(state.top_processes_7d.is_empty());
        assert!(state.top_processes_30d.is_empty());
        assert!(!state.data_changed);
        assert!(state.show_window);
        assert!(!state.should_quit);
    }

    #[test]
    fn test_check_and_send_alert_threshold_logic() {
        let settings = Settings {
            alert_threshold_gb: 50.0,
            start_with_windows: false,
            last_notified_date: None,
            custom_smartctl_path: None,
        };
        let state = new_app_state(settings);
        let date = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();

        // 40 GB: under threshold -> false
        let under = check_and_send_alert(&state, 40_000_000_000, date).unwrap();
        assert!(!under);
        assert_eq!(state.lock().unwrap().settings.last_notified_date, None);

        // 60 GB: over threshold -> true (notified date updated)
        let over = check_and_send_alert(&state, 60_000_000_000, date).unwrap();
        assert!(over);
        assert_eq!(state.lock().unwrap().settings.last_notified_date, Some(date));

        // Another check on the same day: already notified today -> false
        let again = check_and_send_alert(&state, 70_000_000_000, date).unwrap();
        assert!(!again);
    }
}
