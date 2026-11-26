use crate::app_core::DriveConfig;
use crate::theme::{
    self, AppIcon, Theme, ThemeMode, FONT_BODY, FONT_HEADING, FONT_LABEL, FONT_MONO, RADIUS_MD,
    SPACE_LG, SPACE_MD, SPACE_SM, SPACE_XS,
};
use crate::ui::{format_bytes, SsdWatchApp, UiSnapshot};

/// Renders the Settings view with grouped configuration cards.
pub fn render(ui: &mut egui::Ui, theme: &Theme, app: &mut SsdWatchApp, snap: &UiSnapshot) {
    theme::section_header(
        ui,
        theme,
        "Settings",
        Some("Appearance, write alerts, startup behavior, and connected drives"),
    );
    ui.add_space(SPACE_LG);

    egui::ScrollArea::vertical()
        .id_salt("settings_scroll_area")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            render_appearance_card(ui, theme, app);
            ui.add_space(SPACE_MD);

            render_alerts_card(ui, theme, app);
            ui.add_space(SPACE_MD);

            render_startup_logging_card(ui, theme, app);
            ui.add_space(SPACE_MD);

            render_drives_card(ui, theme, app, snap);
            ui.add_space(SPACE_MD);

            render_save_and_data_card(ui, theme, app, snap);
            ui.add_space(SPACE_LG);
        });
}

fn render_appearance_card(ui: &mut egui::Ui, theme: &Theme, app: &mut SsdWatchApp) {
    theme::card(ui, theme, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new("Appearance")
                        .size(FONT_HEADING)
                        .strong()
                        .color(theme.text_primary),
                );
                ui.label(
                    egui::RichText::new("Choose Dark, Light, or follow your Windows theme.")
                        .size(FONT_BODY)
                        .color(theme.text_secondary),
                );
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let options = [
                    (ThemeMode::Dark, "Dark"),
                    (ThemeMode::Light, "Light"),
                    (ThemeMode::System, "System"),
                ];
                if theme::segmented_control(
                    ui,
                    theme,
                    "settings_theme_mode",
                    &mut app.temp_theme_mode,
                    &options,
                ) && let Ok(mut s) = app.state.lock()
                {
                    s.settings.theme_mode = app.temp_theme_mode;
                    let _ = s.settings.save();
                    s.mark_data_changed();
                }
            });
        });
    });
}

fn render_alerts_card(ui: &mut egui::Ui, theme: &Theme, app: &mut SsdWatchApp) {
    theme::card(ui, theme, |ui| {
        ui.set_width(ui.available_width());
        ui.label(
            egui::RichText::new("Daily Write Alerts")
                .size(FONT_HEADING)
                .strong()
                .color(theme.text_primary),
        );
        ui.add_space(SPACE_XS);
        ui.label(
            egui::RichText::new(
                "Show a Windows toast notification once per day when writes exceed this threshold.",
            )
            .size(FONT_BODY)
            .color(theme.text_secondary),
        );
        ui.add_space(SPACE_MD);

        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("Threshold")
                    .size(FONT_BODY)
                    .color(theme.text_primary),
            );
            ui.add_space(SPACE_SM);

            let slider_width = (ui.available_width() - 120.0).clamp(120.0, 360.0);
            ui.add_sized(
                [slider_width, 20.0],
                egui::Slider::new(&mut app.temp_alert_threshold, 1.0..=1000.0).show_value(false),
            );
            ui.add_space(SPACE_SM);

            ui.add(
                egui::DragValue::new(&mut app.temp_alert_threshold)
                    .range(1.0..=1000.0)
                    .speed(1.0)
                    .fixed_decimals(0)
                    .suffix(" GB"),
            );
        });
    });
}

fn render_startup_logging_card(ui: &mut egui::Ui, theme: &Theme, app: &mut SsdWatchApp) {
    theme::card(ui, theme, |ui| {
        ui.set_width(ui.available_width());
        ui.label(
            egui::RichText::new("Startup & Logging")
                .size(FONT_HEADING)
                .strong()
                .color(theme.text_primary),
        );
        ui.add_space(SPACE_MD);

        ui.horizontal_wrapped(|ui| {
            theme::toggle_switch(ui, theme, &mut app.temp_start_with_windows);
            ui.add_space(SPACE_XS);
            ui.label(
                egui::RichText::new("Start with Windows (minimized to tray)")
                    .size(FONT_BODY)
                    .color(theme.text_primary),
            );
            ui.add_space(SPACE_SM);

            if app.autostart_system_configured {
                theme::badge(ui, theme, "Configured in Task Scheduler", theme.health_green);
            } else {
                theme::badge(ui, theme, "Not configured", theme.text_muted);
            }

            ui.add_space(SPACE_XS);
            if theme::pill_button(ui, theme, "Check", None, false).clicked() {
                app.sync_autostart_with_system();
            }
        });

        ui.add_space(SPACE_SM);

        ui.horizontal(|ui| {
            theme::toggle_switch(ui, theme, &mut app.temp_verbose_logging);
            ui.add_space(SPACE_XS);
            ui.label(
                egui::RichText::new("Verbose logging (debug traces)")
                    .size(FONT_BODY)
                    .color(theme.text_primary),
            );
        });
    });
}

fn render_drives_card(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut SsdWatchApp,
    snap: &UiSnapshot,
) {
    theme::card(ui, theme, |ui| {
        ui.set_width(ui.available_width());
        ui.label(
            egui::RichText::new("Connected Drives")
                .size(FONT_HEADING)
                .strong()
                .color(theme.text_primary),
        );
        ui.add_space(SPACE_XS);
        ui.label(
            egui::RichText::new("Enable or disable SMART polling and assign custom drive nicknames.")
                .size(FONT_BODY)
                .color(theme.text_secondary),
        );
        ui.add_space(SPACE_MD);

        if snap.detected_drives.is_empty() {
            ui.label(
                egui::RichText::new("No drives detected yet. Use Rescan in the sidebar to scan.")
                    .size(FONT_BODY)
                    .color(theme.text_muted),
            );
            return;
        }

        for (idx, drive) in snap.detected_drives.iter().enumerate() {
            let enabled_val = app.temp_drive_enabled.entry(drive.id.clone()).or_insert_with(|| {
                snap.settings
                    .drives
                    .get(&drive.id)
                    .map(|c| c.enabled)
                    .unwrap_or(drive.enabled)
            });
            let nickname_buf = app
                .temp_drive_nicknames
                .entry(drive.id.clone())
                .or_insert_with(|| {
                    snap.settings
                        .drives
                        .get(&drive.id)
                        .and_then(|c| c.nickname.clone())
                        .or_else(|| drive.nickname.clone())
                        .unwrap_or_default()
                });

            egui::Frame::new()
                .fill(theme.bg_elevated)
                .stroke(egui::Stroke::new(1.0, theme.border_subtle))
                .corner_radius(egui::CornerRadius::same(RADIUS_MD as u8))
                .inner_margin(egui::Margin::same(SPACE_MD as i8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        theme::toggle_switch(ui, theme, enabled_val);
                        ui.add_space(SPACE_XS);

                        let icon = if drive.protocol.eq_ignore_ascii_case("nvme") {
                            AppIcon::DriveNvme
                        } else {
                            AppIcon::DriveSata
                        };
                        let (icon_rect, _) =
                            ui.allocate_exact_size(egui::Vec2::splat(16.0), egui::Sense::hover());
                        theme::paint_icon(ui.painter(), icon_rect, icon, theme.accent_text);

                        ui.label(
                            egui::RichText::new(&drive.model)
                                .size(FONT_BODY)
                                .strong()
                                .color(theme.text_primary),
                        );
                        theme::badge(ui, theme, &drive.protocol, theme.accent_text);
                        theme::badge(
                            ui,
                            theme,
                            &format_bytes(drive.capacity_bytes),
                            theme.text_secondary,
                        );
                    });

                    ui.add_space(SPACE_SM);

                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            egui::RichText::new("Nickname:")
                                .size(FONT_LABEL)
                                .color(theme.text_muted),
                        );
                        ui.add_sized(
                            [200.0, 24.0],
                            egui::TextEdit::singleline(nickname_buf).hint_text("Optional nickname"),
                        );
                        ui.add_space(SPACE_MD);

                        ui.label(
                            egui::RichText::new(format!(
                                "Serial: {}  |  Device: {}",
                                if drive.serial.is_empty() {
                                    "N/A"
                                } else {
                                    &drive.serial
                                },
                                drive.device_path
                            ))
                            .size(FONT_MONO)
                            .monospace()
                            .color(theme.text_secondary),
                        );
                    });
                });

            if idx + 1 < snap.detected_drives.len() {
                ui.add_space(SPACE_SM);
            }
        }
    });
}

fn render_save_and_data_card(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut SsdWatchApp,
    snap: &UiSnapshot,
) {
    theme::card(ui, theme, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            if theme::pill_button(ui, theme, "Save Settings", Some(AppIcon::Check), true).clicked()
            {
                save_all_settings(app, snap);
            }

            ui.add_space(SPACE_SM);

            if !app.confirm_reset_history
                && theme::pill_button(ui, theme, "Reset History", None, false).clicked()
            {
                app.confirm_reset_history = true;
            }

            if let Some((msg, is_error)) = &app.settings_status_msg {
                ui.add_space(SPACE_SM);
                let color = if *is_error {
                    theme.health_red
                } else {
                    theme.health_green
                };
                ui.label(egui::RichText::new(msg).size(FONT_BODY).color(color));
            }
        });

        if app.confirm_reset_history {
            ui.add_space(SPACE_MD);
            egui::Frame::new()
                .fill(theme.bg_elevated)
                .stroke(egui::Stroke::new(1.0, theme.health_red))
                .corner_radius(egui::CornerRadius::same(RADIUS_MD as u8))
                .inner_margin(egui::Margin::same(SPACE_MD as i8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        let (warn_rect, _) =
                            ui.allocate_exact_size(egui::Vec2::splat(16.0), egui::Sense::hover());
                        theme::paint_icon(
                            ui.painter(),
                            warn_rect,
                            AppIcon::Warning,
                            theme.health_red,
                        );
                        ui.label(
                            egui::RichText::new(
                                "Purge all process write history and prune SSD snapshots to the latest baseline per drive?",
                            )
                            .size(FONT_BODY)
                            .color(theme.text_primary),
                        );
                    });
                    ui.add_space(SPACE_SM);
                    ui.horizontal(|ui| {
                        if theme::pill_button(
                            ui,
                            theme,
                            "Confirm Reset",
                            Some(AppIcon::Check),
                            true,
                        )
                        .clicked()
                        {
                            execute_reset_history(app);
                        }
                        ui.add_space(SPACE_SM);
                        if theme::pill_button(ui, theme, "Cancel", Some(AppIcon::Close), false)
                            .clicked()
                        {
                            app.confirm_reset_history = false;
                        }
                    });
                });
        }
    });
}

fn save_all_settings(app: &mut SsdWatchApp, snap: &UiSnapshot) {
    let custom_path = {
        let trimmed = app.temp_custom_smartctl_path.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    };

    let mut drive_updates: Vec<(String, Option<String>, bool)> = Vec::new();
    for drive in &snap.detected_drives {
        let nick = app
            .temp_drive_nicknames
            .get(&drive.id)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let enabled = app
            .temp_drive_enabled
            .get(&drive.id)
            .copied()
            .unwrap_or(drive.enabled);
        drive_updates.push((drive.id.clone(), nick, enabled));
    }

    let mut errors: Vec<String> = Vec::new();

    if let Ok(mut s) = app.state.lock() {
        s.settings.theme_mode = app.temp_theme_mode;
        s.settings.alert_threshold_gb = app.temp_alert_threshold.clamp(1.0, 1000.0);
        s.settings.start_with_windows = app.temp_start_with_windows;
        s.settings.verbose_logging = app.temp_verbose_logging;
        s.settings.custom_smartctl_path = custom_path;

        for (id, nick, enabled) in &drive_updates {
            s.settings.drives.insert(
                id.clone(),
                DriveConfig {
                    nickname: nick.clone(),
                    enabled: *enabled,
                },
            );
            if let Some(d) = s.detected_drives.iter_mut().find(|d| &d.id == id) {
                d.nickname = nick.clone();
                d.enabled = *enabled;
            }
        }

        if let Err(e) = s.settings.save() {
            errors.push(format!("save settings: {e}"));
        }
        if let Err(e) = s.settings.apply_autostart() {
            errors.push(format!("autostart: {e}"));
        }
        s.mark_data_changed();
    } else {
        errors.push("failed to lock application state".to_string());
    }

    if !drive_updates.is_empty() {
        match crate::db::init_db() {
            Ok(conn) => {
                for (id, nick, enabled) in &drive_updates {
                    if let Err(e) =
                        crate::db::update_drive_settings(&conn, id, nick.as_deref(), *enabled)
                    {
                        errors.push(format!("drive {id}: {e}"));
                    }
                }
            }
            Err(e) => errors.push(format!("database open: {e}")),
        }
    }

    #[cfg(not(test))]
    {
        app.autostart_system_configured = crate::autostart::is_enabled();
    }
    #[cfg(test)]
    {
        app.autostart_system_configured = app.temp_start_with_windows;
    }

    if errors.is_empty() {
        app.settings_status_msg = Some(("Settings saved.".to_string(), false));
    } else {
        app.settings_status_msg = Some((format!("Partial save error: {}", errors.join("; ")), true));
    }
}

fn execute_reset_history(app: &mut SsdWatchApp) {
    app.confirm_reset_history = false;
    match crate::db::init_db().and_then(|conn| crate::db::reset_history(&conn)) {
        Ok(()) => {
            if let Ok(mut s) = app.state.lock() {
                s.daily_writes.clear();
                s.intervals_24h.clear();
                s.per_drive_daily_writes.clear();
                s.per_drive_intervals.clear();
                s.top_processes_today.clear();
                s.top_processes_7d.clear();
                s.top_processes_30d.clear();
                s.latest_ssd_delta_bytes = None;
                s.mark_data_changed();
            }
            app.settings_status_msg =
                Some(("Write history reset to latest baseline.".to_string(), false));
            app.trigger_refresh();
        }
        Err(e) => {
            app.settings_status_msg = Some((format!("Failed to reset history: {e}"), true));
        }
    }
}
