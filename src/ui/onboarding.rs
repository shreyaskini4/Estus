use crate::smart::SmartError;
use crate::theme::{
    self, AppIcon, Theme, FONT_BODY, FONT_HEADING, FONT_LABEL, FONT_MONO, RADIUS_LG, RADIUS_MD,
    RADIUS_SM, SPACE_MD, SPACE_SM, SPACE_XS,
};
use crate::ui::{SsdWatchApp, UiSnapshot};

const WINGET_INSTALL_CMD: &str = "winget install smartmontools.smartmontools";

/// Renders initialization or SMART error banners at the top of the main workspace.
pub fn render_banner(ui: &mut egui::Ui, theme: &Theme, app: &mut SsdWatchApp, snap: &UiSnapshot) {
    if let Some(init_err) = &snap.initialization_error {
        render_alert_frame(
            ui,
            theme,
            theme.health_red,
            "Initialization Error",
            init_err,
            None,
        );
        ui.add_space(SPACE_MD);
    }

    if let Some(smart_err) = &snap.smart_error {
        match smart_err {
            SmartError::SmartctlNotFound(detail) => {
                render_smartctl_missing_banner(ui, theme, app, detail);
                ui.add_space(SPACE_MD);
            }
            SmartError::PermissionDenied(detail) => {
                render_alert_frame(
                    ui,
                    theme,
                    theme.health_amber,
                    "Administrator Privileges Required",
                    detail,
                    Some("SMART hardware queries on Windows require elevated access. Close Estus and relaunch as Administrator."),
                );
                ui.add_space(SPACE_MD);
            }
            SmartError::DeviceNotFound(detail) => {
                render_alert_frame(
                    ui,
                    theme,
                    theme.health_amber,
                    "Storage Device Not Found",
                    detail,
                    Some("Verify the drive is connected or click Rescan in the sidebar."),
                );
                ui.add_space(SPACE_MD);
            }
            SmartError::SpawnError(detail)
            | SmartError::ParseError(detail)
            | SmartError::CommandFailed(detail) => {
                render_alert_frame(
                    ui,
                    theme,
                    theme.health_red,
                    "SMART Telemetry Error",
                    detail,
                    Some("Check the Diagnostics tab for raw smartctl output."),
                );
                ui.add_space(SPACE_MD);
            }
        }
    }
}

/// Renders animated loading skeleton cards while the first data fetch is in flight.
pub fn render_skeleton(ui: &mut egui::Ui, theme: &Theme, app: &mut SsdWatchApp) {
    let time = ui.ctx().input(|i| i.time);
    let pulse = ((time * 2.6).sin() * 0.5 + 0.5) as f32;
    let shimmer_fill = theme
        .bg_elevated
        .lerp_to_gamma(theme.border_subtle, pulse * 0.55);

    if app.shimmer_active {
        ui.ctx().request_repaint();
    }

    theme::card(ui, theme, |ui| {
        ui.set_width(ui.available_width());
        skeleton_block(ui, shimmer_fill, 180.0, 18.0);
        ui.add_space(SPACE_MD);

        let avail = ui.available_width();
        let cols = if avail >= 720.0 { 3 } else { 2 };
        let tile_w = ((avail - SPACE_MD * (cols - 1) as f32) / cols as f32).max(120.0);

        egui::Grid::new("onboarding_skeleton_tiles")
            .num_columns(cols)
            .spacing([SPACE_MD, SPACE_MD])
            .show(ui, |ui| {
                for idx in 0..6 {
                    egui::Frame::new()
                        .fill(theme.bg_elevated)
                        .stroke(egui::Stroke::new(1.0, theme.border_subtle))
                        .corner_radius(egui::CornerRadius::same(RADIUS_MD as u8))
                        .inner_margin(egui::Margin::same(SPACE_MD as i8))
                        .show(ui, |ui| {
                            ui.set_width((tile_w - SPACE_MD * 2.0).max(90.0));
                            skeleton_block(ui, shimmer_fill, 84.0, 10.0);
                            ui.add_space(SPACE_SM);
                            skeleton_block(ui, shimmer_fill, 110.0, 22.0);
                            ui.add_space(SPACE_XS);
                            skeleton_block(ui, shimmer_fill, 96.0, 10.0);
                        });
                    if (idx + 1) % cols == 0 {
                        ui.end_row();
                    }
                }
            });
    });

    ui.add_space(SPACE_MD);

    theme::card(ui, theme, |ui| {
        ui.set_width(ui.available_width());
        skeleton_block(ui, shimmer_fill, 200.0, 16.0);
        ui.add_space(SPACE_MD);
        skeleton_block(ui, shimmer_fill, ui.available_width(), 160.0);
    });
}

/// Renders an actionable onboarding card when no drives are detected or `smartctl` is missing.
pub fn render_empty_state(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut SsdWatchApp,
    snap: &UiSnapshot,
) {
    let missing_smartctl = snap.resolved_smartctl_path.is_none()
        || matches!(snap.smart_error, Some(SmartError::SmartctlNotFound(_)));

    theme::card(ui, theme, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (icon_rect, _) =
                ui.allocate_exact_size(egui::Vec2::splat(22.0), egui::Sense::hover());
            let (icon, color, title) = if missing_smartctl {
                (AppIcon::Warning, theme.health_amber, "smartctl Setup Required")
            } else {
                (AppIcon::DriveAll, theme.accent_text, "No SSD Drives Detected")
            };
            theme::paint_icon(ui.painter(), icon_rect, icon, color);
            ui.add_space(SPACE_XS);
            ui.label(
                egui::RichText::new(title)
                    .size(FONT_HEADING)
                    .strong()
                    .color(theme.text_primary),
            );
        });

        ui.add_space(SPACE_SM);
        ui.label(
            egui::RichText::new(if missing_smartctl {
                "Estus uses smartmontools (smartctl) to read NVMe and SATA SSD wear telemetry."
            } else {
                "No supported SSD drives were found in the latest scan. Ensure smartctl is installed and drives are accessible."
            })
            .size(FONT_BODY)
            .color(theme.text_secondary),
        );

        ui.add_space(SPACE_MD);

        egui::Frame::new()
            .fill(theme.bg_elevated)
            .stroke(egui::Stroke::new(1.0, theme.border_subtle))
            .corner_radius(egui::CornerRadius::same(RADIUS_MD as u8))
            .inner_margin(egui::Margin::same(SPACE_MD as i8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(
                    egui::RichText::new("1. Install smartmontools via Windows Package Manager:")
                        .size(FONT_BODY)
                        .color(theme.text_primary),
                );
                ui.add_space(SPACE_XS);

                ui.horizontal_wrapped(|ui| {
                    egui::Frame::new()
                        .fill(theme.bg_surface)
                        .stroke(egui::Stroke::new(1.0, theme.border_subtle))
                        .corner_radius(egui::CornerRadius::same(RADIUS_SM as u8))
                        .inner_margin(egui::Margin::symmetric(SPACE_SM as i8, SPACE_XS as i8))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(WINGET_INSTALL_CMD)
                                    .size(FONT_MONO)
                                    .monospace()
                                    .color(theme.accent_text),
                            );
                        });
                    ui.add_space(SPACE_XS);
                    if theme::pill_button(ui, theme, "Copy command", None, false).clicked() {
                        ui.ctx().copy_text(WINGET_INSTALL_CMD.to_string());
                    }
                });

                ui.add_space(SPACE_MD);
                ui.label(
                    egui::RichText::new(
                        "2. Or locate an existing smartctl.exe binary and re-detect devices:",
                    )
                    .size(FONT_BODY)
                    .color(theme.text_primary),
                );
                ui.add_space(SPACE_XS);

                ui.horizontal_wrapped(|ui| {
                    if theme::pill_button(
                        ui,
                        theme,
                        "Browse for smartctl.exe",
                        Some(AppIcon::Folder),
                        true,
                    )
                    .clicked()
                    {
                        app.browse_for_smartctl();
                    }
                    ui.add_space(SPACE_XS);
                    if theme::pill_button(ui, theme, "Re-detect", Some(AppIcon::Rescan), false)
                        .clicked()
                    {
                        app.trigger_redetect();
                    }
                    ui.add_space(SPACE_XS);
                    if theme::pill_button(ui, theme, "Rescan Drives", Some(AppIcon::Refresh), false)
                        .clicked()
                    {
                        app.trigger_rescan();
                    }
                });
            });
    });
}

fn render_smartctl_missing_banner(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut SsdWatchApp,
    detail: &str,
) {
    egui::Frame::new()
        .fill(theme.bg_surface)
        .stroke(egui::Stroke::new(1.0, theme.health_amber))
        .corner_radius(egui::CornerRadius::same(RADIUS_LG as u8))
        .inner_margin(egui::Margin::same(SPACE_MD as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                let (icon_rect, _) =
                    ui.allocate_exact_size(egui::Vec2::splat(18.0), egui::Sense::hover());
                theme::paint_icon(ui.painter(), icon_rect, AppIcon::Warning, theme.health_amber);
                ui.label(
                    egui::RichText::new("smartctl Not Found")
                        .size(FONT_BODY)
                        .strong()
                        .color(theme.text_primary),
                );
                ui.label(
                    egui::RichText::new(detail)
                        .size(FONT_LABEL)
                        .color(theme.text_secondary),
                );
            });

            ui.add_space(SPACE_SM);

            ui.horizontal_wrapped(|ui| {
                egui::Frame::new()
                    .fill(theme.bg_elevated)
                    .stroke(egui::Stroke::new(1.0, theme.border_subtle))
                    .corner_radius(egui::CornerRadius::same(RADIUS_SM as u8))
                    .inner_margin(egui::Margin::symmetric(SPACE_SM as i8, SPACE_XS as i8))
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(WINGET_INSTALL_CMD)
                                .size(FONT_MONO)
                                .monospace()
                                .color(theme.accent_text),
                        );
                    });

                if theme::pill_button(ui, theme, "Copy command", None, false).clicked() {
                    ui.ctx().copy_text(WINGET_INSTALL_CMD.to_string());
                }
                if theme::pill_button(
                    ui,
                    theme,
                    "Browse for smartctl.exe",
                    Some(AppIcon::Folder),
                    true,
                )
                .clicked()
                {
                    app.browse_for_smartctl();
                }
                if theme::pill_button(ui, theme, "Re-detect", Some(AppIcon::Rescan), false)
                    .clicked()
                {
                    app.trigger_redetect();
                }
            });
        });
}

fn render_alert_frame(
    ui: &mut egui::Ui,
    theme: &Theme,
    border_color: egui::Color32,
    title: &str,
    detail: &str,
    guidance: Option<&str>,
) {
    egui::Frame::new()
        .fill(theme.bg_surface)
        .stroke(egui::Stroke::new(1.0, border_color))
        .corner_radius(egui::CornerRadius::same(RADIUS_LG as u8))
        .inner_margin(egui::Margin::same(SPACE_MD as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                let (icon_rect, _) =
                    ui.allocate_exact_size(egui::Vec2::splat(18.0), egui::Sense::hover());
                theme::paint_icon(ui.painter(), icon_rect, AppIcon::Warning, border_color);
                ui.label(
                    egui::RichText::new(title)
                        .size(FONT_BODY)
                        .strong()
                        .color(theme.text_primary),
                );
                ui.label(
                    egui::RichText::new(detail)
                        .size(FONT_BODY)
                        .color(theme.text_secondary),
                );
            });
            if let Some(note) = guidance {
                ui.add_space(SPACE_XS);
                ui.label(
                    egui::RichText::new(note)
                        .size(FONT_LABEL)
                        .color(theme.text_muted),
                );
            }
        });
}

fn skeleton_block(ui: &mut egui::Ui, fill: egui::Color32, width: f32, height: f32) {
    let clamped_w = width.min(ui.available_width()).max(16.0);
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::new(clamped_w, height), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(RADIUS_SM as u8), fill);
    }
}


