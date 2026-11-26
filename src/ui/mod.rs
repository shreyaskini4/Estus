pub mod dashboard;
pub mod diagnostics;
pub mod onboarding;
pub mod processes;
pub mod settings;
pub mod sidebar;

use std::sync::Arc;
use std::time::Instant;

use crate::app_core::{AppState, Settings};
use crate::smart::{DriveInfo, SmartError, SmartSnapshot};
use crate::theme::{self, ThemeMode, SPACE_XL};

/// Primary navigation views in the Estus workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NavTab {
    #[default]
    Dashboard,
    Processes,
    Settings,
    Diagnostics,
}

impl std::fmt::Display for NavTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dashboard => write!(f, "Dashboard"),
            Self::Processes => write!(f, "Processes"),
            Self::Settings => write!(f, "Settings"),
            Self::Diagnostics => write!(f, "Diagnostics"),
        }
    }
}

/// Active time filter for top process writes ranking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeFilter {
    #[default]
    Today,
    SevenDays,
    ThirtyDays,
}

impl std::fmt::Display for TimeFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Today => write!(f, "Today"),
            Self::SevenDays => write!(f, "7 Days"),
            Self::ThirtyDays => write!(f, "30 Days"),
        }
    }
}

/// Decoupled read-only snapshot of `SharedState` passed to UI view renderers.
pub struct UiSnapshot {
    pub detected_drives: Vec<DriveInfo>,
    pub selected_drive_id: Option<String>,
    pub selected_drive: Option<DriveInfo>,
    pub selected_snapshot: Option<SmartSnapshot>,
    pub selected_daily_writes: Vec<crate::db::DailyWrites>,
    pub selected_intervals_24h: Vec<crate::db::SnapshotInterval>,
    pub per_drive_snapshots: std::collections::HashMap<String, SmartSnapshot>,
    pub smart_error: Option<SmartError>,
    pub top_today: Vec<(String, u64)>,
    pub top_7d: Vec<(String, u64)>,
    pub top_30d: Vec<(String, u64)>,
    pub settings: Settings,
    pub resolved_smartctl_path: Option<String>,
    pub smartctl_version: Option<String>,
    pub active_device: Option<String>,
    pub last_raw_error: Option<String>,
    pub last_raw_output: Option<String>,
    pub initialization_error: Option<String>,
}

/// Formats a byte quantity into human-readable storage units (B, KB, MB, GB, TB, PB)
/// using standard decimal representation (1 KB = 1,000 B) matching NVMe SMART specifications.
pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1_000.0;
    const MB: f64 = 1_000_000.0;
    const GB: f64 = 1_000_000_000.0;
    const TB: f64 = 1_000_000_000_000.0;
    const PB: f64 = 1_000_000_000_000_000.0;

    let b = bytes as f64;
    if b >= PB {
        format!("{:.2} PB", b / PB)
    } else if b >= TB {
        format!("{:.1} TB", b / TB)
    } else if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{} B", bytes)
    }
}

/// Formats an integer with thousands separator commas (e.g. 1420 -> "1,420").
pub fn format_with_commas(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut out = String::with_capacity(len + (len / 3));
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(b as char);
    }
    out
}

/// Formats projected lifespan in days and approximate years.
/// Caps extremely long estimates (e.g. > 20 or > 30 years) to realistic hardware wear summaries.
pub fn format_projected_lifespan(projected: Option<u64>) -> String {
    match projected {
        None => "Indefinite".to_string(),
        Some(0) => "0 days (Depleted)".to_string(),
        Some(days) => {
            let years = days as f64 / 365.25;
            if years >= 30.0 {
                "> 30 years (Low wear)".to_string()
            } else if years >= 20.0 {
                "> 20 years (Healthy)".to_string()
            } else if days >= 365 {
                let formatted_days = format_with_commas(days);
                format!("{formatted_days} days (~{years:.1} years)")
            } else if days == 1 {
                "1 day".to_string()
            } else {
                let formatted_days = format_with_commas(days);
                format!("{formatted_days} days")
            }
        }
    }
}

/// Formats wear percentage option into a string (e.g. "3%" or "N/A").
pub fn format_percentage(pct: Option<u8>) -> String {
    match pct {
        Some(val) => format!("{val}%"),
        None => "N/A".to_string(),
    }
}

/// Formats a list of 24h snapshot intervals into CSV format with standard headers.
pub fn format_intervals_csv(intervals: &[crate::db::SnapshotInterval]) -> String {
    let mut csv = String::from("timestamp,formatted_time,raw_data_units_written,delta_gb,attribution\n");
    for interval in intervals {
        let formatted_time = chrono::DateTime::from_timestamp(interval.timestamp, 0)
            .map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| interval.timestamp.to_string());
        csv.push_str(&format!(
            "{},{},{},{:.4},{}\n",
            interval.timestamp,
            formatted_time,
            interval.raw_data_units_written,
            interval.delta_gb,
            interval.attribution,
        ));
    }
    csv
}

/// The main Estus UI application.
pub type EstusApp = SsdWatchApp;

pub struct SsdWatchApp {
    pub state: AppState,
    pub active_tab: NavTab,
    pub sidebar_collapsed: bool,
    pub time_filter: TimeFilter,
    pub process_filter: String,
    pub editing_drive_id: Option<String>,
    pub editing_nickname_buf: String,
    pub temp_theme_mode: ThemeMode,
    pub temp_alert_threshold: f64,
    pub temp_start_with_windows: bool,
    pub autostart_system_configured: bool,
    pub temp_custom_smartctl_path: String,
    pub temp_verbose_logging: bool,
    pub temp_drive_nicknames: std::collections::HashMap<String, String>,
    pub temp_drive_enabled: std::collections::HashMap<String, bool>,
    pub confirm_reset_history: bool,
    pub settings_status_msg: Option<(String, bool)>,
    pub csv_status_msg: Option<(String, bool)>,
    pub raw_output_expanded: bool,
    pub gauge_anim_start: Option<Instant>,
    pub prev_anim_key: Option<(Option<String>, u8)>,
    pub shimmer_active: bool,
    pub last_tray_wear: Option<Option<u8>>,
    pub _tray: Option<tray_icon::TrayIcon>,
    pub _show_id: Option<tray_icon::menu::MenuId>,
    pub _quit_id: Option<tray_icon::menu::MenuId>,
}

impl SsdWatchApp {
    pub fn new(state: AppState) -> Self {
        let (threshold, _start_with_win, custom_path, verbose_logging, theme_mode) =
            if let Ok(s) = state.lock() {
                (
                    s.settings.alert_threshold_gb,
                    s.settings.start_with_windows,
                    s.settings.custom_smartctl_path.clone().unwrap_or_default(),
                    s.settings.verbose_logging,
                    s.settings.theme_mode,
                )
            } else {
                (100.0, false, String::new(), false, ThemeMode::default())
            };

        // Query actual system state from Windows Task Scheduler rather than solely trusting JSON
        #[cfg(not(test))]
        let system_autostart = crate::autostart::is_enabled();
        #[cfg(test)]
        let system_autostart = _start_with_win;

        // Keep settings synchronized if they differ from the actual system state
        if let Ok(mut s) = state.lock()
            && s.settings.start_with_windows != system_autostart
        {
            s.settings.start_with_windows = system_autostart;
        }

        Self {
            state,
            active_tab: NavTab::Dashboard,
            sidebar_collapsed: false,
            time_filter: TimeFilter::Today,
            process_filter: String::new(),
            editing_drive_id: None,
            editing_nickname_buf: String::new(),
            temp_theme_mode: theme_mode,
            temp_alert_threshold: threshold,
            temp_start_with_windows: system_autostart,
            autostart_system_configured: system_autostart,
            temp_custom_smartctl_path: custom_path,
            temp_verbose_logging: verbose_logging,
            temp_drive_nicknames: std::collections::HashMap::new(),
            temp_drive_enabled: std::collections::HashMap::new(),
            confirm_reset_history: false,
            settings_status_msg: None,
            csv_status_msg: None,
            raw_output_expanded: false,
            gauge_anim_start: None,
            prev_anim_key: None,
            shimmer_active: false,
            last_tray_wear: None,
            _tray: None,
            _show_id: None,
            _quit_id: None,
        }
    }

    /// Synchronizes the in-memory autostart toggle and status indicator with Windows Task Scheduler.
    pub fn sync_autostart_with_system(&mut self) {
        #[cfg(not(test))]
        let is_sys = crate::autostart::is_enabled();
        #[cfg(test)]
        let is_sys = self.autostart_system_configured;

        self.temp_start_with_windows = is_sys;
        self.autostart_system_configured = is_sys;
        if let Ok(mut s) = self.state.lock() {
            s.settings.start_with_windows = is_sys;
        }
    }

    /// Opens a file dialog allowing the user to select smartctl.exe, saving the choice to settings.
    pub fn browse_for_smartctl(&mut self) {
        if let Some(file_path) = rfd::FileDialog::new()
            .add_filter("smartctl executable", &["exe"])
            .set_title("Locate smartctl.exe")
            .pick_file()
        {
            let path_str = file_path.to_string_lossy().to_string();
            self.temp_custom_smartctl_path = path_str.clone();
            if let Ok(mut s) = self.state.lock() {
                s.settings.custom_smartctl_path = Some(path_str.clone());
                let _ = s.settings.save();
                s.resolved_smartctl_path = Some(path_str.clone());
                s.mark_data_changed();
            }
            crate::smart::set_cached_smartctl_path(Some(file_path));
            self.trigger_refresh();
        }
    }

    pub fn set_tray(
        &mut self,
        tray: tray_icon::TrayIcon,
        show_id: tray_icon::menu::MenuId,
        quit_id: tray_icon::menu::MenuId,
    ) {
        self._tray = Some(tray);
        self._show_id = Some(show_id);
        self._quit_id = Some(quit_id);
    }

    /// Clears the cached smartctl path, re-runs candidate resolution, and refreshes metrics.
    pub fn trigger_redetect(&mut self) {
        crate::smart::clear_cached_smartctl_path();
        self.trigger_refresh();
    }

    /// Triggers an immediate rescan for connected drives.
    pub fn trigger_rescan(&mut self) {
        if let Ok(mut s) = self.state.lock() {
            s.rescan_requested = true;
        }
        self.trigger_refresh();
    }

    /// Spawns an asynchronous background refresh of SMART metrics and database queries.
    pub fn trigger_refresh(&mut self) {
        let state_clone = Arc::clone(&self.state);
        std::thread::spawn(move || {
            crate::app_core::poll_smart_cycle(&state_clone);
        });
    }

    /// Builds a read-only `UiSnapshot` from `SharedState` without holding the mutex during rendering.
    fn take_snapshot(&self) -> Option<UiSnapshot> {
        let mut s = self.state.lock().ok()?;
        s.data_changed = false;
        let selected_drive = s.get_selected_drive().cloned();
        Some(UiSnapshot {
            detected_drives: s.detected_drives.clone(),
            selected_drive_id: s.selected_drive_id.clone(),
            selected_drive,
            selected_snapshot: s.get_selected_snapshot().cloned(),
            selected_daily_writes: s.get_selected_daily_writes().to_vec(),
            selected_intervals_24h: s.get_selected_intervals_24h().to_vec(),
            per_drive_snapshots: s.per_drive_snapshots.clone(),
            smart_error: s.smart_error.clone(),
            top_today: s.top_processes_today.clone(),
            top_7d: s.top_processes_7d.clone(),
            top_30d: s.top_processes_30d.clone(),
            settings: s.settings.clone(),
            resolved_smartctl_path: s.resolved_smartctl_path.clone(),
            smartctl_version: s.smartctl_version.clone(),
            active_device: s.active_device.clone(),
            last_raw_error: s.last_raw_error.clone(),
            last_raw_output: s.last_raw_output.clone(),
            initialization_error: s.initialization_error.clone(),
        })
    }
}

impl eframe::App for SsdWatchApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Drain any pending tray events during UI rendering
        if let (Some(show_id), Some(quit_id)) = (&self._show_id, &self._quit_id) {
            let show = crate::app_core::handle_tray_events(&self.state, show_id, quit_id);
            if show {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
            }
        }

        // Check if an exit was requested
        if let Ok(s) = self.state.lock()
            && s.should_quit
        {
            crate::app_core::take_exit_snapshot(&self.state);
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            std::process::exit(0);
        }

        // Handle window close: cancel close and hide to system tray unless quitting
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            if let Ok(s) = self.state.lock()
                && s.should_quit
            {
                return;
            }
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Visible(false));
            if let Ok(mut s) = self.state.lock() {
                s.show_window = false;
            }
        }

        if let Ok(s) = self.state.lock()
            && s.show_window
        {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Visible(true));
        }

        let Some(snap) = self.take_snapshot() else {
            return;
        };

        let max_wear = snap
            .per_drive_snapshots
            .values()
            .map(|s| s.percent_used)
            .max();
        if self.last_tray_wear != Some(max_wear) {
            self.last_tray_wear = Some(max_wear);
            if let Some(ref tray) = self._tray
                && let Ok(icon) = crate::app_core::build_tray_icon(max_wear)
            {
                let _ = tray.set_icon(Some(icon));
            }
        }

        let theme = theme::apply_theme(ui.ctx(), snap.settings.theme_mode);

        let sidebar_width = if self.sidebar_collapsed { 56.0 } else { 240.0 };
        egui::Panel::left("estus_sidebar")
            .exact_size(sidebar_width)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(theme.bg_sidebar)
                    .stroke(egui::Stroke::new(1.0, theme.border_subtle))
                    .inner_margin(egui::Margin::same(theme::SPACE_SM as i8)),
            )
            .show(ui, |ui| {
                sidebar::render(ui, &theme, self, &snap);
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme.bg_base)
                    .inner_margin(egui::Margin::same(SPACE_XL as i8)),
            )
            .show(ui, |ui| {
                onboarding::render_banner(ui, &theme, self, &snap);
                match self.active_tab {
                    NavTab::Dashboard => dashboard::render(ui, &theme, self, &snap),
                    NavTab::Processes => processes::render(ui, &theme, self, &snap),
                    NavTab::Settings => settings::render(ui, &theme, self, &snap),
                    NavTab::Diagnostics => diagnostics::render(ui, &theme, self, &snap),
                }
            });

        // Idle-CPU repaint contract: only request repaint while an active animation is running.
        let gauge_animating = self
            .gauge_anim_start
            .is_some_and(|start| start.elapsed().as_secs_f32() < 0.45);
        if gauge_animating || self.shimmer_active {
            ui.ctx().request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_core::{new_app_state, Settings};

    #[test]
    fn test_format_bytes_exact_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(1_000), "1.0 KB");
        assert_eq!(format_bytes(1_500), "1.5 KB");
        assert_eq!(format_bytes(1_000_000), "1.0 MB");
        assert_eq!(format_bytes(450_200_000), "450.2 MB");
        assert_eq!(format_bytes(1_000_000_000), "1.0 GB");
        assert_eq!(format_bytes(18_400_000_000), "18.4 GB");
        assert_eq!(format_bytes(1_000_000_000_000), "1.0 TB");
        assert_eq!(format_bytes(14_200_000_000_000), "14.2 TB");
        assert_eq!(format_bytes(2_500_000_000_000_000), "2.50 PB");
    }

    #[test]
    fn test_format_with_commas() {
        assert_eq!(format_with_commas(0), "0");
        assert_eq!(format_with_commas(7), "7");
        assert_eq!(format_with_commas(999), "999");
        assert_eq!(format_with_commas(1000), "1,000");
        assert_eq!(format_with_commas(1420), "1,420");
        assert_eq!(format_with_commas(123456), "123,456");
        assert_eq!(format_with_commas(1000000), "1,000,000");
        assert_eq!(format_with_commas(123456789), "123,456,789");
    }

    #[test]
    fn test_format_projected_lifespan() {
        assert_eq!(format_projected_lifespan(None), "Indefinite");
        assert_eq!(format_projected_lifespan(Some(0)), "0 days (Depleted)");
        assert_eq!(format_projected_lifespan(Some(1)), "1 day");
        assert_eq!(format_projected_lifespan(Some(100)), "100 days");
        assert_eq!(
            format_projected_lifespan(Some(1420)),
            "1,420 days (~3.9 years)"
        );
        assert_eq!(
            format_projected_lifespan(Some(3650)),
            "3,650 days (~10.0 years)"
        );
        assert_eq!(
            format_projected_lifespan(Some(8000)),
            "> 20 years (Healthy)"
        );
        assert_eq!(
            format_projected_lifespan(Some(15000)),
            "> 30 years (Low wear)"
        );
    }

    #[test]
    fn test_format_percentage() {
        assert_eq!(format_percentage(Some(0)), "0%");
        assert_eq!(format_percentage(Some(3)), "3%");
        assert_eq!(format_percentage(Some(99)), "99%");
        assert_eq!(format_percentage(Some(100)), "100%");
        assert_eq!(format_percentage(None), "N/A");
    }

    #[test]
    fn test_time_filter_display_and_default() {
        assert_eq!(TimeFilter::default(), TimeFilter::Today);
        assert_eq!(TimeFilter::Today.to_string(), "Today");
        assert_eq!(TimeFilter::SevenDays.to_string(), "7 Days");
        assert_eq!(TimeFilter::ThirtyDays.to_string(), "30 Days");
    }

    #[test]
    fn test_nav_tab_display_and_default() {
        assert_eq!(NavTab::default(), NavTab::Dashboard);
        assert_eq!(NavTab::Dashboard.to_string(), "Dashboard");
        assert_eq!(NavTab::Processes.to_string(), "Processes");
        assert_eq!(NavTab::Settings.to_string(), "Settings");
        assert_eq!(NavTab::Diagnostics.to_string(), "Diagnostics");
    }

    #[test]
    fn test_ssd_watch_app_creation() {
        let settings = Settings {
            alert_threshold_gb: 75.0,
            start_with_windows: true,
            last_notified_date: None,
            custom_smartctl_path: None,
            drives: std::collections::HashMap::new(),
            verbose_logging: true,
            theme_mode: ThemeMode::Light,
        };
        let state = new_app_state(settings);
        let app = SsdWatchApp::new(state);

        assert_eq!(app.active_tab, NavTab::Dashboard);
        assert!(!app.sidebar_collapsed);
        assert_eq!(app.temp_theme_mode, ThemeMode::Light);
        assert_eq!(app.time_filter, TimeFilter::Today);
        assert!((app.temp_alert_threshold - 75.0).abs() < 1e-6);
        assert!(app.temp_start_with_windows);
        assert!(app.autostart_system_configured);
        assert!(app.temp_verbose_logging);
        assert_eq!(app.settings_status_msg, None);
        assert_eq!(app.csv_status_msg, None);
    }

    #[test]
    fn test_ssd_watch_app_creation_autostart_disabled() {
        let settings = Settings {
            alert_threshold_gb: 50.0,
            start_with_windows: false,
            last_notified_date: None,
            custom_smartctl_path: None,
            drives: std::collections::HashMap::new(),
            verbose_logging: false,
            theme_mode: ThemeMode::Dark,
        };
        let state = new_app_state(settings);
        let app = SsdWatchApp::new(state);

        assert_eq!(app.time_filter, TimeFilter::Today);
        assert!((app.temp_alert_threshold - 50.0).abs() < 1e-6);
        assert!(!app.temp_start_with_windows);
        assert!(!app.autostart_system_configured);
        assert!(!app.temp_verbose_logging);
        assert_eq!(app.settings_status_msg, None);
        assert_eq!(app.csv_status_msg, None);
    }

    #[test]
    fn test_ssd_watch_app_autostart_sync() {
        let settings = Settings {
            alert_threshold_gb: 75.0,
            start_with_windows: false,
            last_notified_date: None,
            custom_smartctl_path: None,
            drives: std::collections::HashMap::new(),
            verbose_logging: false,
            theme_mode: ThemeMode::System,
        };
        let state = new_app_state(settings);
        let mut app = SsdWatchApp::new(state);

        assert!(!app.temp_start_with_windows);
        assert!(!app.autostart_system_configured);
        assert!(!app.temp_verbose_logging);

        app.temp_start_with_windows = true;
        app.autostart_system_configured = true;
        app.temp_verbose_logging = true;
        assert!(app.temp_start_with_windows);
        assert!(app.autostart_system_configured);
        assert!(app.temp_verbose_logging);

        app.sync_autostart_with_system();
        assert_eq!(app.temp_start_with_windows, app.autostart_system_configured);
    }

    #[test]
    fn test_ssd_watch_app_confirm_reset_history_lifecycle() {
        let settings = Settings::default();
        let state = new_app_state(settings);
        let mut app = SsdWatchApp::new(state);

        assert!(!app.confirm_reset_history);
        app.confirm_reset_history = true;
        assert!(app.confirm_reset_history);

        app.confirm_reset_history = false;
        assert!(!app.confirm_reset_history);
    }

    #[test]
    fn test_format_intervals_csv_empty() {
        let csv = format_intervals_csv(&[]);
        assert_eq!(
            csv,
            "timestamp,formatted_time,raw_data_units_written,delta_gb,attribution\n"
        );
    }

    #[test]
    fn test_format_intervals_csv_populated() {
        let intervals = vec![
            crate::db::SnapshotInterval {
                timestamp: 1700000000,
                raw_data_units_written: 10000,
                delta_gb: 0.0,
                attribution: "baseline".to_string(),
            },
            crate::db::SnapshotInterval {
                timestamp: 1700000300,
                raw_data_units_written: 10500,
                delta_gb: 0.2560,
                attribution: "2026-11-24".to_string(),
            },
            crate::db::SnapshotInterval {
                timestamp: 1700002000,
                raw_data_units_written: 11000,
                delta_gb: 0.2560,
                attribution: "app not running".to_string(),
            },
        ];

        let csv = format_intervals_csv(&intervals);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(
            lines[0],
            "timestamp,formatted_time,raw_data_units_written,delta_gb,attribution"
        );

        assert!(lines[1].starts_with("1700000000,"));
        assert!(lines[1].ends_with(",10000,0.0000,baseline"));

        assert!(lines[2].starts_with("1700000300,"));
        assert!(lines[2].ends_with(",10500,0.2560,2026-11-24"));

        assert!(lines[3].starts_with("1700002000,"));
        assert!(lines[3].ends_with(",11000,0.2560,app not running"));
    }
}
