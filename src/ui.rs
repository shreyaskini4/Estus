use std::sync::Arc;

use chrono::NaiveDate;
use egui::{Color32, RichText, Stroke};

use crate::app_core::{AppState, DriveConfig, Settings};
use crate::smart::{DriveInfo, SmartError, SmartSnapshot};

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

/// The main Estus UI application.
pub type EstusApp = SsdWatchApp;

pub struct SsdWatchApp {
    pub state: AppState,
    pub time_filter: TimeFilter,
    pub temp_alert_threshold: f64,
    pub temp_start_with_windows: bool,
    pub autostart_system_configured: bool,
    pub temp_custom_smartctl_path: String,
    pub temp_verbose_logging: bool,
    pub temp_drive_nicknames: std::collections::HashMap<String, String>,
    pub temp_drive_enabled: std::collections::HashMap<String, bool>,
    pub settings_status_msg: Option<(String, bool)>,
    pub _tray: Option<tray_icon::TrayIcon>,
    pub _show_id: Option<tray_icon::menu::MenuId>,
    pub _quit_id: Option<tray_icon::menu::MenuId>,
}

impl SsdWatchApp {
    pub fn new(state: AppState) -> Self {
        let (threshold, _start_with_win, custom_path, verbose_logging) = if let Ok(s) = state.lock() {
            (
                s.settings.alert_threshold_gb,
                s.settings.start_with_windows,
                s.settings.custom_smartctl_path.clone().unwrap_or_default(),
                s.settings.verbose_logging,
            )
        } else {
            (100.0, false, String::new(), false)
        };

        // Query actual system state from Windows Task Scheduler rather than solely trusting JSON
        #[cfg(not(test))]
        let system_autostart = crate::autostart::is_enabled();
        #[cfg(test)]
        let system_autostart = _start_with_win;

        // Keep settings synchronized if they differ from the actual system state
        if let Ok(mut s) = state.lock()
            && s.settings.start_with_windows != system_autostart {
                s.settings.start_with_windows = system_autostart;
            }

        Self {
            state,
            time_filter: TimeFilter::Today,
            temp_alert_threshold: threshold,
            temp_start_with_windows: system_autostart,
            autostart_system_configured: system_autostart,
            temp_custom_smartctl_path: custom_path,
            temp_verbose_logging: verbose_logging,
            temp_drive_nicknames: std::collections::HashMap::new(),
            temp_drive_enabled: std::collections::HashMap::new(),
            settings_status_msg: None,
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
}

impl eframe::App for SsdWatchApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.ctx().set_visuals(egui::Visuals::dark());
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));

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
            && s.should_quit {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                std::process::exit(0);
            }

        // Handle window close: cancel close and hide to system tray unless quitting
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            if let Ok(s) = self.state.lock()
                && s.should_quit {
                    return;
                }
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Visible(false));
            if let Ok(mut s) = self.state.lock() {
                s.show_window = false;
            }
        }

        if let Ok(s) = self.state.lock()
            && s.show_window {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Visible(true));
            }

        let (
            detected_drives,
            selected_drive_id,
            selected_snapshot,
            selected_daily_writes,
            per_drive_snapshots,
            smart_error,
            top_today,
            top_7d,
            top_30d,
            settings,
            resolved_path,
            smartctl_version,
            active_dev,
            last_raw_err,
            last_raw_output,
            initialization_error,
        ) = {
            if let Ok(s) = self.state.lock() {
                (
                    s.detected_drives.clone(),
                    s.selected_drive_id.clone(),
                    s.get_selected_snapshot().cloned(),
                    s.get_selected_daily_writes().to_vec(),
                    s.per_drive_snapshots.clone(),
                    s.smart_error.clone(),
                    s.top_processes_today.clone(),
                    s.top_processes_7d.clone(),
                    s.top_processes_30d.clone(),
                    s.settings.clone(),
                    s.resolved_smartctl_path.clone(),
                    s.smartctl_version.clone(),
                    s.active_device.clone(),
                    s.last_raw_error.clone(),
                    s.last_raw_output.clone(),
                    s.initialization_error.clone(),
                )
            } else {
                return;
            }
        };

        let selected_drive = selected_drive_id
            .as_ref()
            .and_then(|id| detected_drives.iter().find(|d| &d.id == id));

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(8.0);

                // Top Header Bar
                render_header(ui, self);

                ui.add_space(6.0);

                // Drive Selector Tabs & Rescan button
                render_drive_selector(ui, self, &detected_drives, selected_drive_id.as_deref());

                ui.add_space(6.0);

                // Detection status bar (resolved path, active device, browse button)
                render_status_area(
                    ui,
                    self,
                    resolved_path.as_deref(),
                    smartctl_version.as_deref(),
                    active_dev.as_deref(),
                    last_raw_err.as_deref(),
                );

                ui.add_space(8.0);

                // Top Banner / Initialization Error Card
                if let Some(ref init_err) = initialization_error {
                    render_initialization_error_banner(ui, init_err);
                    ui.add_space(6.0);
                }

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
                render_metric_cards(
                    ui,
                    selected_drive,
                    selected_snapshot.as_ref(),
                    &selected_daily_writes,
                    &detected_drives,
                    &per_drive_snapshots,
                    &settings,
                );

                ui.add_space(16.0);

                // Chart Section
                render_chart_section(ui, selected_drive, &selected_daily_writes);

                ui.add_space(16.0);

                // Top Writing Processes Table (system-wide All drives)
                render_process_table(
                    ui,
                    &mut self.time_filter,
                    &top_today,
                    &top_7d,
                    &top_30d,
                );

                ui.add_space(16.0);

                // Settings Panel (includes per-drive settings)
                render_settings_panel(ui, self, &detected_drives);

                ui.add_space(16.0);

                // Diagnostics Panel
                render_diagnostics_panel(
                    ui,
                    self,
                    resolved_path.as_deref(),
                    smartctl_version.as_deref(),
                    active_dev.as_deref(),
                    last_raw_err.as_deref(),
                    last_raw_output.as_deref(),
                );

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
            RichText::new("SSD Health & Write Monitor")
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

/// Renders horizontal tabs/buttons for selecting individual drives or "[ All drives ]", plus Rescan button.
fn render_drive_selector(
    ui: &mut egui::Ui,
    app: &mut SsdWatchApp,
    detected_drives: &[DriveInfo],
    selected_drive_id: Option<&str>,
) {
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(22, 24, 30))
        .stroke(Stroke::new(1.0, Color32::from_rgb(45, 48, 58)))
        .corner_radius(6.0)
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new("DRIVES:")
                        .size(11.0)
                        .strong()
                        .color(Color32::from_rgb(150, 155, 170)),
                );

                let is_all_selected = selected_drive_id.is_none();
                if ui
                    .selectable_label(
                        is_all_selected,
                        RichText::new("🌐 [ All drives ]").strong(),
                    )
                    .clicked()
                    && let Ok(mut s) = app.state.lock() {
                        s.selected_drive_id = None;
                        s.data_changed = true;
                    }

                for drive in detected_drives {
                    let is_selected = selected_drive_id == Some(&drive.id);
                    let label = drive.nickname.as_deref().unwrap_or_else(|| {
                        if !drive.model.is_empty() {
                            &drive.model
                        } else {
                            &drive.serial
                        }
                    });
                    let icon = if drive.protocol.to_uppercase().contains("NVME") {
                        "⚡"
                    } else {
                        "💾"
                    };
                    let button_text = format!("{icon} {label}");
                    let text = if drive.enabled {
                        RichText::new(button_text)
                    } else {
                        RichText::new(format!("{button_text} (disabled)")).color(Color32::GRAY)
                    };

                    if ui.selectable_label(is_selected, text).clicked()
                        && let Ok(mut s) = app.state.lock() {
                            s.selected_drive_id = Some(drive.id.clone());
                            s.data_changed = true;
                        }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button("🔄 Rescan")
                        .on_hover_text("Scan for newly plugged-in or unplugged SSDs")
                        .clicked()
                    {
                        app.trigger_rescan();
                    }
                });
            });
        });
}

/// Renders the small status bar directly below the drive selector.
fn render_status_area(
    ui: &mut egui::Ui,
    app: &mut SsdWatchApp,
    resolved_path: Option<&str>,
    smartctl_version: Option<&str>,
    active_device: Option<&str>,
    last_raw_error: Option<&str>,
) {
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(22, 24, 30))
        .stroke(Stroke::new(1.0, Color32::from_rgb(38, 42, 50)))
        .corner_radius(4.0)
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("smartctl:").small().strong().color(Color32::from_rgb(160, 165, 180)));

                if let Some(path) = resolved_path {
                    let short_name = std::path::Path::new(path)
                        .file_name()
                        .and_then(|f| f.to_str())
                        .unwrap_or(path);
                    ui.monospace(RichText::new(short_name).small().color(Color32::from_rgb(120, 200, 255)));

                    if let Some(ver) = smartctl_version {
                        ui.label(RichText::new(format!("({ver})")).small().color(Color32::from_rgb(120, 220, 140)));
                    }
                } else {
                    ui.label(RichText::new("not detected").small().color(Color32::from_rgb(255, 120, 120)));
                }

                if let Some(dev) = active_device {
                    ui.separator();
                    ui.label(RichText::new("Device:").small().strong().color(Color32::from_rgb(160, 165, 180)));
                    ui.monospace(RichText::new(dev).small().color(Color32::from_rgb(120, 220, 140)));
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(RichText::new("📁 Browse...").small()).clicked() {
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

/// Renders a dedicated Diagnostics panel showing resolved smartctl path, version, last error, raw output, Browse button, and Re-detect button.
fn render_diagnostics_panel(
    ui: &mut egui::Ui,
    app: &mut SsdWatchApp,
    resolved_path: Option<&str>,
    smartctl_version: Option<&str>,
    active_device: Option<&str>,
    last_raw_error: Option<&str>,
    last_raw_output: Option<&str>,
) {
    egui::CollapsingHeader::new(
        RichText::new("🛠 Diagnostics")
            .size(16.0)
            .strong()
            .color(Color32::from_rgb(220, 225, 235)),
    )
    .default_open(true)
    .show(ui, |ui| {
        egui::Frame::group(ui.style())
            .fill(Color32::from_rgb(20, 22, 28))
            .stroke(Stroke::new(1.0, Color32::from_rgb(45, 50, 62)))
            .corner_radius(6.0)
            .inner_margin(12.0)
            .show(ui, |ui| {
                egui::Grid::new("diagnostics_details_grid")
                    .num_columns(2)
                    .spacing([14.0, 6.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new("Resolved smartctl path:").strong().color(Color32::from_rgb(180, 185, 200)));
                        if let Some(p) = resolved_path {
                            ui.monospace(RichText::new(p).color(Color32::from_rgb(130, 200, 255)));
                        } else {
                            ui.label(RichText::new("Not found").color(Color32::from_rgb(255, 120, 120)));
                        }
                        ui.end_row();

                        ui.label(RichText::new("smartctl version:").strong().color(Color32::from_rgb(180, 185, 200)));
                        if let Some(ver) = smartctl_version {
                            ui.monospace(RichText::new(ver).color(Color32::from_rgb(120, 225, 150)));
                        } else {
                            ui.label(RichText::new("Unavailable").color(Color32::from_rgb(150, 155, 165)));
                        }
                        ui.end_row();

                        if let Some(dev) = active_device {
                            ui.label(RichText::new("Active device path:").strong().color(Color32::from_rgb(180, 185, 200)));
                            ui.monospace(RichText::new(dev).color(Color32::from_rgb(120, 225, 150)));
                            ui.end_row();
                        }

                        if let Some(err) = last_raw_error {
                            ui.label(RichText::new("Last error:").strong().color(Color32::from_rgb(255, 120, 120)));
                            ui.label(RichText::new(err).color(Color32::from_rgb(255, 140, 140)));
                            ui.end_row();
                        }
                    });

                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    if ui.button("📁 Browse for smartctl.exe").clicked() {
                        app.browse_for_smartctl();
                    }
                    if ui.button("🔄 Re-detect").clicked() {
                        app.trigger_redetect();
                    }
                });

                ui.add_space(10.0);
                ui.label(RichText::new("Raw Output (last smartctl execution):").strong().color(Color32::from_rgb(180, 185, 200)));

                let raw_str = last_raw_output.unwrap_or("No smartctl output captured yet.");
                egui::ScrollArea::vertical()
                    .max_height(160.0)
                    .show(ui, |ui| {
                        let mut display_text = raw_str.to_string();
                        ui.add(
                            egui::TextEdit::multiline(&mut display_text)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(f32::INFINITY)
                                .interactive(false),
                        );
                    });
            });
    });
}

/// Renders guidance banner if application startup or directory creation encountered an error.
fn render_initialization_error_banner(ui: &mut egui::Ui, err: &str) {
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(45, 20, 20))
        .stroke(Stroke::new(1.0, Color32::from_rgb(220, 53, 69)))
        .corner_radius(6.0)
        .inner_margin(12.0)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("⚠️").size(24.0));
                ui.vertical(|ui| {
                    ui.strong(
                        RichText::new("Application Initialization Error")
                            .color(Color32::from_rgb(255, 100, 100)),
                    );
                    ui.label(
                        "Estus encountered an error creating required application data or log directories.",
                    );
                    ui.label(RichText::new(err).small().color(Color32::LIGHT_GRAY));
                });
            });
        });
}

/// Renders guidance banner if SMART collection encountered an error.
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
                                "Administrator permission required to read SMART data. Please right-click Estus and select 'Run as Administrator'.",
                            );
                            ui.label(RichText::new(format!("Details: {msg}")).small().color(Color32::LIGHT_GRAY));
                        }
                        SmartError::DeviceNotFound(msg) => {
                            ui.strong(
                                RichText::new("Device Not Found")
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
/// Reflects selected drive or aggregated sums across "All drives".
fn render_metric_cards(
    ui: &mut egui::Ui,
    selected_drive: Option<&DriveInfo>,
    selected_snapshot: Option<&SmartSnapshot>,
    daily_writes: &[(NaiveDate, u64)],
    detected_drives: &[DriveInfo],
    per_drive_snapshots: &std::collections::HashMap<String, SmartSnapshot>,
    settings: &Settings,
) {
    let today = chrono::Local::now().date_naive();
    let threshold_gb = settings.alert_threshold_gb;

    let today_bytes = daily_writes
        .iter()
        .find(|(d, _)| *d == today)
        .map(|(_, b)| *b)
        .unwrap_or(0);
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

    if let Some(_drive) = selected_drive {
        // Specific drive view
        let (pct_str, pct_color, pct_subtext, pct_val) = match selected_snapshot {
            Some(s) => {
                let color = if s.percent_used < 50 {
                    Color32::from_rgb(72, 199, 142)
                } else if s.percent_used < 80 {
                    Color32::from_rgb(255, 183, 77)
                } else {
                    Color32::from_rgb(244, 67, 54)
                };

                let remaining_life = 100u8.saturating_sub(s.percent_used);
                let subtext = if s.available_spare > 0 && s.available_spare <= 100 {
                    format!("Remaining Life: {}% | Spare: {}%", remaining_life, s.available_spare)
                } else {
                    format!("Remaining Life: {}%", remaining_life)
                };

                (
                    format!("{}%", s.percent_used),
                    color,
                    subtext,
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

        let (host_writes_str, host_writes_subtext) = match selected_snapshot {
            Some(s) => (
                format_bytes(s.host_bytes_written()),
                format!("Hours: {}h | Cycles: {}", s.power_on_hours, s.power_cycles),
            ),
            None => ("N/A".to_string(), "No readings recorded".to_string()),
        };

        let (effective_daily_gb, lifespan_subtext) = match selected_snapshot {
            Some(s) => {
                let hist_gb = crate::analytics::historical_daily_writes_gb(s.host_bytes_written(), s.power_on_hours);
                if daily_writes.len() >= 7 {
                    let avg7 = crate::analytics::average_daily_writes_gb(daily_writes);
                    (avg7.max(0.1), format!("Rate: {:.1} GB/day (7d avg)", avg7))
                } else if hist_gb > 0.0 {
                    (hist_gb, format!("Rate: {:.1} GB/day (lifetime avg)", hist_gb))
                } else {
                    let avg7 = crate::analytics::average_daily_writes_gb(daily_writes);
                    let sub = if avg7 > 0.0 {
                        format!("Rate: {:.1} GB/day (7d avg)", avg7)
                    } else {
                        "Awaiting write rate baseline".to_string()
                    };
                    (avg7, sub)
                }
            }
            None => {
                let avg7 = crate::analytics::average_daily_writes_gb(daily_writes);
                let sub = if avg7 > 0.0 {
                    format!("Rate: {:.1} GB/day (7d avg)", avg7)
                } else {
                    "Awaiting write rate baseline".to_string()
                };
                (avg7, sub)
            }
        };

        let effective_daily_bytes = (effective_daily_gb * crate::analytics::BYTES_PER_GB) as u64;
        let projected_days = selected_snapshot.and_then(|s| {
            crate::analytics::projected_days_to_depletion(
                s.percent_used,
                effective_daily_bytes,
                s.host_bytes_written(),
            )
        });
        let lifespan_str = format_projected_lifespan(projected_days);

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
                &format_bytes(today_bytes),
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
    } else {
        // "All drives" aggregate view
        let (pct_str, pct_color, pct_subtext, pct_val) = if !per_drive_snapshots.is_empty() {
            let max_wear = per_drive_snapshots.values().map(|s| s.percent_used).max().unwrap_or(0);
            let avg_wear = per_drive_snapshots.values().map(|s| s.percent_used as f64).sum::<f64>()
                / per_drive_snapshots.len() as f64;
            let color = if max_wear < 50 {
                Color32::from_rgb(72, 199, 142)
            } else if max_wear < 80 {
                Color32::from_rgb(255, 183, 77)
            } else {
                Color32::from_rgb(244, 67, 54)
            };
            let avg_remaining = 100.0 - avg_wear;
            (
                format!("{max_wear}% (max)"),
                color,
                format!("Remaining: {:.0}% avg ({} drives)", avg_remaining, per_drive_snapshots.len()),
                Some(max_wear as f32 / 100.0),
            )
        } else {
            (
                "N/A".to_string(),
                Color32::from_rgb(160, 160, 160),
                if detected_drives.is_empty() {
                    "No drives detected".to_string()
                } else {
                    "SMART unavailable".to_string()
                },
                None,
            )
        };

        let total_host_bytes: u64 = per_drive_snapshots.values().map(|s| s.host_bytes_written()).sum();
        let host_writes_str = if !per_drive_snapshots.is_empty() {
            format_bytes(total_host_bytes)
        } else {
            "N/A".to_string()
        };
        let host_writes_subtext = format!("Total across {} monitored drives", per_drive_snapshots.len());

        let avg_daily_gb = crate::analytics::average_daily_writes_gb(daily_writes);
        let mut min_projected: Option<u64> = None;
        let mut total_effective_rate_gb = 0.0;
        let num_drives = per_drive_snapshots.len().max(1) as f64;

        for snap in per_drive_snapshots.values() {
            let hist_gb = crate::analytics::historical_daily_writes_gb(snap.host_bytes_written(), snap.power_on_hours);
            let drive_rate_gb = if daily_writes.len() >= 7 {
                (avg_daily_gb / num_drives).max(0.1)
            } else if hist_gb > 0.0 {
                hist_gb
            } else {
                avg_daily_gb / num_drives
            };
            total_effective_rate_gb += drive_rate_gb;
            let drive_daily_bytes = (drive_rate_gb * crate::analytics::BYTES_PER_GB) as u64;

            if let Some(days) = crate::analytics::projected_days_to_depletion(
                snap.percent_used,
                drive_daily_bytes,
                snap.host_bytes_written(),
            ) {
                min_projected = Some(match min_projected {
                    Some(m) => m.min(days),
                    None => days,
                });
            }
        }
        let lifespan_str = format_projected_lifespan(min_projected);
        let lifespan_subtext = if total_effective_rate_gb > 0.0 {
            if daily_writes.len() >= 7 {
                format!("Total rate: {:.1} GB/day (7d avg)", total_effective_rate_gb)
            } else {
                format!("Total rate: {:.1} GB/day (lifetime avg)", total_effective_rate_gb)
            }
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
                &format_bytes(today_bytes),
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
}

/// Renders the 30-day daily writes bar chart using `egui_plot`.
fn render_chart_section(
    ui: &mut egui::Ui,
    selected_drive: Option<&DriveInfo>,
    daily_writes: &[(NaiveDate, u64)],
) {
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(28, 30, 36))
        .stroke(Stroke::new(1.0, Color32::from_rgb(45, 48, 58)))
        .corner_radius(6.0)
        .inner_margin(12.0)
        .show(ui, |ui| {
            let title = if let Some(drive) = selected_drive {
                let name = drive.nickname.as_deref().unwrap_or(&drive.model);
                format!("DAILY WRITES (LAST 30 DAYS) — {name}")
            } else {
                "DAILY WRITES (LAST 30 DAYS) — All Drives".to_string()
            };

            ui.label(
                RichText::new(title)
                    .size(12.0)
                    .strong()
                    .color(Color32::from_rgb(180, 185, 200)),
            );
            ui.add_space(6.0);

            if daily_writes.is_empty() {
                ui.label(
                    RichText::new(
                        "No daily write history recorded yet for this selection. Background monitoring active.",
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
/// Labeled "All drives" as process writes are monitored globally.
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
                    RichText::new("TOP WRITING PROCESSES (All drives)")
                        .size(12.0)
                        .strong()
                        .color(Color32::from_rgb(180, 185, 200)),
                );
                ui.label(
                    RichText::new("— global system-wide")
                        .small()
                        .color(Color32::from_rgb(130, 135, 145)),
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

/// Renders the collapsible Settings Panel with drive configurations, alert threshold, and auto-launch.
fn render_settings_panel(ui: &mut egui::Ui, app: &mut SsdWatchApp, detected_drives: &[DriveInfo]) {
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

                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    ui.label("Daily Alert Threshold (GB):");
                    ui.add(
                        egui::Slider::new(&mut app.temp_alert_threshold, 1.0..=1000.0)
                            .step_by(5.0)
                            .suffix(" GB"),
                    );
                });

                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    ui.checkbox(
                        &mut app.temp_start_with_windows,
                        "Start with Windows (Launch on system startup)",
                    );
                    let (status_text, status_color) = if app.autostart_system_configured {
                        (
                            "(Scheduled Task configured: Yes)",
                            Color32::from_rgb(100, 220, 120),
                        )
                    } else {
                        (
                            "(Scheduled Task configured: No)",
                            Color32::from_rgb(160, 165, 180),
                        )
                    };
                    ui.label(RichText::new(status_text).color(status_color).small());
                    if ui.small_button("🔄 Check").clicked() {
                        app.sync_autostart_with_system();
                    }
                });

                ui.label(
                    RichText::new(
                        "Estus runs via Windows Task Scheduler with highest available privileges (no UAC prompt on logon).",
                    )
                    .small()
                    .color(Color32::from_rgb(140, 145, 160)),
                );

                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    ui.checkbox(
                        &mut app.temp_verbose_logging,
                        "Verbose logging (include debug traces for troubleshooting)",
                    );
                });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);

                ui.label(
                    RichText::new("Connected Drives Configuration:")
                        .strong()
                        .color(Color32::from_rgb(180, 185, 200)),
                );
                ui.add_space(4.0);

                if detected_drives.is_empty() {
                    ui.label(
                        RichText::new("No drives detected yet. Click 'Rescan' to detect connected SSDs.")
                            .color(Color32::GRAY)
                            .italics(),
                    );
                } else {
                    for drive in detected_drives {
                        let nickname_val = app
                            .temp_drive_nicknames
                            .entry(drive.id.clone())
                            .or_insert_with(|| drive.nickname.clone().unwrap_or_default());
                        let enabled_val = app
                            .temp_drive_enabled
                            .entry(drive.id.clone())
                            .or_insert(drive.enabled);

                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                ui.checkbox(enabled_val, "Enabled");
                                ui.label(
                                    RichText::new(&drive.model)
                                        .strong()
                                        .color(Color32::from_rgb(220, 225, 240)),
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "({}, {})",
                                        drive.protocol,
                                        format_bytes(drive.capacity_bytes)
                                    ))
                                    .small()
                                    .color(Color32::from_rgb(150, 155, 170)),
                                );
                            });

                            ui.horizontal(|ui| {
                                ui.label("Nickname:");
                                ui.add(
                                    egui::TextEdit::singleline(nickname_val)
                                        .hint_text("Custom nickname (optional)")
                                        .desired_width(200.0),
                                );
                                ui.label(
                                    RichText::new(format!("Serial: {}", drive.serial))
                                        .small()
                                        .color(Color32::GRAY),
                                );
                            });
                        });
                        ui.add_space(4.0);
                    }
                }

                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    if ui.button("💾 Save Settings").clicked() {
                        let mut result = Ok(());
                        if let Ok(mut s) = app.state.lock() {
                            s.settings.alert_threshold_gb = app.temp_alert_threshold;
                            s.settings.start_with_windows = app.temp_start_with_windows;
                            s.settings.verbose_logging = app.temp_verbose_logging;
                            crate::logger::set_verbose_logging(app.temp_verbose_logging);
                            s.settings.custom_smartctl_path = if app.temp_custom_smartctl_path.trim().is_empty() {
                                None
                            } else {
                                Some(app.temp_custom_smartctl_path.trim().to_string())
                            };

                            let conn_opt = crate::db::init_db().ok();

                            for drive in detected_drives {
                                let nick_str = app.temp_drive_nicknames.get(&drive.id).cloned().unwrap_or_default();
                                let nick_opt = if nick_str.trim().is_empty() {
                                    None
                                } else {
                                    Some(nick_str.trim().to_string())
                                };
                                let en = *app.temp_drive_enabled.get(&drive.id).unwrap_or(&true);

                                s.settings.drives.insert(
                                    drive.id.clone(),
                                    DriveConfig {
                                        nickname: nick_opt.clone(),
                                        enabled: en,
                                    },
                                );

                                if let Some(ref conn) = conn_opt {
                                    let _ = crate::db::update_drive_settings(
                                        conn,
                                        &drive.id,
                                        nick_opt.as_deref(),
                                        en,
                                    );
                                }
                            }

                            if let Err(e) = s.settings.save() {
                                result = Err(format!(
                                    "Failed to save settings: {e}. See {} for details",
                                    crate::logger::get_log_file_path().display()
                                ));
                            } else if let Err(e) = s.settings.apply_autostart() {
                                result = Err(format!(
                                    "Saved settings, but failed to configure Windows autostart: {e}. See {} for details",
                                    crate::logger::get_log_file_path().display()
                                ));
                            } else {
                                s.data_changed = true;
                            }
                        } else {
                            result = Err("Could not acquire state lock".to_string());
                        }

                        #[cfg(not(test))]
                        {
                            app.autostart_system_configured = crate::autostart::is_enabled();
                        }
                        #[cfg(test)]
                        {
                            app.autostart_system_configured = app.temp_start_with_windows;
                        }

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
                        app.sync_autostart_with_system();
                        app.trigger_refresh();
                    }
                });

                if let Some((ref msg, is_err)) = app.settings_status_msg {
                    ui.add_space(6.0);
                    let color = if is_err {
                        Color32::from_rgb(255, 100, 100)
                    } else {
                        Color32::from_rgb(100, 220, 120)
                    };
                    ui.add(egui::Label::new(RichText::new(msg).color(color).strong()).wrap());
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
    fn test_ssd_watch_app_creation() {
        let settings = Settings {
            alert_threshold_gb: 75.0,
            start_with_windows: true,
            last_notified_date: None,
            custom_smartctl_path: None,
            drives: std::collections::HashMap::new(),
            verbose_logging: true,
        };
        let state = new_app_state(settings);
        let app = SsdWatchApp::new(state);

        assert_eq!(app.time_filter, TimeFilter::Today);
        assert!((app.temp_alert_threshold - 75.0).abs() < 1e-6);
        assert!(app.temp_start_with_windows);
        assert!(app.autostart_system_configured);
        assert!(app.temp_verbose_logging);
        assert_eq!(app.settings_status_msg, None);
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
        };
        let state = new_app_state(settings);
        let app = SsdWatchApp::new(state);

        assert_eq!(app.time_filter, TimeFilter::Today);
        assert!((app.temp_alert_threshold - 50.0).abs() < 1e-6);
        assert!(!app.temp_start_with_windows);
        assert!(!app.autostart_system_configured);
        assert!(!app.temp_verbose_logging);
        assert_eq!(app.settings_status_msg, None);
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
        };
        let state = new_app_state(settings);
        let mut app = SsdWatchApp::new(state);

        assert!(!app.temp_start_with_windows);
        assert!(!app.autostart_system_configured);
        assert!(!app.temp_verbose_logging);

        // Verify modifying temporary settings and synchronizing
        app.temp_start_with_windows = true;
        app.autostart_system_configured = true;
        app.temp_verbose_logging = true;
        assert!(app.temp_start_with_windows);
        assert!(app.autostart_system_configured);
        assert!(app.temp_verbose_logging);

        app.sync_autostart_with_system();
        assert_eq!(app.temp_start_with_windows, app.autostart_system_configured);
    }
}
