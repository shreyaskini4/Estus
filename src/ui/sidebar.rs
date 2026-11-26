use egui::{Color32, CornerRadius, FontFamily, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

use crate::app_core::DriveConfig;
use crate::smart::DriveInfo;
use crate::theme::{
    self, AppIcon, Theme, FONT_BODY, FONT_HEADING, FONT_LABEL, RADIUS_MD, RADIUS_PILL, RADIUS_SM,
    RADIUS_XS, SPACE_SM, SPACE_XS,
};
use crate::ui::{format_bytes, NavTab, SsdWatchApp, UiSnapshot};

const NAV_ITEMS: [(NavTab, AppIcon, &str); 4] = [
    (NavTab::Dashboard, AppIcon::Dashboard, "Dashboard"),
    (NavTab::Processes, AppIcon::Processes, "Processes"),
    (NavTab::Settings, AppIcon::Settings, "Settings"),
    (NavTab::Diagnostics, AppIcon::Diagnostics, "Diagnostics"),
];

/// Returns the display name for a drive (trimmed nickname if non-empty, otherwise model).
pub fn drive_display_name(drive: &DriveInfo) -> &str {
    drive
        .nickname
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&drive.model)
}

/// Returns the appropriate vector icon for a drive protocol.
pub fn drive_icon(protocol: &str) -> AppIcon {
    if protocol.to_ascii_lowercase().contains("nvme") {
        AppIcon::DriveNvme
    } else {
        AppIcon::DriveSata
    }
}

fn select_drive(app: &mut SsdWatchApp, drive_id: Option<String>) {
    if let Ok(mut s) = app.state.lock()
        && s.selected_drive_id != drive_id
    {
        s.selected_drive_id = drive_id;
        s.mark_data_changed();
    }
}

fn commit_drive_nickname(app: &mut SsdWatchApp, drive: &DriveInfo) {
    let trimmed = app.editing_nickname_buf.trim();
    let new_nickname = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    };

    if let Ok(mut s) = app.state.lock() {
        let entry = s
            .settings
            .drives
            .entry(drive.id.clone())
            .or_insert_with(|| DriveConfig {
                nickname: None,
                enabled: drive.enabled,
            });
        entry.nickname = new_nickname.clone();
        let _ = s.settings.save();

        if let Some(d) = s.detected_drives.iter_mut().find(|d| d.id == drive.id) {
            d.nickname = new_nickname.clone();
        }
        s.mark_data_changed();
    }

    if let Ok(conn) = crate::db::init_db() {
        let _ = crate::db::update_drive_settings(
            &conn,
            &drive.id,
            new_nickname.as_deref(),
            drive.enabled,
        );
    }

    app.temp_drive_nicknames
        .insert(drive.id.clone(), new_nickname.unwrap_or_default());
    app.editing_drive_id = None;
    app.editing_nickname_buf.clear();
}

/// Renders the collapsible left sidebar (brand header, drives list, navigation, and smartctl footer).
pub fn render(ui: &mut egui::Ui, theme: &Theme, app: &mut SsdWatchApp, snap: &UiSnapshot) {
    let collapsed = app.sidebar_collapsed;

    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = SPACE_SM;

        // 1. Top Header
        render_header(ui, theme, app, collapsed);
        ui.add_space(SPACE_XS);

        // 2. Drives Section
        let footer_reserve = 44.0;
        let nav_reserve = if collapsed { 160.0 } else { 188.0 };
        let drives_max_h = (ui.available_height() - nav_reserve - footer_reserve).max(80.0);

        if collapsed {
            render_drives_collapsed(ui, theme, app, snap, drives_max_h);
        } else {
            render_drives_expanded(ui, theme, app, snap, drives_max_h);
        }

        // Divider between drives and navigation
        ui.add_space(SPACE_XS);
        paint_divider(ui, theme);
        ui.add_space(SPACE_XS);

        // 3. Navigation Section
        render_navigation(ui, theme, app, collapsed);

        // Push footer to bottom
        let remaining = (ui.available_height() - footer_reserve).max(0.0);
        if remaining > 0.0 {
            ui.add_space(remaining);
        }

        // 4. Footer: smartctl status pill
        render_footer(ui, theme, app, snap, collapsed);
    });
}

fn render_header(ui: &mut egui::Ui, theme: &Theme, app: &mut SsdWatchApp, collapsed: bool) {
    if collapsed {
        ui.vertical_centered(|ui| {
            let (flame_rect, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
            theme::paint_icon(ui.painter(), flame_rect, AppIcon::Flame, theme.accent);
            if theme::icon_button(ui, theme, AppIcon::ChevronRight, "Expand sidebar").clicked() {
                app.sidebar_collapsed = false;
            }
        });
    } else {
        ui.horizontal(|ui| {
            let (flame_rect, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
            theme::paint_icon(ui.painter(), flame_rect, AppIcon::Flame, theme.accent);
            ui.label(
                egui::RichText::new("Estus")
                    .size(FONT_HEADING)
                    .strong()
                    .color(theme.text_primary),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::icon_button(ui, theme, AppIcon::ChevronLeft, "Collapse sidebar")
                    .clicked()
                {
                    app.sidebar_collapsed = true;
                    app.editing_drive_id = None;
                }
            });
        });
    }
}

fn render_drives_expanded(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut SsdWatchApp,
    snap: &UiSnapshot,
    max_height: f32,
) {
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("DRIVES")
                .size(FONT_LABEL)
                .strong()
                .color(theme.text_muted),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::icon_button(ui, theme, AppIcon::Rescan, "Rescan connected drives").clicked() {
                app.trigger_rescan();
            }
        });
    });

    // "All Drives" entry
    let total_cap: u64 = snap
        .detected_drives
        .iter()
        .filter(|d| d.enabled)
        .map(|d| d.capacity_bytes)
        .sum();
    let drive_count = snap.detected_drives.len();
    let count_label = if drive_count == 1 {
        "1 drive".to_string()
    } else {
        format!("{drive_count} drives")
    };
    let all_subtitle = if total_cap > 0 {
        format!("{} · {}", format_bytes(total_cap), count_label)
    } else {
        count_label
    };
    let all_selected = snap.selected_drive_id.is_none();

    if render_all_drives_card(ui, theme, all_selected, &all_subtitle).clicked() {
        select_drive(app, None);
    }

    // Scrollable list of individual drives
    egui::ScrollArea::vertical()
        .id_salt("sidebar_drives_scroll")
        .max_height(max_height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = SPACE_XS + 2.0;
            for drive in &snap.detected_drives {
                render_drive_card_expanded(ui, theme, app, snap, drive);
            }
        });
}

fn render_all_drives_card(
    ui: &mut egui::Ui,
    theme: &Theme,
    selected: bool,
    subtitle: &str,
) -> egui::Response {
    let desired = Vec2::new(ui.available_width(), 44.0);
    let (rect, resp) = ui.allocate_exact_size(desired, Sense::click());

    if ui.is_rect_visible(rect) {
        let fill = if selected {
            theme.accent_muted
        } else if resp.hovered() {
            theme.bg_elevated
        } else {
            theme.bg_surface
        };
        let border = if selected {
            theme.accent
        } else if resp.hovered() {
            theme.border_strong
        } else {
            theme.border_subtle
        };

        ui.painter()
            .rect_filled(rect, CornerRadius::same(RADIUS_MD as u8), fill);
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(RADIUS_MD as u8),
            Stroke::new(1.0, border),
            StrokeKind::Inside,
        );

        let icon_rect = Rect::from_center_size(
            Pos2::new(rect.left() + 18.0, rect.center().y),
            Vec2::splat(16.0),
        );
        let icon_color = if selected {
            theme.accent_text
        } else {
            theme.text_secondary
        };
        theme::paint_icon(ui.painter(), icon_rect, AppIcon::DriveAll, icon_color);

        let title_pos = Pos2::new(rect.left() + 34.0, rect.top() + 7.0);
        let sub_pos = Pos2::new(rect.left() + 34.0, rect.top() + 24.0);
        ui.painter().text(
            title_pos,
            egui::Align2::LEFT_TOP,
            "All Drives",
            FontId::new(FONT_BODY, FontFamily::Proportional),
            if selected {
                theme.accent_text
            } else {
                theme.text_primary
            },
        );
        ui.painter().text(
            sub_pos,
            egui::Align2::LEFT_TOP,
            subtitle,
            FontId::new(FONT_LABEL, FontFamily::Proportional),
            theme.text_secondary,
        );
    }

    resp
}

fn render_drive_card_expanded(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut SsdWatchApp,
    snap: &UiSnapshot,
    drive: &DriveInfo,
) {
    let selected = snap.selected_drive_id.as_deref() == Some(drive.id.as_str());
    let is_editing = app.editing_drive_id.as_deref() == Some(drive.id.as_str());
    let drive_snap = snap.per_drive_snapshots.get(&drive.id);
    let wear_pct = drive_snap.map(|s| s.percent_used);

    let health_dot_color = if !drive.enabled {
        theme.text_muted
    } else if let Some(wear) = wear_pct {
        theme.health_color(wear)
    } else {
        theme.text_muted
    };

    let bg = if selected {
        theme.accent_muted
    } else {
        theme.bg_surface
    };
    let border = if selected {
        theme.accent
    } else {
        theme.border_subtle
    };

    let frame_resp = egui::Frame::new()
        .fill(bg)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(RADIUS_MD as u8))
        .inner_margin(egui::Margin::symmetric(SPACE_SM as i8, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if is_editing {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = SPACE_XS;
                    let edit_w = (ui.available_width() - 62.0).max(60.0);
                    let resp = ui.add_sized(
                        Vec2::new(edit_w, 24.0),
                        egui::TextEdit::singleline(&mut app.editing_nickname_buf)
                            .hint_text("Drive nickname"),
                    );
                    resp.request_focus();
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));

                    if theme::icon_button(ui, theme, AppIcon::Check, "Save nickname").clicked()
                        || enter
                    {
                        commit_drive_nickname(app, drive);
                    } else if theme::icon_button(ui, theme, AppIcon::Close, "Cancel").clicked()
                        || escape
                    {
                        app.editing_drive_id = None;
                        app.editing_nickname_buf.clear();
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = SPACE_SM;
                    let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                    let icon_col = if selected {
                        theme.accent_text
                    } else if drive.enabled {
                        theme.text_secondary
                    } else {
                        theme.text_muted
                    };
                    theme::paint_icon(
                        ui.painter(),
                        icon_rect,
                        drive_icon(&drive.protocol),
                        icon_col,
                    );

                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        let name = drive_display_name(drive);
                        let name_color = if !drive.enabled {
                            theme.text_muted
                        } else if selected {
                            theme.accent_text
                        } else {
                            theme.text_primary
                        };
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(name)
                                    .size(FONT_BODY)
                                    .strong()
                                    .color(name_color),
                            )
                            .truncate(),
                        );

                        let sub_text = if drive.enabled {
                            format_bytes(drive.capacity_bytes)
                        } else {
                            format!("{} · Disabled", format_bytes(drive.capacity_bytes))
                        };
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(sub_text)
                                    .size(FONT_LABEL)
                                    .color(theme.text_secondary),
                            )
                            .truncate(),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = SPACE_XS;
                        if theme::icon_button(ui, theme, AppIcon::Pencil, "Rename drive").clicked()
                        {
                            app.editing_drive_id = Some(drive.id.clone());
                            app.editing_nickname_buf = drive
                                .nickname
                                .clone()
                                .filter(|n| !n.trim().is_empty())
                                .unwrap_or_else(|| drive.model.clone());
                        }
                        let (dot_rect, _) =
                            ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                        ui.painter()
                            .circle_filled(dot_rect.center(), 3.5, health_dot_color);
                    });
                });

                // Mini 3px wear progress bar
                ui.add_space(4.0);
                let (bar_rect, _) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 3.0), Sense::hover());
                ui.painter().rect_filled(
                    bar_rect,
                    CornerRadius::same(RADIUS_XS as u8),
                    theme.border_subtle,
                );
                if let Some(wear) = wear_pct {
                    let frac = (wear as f32 / 100.0).clamp(0.0, 1.0);
                    let fill_w = (bar_rect.width() * frac).max(if wear > 0 { 2.0 } else { 0.0 });
                    if fill_w > 0.5 {
                        let fill_rect =
                            Rect::from_min_size(bar_rect.min, Vec2::new(fill_w, bar_rect.height()));
                        ui.painter().rect_filled(
                            fill_rect,
                            CornerRadius::same(RADIUS_XS as u8),
                            health_dot_color,
                        );
                    }
                }
            }
        });

    if !is_editing {
        let click_resp = ui.interact(
            frame_resp.response.rect,
            ui.scope_id().with(("drive_card_click", &drive.id)),
            Sense::click(),
        );
        if click_resp.clicked() {
            select_drive(app, Some(drive.id.clone()));
        }
    }
}

fn render_drives_collapsed(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut SsdWatchApp,
    snap: &UiSnapshot,
    max_height: f32,
) {
    let all_selected = snap.selected_drive_id.is_none();
    ui.vertical_centered(|ui| {
        if collapsed_icon_tile(
            ui,
            theme,
            AppIcon::DriveAll,
            all_selected,
            None,
            &format!("All Drives ({})", snap.detected_drives.len()),
        )
        .clicked()
        {
            select_drive(app, None);
        }

        egui::ScrollArea::vertical()
            .id_salt("sidebar_drives_collapsed_scroll")
            .max_height(max_height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.spacing_mut().item_spacing.y = SPACE_XS;
                    for drive in &snap.detected_drives {
                        let selected =
                            snap.selected_drive_id.as_deref() == Some(drive.id.as_str());
                        let wear_pct = snap.per_drive_snapshots.get(&drive.id).map(|s| s.percent_used);
                        let dot_color = if !drive.enabled {
                            theme.text_muted
                        } else if let Some(w) = wear_pct {
                            theme.health_color(w)
                        } else {
                            theme.text_muted
                        };
                        let tooltip = match wear_pct {
                            Some(w) if drive.enabled => format!(
                                "{} ({}) · {}% wear",
                                drive_display_name(drive),
                                format_bytes(drive.capacity_bytes),
                                w
                            ),
                            _ if !drive.enabled => {
                                format!("{} (Disabled)", drive_display_name(drive))
                            }
                            _ => format!(
                                "{} ({})",
                                drive_display_name(drive),
                                format_bytes(drive.capacity_bytes)
                            ),
                        };

                        if collapsed_icon_tile(
                            ui,
                            theme,
                            drive_icon(&drive.protocol),
                            selected,
                            Some(dot_color),
                            &tooltip,
                        )
                        .clicked()
                        {
                            select_drive(app, Some(drive.id.clone()));
                        }
                    }
                });
            });
    });
}

fn collapsed_icon_tile(
    ui: &mut egui::Ui,
    theme: &Theme,
    icon: AppIcon,
    selected: bool,
    dot_color: Option<Color32>,
    tooltip: &str,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(34.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if selected {
            theme.accent_muted
        } else if resp.hovered() {
            theme.bg_elevated
        } else {
            Color32::TRANSPARENT
        };
        let border = if selected {
            theme.accent
        } else if resp.hovered() {
            theme.border_subtle
        } else {
            Color32::TRANSPARENT
        };
        ui.painter()
            .rect_filled(rect, CornerRadius::same(RADIUS_SM as u8), fill);
        if border != Color32::TRANSPARENT {
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(RADIUS_SM as u8),
                Stroke::new(1.0, border),
                StrokeKind::Inside,
            );
        }
        let icon_color = if selected {
            theme.accent_text
        } else if resp.hovered() {
            theme.text_primary
        } else {
            theme.text_secondary
        };
        theme::paint_icon(ui.painter(), rect.shrink(8.0), icon, icon_color);
        if let Some(dot) = dot_color {
            let dot_pos = Pos2::new(rect.right() - 6.0, rect.top() + 6.0);
            ui.painter().circle_filled(dot_pos, 3.0, dot);
        }
    }
    resp.on_hover_text(tooltip)
}

fn render_navigation(ui: &mut egui::Ui, theme: &Theme, app: &mut SsdWatchApp, collapsed: bool) {
    if !collapsed {
        ui.label(
            egui::RichText::new("NAVIGATION")
                .size(FONT_LABEL)
                .strong()
                .color(theme.text_muted),
        );
    }

    for (tab, icon, label) in NAV_ITEMS {
        let selected = app.active_tab == tab;
        if collapsed {
            ui.vertical_centered(|ui| {
                if collapsed_icon_tile(ui, theme, icon, selected, None, label).clicked() {
                    app.active_tab = tab;
                }
            });
        } else {
            let desired = Vec2::new(ui.available_width(), 32.0);
            let (rect, resp) = ui.allocate_exact_size(desired, Sense::click());
            if ui.is_rect_visible(rect) {
                let fill = if selected {
                    theme.accent_muted
                } else if resp.hovered() {
                    theme.bg_elevated
                } else {
                    Color32::TRANSPARENT
                };
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(RADIUS_SM as u8), fill);
                if selected {
                    ui.painter().rect_stroke(
                        rect,
                        CornerRadius::same(RADIUS_SM as u8),
                        Stroke::new(1.0, theme.accent),
                        StrokeKind::Inside,
                    );
                }

                let fg = if selected {
                    theme.accent_text
                } else if resp.hovered() {
                    theme.text_primary
                } else {
                    theme.text_secondary
                };
                let icon_rect = Rect::from_center_size(
                    Pos2::new(rect.left() + 16.0, rect.center().y),
                    Vec2::splat(15.0),
                );
                theme::paint_icon(ui.painter(), icon_rect, icon, fg);
                ui.painter().text(
                    Pos2::new(rect.left() + 32.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    label,
                    FontId::new(FONT_BODY, FontFamily::Proportional),
                    fg,
                );
            }
            if resp.clicked() {
                app.active_tab = tab;
            }
        }
    }
}

fn render_footer(
    ui: &mut egui::Ui,
    theme: &Theme,
    app: &mut SsdWatchApp,
    snap: &UiSnapshot,
    collapsed: bool,
) {
    let found = snap.resolved_smartctl_path.is_some();
    let status_color = if found {
        theme.health_green
    } else {
        theme.health_red
    };
    let status_text = if found {
        "smartctl: Found"
    } else {
        "smartctl: Missing"
    };

    if collapsed {
        ui.vertical_centered(|ui| {
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
            if ui.is_rect_visible(rect) {
                ui.painter().rect_filled(
                    rect,
                    CornerRadius::same(RADIUS_SM as u8),
                    status_color.gamma_multiply(0.16),
                );
                ui.painter().rect_stroke(
                    rect,
                    CornerRadius::same(RADIUS_SM as u8),
                    Stroke::new(1.0, status_color.gamma_multiply(0.4)),
                    StrokeKind::Inside,
                );
                ui.painter().circle_filled(rect.center(), 4.0, status_color);
            }
            if resp.on_hover_text(status_text).clicked() {
                app.active_tab = NavTab::Diagnostics;
            }
        });
    } else {
        let desired = Vec2::new(ui.available_width(), 28.0);
        let (rect, resp) = ui.allocate_exact_size(desired, Sense::click());
        if ui.is_rect_visible(rect) {
            let bg = if resp.hovered() {
                status_color.gamma_multiply(0.24)
            } else {
                status_color.gamma_multiply(0.14)
            };
            ui.painter()
                .rect_filled(rect, CornerRadius::same(RADIUS_PILL as u8), bg);
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(RADIUS_PILL as u8),
                Stroke::new(1.0, status_color.gamma_multiply(0.4)),
                StrokeKind::Inside,
            );
            ui.painter()
                .circle_filled(Pos2::new(rect.left() + 14.0, rect.center().y), 3.5, status_color);
            ui.painter().text(
                Pos2::new(rect.left() + 24.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                status_text,
                FontId::new(FONT_LABEL, FontFamily::Proportional),
                status_color,
            );
        }
        if resp
            .on_hover_text("Open Diagnostics")
            .clicked()
        {
            app.active_tab = NavTab::Diagnostics;
        }
    }
}

fn paint_divider(ui: &mut egui::Ui, theme: &Theme) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().line_segment(
            [Pos2::new(rect.left(), rect.center().y), Pos2::new(rect.right(), rect.center().y)],
            Stroke::new(1.0, theme.border_subtle),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_drive(nickname: Option<&str>, protocol: &str) -> DriveInfo {
        DriveInfo {
            id: "S1_Samsung".to_string(),
            serial: "S1".to_string(),
            model: "Samsung 990 Pro".to_string(),
            capacity_bytes: 2_000_000_000_000,
            protocol: protocol.to_string(),
            device_path: "/dev/nvme0".to_string(),
            nickname: nickname.map(str::to_string),
            enabled: true,
        }
    }

    #[test]
    fn test_drive_display_name_prefers_non_empty_nickname() {
        let with_nick = sample_drive(Some("OS Drive"), "NVMe");
        assert_eq!(drive_display_name(&with_nick), "OS Drive");

        let blank_nick = sample_drive(Some("   "), "NVMe");
        assert_eq!(drive_display_name(&blank_nick), "Samsung 990 Pro");

        let no_nick = sample_drive(None, "SATA");
        assert_eq!(drive_display_name(&no_nick), "Samsung 990 Pro");
    }

    #[test]
    fn test_drive_icon_protocol_mapping() {
        assert_eq!(drive_icon("NVMe"), AppIcon::DriveNvme);
        assert_eq!(drive_icon("nvme"), AppIcon::DriveNvme);
        assert_eq!(drive_icon("SATA"), AppIcon::DriveSata);
        assert_eq!(drive_icon("ATA"), AppIcon::DriveSata);
    }
}
