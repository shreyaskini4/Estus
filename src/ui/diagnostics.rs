use crate::theme::{
    self, AppIcon, Theme, FONT_BODY, FONT_HEADING, FONT_LABEL, FONT_MONO, RADIUS_MD, SPACE_LG,
    SPACE_MD, SPACE_SM, SPACE_XS,
};
use crate::ui::{format_intervals_csv, format_with_commas, SsdWatchApp, UiSnapshot};

/// Renders the Diagnostics view (`smartctl` status, raw output, and 24-hour intervals).
pub fn render(ui: &mut egui::Ui, theme: &Theme, app: &mut SsdWatchApp, snap: &UiSnapshot) {
    theme::section_header(
        ui,
        theme,
        "Diagnostics",
        Some("smartctl resolution, raw telemetry output, and 24-hour write intervals"),
    );
    ui.add_space(SPACE_LG);

    egui::ScrollArea::vertical()
        .id_salt("diagnostics_scroll_area")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            render_smartctl_status_card(ui, theme, app, snap);
            ui.add_space(SPACE_MD);

            render_raw_output_card(ui, theme, app, snap);
            ui.add_space(SPACE_MD);

            render_intervals_card(ui, theme, app, snap);
            ui.add_space(SPACE_LG);
        });
}

fn render_smartctl_status_card(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut SsdWatchApp,
    snap: &UiSnapshot,
) {
    theme::card(ui, theme, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new("smartctl Status")
                    .size(FONT_HEADING)
                    .strong()
                    .color(theme.text_primary),
            );
            ui.add_space(SPACE_SM);

            if snap.resolved_smartctl_path.is_some() {
                theme::badge(ui, theme, "Found", theme.health_green);
            } else {
                theme::badge(ui, theme, "Not Found", theme.health_red);
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::pill_button(ui, theme, "Re-detect", Some(AppIcon::Rescan), false)
                    .clicked()
                {
                    app.trigger_redetect();
                }
                ui.add_space(SPACE_XS);
                if theme::pill_button(ui, theme, "Browse...", Some(AppIcon::Folder), false)
                    .clicked()
                {
                    app.browse_for_smartctl();
                }
            });
        });

        ui.add_space(SPACE_MD);

        egui::Grid::new("diagnostics_smartctl_meta")
            .num_columns(2)
            .spacing([SPACE_LG, SPACE_XS])
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("Resolved Path")
                        .size(FONT_LABEL)
                        .color(theme.text_muted),
                );
                ui.label(
                    egui::RichText::new(
                        snap.resolved_smartctl_path
                            .as_deref()
                            .unwrap_or("Not resolved"),
                    )
                    .size(FONT_MONO)
                    .monospace()
                    .color(theme.text_primary),
                );
                ui.end_row();

                ui.label(
                    egui::RichText::new("Version")
                        .size(FONT_LABEL)
                        .color(theme.text_muted),
                );
                ui.label(
                    egui::RichText::new(snap.smartctl_version.as_deref().unwrap_or("Unknown"))
                        .size(FONT_MONO)
                        .monospace()
                        .color(theme.text_secondary),
                );
                ui.end_row();

                ui.label(
                    egui::RichText::new("Active Device")
                        .size(FONT_LABEL)
                        .color(theme.text_muted),
                );
                ui.label(
                    egui::RichText::new(snap.active_device.as_deref().unwrap_or("None"))
                        .size(FONT_MONO)
                        .monospace()
                        .color(theme.text_secondary),
                );
                ui.end_row();

                if let Some(err) = &snap.last_raw_error {
                    ui.label(
                        egui::RichText::new("Last Error")
                            .size(FONT_LABEL)
                            .color(theme.health_red),
                    );
                    ui.label(
                        egui::RichText::new(err)
                            .size(FONT_MONO)
                            .monospace()
                            .color(theme.health_red),
                    );
                    ui.end_row();
                }
            });
    });
}

fn render_raw_output_card(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut SsdWatchApp,
    snap: &UiSnapshot,
) {
    theme::card(ui, theme, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            let toggle_resp = ui.horizontal(|ui| {
                let icon = if app.raw_output_expanded {
                    AppIcon::ChevronLeft
                } else {
                    AppIcon::ChevronRight
                };
                let (icon_rect, icon_resp) =
                    ui.allocate_exact_size(egui::Vec2::splat(16.0), egui::Sense::click());
                theme::paint_icon(ui.painter(), icon_rect, icon, theme.text_secondary);
                let title_resp = ui.add(
                    egui::Label::new(
                        egui::RichText::new("Raw smartctl Output")
                            .size(FONT_HEADING)
                            .strong()
                            .color(theme.text_primary),
                    )
                    .sense(egui::Sense::click()),
                );
                icon_resp.union(title_resp)
            });
            if toggle_resp.inner.clicked() {
                app.raw_output_expanded = !app.raw_output_expanded;
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(raw) = &snap.last_raw_output
                    && theme::pill_button(ui, theme, "Copy", None, false).clicked()
                {
                    ui.ctx().copy_text(raw.clone());
                }
            });
        });

        if app.raw_output_expanded {
            ui.add_space(SPACE_MD);
            egui::Frame::new()
                .fill(theme.bg_elevated)
                .stroke(egui::Stroke::new(1.0, theme.border_subtle))
                .corner_radius(egui::CornerRadius::same(RADIUS_MD as u8))
                .inner_margin(egui::Margin::same(SPACE_MD as i8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let text = snap
                        .last_raw_output
                        .as_deref()
                        .unwrap_or("No raw smartctl output captured yet.");
                    egui::ScrollArea::both()
                        .id_salt("diagnostics_raw_output_scroll")
                        .max_height(260.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(text)
                                        .size(FONT_MONO)
                                        .monospace()
                                        .color(theme.text_secondary),
                                )
                                .selectable(true),
                            );
                        });
                });
        }
    });
}

fn render_intervals_card(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut SsdWatchApp,
    snap: &UiSnapshot,
) {
    theme::card(ui, theme, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new("Intervals (Last 24 Hours)")
                    .size(FONT_HEADING)
                    .strong()
                    .color(theme.text_primary),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::pill_button(ui, theme, "Export CSV", Some(AppIcon::Export), false)
                    .clicked()
                {
                    export_intervals_csv(app, &snap.selected_intervals_24h);
                }
            });
        });

        if let Some((msg, is_error)) = &app.csv_status_msg {
            ui.add_space(SPACE_XS);
            let color = if *is_error {
                theme.health_red
            } else {
                theme.health_green
            };
            ui.label(egui::RichText::new(msg).size(FONT_BODY).color(color));
        }

        ui.add_space(SPACE_MD);

        if snap.selected_intervals_24h.is_empty() {
            ui.label(
                egui::RichText::new(
                    "No 24-hour interval snapshots recorded yet for the current selection.",
                )
                .size(FONT_BODY)
                .color(theme.text_muted),
            );
            return;
        }

        egui::ScrollArea::vertical()
            .id_salt("diagnostics_intervals_scroll")
            .max_height(320.0)
            .show(ui, |ui| {
                egui::Grid::new("diagnostics_intervals_grid")
                    .num_columns(4)
                    .striped(true)
                    .spacing([SPACE_LG, SPACE_SM])
                    .min_col_width(90.0)
                    .show(ui, |ui| {
                        for header in ["Local Time", "Raw Data Units", "Delta (GB)", "Attribution"] {
                            ui.label(
                                egui::RichText::new(header)
                                    .size(FONT_LABEL)
                                    .strong()
                                    .color(theme.text_muted),
                            );
                        }
                        ui.end_row();

                        for interval in &snap.selected_intervals_24h {
                            let local_time = format_local_timestamp(interval.timestamp);
                            ui.label(
                                egui::RichText::new(local_time)
                                    .size(FONT_MONO)
                                    .monospace()
                                    .color(theme.text_primary),
                            );
                            ui.label(
                                egui::RichText::new(format_with_commas(
                                    interval.raw_data_units_written,
                                ))
                                .size(FONT_MONO)
                                .monospace()
                                .color(theme.text_secondary),
                            );
                            ui.label(
                                egui::RichText::new(format!("{:.4} GB", interval.delta_gb))
                                    .size(FONT_MONO)
                                    .monospace()
                                    .color(theme.text_primary),
                            );

                            let badge_color = attribution_badge_color(theme, &interval.attribution);
                            theme::badge(ui, theme, &interval.attribution, badge_color);
                            ui.end_row();
                        }
                    });
            });
    });
}

fn export_intervals_csv(app: &mut SsdWatchApp, intervals: &[crate::db::SnapshotInterval]) {
    if let Some(path) = rfd::FileDialog::new()
        .add_filter("CSV files", &["csv"])
        .set_file_name("estus_intervals_24h.csv")
        .set_title("Export 24-Hour Write Intervals")
        .save_file()
    {
        let csv = format_intervals_csv(intervals);
        match std::fs::write(&path, csv) {
            Ok(()) => {
                app.csv_status_msg = Some((
                    format!("Exported {} intervals to {}", intervals.len(), path.display()),
                    false,
                ));
            }
            Err(e) => {
                app.csv_status_msg = Some((format!("Failed to export CSV: {e}"), true));
            }
        }
    }
}

fn format_local_timestamp(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| ts.to_string())
}

fn attribution_badge_color(theme: &Theme, attribution: &str) -> egui::Color32 {
    match attribution {
        "app not running" => theme.health_amber,
        "baseline" => theme.text_muted,
        _ => theme.accent_text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attribution_badge_color_variants() {
        let dark = Theme::dark();
        assert_eq!(attribution_badge_color(&dark, "app not running"), dark.health_amber);
        assert_eq!(attribution_badge_color(&dark, "baseline"), dark.text_muted);
        assert_eq!(attribution_badge_color(&dark, "2026-11-26"), dark.accent_text);

        let light = Theme::light();
        assert_eq!(attribution_badge_color(&light, "app not running"), light.health_amber);
        assert_eq!(attribution_badge_color(&light, "baseline"), light.text_muted);
        assert_eq!(attribution_badge_color(&light, "2026-11-26"), light.accent_text);
    }
}
