use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::smart::{DriveInfo, SmartError, SmartSnapshot};

/// Configuration options for an individual drive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DriveConfig {
    pub nickname: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Application configuration and persistence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    pub alert_threshold_gb: f64,
    pub start_with_windows: bool,
    pub last_notified_date: Option<NaiveDate>,
    #[serde(default)]
    pub custom_smartctl_path: Option<String>,
    #[serde(default)]
    pub drives: std::collections::HashMap<String, DriveConfig>,
    #[serde(default)]
    pub verbose_logging: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            alert_threshold_gb: 100.0,
            start_with_windows: false,
            last_notified_date: None,
            custom_smartctl_path: None,
            drives: std::collections::HashMap::new(),
            verbose_logging: false,
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
            let def = Self::default();
            crate::logger::set_verbose_logging(def.verbose_logging);
            def
        }
    }

    /// Loads settings from a specified file path. Returns default on error or nonexistence.
    pub fn load_from_path(path: &Path) -> Self {
        if !path.exists() {
            let def = Self::default();
            crate::logger::set_verbose_logging(def.verbose_logging);
            return def;
        }

        let settings: Self = match std::fs::read_to_string(path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        };
        crate::logger::set_verbose_logging(settings.verbose_logging);
        settings
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
        crate::logger::set_verbose_logging(self.verbose_logging);
        Ok(())
    }

    /// Applies Windows startup configuration using scheduled-task autostart.
    pub fn apply_autostart(&self) -> Result<()> {
        if self.start_with_windows {
            crate::autostart::enable()?;
        } else {
            crate::autostart::disable()?;
        }
        Ok(())
    }

    /// Backwards-compatible alias for `apply_autostart`.
    pub fn apply_auto_launch(&self) -> Result<()> {
        self.apply_autostart()
    }
}

/// Global shared state accessible by UI and background worker threads.
#[derive(Debug, Clone)]
pub struct SharedState {
    pub detected_drives: Vec<DriveInfo>,
    pub selected_drive_id: Option<String>,
    pub per_drive_snapshots: std::collections::HashMap<String, SmartSnapshot>,
    pub per_drive_daily_writes: std::collections::HashMap<String, Vec<crate::db::DailyWrites>>,
    pub per_drive_intervals: std::collections::HashMap<String, Vec<crate::db::SnapshotInterval>>,
    pub latest_smart: Option<SmartSnapshot>,
    pub smart_error: Option<SmartError>,
    pub daily_writes: Vec<crate::db::DailyWrites>,
    pub intervals_24h: Vec<crate::db::SnapshotInterval>,
    pub top_processes_today: Vec<(String, u64)>,
    pub top_processes_7d: Vec<(String, u64)>,
    pub top_processes_30d: Vec<(String, u64)>,
    pub latest_ssd_delta_bytes: Option<u64>,
    pub settings: Settings,
    pub data_changed: bool,
    pub show_window: bool,
    pub should_quit: bool,
    pub resolved_smartctl_path: Option<String>,
    pub smartctl_version: Option<String>,
    pub last_raw_error: Option<String>,
    pub last_raw_output: Option<String>,
    pub active_device: Option<String>,
    pub rescan_requested: bool,
    pub is_logon_startup: bool,
    pub initialization_error: Option<String>,
}

impl SharedState {
    pub fn new(settings: Settings) -> Self {
        crate::logger::set_verbose_logging(settings.verbose_logging);
        Self {
            detected_drives: Vec::new(),
            selected_drive_id: None,
            per_drive_snapshots: std::collections::HashMap::new(),
            per_drive_daily_writes: std::collections::HashMap::new(),
            per_drive_intervals: std::collections::HashMap::new(),
            latest_smart: None,
            smart_error: None,
            daily_writes: Vec::new(),
            intervals_24h: Vec::new(),
            top_processes_today: Vec::new(),
            top_processes_7d: Vec::new(),
            top_processes_30d: Vec::new(),
            latest_ssd_delta_bytes: None,
            settings,
            data_changed: false,
            show_window: true,
            should_quit: false,
            resolved_smartctl_path: None,
            smartctl_version: None,
            last_raw_error: None,
            last_raw_output: None,
            active_device: None,
            rescan_requested: false,
            is_logon_startup: false,
            initialization_error: None,
        }
    }

    pub fn get_selected_drive(&self) -> Option<&DriveInfo> {
        self.selected_drive_id
            .as_ref()
            .and_then(|id| self.detected_drives.iter().find(|d| &d.id == id))
    }

    pub fn get_selected_snapshot(&self) -> Option<&SmartSnapshot> {
        if let Some(ref id) = self.selected_drive_id {
            self.per_drive_snapshots.get(id)
        } else {
            self.latest_smart.as_ref()
        }
    }

    pub fn get_selected_daily_writes(&self) -> &[crate::db::DailyWrites] {
        if let Some(ref id) = self.selected_drive_id {
            self.per_drive_daily_writes
                .get(id)
                .map(|v| v.as_slice())
                .unwrap_or(&[])
        } else {
            &self.daily_writes
        }
    }

    pub fn get_selected_intervals_24h(&self) -> &[crate::db::SnapshotInterval] {
        if let Some(ref id) = self.selected_drive_id {
            self.per_drive_intervals
                .get(id)
                .map(|v| v.as_slice())
                .unwrap_or(&[])
        } else {
            &self.intervals_24h
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

/// Triggers a notification for a specific drive exceeding its threshold.
pub fn fire_drive_notification(drive_name: &str, today_gb: f64, threshold_gb: f64) -> Result<()> {
    notify_rust::Notification::new()
        .appname("Estus")
        .summary("SSD Write Alert")
        .body(&format!(
            "Today's writes on {} ({:.1} GB) exceeded daily threshold of {:.1} GB.",
            drive_name, today_gb, threshold_gb
        ))
        .show()?;
    Ok(())
}

/// Checks if a drive's today writes exceed the threshold, sending at most one notification per day.
pub fn check_and_send_drive_alert(
    state: &AppState,
    _drive_id: &str,
    drive_name: &str,
    today_bytes: u64,
    today_date: NaiveDate,
) -> Result<bool> {
    let mut state_guard = state
        .lock()
        .map_err(|e| anyhow::anyhow!("State mutex poisoned: {e}"))?;

    let threshold_gb = state_guard.settings.alert_threshold_gb;
    if crate::analytics::is_daily_threshold_exceeded(today_bytes, threshold_gb)
        && state_guard.settings.last_notified_date != Some(today_date)
    {
        let today_gb = today_bytes as f64 / crate::analytics::BYTES_PER_GB;
        if let Err(e) = fire_drive_notification(drive_name, today_gb, threshold_gb) {
            crate::logger::log_warn(&format!("Failed to display notification: {e}"));
        }
        state_guard.settings.last_notified_date = Some(today_date);
        if let Err(e) = state_guard.settings.save() {
            crate::logger::log_error(&format!("Failed to save settings with updated notification date: {e}"));
        }
        return Ok(true);
    }
    Ok(false)
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
                crate::logger::log_warn(&format!("Failed to display notification: {e}"));
            }
            state_guard.settings.last_notified_date = Some(today_date);
            if let Err(e) = state_guard.settings.save() {
                crate::logger::log_error(&format!("Failed to save settings with updated notification date: {e}"));
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
/// Returns true if a "Show" event was triggered.
pub fn handle_tray_events(
    state: &AppState,
    show_id: &tray_icon::menu::MenuId,
    quit_id: &tray_icon::menu::MenuId,
) -> bool {
    let mut show_requested = false;

    while let Ok(event) = tray_icon::menu::MenuEvent::receiver().try_recv() {
        if event.id == *show_id {
            crate::logger::log_info("Tray menu: Show requested");
            if let Ok(mut s) = state.lock() {
                s.show_window = true;
            }
            show_requested = true;
        } else if event.id == *quit_id {
            crate::logger::log_info("Tray menu: Quit clicked. Terminating application.");
            take_exit_snapshot(state);
            if let Ok(mut s) = state.lock() {
                s.should_quit = true;
            }
            std::process::exit(0);
        }
    }

    while let Ok(event) = tray_icon::TrayIconEvent::receiver().try_recv() {
        match event {
            tray_icon::TrayIconEvent::DoubleClick { .. } => {
                crate::logger::log_info("Tray icon double-clicked: showing window");
                if let Ok(mut s) = state.lock() {
                    s.show_window = true;
                }
                show_requested = true;
            }
            tray_icon::TrayIconEvent::Click {
                button: tray_icon::MouseButton::Left,
                ..
            } => {
                crate::logger::log_info("Tray icon left-clicked: showing window");
                if let Ok(mut s) = state.lock() {
                    s.show_window = true;
                }
                show_requested = true;
            }
            _ => {}
        }
    }

    show_requested
}

/// Ensures that the platform-specific data and configuration directories exist.
pub fn ensure_app_directories() -> Result<(PathBuf, PathBuf)> {
    let proj_dirs = directories::ProjectDirs::from("com", "estus", "Estus")
        .ok_or_else(|| anyhow::anyhow!("Failed to determine project directories"))?;
    let data_dir = proj_dirs.data_local_dir().to_path_buf();
    let config_dir = proj_dirs.config_dir().to_path_buf();

    std::fs::create_dir_all(&data_dir)
        .map_err(|e| anyhow::anyhow!("Failed to create data directory ({}): {e}", data_dir.display()))?;
    std::fs::create_dir_all(&config_dir)
        .map_err(|e| anyhow::anyhow!("Failed to create config directory ({}): {e}", config_dir.display()))?;

    Ok((data_dir, config_dir))
}

/// Initializes app environment directories on startup.
/// If directory creation fails, logs an error to `estus.log` and stores an initialization error in `AppState`.
pub fn init_app_environment(state: &AppState) {
    if let Err(e) = ensure_app_directories() {
        let err_msg = format!("Failed to create application directories: {e}");
        crate::logger::log_error(&err_msg);
        if let Ok(mut s) = state.lock() {
            s.initialization_error = Some(err_msg);
            s.data_changed = true;
        }
    }
}

/// Performs startup initialization: prunes old records and pre-loads recent historical metrics from DB.
pub fn initialize_database_and_state(state: &AppState) -> Result<()> {
    let conn = match crate::db::init_db() {
        Ok(c) => c,
        Err(e) => {
            let err_msg = format!("Failed to initialize database: {e}");
            crate::logger::log_error(&err_msg);
            if let Ok(mut s) = state.lock()
                && s.initialization_error.is_none()
            {
                s.initialization_error = Some(err_msg);
                s.data_changed = true;
            }
            return Err(e);
        }
    };

    if let Err(e) = crate::db::prune_old_records(&conn, 90) {
        crate::logger::log_warn(&format!("Warning: Failed to prune old database records: {e}"));
    }

    let drives = crate::db::get_drives(&conn)?;
    let mut per_drive_snaps = std::collections::HashMap::new();
    let mut per_drive_writes = std::collections::HashMap::new();
    let mut per_drive_intervals = std::collections::HashMap::new();
    let mut first_snap = None;

    for drive in &drives {
        if let Ok(Some(snap)) = crate::db::latest_snapshot(&conn, &drive.id) {
            if first_snap.is_none() {
                first_snap = Some(snap.clone());
            }
            per_drive_snaps.insert(drive.id.clone(), snap);
        }
        if let Ok(daily) = crate::db::daily_writes(&conn, Some(&drive.id), 30) {
            per_drive_writes.insert(drive.id.clone(), daily);
        }
        if let Ok(intervals) = crate::db::get_intervals_24h(&conn, Some(&drive.id)) {
            per_drive_intervals.insert(drive.id.clone(), intervals);
        }
    }

    let daily_writes = crate::db::daily_writes(&conn, None, 30)?;
    let intervals_24h = crate::db::get_intervals_24h(&conn, None).unwrap_or_default();

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
        s.detected_drives = drives;
        s.per_drive_snapshots = per_drive_snaps;
        s.per_drive_daily_writes = per_drive_writes;
        s.per_drive_intervals = per_drive_intervals;
        s.latest_smart = first_snap;
        s.daily_writes = daily_writes;
        s.intervals_24h = intervals_24h;
        s.top_processes_today = top_today;
        s.top_processes_7d = top_7d;
        s.top_processes_30d = top_30d;
        s.data_changed = true;
    }

    Ok(())
}

pub const LOGON_INITIAL_DELAY_SECS: u64 = 20;
pub const LOGON_RETRY_INTERVAL_SECS: u64 = 5;
pub const LOGON_MAX_RETRIES: usize = 3;

/// Executes a complete SMART scan and query cycle for all detected and enabled drives.
/// Returns `true` if drives were successfully discovered and queried, `false` otherwise.
pub fn poll_smart_cycle(state: &AppState) -> bool {
    crate::logger::log_debug("Beginning routine SMART polling cycle");
    let (custom_path, drives_config) = if let Ok(s) = state.lock() {
        (
            s.settings.custom_smartctl_path.clone(),
            s.settings.drives.clone(),
        )
    } else {
        (None, std::collections::HashMap::new())
    };

    let resolved_exe = crate::smart::resolve_smartctl(custom_path.as_deref());
    let version = resolved_exe.as_ref().and_then(|p| crate::smart::get_smartctl_version(p));
    let raw_out = crate::smart::get_last_raw_output();

    let exe = match resolved_exe {
        Some(p) => p,
        None => {
            let err = SmartError::SmartctlNotFound(
                "smartctl was not found in settings, PATH, %ProgramFiles%\\smartmontools\\bin, %ProgramFiles(x86)%\\smartmontools\\bin, or Windows Registry".to_string(),
            );
            if let Ok(mut s) = state.lock() {
                s.smart_error = Some(err);
                s.resolved_smartctl_path = None;
                s.smartctl_version = None;
                s.last_raw_output = raw_out;
                s.data_changed = true;
            }
            return false;
        }
    };

    let resolved_path_str = exe.display().to_string();

    match crate::smart::discover_and_query_all_drives(custom_path.as_deref()) {
        Ok(discovered) => {
            let conn_res = crate::db::init_db();
            let today = chrono::Local::now().date_naive();
            let mut per_drive_snaps = std::collections::HashMap::new();
            let mut per_drive_writes = std::collections::HashMap::new();
            let mut per_drive_intervals = std::collections::HashMap::new();
            let mut active_drives = Vec::new();
            let mut cycle_ssd_delta: u64 = 0;

            for (drive_info, snapshot) in &discovered {
                let mut info = drive_info.clone();
                if let Some(cfg) = drives_config.get(&info.id) {
                    info.nickname = cfg.nickname.clone();
                    info.enabled = cfg.enabled;
                }

                if let Ok(ref conn) = conn_res {
                    let _ = crate::db::upsert_drive(conn, &info);
                    if info.enabled {
                        if let Ok(Some(prev)) = crate::db::latest_snapshot(conn, &info.id) {
                            let elapsed = snapshot.ts.saturating_sub(prev.ts);
                            if snapshot.data_units_written >= prev.data_units_written {
                                let delta_units = snapshot.data_units_written - prev.data_units_written;
                                let delta_bytes = delta_units.saturating_mul(512_000);
                                if delta_bytes > 5_000_000_000 {
                                    let delta_gb = delta_bytes as f64 / 1_000_000_000.0;
                                    crate::logger::log_info(&format!(
                                        "High write volume detected in single interval for drive '{}': delta = {:.2} GB, previous raw = {}, current raw = {}, interval = {}s",
                                        info.id, delta_gb, prev.data_units_written, snapshot.data_units_written, elapsed
                                    ));
                                }
                                if elapsed <= crate::db::GAP_THRESHOLD_SECS {
                                    cycle_ssd_delta = cycle_ssd_delta.saturating_add(delta_bytes);
                                }
                            }
                        }

                        let _ = crate::db::insert_snapshot(conn, &info.id, snapshot);
                        let daily = crate::db::daily_writes(conn, Some(&info.id), 30).unwrap_or_default();
                        let intervals = crate::db::get_intervals_24h(conn, Some(&info.id)).unwrap_or_default();

                        let today_bytes = daily
                            .iter()
                            .find(|dw| dw.date == today)
                            .map(|dw| dw.total_bytes())
                            .unwrap_or(0);

                        let drive_label = info
                            .nickname
                            .as_deref()
                            .unwrap_or(&info.model);
                        let _ = check_and_send_drive_alert(
                            state,
                            &info.id,
                            drive_label,
                            today_bytes,
                            today,
                        );

                        per_drive_writes.insert(info.id.clone(), daily);
                        per_drive_intervals.insert(info.id.clone(), intervals);
                        per_drive_snaps.insert(info.id.clone(), snapshot.clone());
                    }
                }
                active_drives.push(info);
            }

            let (all_db_drives, aggregated_daily, aggregated_intervals) = if let Ok(ref conn) = conn_res {
                let d = crate::db::get_drives(conn).unwrap_or_else(|_| active_drives.clone());
                let agg = crate::db::daily_writes(conn, None, 30).unwrap_or_default();
                let agg_int = crate::db::get_intervals_24h(conn, None).unwrap_or_default();
                (d, agg, agg_int)
            } else {
                (active_drives, Vec::new(), Vec::new())
            };

            let first_snap = per_drive_snaps.values().next().cloned();
            let active_dev = all_db_drives.first().map(|d| d.device_path.clone());
            let has_drives = !discovered.is_empty();

            crate::logger::log_debug(&format!(
                "Routine SMART polling cycle completed: {} drive(s) queried",
                discovered.len()
            ));

            if let Ok(mut s) = state.lock() {
                s.latest_ssd_delta_bytes = Some(cycle_ssd_delta);
                s.detected_drives = all_db_drives;
                s.per_drive_snapshots = per_drive_snaps;
                s.per_drive_daily_writes = per_drive_writes;
                s.per_drive_intervals = per_drive_intervals;
                s.daily_writes = aggregated_daily;
                s.intervals_24h = aggregated_intervals;
                if s.latest_smart.is_none() || first_snap.is_some() {
                    s.latest_smart = first_snap;
                }
                s.smart_error = None;
                s.resolved_smartctl_path = Some(resolved_path_str);
                s.smartctl_version = version;
                s.last_raw_output = crate::smart::get_last_raw_output();
                s.active_device = active_dev;
                s.last_raw_error = None;
                s.data_changed = true;
            }
            has_drives
        }
        Err(err) => {
            let raw_err = err.to_string();
            if let Ok(mut s) = state.lock() {
                s.smart_error = Some(err);
                s.resolved_smartctl_path = Some(resolved_path_str);
                s.smartctl_version = version;
                s.last_raw_output = crate::smart::get_last_raw_output();
                s.last_raw_error = Some(raw_err);
                s.data_changed = true;
            }
            false
        }
    }
}

/// SMART polling interval: 5 minutes (300 seconds).
pub const SMART_POLL_INTERVAL_SECS: u64 = 300;

/// Forces an immediate synchronous SMART poll and snapshot cycle, recording snapshots for all enabled drives.
pub fn poll_smart_cycle_force(state: &AppState) -> bool {
    crate::logger::log_info("Executing forced SMART polling cycle...");
    poll_smart_cycle(state)
}

/// Takes a final SMART snapshot for all enabled drives before application shutdown to minimize gap times.
pub fn take_exit_snapshot(state: &AppState) {
    crate::logger::log_info("Taking exit SMART snapshot before shutdown...");
    poll_smart_cycle(state);
}

/// Atomic flags ensuring background pollers run as singletons.
pub static SMART_POLLER_RUNNING: AtomicBool = AtomicBool::new(false);
pub static PROCS_POLLER_RUNNING: AtomicBool = AtomicBool::new(false);

/// Spawns the SMART Poller thread. Runs immediately (or delayed at logon startup),
/// retries on early failure, and then repeats every 5 minutes.
/// Listens for manual rescan trigger via `rescan_requested`.
pub fn spawn_smart_poller(state: AppState, is_logon_startup: bool) -> std::thread::JoinHandle<()> {
    if SMART_POLLER_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        crate::logger::log_warn(
            "spawn_smart_poller called while SMART poller is already running; returning no-op handle",
        );
        return std::thread::spawn(|| {});
    }

    std::thread::Builder::new()
        .name("estus-smart-poller".to_string())
        .spawn(move || {
            struct Guard;
            impl Drop for Guard {
                fn drop(&mut self) {
                    SMART_POLLER_RUNNING.store(false, Ordering::SeqCst);
                }
            }
            let _guard = Guard;

            if is_logon_startup {
                crate::logger::log_info(&format!(
                    "Startup was minimized (logon startup); delaying initial SMART poll by {} seconds for disk drivers to initialize",
                    LOGON_INITIAL_DELAY_SECS
                ));
                for _ in 0..LOGON_INITIAL_DELAY_SECS {
                    if let Ok(s) = state.lock() {
                        if s.should_quit {
                            return;
                        }
                        if s.rescan_requested {
                            break;
                        }
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            }

            // Initial SMART query
            let mut success = poll_smart_cycle(&state);

            // If the first SMART query fails during logon startup, retry after 5 seconds up to 3 times
            if !success && is_logon_startup {
                for attempt in 1..=LOGON_MAX_RETRIES {
                    if let Ok(s) = state.lock()
                        && s.should_quit {
                            return;
                        }
                    crate::logger::log_warn(&format!(
                        "Initial logon SMART poll failed; retrying in {} seconds (attempt {}/{})",
                        LOGON_RETRY_INTERVAL_SECS, attempt, LOGON_MAX_RETRIES
                    ));
                    for _ in 0..LOGON_RETRY_INTERVAL_SECS {
                        if let Ok(s) = state.lock() {
                            if s.should_quit {
                                return;
                            }
                            if s.rescan_requested {
                                break;
                            }
                        }
                        std::thread::sleep(Duration::from_secs(1));
                    }
                    success = poll_smart_cycle(&state);
                    if success {
                        crate::logger::log_info("SMART poll succeeded on retry.");
                        break;
                    }
                }
            }

            loop {
                // Sleep for up to 5 minutes (300s), checking rescan_requested and should_quit every 500ms
                for _ in 0..(SMART_POLL_INTERVAL_SECS * 2) {
                    if let Ok(mut s) = state.lock() {
                        if s.should_quit {
                            return;
                        }
                        if s.rescan_requested {
                            s.rescan_requested = false;
                            break;
                        }
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }

                if let Ok(s) = state.lock()
                    && s.should_quit {
                        return;
                    }

                poll_smart_cycle(&state);
            }
        })
        .expect("Failed to spawn smart poller thread")
}

/// Spawns the Process Poller thread. Runs every 30 seconds.
pub fn spawn_procs_poller(state: AppState) -> std::thread::JoinHandle<()> {
    if PROCS_POLLER_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        crate::logger::log_warn(
            "spawn_procs_poller called while procs poller is already running; returning no-op handle",
        );
        return std::thread::spawn(|| {});
    }

    std::thread::Builder::new()
        .name("estus-procs-poller".to_string())
        .spawn(move || {
            struct Guard;
            impl Drop for Guard {
                fn drop(&mut self) {
                    PROCS_POLLER_RUNNING.store(false, Ordering::SeqCst);
                }
            }
            let _guard = Guard;

            let mut collector = crate::procs::ProcCollector::new();
            let mut accumulated_deltas: std::collections::HashMap<String, u64> =
                std::collections::HashMap::new();

            loop {
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

                let raw_deltas = collector.poll_deltas();
                for (name, bytes) in raw_deltas {
                    *accumulated_deltas.entry(name).or_insert(0) += bytes;
                }

                let ssd_delta_opt = if let Ok(mut s) = state.lock() {
                    s.latest_ssd_delta_bytes.take()
                } else {
                    None
                };

                if let Some(ssd_delta) = ssd_delta_opt {
                    let acc_vec: Vec<(String, u64)> = accumulated_deltas.drain().collect();
                    let deltas = crate::procs::normalize_process_deltas(&acc_vec, ssd_delta);
                    let now_ts = chrono::Utc::now().timestamp();

                    if let Ok(conn) = crate::db::init_db() {
                        if !deltas.is_empty() {
                            let _ = crate::db::insert_process_deltas(&conn, now_ts, &deltas);
                        }

                        let daily = crate::db::daily_writes(&conn, None, 30).unwrap_or_default();
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

                        let today_bytes = daily
                            .iter()
                            .find(|dw| dw.date == today)
                            .map(|dw| dw.total_bytes())
                            .unwrap_or(0);

                        let _ = check_and_send_alert(&state, today_bytes, today);

                        if let Ok(mut s) = state.lock() {
                            s.daily_writes = daily;
                            s.top_processes_today = top_today;
                            s.top_processes_7d = top_7d;
                            s.top_processes_30d = top_30d;
                            s.data_changed = true;
                        }
                    }
                }
            }
        })
        .expect("Failed to spawn procs poller thread")
}

/// Spawns both poller threads and performs startup pruning / state initialization.
/// If pollers are already running, logs a warning and returns no-op handles.
pub fn start_background_threads(
    state: AppState,
    is_logon_startup: bool,
) -> (std::thread::JoinHandle<()>, std::thread::JoinHandle<()>) {
    if SMART_POLLER_RUNNING.load(Ordering::SeqCst) && PROCS_POLLER_RUNNING.load(Ordering::SeqCst) {
        crate::logger::log_warn(
            "start_background_threads called while background pollers are already running; skipping initialization",
        );
        return (
            spawn_smart_poller(Arc::clone(&state), is_logon_startup),
            spawn_procs_poller(Arc::clone(&state)),
        );
    }

    if !is_logon_startup {
        poll_smart_cycle_force(&state);
    }
    let _ = initialize_database_and_state(&state);
    let smart_handle = spawn_smart_poller(Arc::clone(&state), is_logon_startup);
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
        assert_eq!(s.custom_smartctl_path, None);
        assert!(s.drives.is_empty());
        assert!(!s.verbose_logging);
    }

    #[test]
    fn test_settings_serialization() {
        let mut drives = std::collections::HashMap::new();
        drives.insert(
            "drive1".to_string(),
            DriveConfig {
                nickname: Some("OS SSD".to_string()),
                enabled: true,
            },
        );
        let test_smartctl_path = std::env::temp_dir()
            .join("bin")
            .join("smartctl.exe")
            .to_string_lossy()
            .to_string();
        let s = Settings {
            alert_threshold_gb: 250.0,
            start_with_windows: true,
            last_notified_date: NaiveDate::from_ymd_opt(2026, 11, 16),
            custom_smartctl_path: Some(test_smartctl_path),
            drives,
            verbose_logging: true,
        };

        let json = serde_json::to_string(&s).unwrap();
        let loaded: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(s, loaded);
        assert_eq!(loaded.drives.get("drive1").unwrap().nickname, Some("OS SSD".to_string()));
        assert!(loaded.verbose_logging);
    }

    #[test]
    fn test_shared_state_initialization() {
        let settings = Settings::default();
        let state = SharedState::new(settings.clone());

        assert!(state.detected_drives.is_empty());
        assert_eq!(state.selected_drive_id, None);
        assert!(state.per_drive_snapshots.is_empty());
        assert!(state.per_drive_intervals.is_empty());
        assert!(state.latest_smart.is_none());
        assert!(state.smart_error.is_none());
        assert!(state.daily_writes.is_empty());
        assert!(state.intervals_24h.is_empty());
        assert!(state.top_processes_today.is_empty());
        assert!(state.top_processes_7d.is_empty());
        assert!(state.top_processes_30d.is_empty());
        assert_eq!(state.latest_ssd_delta_bytes, None);
        assert_eq!(state.settings, settings);
        assert!(!state.data_changed);
        assert!(state.show_window);
        assert!(!state.should_quit);
        assert_eq!(state.resolved_smartctl_path, None);
        assert_eq!(state.smartctl_version, None);
        assert_eq!(state.last_raw_error, None);
        assert_eq!(state.last_raw_output, None);
        assert_eq!(state.active_device, None);
        assert!(!state.rescan_requested);
        assert!(!state.is_logon_startup);
        assert_eq!(state.initialization_error, None);
    }

    #[test]
    fn test_settings_save_and_load_from_path() {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join("estus_test_settings.json");

        let s = Settings {
            alert_threshold_gb: 50.0,
            start_with_windows: false,
            custom_smartctl_path: Some("custom_smartctl.exe".to_string()),
            ..Default::default()
        };

        s.save_to_path(&path).unwrap();
        let loaded = Settings::load_from_path(&path);
        assert_eq!(s, loaded);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_settings_load_nonexistent_file() {
        let path = std::env::temp_dir()
            .join("definitely_nonexistent_directory_xyz")
            .join("settings.json");
        let loaded = Settings::load_from_path(&path);
        assert_eq!(loaded, Settings::default());
    }

    #[test]
    fn test_settings_load_corrupted_file() {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join("estus_corrupted_settings.json");

        std::fs::write(&path, "this is definitely not valid json").unwrap();
        let loaded = Settings::load_from_path(&path);
        assert_eq!(loaded, Settings::default());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_check_and_send_alert_threshold_logic() {
        let settings = Settings {
            alert_threshold_gb: 10.0,
            start_with_windows: false,
            last_notified_date: None,
            custom_smartctl_path: None,
            drives: std::collections::HashMap::new(),
            verbose_logging: false,
        };
        let state = new_app_state(settings);
        let today = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();

        // 5 GB: below 10 GB threshold -> no alert
        let res_below = check_and_send_alert(&state, 5_000_000_000, today).unwrap();
        assert!(!res_below);
        assert_eq!(state.lock().unwrap().settings.last_notified_date, None);

        // 15 GB: exceeds 10 GB threshold -> alert triggered
        let res_above = check_and_send_alert(&state, 15_000_000_000, today).unwrap();
        assert!(res_above);
        assert_eq!(
            state.lock().unwrap().settings.last_notified_date,
            Some(today)
        );

        // Second check on the same day -> should not fire again
        let res_duplicate = check_and_send_alert(&state, 20_000_000_000, today).unwrap();
        assert!(!res_duplicate);
    }

    #[test]
    fn test_logon_startup_constants() {
        assert_eq!(LOGON_INITIAL_DELAY_SECS, 20);
        assert_eq!(LOGON_RETRY_INTERVAL_SECS, 5);
        assert_eq!(LOGON_MAX_RETRIES, 3);
    }

    #[test]
    fn test_ensure_app_directories_and_init_environment() {
        let (data_dir, config_dir) = ensure_app_directories().expect("ensure_app_directories failed");
        assert!(data_dir.exists());
        assert!(config_dir.exists());

        let state = new_app_state(Settings::default());
        init_app_environment(&state);
        assert_eq!(state.lock().unwrap().initialization_error, None);
    }

    #[test]
    fn test_shared_state_logon_startup_flag() {
        let settings = Settings::default();
        let mut state = SharedState::new(settings);
        assert!(!state.is_logon_startup);
        assert!(state.show_window);

        state.is_logon_startup = true;
        state.show_window = false;
        assert!(state.is_logon_startup);
        assert!(!state.show_window);
    }

    #[test]
    fn test_verbose_logging_setting_sync() {
        let temp_dir = std::env::temp_dir();
        let path = temp_dir.join(format!("estus_verbose_test_{}.json", std::process::id()));

        // Save with verbose_logging = true
        let mut s = Settings {
            verbose_logging: true,
            ..Default::default()
        };
        s.save_to_path(&path).unwrap();

        // Loading should automatically update the logger active level
        let loaded = Settings::load_from_path(&path);
        assert!(loaded.verbose_logging);
        assert!(crate::logger::is_verbose_logging());

        // Update to false and save
        s.verbose_logging = false;
        s.save_to_path(&path).unwrap();

        let loaded_false = Settings::load_from_path(&path);
        assert!(!loaded_false.verbose_logging);
        assert!(!crate::logger::is_verbose_logging());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_poller_double_spawn_guard() {
        let settings = Settings::default();
        let state = new_app_state(settings);
        state.lock().unwrap().should_quit = true;

        // Manually simulate SMART_POLLER_RUNNING flag
        SMART_POLLER_RUNNING.store(true, Ordering::SeqCst);
        let handle1 = spawn_smart_poller(Arc::clone(&state), false);
        // handle1 should be a no-op handle that finishes immediately
        handle1.join().unwrap();
        // Reset flag
        SMART_POLLER_RUNNING.store(false, Ordering::SeqCst);

        // Manually simulate PROCS_POLLER_RUNNING flag
        PROCS_POLLER_RUNNING.store(true, Ordering::SeqCst);
        let handle2 = spawn_procs_poller(Arc::clone(&state));
        // handle2 should be a no-op handle that finishes immediately
        handle2.join().unwrap();
        // Reset flag
        PROCS_POLLER_RUNNING.store(false, Ordering::SeqCst);
    }

    #[test]
    fn test_start_background_threads_double_call_guard() {
        let settings = Settings::default();
        let state = new_app_state(settings);
        state.lock().unwrap().should_quit = true;

        // Set both flags to true
        SMART_POLLER_RUNNING.store(true, Ordering::SeqCst);
        PROCS_POLLER_RUNNING.store(true, Ordering::SeqCst);

        let (h1, h2) = start_background_threads(Arc::clone(&state), false);
        h1.join().unwrap();
        h2.join().unwrap();

        SMART_POLLER_RUNNING.store(false, Ordering::SeqCst);
        PROCS_POLLER_RUNNING.store(false, Ordering::SeqCst);
    }
}
