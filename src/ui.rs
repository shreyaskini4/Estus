use std::sync::Arc;

use chrono::NaiveDate;
use egui::{Color32, RichText, Stroke};

use crate::app_core::{AppState, Settings};
use crate::smart::{SmartError, SmartSnapshot};

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

/// Formats projected lifespan in days and optional approximate years.
pub fn format_projected_lifespan(projected: Option<u64>) -> String {
    match projected {
        None => "Indefinite".to_string(),
        Some(0) => "0 days (Depleted)".to_string(),
        Some(days) => {
            let formatted_days = format_with_commas(days);
            if days >= 365 {
                let years = days as f64 / 365.25;
                format!("{formatted_days} days (~{years:.1} years)")
            } else if days == 1 {
                "1 day".to_string()
            } else {
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

/// The main Estus UI application.
pub type EstusApp = SsdWatchApp;

pub struct SsdWatchApp {
    pub state: AppState,
    pub time_filter: TimeFilter,
    pub temp_alert_threshold: f64,
    pub temp_start_with_windows: bool,
    pub temp_custom_smartctl_path: String,
    pub settings_status_msg: Option<(String, bool)>,
    pub is_refreshing: bool,
    pub _tray: Option<tray_icon::TrayIcon>,
    pub _show_id: Option<tray_icon::menu::MenuId>,
    pub _quit_id: Option<tray_icon::menu::MenuId>,
}

impl SsdWatchApp {
    pub fn new(state: AppState) -> Self {
        let (threshold, start_with_win, custom_path) = if let Ok(s) = state.lock() {
            (
                s.settings.alert_threshold_gb,
                s.settings.start_with_windows,
                s.settings.custom_smartctl_path.clone().unwrap_or_default(),
            )
        } else {
            (100.0, false, String::new())
        };

        Self {
            state,
            time_filter: TimeFilter::Today,
            temp_alert_threshold: threshold,
            temp_start_with_windows: start_with_win,
            temp_custom_smartctl_path: custom_path,
            settings_status_msg: None,
            is_refreshing: false,
            _tray: None,
            _show_id: None,
            _quit_id: None,
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
                s.data_changed = true;
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

    /// Spawns an asynchronous background refresh of NVMe SMART metrics and database queries.
    pub fn trigger_refresh(&mut self) {
        let state_clone = Arc::clone(&self.state);
        std::thread::spawn(move || {
            let custom_path = if let Ok(s) = state_clone.lock() {
                s.settings.custom_smartctl_path.clone()
            } else {
                None
            };

            match crate::smart::run_smartctl_with_config(custom_path.as_deref(), None) {
                Ok((snapshot, resolved_path, active_dev)) => {
                    if let Ok(conn) = crate::db::init_db() {
                        let _ = crate::db::insert_snapshot(&conn, &snapshot);
                        let daily = crate::db::daily_writes(&conn, 30).unwrap_or_default();
                        let now_ts = chrono::Utc::now().timestamp();
                        let today = chrono::Local::now().date_naive();
                        let today_start_ts = today
                            .and_hms_opt(0, 0, 0)
                            .and_then(|t| t.and_local_timezone(chrono::Local).single())
                            .map(|dt| dt.timestamp())
                            .unwrap_or(now_ts - 86_400);

                        let top_today =
                            crate::db::top_processes(&conn, today_start_ts, 10).unwrap_or_default();
                        let top_7d = crate::db::top_processes(&conn, now_ts - 7 * 86_400, 10)
                            .unwrap_or_default();
                        let top_30d = crate::db::top_processes(&conn, now_ts - 30 * 86_400, 10)
                            .unwrap_or_default();

                        if let Ok(mut s) = state_clone.lock() {
                            s.latest_smart = Some(snapshot);
                            s.smart_error = None;
                            s.resolved_smartctl_path = Some(resolved_path);
                            s.active_device = Some(active_dev);
                            s.last_raw_error = None;
                            s.daily_writes = daily;
                            s.top_processes_today = top_today;
                            s.top_processes_7d = top_7d;
                            s.top_processes_30d = top_30d;
                            s.data_changed = true;
                        }
                    }
                }
                Err(err) => {
                    let raw_err = err.to_string();
                    let resolved = crate::smart::get_or_resolve_smartctl_path(custom_path.as_deref())
                        .map(|p| p.display().to_string());
                    if let Ok(mut s) = state_clone.lock() {
                        s.smart_error = Some(err);
                        s.resolved_smartctl_path = resolved;
                        s.last_raw_error = Some(raw_err);
                        s.data_changed = true;
                    }
                }
            }
        });
    }
}

impl eframe::App for SsdWatchApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // 1. Enforce egui dark theme
        ui.ctx().set_visuals(egui::Visuals::dark());

        // 2. Low CPU usage: request repaint at 1s interval so idle CPU stays near 0%
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));

        // Intercept close request to hide to system tray instead of exiting
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Visible(false));
            if let Ok(mut s) = self.state.lock() {
                s.show_window = false;
            }
        }

        // Check if tray clicked "Show" or "Quit"
        let (show_win, should_quit) = if let Ok(s) = self.state.lock() {
            (s.show_window, s.should_quit)
        } else {
            (true, false)
        };

        if should_quit {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        if show_win {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Visible(true));
        }

        // Snapshot current state data under brief lock
        let (
            latest_smart,
            smart_error,
            daily_writes,
            top_today,
            top_7d,
            top_30d,
            settings,
            resolved_path,
            active_dev,
            last_raw_err,
        ) = {
            if let Ok(s) = self.state.lock() {
                (
                    s.latest_smart.clone(),
                    s.smart_error.clone(),
                    s.daily_writes.clone(),
                    s.top_processes_today.clone(),
                    s.top_processes_7d.clone(),
                    s.top_processes_30d.clone(),
                    s.settings.clone(),
                    s.resolved_smartctl_path.clone(),
                    s.active_device.clone(),
                    s.last_raw_error.clone(),
                )
            } else {
                return;
            }
        };

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(8.0);

                // Top Header Bar
                render_header(ui, self);

                ui.add_space(6.0);

                // Detection status bar (resolved path, active device, browse button)
                render_status_area(
                    ui,
                    self,
                    resolved_path.as_deref(),
                    active_dev.as_deref(),
                    last_raw_err.as_deref(),
                );

                ui.add_space(8.0);

                // Top Banner / Error Card
                if let Some(ref err) = smart_error {
                    render_error_banner(
                        ui,
                        self,
                        err,
                        resolved_path.as_deref(),
                        last_raw_err.as_deref(),
                    );
                }

                // Top 4 Metric Cards
                render_metric_cards(ui, latest_smart.as_ref(), &daily_writes, &settings);

                ui.add_space(16.0);

                // Chart Section
                render_chart_section(ui, &daily_writes);

                ui.add_space(16.0);

                // Process Table
                render_process_table(
                    ui,
                    &mut self.time_filter,
                    &top_today,
                    &top_7d,
                    &top_30d,
                );

                ui.add_space(16.0);

                // Settings Panel
                render_settings_panel(ui, self);

                ui.add_space(16.0);
            });
        });
    }
}

/// Renders the title bar and manual refresh action.
fn render_header(ui: &mut egui::Ui, app: &mut SsdWatchApp) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Estus")
                .size(20.0)
                .strong()
                .color(Color32::from_rgb(230, 235, 245)),
        );
        ui.label(
            RichText::new("NVMe Health & Write Monitor")
                .size(12.0)
                .color(Color32::from_rgb(140, 145, 160)),
        );

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("🔄 Refresh Now").clicked() {
                app.trigger_refresh();
            }
        });
    });
}

/// Renders the status area displaying resolved smartctl path, active NVMe device, last error, and Browse button.
fn render_status_area(
    ui: &mut egui::Ui,
    app: &mut SsdWatchApp,
    resolved_path: Option<&str>,
    active_device: Option<&str>,
    last_raw_error: Option<&str>,
) {
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(22, 24, 30))
        .stroke(Stroke::new(1.0, Color32::from_rgb(45, 50, 62)))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("smartctl:").strong().color(Color32::from_rgb(180, 185, 200)));
                if let Some(p) = resolved_path {
                    ui.monospace(RichText::new(p).color(Color32::from_rgb(130, 200, 255)));
                } else {
                    ui.label(RichText::new("Not found").color(Color32::from_rgb(255, 120, 120)));
                }

                if let Some(dev) = active_device {
                    ui.separator();
                    ui.label(RichText::new("NVMe:").strong().color(Color32::from_rgb(180, 185, 200)));
                    ui.monospace(RichText::new(dev).color(Color32::from_rgb(120, 225, 150)));
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("📁 Browse for smartctl.exe").clicked() {
                        app.browse_for_smartctl();
                    }
                });
            });

            if let Some(raw_err) = last_raw_error {
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Last raw error:").small().strong().color(Color32::from_rgb(255, 130, 130)));
                    ui.monospace(RichText::new(raw_err).small().color(Color32::from_rgb(240, 170, 170)));
                });
            }
        });
}

/// Renders prominent guidance banner if SMART collection encountered an error.
fn render_error_banner(
    ui: &mut egui::Ui,
    app: &mut SsdWatchApp,
    err: &SmartError,
    resolved_path: Option<&str>,
    last_raw_error: Option<&str>,
) {
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(45, 20, 20))
        .stroke(Stroke::new(1.0, Color32::from_rgb(220, 53, 69)))
        .corner_radius(6.0)
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("⚠️").size(24.0));
                ui.vertical(|ui| {
                    match err {
                        SmartError::SmartctlNotFound(msg) => {
                            ui.strong(
                                RichText::new("smartctl Executable Not Found")
                                    .color(Color32::from_rgb(255, 100, 100)),
                            );
                            ui.label(
                                "smartmontools is not installed, not in PATH, or not found at the configured location. Install it via `winget install smartmontools`, or browse directly for smartctl.exe below.",
                            );
                            ui.label(RichText::new(format!("Search info: {msg}")).small().color(Color32::LIGHT_GRAY));
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                if ui.button("📁 Browse for smartctl.exe").clicked() {
                                    app.browse_for_smartctl();
                                }
                                ui.label(
                                    RichText::new("PowerShell:")
                                        .small()
                                        .color(Color32::LIGHT_GRAY),
                                );
                                ui.monospace("winget install smartmontools");
                            });
                        }
                        SmartError::PermissionDenied(msg) => {
                            ui.strong(
                                RichText::new("Administrator Permission Required")
                                    .color(Color32::from_rgb(255, 100, 100)),
                            );
                            ui.label(
                                "Administrator permission required to read NVMe SMART data. Please right-click Estus and select 'Run as Administrator'.",
                            );
                            ui.label(RichText::new(format!("Details: {msg}")).small().color(Color32::LIGHT_GRAY));
                        }
                        SmartError::DeviceNotFound(msg) => {
                            ui.strong(
                                RichText::new("NVMe Device Not Found")
                                    .color(Color32::from_rgb(255, 140, 100)),
                            );
                            ui.label(format!("Device query failed: {msg}"));
                        }
                        SmartError::SpawnError(msg) => {
                            ui.strong(
                                RichText::new("Failed to Spawn smartctl Process")
                                    .color(Color32::from_rgb(255, 100, 100)),
                            );
                            ui.label(format!("Process error: {msg}"));
                        }
                        other => {
                            ui.strong(
                                RichText::new("SMART Error")
                                    .color(Color32::from_rgb(255, 100, 100)),
                            );
                            ui.label(format!("{other}"));
                        }
                    }

                    if let Some(path) = resolved_path {
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Path tried:").small().color(Color32::GRAY));
                            ui.monospace(RichText::new(path).small().color(Color32::LIGHT_GRAY));
                        });
                    }

                    if let Some(raw_err) = last_raw_error {
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Last raw error:").small().color(Color32::from_rgb(255, 160, 160)));
                            ui.monospace(RichText::new(raw_err).small().color(Color32::from_rgb(255, 200, 200)));
                        });
                    }
                });
            });
        });
    ui.add_space(8.0);
}

/// Helper function to render an individual metric card.
fn render_metric_card(
    ui: &mut egui::Ui,
    title: &str,
    value: &str,
    subtext: &str,
    value_color: Color32,
    progress: Option<f32>,
) {
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(28, 30, 36))
        .stroke(Stroke::new(1.0, Color32::from_rgb(45, 48, 58)))
        .corner_radius(6.0)
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(title)
                        .size(11.0)
                        .strong()
                        .color(Color32::from_rgb(140, 145, 160)),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(value)
                        .size(20.0)
                        .strong()
                        .color(value_color),
                );
                if let Some(p) = progress {
                    ui.add_space(4.0);
                    ui.add(
                        egui::ProgressBar::new(p.clamp(0.0, 1.0))
                            .animate(false)
                            .desired_height(4.0),
                    );
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new(subtext)
                        .size(11.0)
                        .color(Color32::from_rgb(120, 125, 135)),
                );
            });
        });
}

/// Renders the 4 top metric cards: Percentage Used, Total Host Writes, Writes Today, Projected Lifespan.
fn render_metric_cards(
    ui: &mut egui::Ui,
    latest_smart: Option<&SmartSnapshot>,
    daily_writes: &[(NaiveDate, u64)],
    settings: &Settings,
) {
    // 1. Percentage Used
    let (pct_str, pct_color, pct_subtext, pct_val) = match latest_smart {
        Some(s) => {
            let color = if s.percent_used < 50 {
                Color32::from_rgb(72, 199, 142)
            } else if s.percent_used < 80 {
                Color32::from_rgb(255, 183, 77)
            } else {
                Color32::from_rgb(244, 67, 54)
            };
            (
                format!("{}%", s.percent_used),
                color,
                format!("Available Spare: {}%", s.available_spare),
                Some(s.percent_used as f32 / 100.0),
            )
        }
        None => (
            "N/A".to_string(),
            Color32::from_rgb(160, 160, 160),
            "SMART unavailable".to_string(),
            None,
        ),
    };

    // 2. Total Host Writes
    let (host_writes_str, host_writes_subtext) = match latest_smart {
        Some(s) => (
            format_bytes(s.host_bytes_written()),
            format!(
                "Hours: {}h | Cycles: {}",
                s.power_on_hours, s.power_cycles
            ),
        ),
        None => ("N/A".to_string(), "No readings recorded".to_string()),
    };

    // 3. Writes Today
    let today = chrono::Local::now().date_naive();
    let today_bytes = daily_writes
        .iter()
        .find(|(d, _)| *d == today)
        .map(|(_, b)| *b)
        .unwrap_or(0);
    let today_writes_str = format_bytes(today_bytes);
    let threshold_gb = settings.alert_threshold_gb;
    let today_gb = today_bytes as f64 / crate::analytics::BYTES_PER_GB;
    let (today_color, today_subtext) = if crate::analytics::is_daily_threshold_exceeded(today_bytes, threshold_gb) {
        (
            Color32::from_rgb(244, 67, 54),
            format!("Alert: {:.1} GB > {:.0} GB limit", today_gb, threshold_gb),
        )
    } else {
        (
            Color32::from_rgb(72, 199, 142),
            format!("Daily limit: {:.0} GB ({:.1} GB left)", threshold_gb, (threshold_gb - today_gb).max(0.0)),
        )
    };

    // 4. Projected Lifespan
    let avg_daily_gb = crate::analytics::average_daily_writes_gb(daily_writes);
    let avg_daily_bytes = (avg_daily_gb * crate::analytics::BYTES_PER_GB) as u64;
    let projected_days = latest_smart.and_then(|s| {
        crate::analytics::projected_days_to_depletion(
            s.percent_used,
            avg_daily_bytes,
            s.host_bytes_written(),
        )
    });
    let lifespan_str = format_projected_lifespan(projected_days);
    let lifespan_subtext = if avg_daily_gb > 0.0 {
        format!("Rate: {:.1} GB/day (7d avg)", avg_daily_gb)
    } else {
        "Awaiting write rate baseline".to_string()
    };

    ui.columns(4, |cols| {
        render_metric_card(
            &mut cols[0],
            "PERCENTAGE USED",
            &pct_str,
            &pct_subtext,
            pct_color,
            pct_val,
        );

        render_metric_card(
            &mut cols[1],
            "TOTAL HOST WRITES",
            &host_writes_str,
            &host_writes_subtext,
            Color32::from_rgb(70, 160, 245),
            None,
        );

        render_metric_card(
            &mut cols[2],
            "WRITES TODAY",
            &today_writes_str,
            &today_subtext,
            today_color,
            None,
        );

        render_metric_card(
            &mut cols[3],
            "PROJECTED LIFESPAN",
            &lifespan_str,
            &lifespan_subtext,
            Color32::from_rgb(120, 220, 140),
            None,
        );
    });
}

/// Renders the 30-day daily writes bar chart using `egui_plot`.
fn render_chart_section(ui: &mut egui::Ui, daily_writes: &[(NaiveDate, u64)]) {
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(28, 30, 36))
        .stroke(Stroke::new(1.0, Color32::from_rgb(45, 48, 58)))
        .corner_radius(6.0)
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.label(
                RichText::new("DAILY WRITES (LAST 30 DAYS)")
                    .size(12.0)
                    .strong()
                    .color(Color32::from_rgb(180, 185, 200)),
            );
            ui.add_space(6.0);

            if daily_writes.is_empty() {
                ui.label(
                    RichText::new(
                        "No daily write history recorded yet. Background monitoring active.",
                    )
                    .color(Color32::GRAY)
                    .italics(),
                );
            } else {
                let mut sorted = daily_writes.to_vec();
                sorted.sort_by_key(|(d, _)| *d);
                let start_idx = sorted.len().saturating_sub(30);
                let recent = &sorted[start_idx..];

                let date_labels: Vec<String> = recent
                    .iter()
                    .map(|(d, _)| d.format("%m/%d").to_string())
                    .collect();

                let mut bars = Vec::with_capacity(recent.len());
                for (idx, (d, bytes)) in recent.iter().enumerate() {
                    let gb = *bytes as f64 / crate::analytics::BYTES_PER_GB;
                    let label = format!("{}: {:.2} GB", d.format("%b %d, %Y"), gb);
                    let bar = egui_plot::Bar::new(idx as f64, gb)
                        .name(label)
                        .width(0.65)
                        .fill(Color32::from_rgb(59, 130, 246));
                    bars.push(bar);
                }

                let chart = egui_plot::BarChart::new("Daily Writes", bars)
                    .width(0.65)
                    .color(Color32::from_rgb(59, 130, 246));

                let labels_arc = Arc::new(date_labels);
                let labels_formatter = Arc::clone(&labels_arc);

                egui_plot::Plot::new("daily_writes_chart")
                    .height(180.0)
                    .show_axes([true, true])
                    .show_grid([true, true])
                    .y_axis_label("GB Written")
                    .x_axis_label("Date")
                    .allow_zoom(false)
                    .allow_drag(false)
                    .allow_scroll(false)
                    .x_axis_formatter(move |mark, _range| {
                        let idx = mark.value.round() as usize;
                        if mark.value >= 0.0
                            && (mark.value - idx as f64).abs() < 1e-4
                            && idx < labels_formatter.len()
                        {
                            labels_formatter[idx].clone()
                        } else {
                            String::new()
                        }
                    })
                    .show(ui, |plot_ui| {
                        plot_ui.bar_chart(chart);
                    });
            }
        });
}

/// Renders the top process writing table with Today / 7 Days / 30 Days toggle.
fn render_process_table(
    ui: &mut egui::Ui,
    time_filter: &mut TimeFilter,
    top_today: &[(String, u64)],
    top_7d: &[(String, u64)],
    top_30d: &[(String, u64)],
) {
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(28, 30, 36))
        .stroke(Stroke::new(1.0, Color32::from_rgb(45, 48, 58)))
        .corner_radius(6.0)
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("TOP WRITING PROCESSES")
                        .size(12.0)
                        .strong()
                        .color(Color32::from_rgb(180, 185, 200)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .selectable_label(*time_filter == TimeFilter::ThirtyDays, "30 Days")
                        .clicked()
                    {
                        *time_filter = TimeFilter::ThirtyDays;
                    }
                    if ui
                        .selectable_label(*time_filter == TimeFilter::SevenDays, "7 Days")
                        .clicked()
                    {
                        *time_filter = TimeFilter::SevenDays;
                    }
                    if ui
                        .selectable_label(*time_filter == TimeFilter::Today, "Today")
                        .clicked()
                    {
                        *time_filter = TimeFilter::Today;
                    }
                });
            });

            ui.add_space(8.0);

            let procs = match *time_filter {
                TimeFilter::Today => top_today,
                TimeFilter::SevenDays => top_7d,
                TimeFilter::ThirtyDays => top_30d,
            };

            if procs.is_empty() {
                ui.label(
                    RichText::new("No process write deltas recorded yet for this period.")
                        .color(Color32::GRAY)
                        .italics(),
                );
            } else {
                let total_proc_bytes: u64 = procs.iter().map(|(_, b)| *b).sum();

                egui::Grid::new("process_table_grid")
                    .striped(true)
                    .min_col_width(60.0)
                    .spacing([24.0, 8.0])
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new("Rank")
                                .strong()
                                .color(Color32::from_rgb(160, 165, 180)),
                        );
                        ui.label(
                            RichText::new("Process Name")
                                .strong()
                                .color(Color32::from_rgb(160, 165, 180)),
                        );
                        ui.label(
                            RichText::new("Bytes Written")
                                .strong()
                                .color(Color32::from_rgb(160, 165, 180)),
                        );
                        ui.label(
                            RichText::new("% of Total")
                                .strong()
                                .color(Color32::from_rgb(160, 165, 180)),
                        );
                        ui.end_row();

                        for (idx, (name, bytes)) in procs.iter().take(10).enumerate() {
                            let pct = if total_proc_bytes > 0 {
                                (*bytes as f64 / total_proc_bytes as f64) * 100.0
                            } else {
                                0.0
                            };

                            ui.label(RichText::new(format!("#{}", idx + 1)).strong());
                            ui.monospace(name);
                            ui.label(format_bytes(*bytes));
                            ui.label(format!("{:.1}%", pct));
                            ui.end_row();
                        }
                    });
            }
        });
}

/// Renders the collapsible Settings Panel with alert threshold slider and start-with-Windows checkbox.
fn render_settings_panel(ui: &mut egui::Ui, app: &mut SsdWatchApp) {
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(28, 30, 36))
        .stroke(Stroke::new(1.0, Color32::from_rgb(45, 48, 58)))
        .corner_radius(6.0)
        .inner_margin(12.0)
        .show(ui, |ui| {
            egui::CollapsingHeader::new(
                RichText::new("⚙ Settings & Actions")
                    .size(13.0)
                    .strong()
                    .color(Color32::from_rgb(200, 205, 220)),
            )
            .default_open(false)
            .show(ui, |ui| {
                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    ui.label("smartctl.exe Path:");
                    ui.add(
                        egui::TextEdit::singleline(&mut app.temp_custom_smartctl_path)
                            .hint_text("Auto-detected / Default")
                            .desired_width(280.0),
                    );
                    if ui.button("📁 Browse...").clicked() {
                        app.browse_for_smartctl();
                    }
                    if ui.button("Auto-detect / Clear").clicked() {
                        app.temp_custom_smartctl_path.clear();
                        if let Ok(mut s) = app.state.lock() {
                            s.settings.custom_smartctl_path = None;
                            let _ = s.settings.save();
                            s.data_changed = true;
                        }
                        crate::smart::clear_cached_smartctl_path();
                        app.trigger_refresh();
                    }
                });

                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    ui.label("Daily Alert Threshold (GB):");
                    ui.add(
                        egui::Slider::new(&mut app.temp_alert_threshold, 1.0..=1000.0)
                            .step_by(5.0)
                            .suffix(" GB"),
                    );
                });

                ui.add_space(4.0);

                ui.checkbox(
                    &mut app.temp_start_with_windows,
                    "Start with Windows (Launch on system startup)",
                );

                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    if ui.button("💾 Save Settings").clicked() {
                        let mut result = Ok(());
                        if let Ok(mut s) = app.state.lock() {
                            s.settings.alert_threshold_gb = app.temp_alert_threshold;
                            s.settings.start_with_windows = app.temp_start_with_windows;
                            s.settings.custom_smartctl_path = if app.temp_custom_smartctl_path.trim().is_empty() {
                                None
                            } else {
                                Some(app.temp_custom_smartctl_path.trim().to_string())
                            };
                            if let Err(e) = s.settings.save() {
                                result = Err(format!("Failed to save settings: {e}"));
                            } else if let Err(e) = s.settings.apply_auto_launch() {
                                result = Err(format!("Saved settings, but auto-launch error: {e}"));
                            } else {
                                s.data_changed = true;
                            }
                        } else {
                            result = Err("Could not acquire state lock".to_string());
                        }

                        crate::smart::clear_cached_smartctl_path();
                        app.trigger_refresh();

                        match result {
                            Ok(()) => {
                                app.settings_status_msg = Some((
                                    "✓ Settings saved and applied successfully".to_string(),
                                    false,
                                ));
                            }
                            Err(err) => {
                                app.settings_status_msg = Some((format!("✗ {err}"), true));
                            }
                        }
                    }

                    if ui.button("🔄 Refresh Now").clicked() {
                        app.trigger_refresh();
                    }
                });

                if let Some((ref msg, is_err)) = app.settings_status_msg {
                    ui.add_space(4.0);
                    let color = if is_err {
                        Color32::from_rgb(255, 100, 100)
                    } else {
                        Color32::from_rgb(100, 220, 120)
                    };
                    ui.label(RichText::new(msg).color(color));
                }
            });
        });
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
        // 1420 days / 365.25 ≈ 3.9 years
        assert_eq!(
            format_projected_lifespan(Some(1420)),
            "1,420 days (~3.9 years)"
        );
        // 3650 days / 365.25 ≈ 10.0 years
        assert_eq!(
            format_projected_lifespan(Some(3650)),
            "3,650 days (~10.0 years)"
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
    fn test_ssd_watch_app_creation() {
        let settings = Settings {
            alert_threshold_gb: 75.0,
            start_with_windows: true,
            last_notified_date: None,
            custom_smartctl_path: None,
        };
        let state = new_app_state(settings);
        let app = SsdWatchApp::new(state);

        assert_eq!(app.time_filter, TimeFilter::Today);
        assert!((app.temp_alert_threshold - 75.0).abs() < 1e-6);
        assert!(app.temp_start_with_windows);
        assert_eq!(app.settings_status_msg, None);
        assert!(!app.is_refreshing);
    }
}
