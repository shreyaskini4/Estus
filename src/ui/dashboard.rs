use std::collections::HashMap;
use std::time::Instant;

use chrono::{Duration, Local, NaiveDate};
use egui::epaint::PathShape;
use egui::{
    Align2, Color32, CornerRadius, FontFamily, FontId, Painter, Pos2, Rect, Sense, Shape, Stroke,
    StrokeKind, Ui, Vec2,
};

use crate::analytics::{
    average_daily_writes_gb, historical_daily_writes_gb, is_daily_threshold_exceeded,
    projected_days_to_depletion, writes_per_power_on_hour_gb, BYTES_PER_GB,
};
use crate::db::DailyWrites;
use crate::theme::{
    self, AppIcon, Theme, FONT_BODY, FONT_HERO, FONT_LABEL, FONT_METRIC, RADIUS_MD, RADIUS_XS,
    SPACE_LG, SPACE_MD, SPACE_SM, SPACE_XS,
};
use crate::ui::{
    format_bytes, format_projected_lifespan, format_with_commas, SsdWatchApp, UiSnapshot,
};

const GAUGE_ANIM_DURATION_SECS: f32 = 0.45;
const CHART_DAYS: usize = 30;

/// Renders the main Dashboard view: header, hero wear gauge + lifespan, 6 stat tiles, and 30-day chart.
pub fn render(ui: &mut Ui, theme: &Theme, app: &mut SsdWatchApp, snap: &UiSnapshot) {
    render_header(ui, theme, app, snap);
    ui.add_space(SPACE_LG);

    if snap.detected_drives.is_empty() {
        if snap.smart_error.is_some() {
            super::onboarding::render_empty_state(ui, theme, app, snap);
        } else {
            super::onboarding::render_skeleton(ui, theme, app);
        }
        return;
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            render_hero_section(ui, theme, app, snap);
            ui.add_space(SPACE_LG);
            render_stat_tiles(ui, theme, snap);
            ui.add_space(SPACE_LG);
            render_daily_writes_chart(ui, theme, snap);
            ui.add_space(SPACE_SM);
        });
}

fn render_header(ui: &mut Ui, theme: &Theme, app: &mut SsdWatchApp, snap: &UiSnapshot) {
    let (title, subtitle, icon) = if let Some(drive) = &snap.selected_drive {
        let name = drive
            .nickname
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&drive.model)
            .to_string();
        let sub = format!(
            "{}  |  S/N: {}  |  {}  |  {}",
            drive.model,
            drive.serial,
            format_bytes(drive.capacity_bytes),
            drive.protocol
        );
        let ic = if drive.protocol.eq_ignore_ascii_case("nvme") {
            AppIcon::DriveNvme
        } else {
            AppIcon::DriveSata
        };
        (name, sub, ic)
    } else {
        let enabled_drives: Vec<_> = snap.detected_drives.iter().filter(|d| d.enabled).collect();
        let total_cap: u64 = enabled_drives.iter().map(|d| d.capacity_bytes).sum();
        let sub = if enabled_drives.is_empty() {
            "Scanning connected storage devices...".to_string()
        } else {
            format!(
                "{} monitored {}  |  Total capacity: {}",
                enabled_drives.len(),
                if enabled_drives.len() == 1 {
                    "drive"
                } else {
                    "drives"
                },
                format_bytes(total_cap)
            )
        };
        ("All Drives".to_string(), sub, AppIcon::DriveAll)
    };

    ui.horizontal(|ui| {
        let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
        theme::paint_icon(ui.painter(), icon_rect, icon, theme.accent_text);
        ui.add_space(SPACE_SM);
        theme::section_header(ui, theme, &title, Some(&subtitle));

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::pill_button(ui, theme, "Refresh Now", Some(AppIcon::Refresh), false).clicked()
            {
                app.trigger_refresh();
            }
        });
    });
}

fn render_hero_section(ui: &mut Ui, theme: &Theme, app: &mut SsdWatchApp, snap: &UiSnapshot) {
    let wear_pct = snap
        .selected_snapshot
        .as_ref()
        .map(|s| s.percent_used)
        .unwrap_or(0);
    let has_snapshot = snap.selected_snapshot.is_some();

    let anim_key = (snap.selected_drive_id.clone(), wear_pct);
    if app.prev_anim_key.as_ref() != Some(&anim_key) {
        app.prev_anim_key = Some(anim_key);
        app.gauge_anim_start = Some(Instant::now());
    }

    let t = app
        .gauge_anim_start
        .map_or(1.0, |s| (s.elapsed().as_secs_f32() / GAUGE_ANIM_DURATION_SECS).clamp(0.0, 1.0));
    if t < 1.0 {
        ui.ctx().request_repaint();
    }
    let eased = 1.0 - (1.0 - t).powi(3);
    let animated_wear = (wear_pct as f32) * eased;

    let avg_7d_gb = average_daily_writes_gb(&snap.selected_daily_writes);
    let total_written_bytes = snap
        .selected_snapshot
        .as_ref()
        .map(|s| s.host_bytes_written())
        .unwrap_or(0);
    let power_on_hours = snap
        .selected_snapshot
        .as_ref()
        .map(|s| s.power_on_hours)
        .unwrap_or(0);

    let effective_rate_gb = if avg_7d_gb > 0.0 {
        avg_7d_gb
    } else {
        historical_daily_writes_gb(total_written_bytes, power_on_hours)
    };
    let avg_daily_bytes = (avg_7d_gb * BYTES_PER_GB).round() as u64;
    let projected_days = snap.selected_snapshot.as_ref().and_then(|s| {
        projected_days_to_depletion(s.percent_used, avg_daily_bytes, s.host_bytes_written())
    });

    let wide = ui.available_width() >= 760.0;

    theme::card(ui, theme, |ui| {
        if wide {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_LG * 1.5;
                paint_wear_gauge(ui, theme, wear_pct, animated_wear, has_snapshot, snap);
                ui.vertical(|ui| {
                    ui.add_space(SPACE_SM);
                    render_lifespan_summary(
                        ui,
                        theme,
                        wear_pct,
                        has_snapshot,
                        projected_days,
                        effective_rate_gb,
                        avg_7d_gb > 0.0,
                    );
                });
            });
        } else {
            ui.vertical_centered(|ui| {
                paint_wear_gauge(ui, theme, wear_pct, animated_wear, has_snapshot, snap);
                ui.add_space(SPACE_MD);
                render_lifespan_summary(
                    ui,
                    theme,
                    wear_pct,
                    has_snapshot,
                    projected_days,
                    effective_rate_gb,
                    avg_7d_gb > 0.0,
                );
            });
        }
    });
}

fn paint_wear_gauge(
    ui: &mut Ui,
    theme: &Theme,
    wear_pct: u8,
    animated_wear: f32,
    has_snapshot: bool,
    snap: &UiSnapshot,
) {
    let gauge_size = Vec2::new(176.0, 164.0);
    let (rect, _) = ui.allocate_exact_size(gauge_size, Sense::hover());

    if !ui.is_rect_visible(rect) {
        return;
    }

    let painter = ui.painter();
    let center = Pos2::new(rect.center().x, rect.center().y + 4.0);
    let radius = 66.0;
    let stroke_w = 12.0;

    // 270-degree arc open at the bottom: 135 deg -> 405 deg
    let start_angle = 135.0_f32.to_radians();
    let total_sweep = 270.0_f32.to_radians();
    let segments = 64usize;

    paint_arc_stroke(
        painter,
        center,
        radius,
        start_angle,
        total_sweep,
        segments,
        Stroke::new(stroke_w, theme.border_subtle),
        true,
    );

    let health_color = if has_snapshot {
        theme.health_color(wear_pct)
    } else {
        theme.text_muted
    };

    let fill_frac = (animated_wear / 100.0).clamp(0.0, 1.0);
    if has_snapshot && fill_frac > 0.002 {
        paint_arc_stroke(
            painter,
            center,
            radius,
            start_angle,
            total_sweep * fill_frac,
            ((segments as f32 * fill_frac).ceil() as usize).max(4),
            Stroke::new(stroke_w, health_color),
            true,
        );
    }

    let hero_text = if has_snapshot {
        format!("{wear_pct}%")
    } else {
        "N/A".to_string()
    };
    painter.text(
        Pos2::new(center.x, center.y - 8.0),
        Align2::CENTER_CENTER,
        hero_text,
        FontId::new(FONT_HERO, FontFamily::Proportional),
        if has_snapshot {
            theme.text_primary
        } else {
            theme.text_muted
        },
    );

    painter.text(
        Pos2::new(center.x, center.y + 20.0),
        Align2::CENTER_CENTER,
        "USED",
        FontId::new(FONT_LABEL, FontFamily::Proportional),
        theme.text_muted,
    );

    let subtext = if !has_snapshot {
        "Awaiting SMART".to_string()
    } else if snap.selected_drive_id.is_none() {
        let count = snap.detected_drives.iter().filter(|d| d.enabled).count();
        format!("Max across {count} {}", if count == 1 { "drive" } else { "drives" })
    } else {
        format!("{}% remaining", 100u8.saturating_sub(wear_pct))
    };

    painter.text(
        Pos2::new(center.x, rect.bottom() - 8.0),
        Align2::CENTER_CENTER,
        subtext,
        FontId::new(FONT_LABEL, FontFamily::Proportional),
        theme.text_secondary,
    );
}

#[allow(clippy::too_many_arguments)]
fn paint_arc_stroke(
    painter: &Painter,
    center: Pos2,
    radius: f32,
    start_angle: f32,
    sweep: f32,
    segments: usize,
    stroke: Stroke,
    round_caps: bool,
) {
    let steps = segments.max(2);
    let pts: Vec<Pos2> = (0..=steps)
        .map(|i| {
            let frac = i as f32 / steps as f32;
            let angle = start_angle + sweep * frac;
            Pos2::new(center.x + radius * angle.cos(), center.y + radius * angle.sin())
        })
        .collect();

    if round_caps && !pts.is_empty() {
        let cap_r = stroke.width * 0.5;
        painter.circle_filled(pts[0], cap_r, stroke.color);
        if let Some(&last) = pts.last() {
            painter.circle_filled(last, cap_r, stroke.color);
        }
    }
    painter.add(Shape::Path(PathShape::line(pts, stroke)));
}

fn render_lifespan_summary(
    ui: &mut Ui,
    theme: &Theme,
    wear_pct: u8,
    has_snapshot: bool,
    projected_days: Option<u64>,
    rate_gb_per_day: f64,
    is_7d_rate: bool,
) {
    ui.label(
        egui::RichText::new("PROJECTED LIFESPAN")
            .size(FONT_LABEL)
            .strong()
            .color(theme.text_muted),
    );
    ui.add_space(SPACE_XS);

    let lifespan_text = if has_snapshot {
        format_projected_lifespan(projected_days)
    } else {
        "Unavailable".to_string()
    };

    let headline_color = if !has_snapshot {
        theme.text_muted
    } else if projected_days == Some(0) || wear_pct >= 80 {
        theme.health_red
    } else {
        theme.text_primary
    };

    ui.label(
        egui::RichText::new(lifespan_text)
            .size(FONT_METRIC)
            .strong()
            .color(headline_color),
    );

    ui.add_space(SPACE_SM);

    let rate_label = if is_7d_rate {
        format!("Rate: {:.1} GB/day (7-day avg)", rate_gb_per_day)
    } else {
        format!("Rate: {:.1} GB/day (lifetime avg)", rate_gb_per_day)
    };
    ui.label(
        egui::RichText::new(rate_label)
            .size(FONT_BODY)
            .color(theme.text_secondary),
    );

    ui.add_space(SPACE_MD);

    if has_snapshot {
        let (status_label, status_color) = if wear_pct < 50 {
            ("Healthy", theme.health_green)
        } else if wear_pct < 80 {
            ("Moderate Wear", theme.health_amber)
        } else {
            ("High Wear", theme.health_red)
        };
        theme::badge(ui, theme, status_label, status_color);
    }
}

fn render_stat_tiles(ui: &mut Ui, theme: &Theme, snap: &UiSnapshot) {
    let snapshot = snap.selected_snapshot.as_ref();
    let total_written = snapshot.map(|s| s.host_bytes_written()).unwrap_or(0);
    let total_read = snapshot.map(|s| s.host_bytes_read()).unwrap_or(0);
    let avail_spare = snapshot.map(|s| s.available_spare);
    let power_on_hours = snapshot.map(|s| s.power_on_hours).unwrap_or(0);
    let power_cycles = snapshot.map(|s| s.power_cycles).unwrap_or(0);

    let today = Local::now().date_naive();
    let today_dw = snap
        .selected_daily_writes
        .iter()
        .find(|dw| dw.date == today)
        .copied()
        .unwrap_or_default();
    let today_bytes = today_dw.total_bytes();
    let today_gb = today_bytes as f64 / BYTES_PER_GB;
    let threshold_gb = snap.settings.alert_threshold_gb;
    let exceeded = is_daily_threshold_exceeded(today_bytes, threshold_gb);

    let avg_7d_gb = average_daily_writes_gb(&snap.selected_daily_writes);
    let lifetime_avg_gb = historical_daily_writes_gb(total_written, power_on_hours);
    let display_avg_gb = if avg_7d_gb > 0.0 {
        avg_7d_gb
    } else {
        lifetime_avg_gb
    };
    let writes_per_hr = writes_per_power_on_hour_gb(total_written, power_on_hours);

    let enabled_count = snap.detected_drives.iter().filter(|d| d.enabled).count();
    let is_all_drives = snap.selected_drive_id.is_none();

    let tile_1_val = if snapshot.is_some() {
        format_bytes(total_written)
    } else {
        "N/A".to_string()
    };
    let tile_1_sub = if is_all_drives {
        format!(
            "{} monitored {}  |  Read: {}",
            enabled_count,
            if enabled_count == 1 { "drive" } else { "drives" },
            format_bytes(total_read)
        )
    } else {
        format!("Read: {}", format_bytes(total_read))
    };

    let tile_2_val = format_bytes(today_bytes);
    let tile_2_sub = if today_dw.gap_bytes > 0 {
        format!(
            "Active: {}  |  Offline: {}",
            format_bytes(today_dw.active_bytes),
            format_bytes(today_dw.gap_bytes)
        )
    } else if threshold_gb > 0.0 {
        format!("of {:.0} GB daily limit", threshold_gb)
    } else {
        "Active host writes".to_string()
    };
    let tile_2_progress = if threshold_gb > 0.0 {
        Some((today_gb / threshold_gb).clamp(0.0, 1.0) as f32)
    } else {
        None
    };
    let tile_2_color = if exceeded {
        theme.health_red
    } else {
        theme.accent_text
    };

    let tile_3_val = format!("{:.1} GB", display_avg_gb);
    let tile_3_sub = if avg_7d_gb > 0.0 {
        "7-day rolling average".to_string()
    } else {
        "Lifetime daily average".to_string()
    };

    let tile_4_val = avail_spare
        .map(|v| format!("{v}%"))
        .unwrap_or_else(|| "N/A".to_string());
    let tile_4_sub = if is_all_drives {
        "Min across drives".to_string()
    } else {
        "Reserve blocks".to_string()
    };
    let tile_4_color = match avail_spare {
        Some(v) if v <= 10 => theme.health_red,
        Some(v) if v <= 25 => theme.health_amber,
        Some(_) => theme.health_green,
        None => theme.text_muted,
    };

    let tile_5_val = if snapshot.is_some() {
        format!("{} h", format_with_commas(power_on_hours))
    } else {
        "N/A".to_string()
    };
    let tile_5_sub = format!("~{} days active", format_with_commas(power_on_hours / 24));

    let tile_6_val = if snapshot.is_some() {
        format!("{:.2} GB/h", writes_per_hr)
    } else {
        "N/A".to_string()
    };
    let tile_6_sub = format!("Power cycles: {}", format_with_commas(power_cycles));

    let tiles: [(&str, String, String, Color32, Option<f32>); 6] = [
        ("Total Host Writes", tile_1_val, tile_1_sub, theme.text_primary, None),
        ("Writes Today", tile_2_val, tile_2_sub, tile_2_color, tile_2_progress),
        ("Avg GB / Day", tile_3_val, tile_3_sub, theme.text_primary, None),
        ("Available Spare", tile_4_val, tile_4_sub, tile_4_color, None),
        ("Power-On Hours", tile_5_val, tile_5_sub, theme.text_primary, None),
        ("Writes / Hour", tile_6_val, tile_6_sub, theme.text_primary, None),
    ];

    let cols = if ui.available_width() >= 720.0 { 3 } else { 2 };
    let gap = SPACE_MD;
    let avail_w = ui.available_width().max(320.0);
    let col_w = ((avail_w - gap * (cols as f32 - 1.0)) / cols as f32).floor();

    for row_tiles in tiles.chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (label, val, sub, color, prog) in row_tiles {
                ui.allocate_ui_with_layout(
                    Vec2::new(col_w, 0.0),
                    egui::Layout::top_down_justified(egui::Align::LEFT),
                    |ui| {
                        theme::stat_tile(ui, theme, label, val, sub, *color, *prog);
                    },
                );
            }
        });
        ui.add_space(gap);
    }
}

fn render_daily_writes_chart(ui: &mut Ui, theme: &Theme, snap: &UiSnapshot) {
    let today = Local::now().date_naive();
    let series = build_30_day_series(&snap.selected_daily_writes, today);
    let threshold_gb = snap.settings.alert_threshold_gb;

    theme::card(ui, theme, |ui| {
        ui.horizontal(|ui| {
            theme::section_header(
                ui,
                theme,
                "Daily Writes (Last 30 Days)",
                Some("Active host writes vs. offline gap"),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if threshold_gb > 0.0 {
                    paint_legend_item(ui, theme, "Limit", theme.health_red, true);
                    ui.add_space(SPACE_MD);
                }
                paint_legend_item(ui, theme, "Offline gap", theme.health_amber, false);
                ui.add_space(SPACE_MD);
                paint_legend_item(ui, theme, "Active", theme.accent, false);
            });
        });

        ui.add_space(SPACE_MD);

        let chart_height = 220.0;
        let chart_width = ui.available_width().max(240.0);
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(chart_width, chart_height), Sense::hover());

        if !ui.is_rect_visible(rect) {
            return;
        }

        let painter = ui.painter();
        let left_pad = 48.0;
        let right_pad = 10.0;
        let top_pad = 12.0;
        let bottom_pad = 24.0;

        let plot = Rect::from_min_max(
            Pos2::new(rect.left() + left_pad, rect.top() + top_pad),
            Pos2::new(rect.right() - right_pad, rect.bottom() - bottom_pad),
        );

        if plot.width() <= 10.0 || plot.height() <= 10.0 {
            return;
        }

        let data_max_gb = series
            .iter()
            .map(|d| d.total_bytes() as f64 / BYTES_PER_GB)
            .fold(0.0_f64, f64::max);

        let effective_max = if threshold_gb > 0.0 && threshold_gb <= data_max_gb * 2.2 {
            data_max_gb.max(threshold_gb * 1.1)
        } else if data_max_gb == 0.0 && threshold_gb > 0.0 {
            threshold_gb * 1.1
        } else {
            data_max_gb
        };
        let y_max = nice_axis_max(effective_max);

        // Horizontal grid lines + Y-axis labels
        let grid_color = theme.border_subtle.gamma_multiply(0.6);
        let grid_steps = 4usize;
        for i in 0..=grid_steps {
            let frac = i as f32 / grid_steps as f32;
            let y = plot.bottom() - frac * plot.height();
            painter.line_segment(
                [Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)],
                Stroke::new(1.0, grid_color),
            );
            let label_val = y_max * (frac as f64);
            let label = if y_max >= 10.0 {
                format!("{:.0} GB", label_val)
            } else {
                format!("{:.1} GB", label_val)
            };
            painter.text(
                Pos2::new(plot.left() - 6.0, y),
                Align2::RIGHT_CENTER,
                label,
                FontId::new(FONT_LABEL, FontFamily::Monospace),
                theme.text_muted,
            );
        }

        let col_w = plot.width() / CHART_DAYS as f32;
        let bar_w = (col_w * 0.64).clamp(3.0, 20.0);

        let hovered_idx = response.hover_pos().and_then(|pos| {
            if plot.contains(pos) {
                let idx = ((pos.x - plot.left()) / col_w).floor() as isize;
                if idx >= 0 && (idx as usize) < series.len() {
                    Some(idx as usize)
                } else {
                    None
                }
            } else {
                None
            }
        });

        for (i, day) in series.iter().enumerate() {
            let col_left = plot.left() + i as f32 * col_w;
            let col_rect = Rect::from_min_max(
                Pos2::new(col_left, plot.top()),
                Pos2::new(col_left + col_w, plot.bottom()),
            );

            let is_hovered = hovered_idx == Some(i);
            let is_today = i + 1 == series.len();

            if is_hovered {
                painter.rect_filled(
                    col_rect,
                    CornerRadius::same(RADIUS_XS as u8),
                    theme.bg_elevated.gamma_multiply(0.75),
                );
            }

            let active_gb = day.active_bytes as f64 / BYTES_PER_GB;
            let gap_gb = day.gap_bytes as f64 / BYTES_PER_GB;

            let active_h = ((active_gb / y_max) as f32 * plot.height()).clamp(0.0, plot.height());
            let gap_h = ((gap_gb / y_max) as f32 * plot.height())
                .clamp(0.0, (plot.height() - active_h).max(0.0));

            let bar_x = col_left + (col_w - bar_w) * 0.5;
            let top_round = CornerRadius {
                nw: RADIUS_XS as u8,
                ne: RADIUS_XS as u8,
                sw: 0,
                se: 0,
            };

            let active_color = if is_today || is_hovered {
                theme.accent
            } else {
                theme.accent.gamma_multiply(0.85)
            };
            let gap_color = if is_today || is_hovered {
                theme.health_amber
            } else {
                theme.health_amber.gamma_multiply(0.85)
            };

            if active_h > 0.5 {
                let active_rect = Rect::from_min_max(
                    Pos2::new(bar_x, plot.bottom() - active_h),
                    Pos2::new(bar_x + bar_w, plot.bottom()),
                );
                let rounding = if gap_h <= 0.5 {
                    top_round
                } else {
                    CornerRadius::ZERO
                };
                painter.rect_filled(active_rect, rounding, active_color);
            }

            if gap_h > 0.5 {
                let gap_bottom = plot.bottom() - active_h;
                let gap_rect = Rect::from_min_max(
                    Pos2::new(bar_x, gap_bottom - gap_h),
                    Pos2::new(bar_x + bar_w, gap_bottom),
                );
                painter.rect_filled(gap_rect, top_round, gap_color);
            }

            if is_today && (active_h + gap_h) > 1.0 {
                let full_rect = Rect::from_min_max(
                    Pos2::new(bar_x, plot.bottom() - active_h - gap_h),
                    Pos2::new(bar_x + bar_w, plot.bottom()),
                );
                painter.rect_stroke(
                    full_rect,
                    top_round,
                    Stroke::new(1.2, theme.text_primary.gamma_multiply(0.6)),
                    StrokeKind::Inside,
                );
            }

            // X-axis labels every 5 days + Today
            if i % 5 == 0 || is_today {
                let tick_text = if is_today {
                    "Today".to_string()
                } else {
                    day.date.format("%m/%d").to_string()
                };
                painter.text(
                    Pos2::new(col_left + col_w * 0.5, plot.bottom() + 12.0),
                    Align2::CENTER_CENTER,
                    tick_text,
                    FontId::new(FONT_LABEL, FontFamily::Proportional),
                    if is_today {
                        theme.text_primary
                    } else {
                        theme.text_muted
                    },
                );
            }
        }

        // Dashed horizontal Daily Alert Threshold line
        if threshold_gb > 0.0 && threshold_gb <= y_max {
            let ty = plot.bottom() - ((threshold_gb / y_max) as f32 * plot.height());
            let limit_color = theme.health_red.gamma_multiply(0.8);
            paint_dashed_line(painter, plot.left(), plot.right(), ty, limit_color);
            painter.text(
                Pos2::new(plot.right() - 4.0, ty - 8.0),
                Align2::RIGHT_CENTER,
                format!("Limit: {:.0} GB", threshold_gb),
                FontId::new(FONT_LABEL, FontFamily::Proportional),
                theme.health_red,
            );
        }

        // Interactive hover tooltip card
        if let (Some(idx), Some(hover_pos)) = (hovered_idx, response.hover_pos()) {
            let day = &series[idx];
            paint_chart_tooltip(ui, theme, plot, hover_pos, day, idx + 1 == series.len());
        }
    });
}

fn paint_legend_item(ui: &mut Ui, theme: &Theme, label: &str, color: Color32, dashed: bool) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = SPACE_XS;
        let (swatch, _) = ui.allocate_exact_size(Vec2::new(12.0, 12.0), Sense::hover());
        if dashed {
            let y = swatch.center().y;
            ui.painter().line_segment(
                [Pos2::new(swatch.left(), y), Pos2::new(swatch.left() + 5.0, y)],
                Stroke::new(2.0, color),
            );
            ui.painter().line_segment(
                [Pos2::new(swatch.left() + 7.0, y), Pos2::new(swatch.right(), y)],
                Stroke::new(2.0, color),
            );
        } else {
            ui.painter().rect_filled(
                swatch.shrink(2.0),
                CornerRadius::same(2),
                color,
            );
        }
        ui.label(
            egui::RichText::new(label)
                .size(FONT_LABEL)
                .color(theme.text_secondary),
        );
    });
}

fn paint_dashed_line(painter: &Painter, x_start: f32, x_end: f32, y: f32, color: Color32) {
    let dash = 6.0;
    let gap = 4.0;
    let mut x = x_start;
    let stroke = Stroke::new(1.5, color);
    while x < x_end {
        let seg_end = (x + dash).min(x_end);
        painter.line_segment([Pos2::new(x, y), Pos2::new(seg_end, y)], stroke);
        x += dash + gap;
    }
}

fn paint_chart_tooltip(
    ui: &Ui,
    theme: &Theme,
    plot: Rect,
    hover_pos: Pos2,
    day: &DailyWrites,
    is_today: bool,
) {
    let painter = ui.painter();
    let date_str = if is_today {
        format!("Today ({})", day.date.format("%Y-%m-%d"))
    } else {
        day.date.format("%Y-%m-%d").to_string()
    };
    let active_gb = day.active_bytes as f64 / BYTES_PER_GB;
    let gap_gb = day.gap_bytes as f64 / BYTES_PER_GB;
    let total_gb = day.total_bytes() as f64 / BYTES_PER_GB;

    let lines = [
        (date_str, theme.text_primary, true),
        (format!("Active:      {active_gb:.2} GB"), theme.accent_text, false),
        (format!("Offline gap: {gap_gb:.2} GB"), theme.health_amber, false),
        (format!("Total:       {total_gb:.2} GB"), theme.text_primary, true),
    ];

    let tip_w = 176.0;
    let tip_h = 84.0;
    let mut tip_x = hover_pos.x + 12.0;
    if tip_x + tip_w > plot.right() {
        tip_x = (hover_pos.x - tip_w - 12.0).max(plot.left());
    }
    let tip_y = (hover_pos.y - tip_h * 0.5).clamp(plot.top(), (plot.bottom() - tip_h).max(plot.top()));

    let tip_rect = Rect::from_min_size(Pos2::new(tip_x, tip_y), Vec2::new(tip_w, tip_h));
    painter.rect_filled(tip_rect, CornerRadius::same(RADIUS_MD as u8), theme.bg_elevated);
    painter.rect_stroke(
        tip_rect,
        CornerRadius::same(RADIUS_MD as u8),
        Stroke::new(1.0, theme.border_strong),
        StrokeKind::Inside,
    );

    for (idx, (text, color, bold)) in lines.iter().enumerate() {
        let family = if idx == 0 {
            FontFamily::Proportional
        } else {
            FontFamily::Monospace
        };
        let size = if *bold && idx == 0 {
            FONT_BODY
        } else {
            FONT_LABEL
        };
        painter.text(
            Pos2::new(tip_rect.left() + 10.0, tip_rect.top() + 14.0 + idx as f32 * 18.0),
            Align2::LEFT_CENTER,
            text,
            FontId::new(size, family),
            *color,
        );
    }
}

/// Builds a contiguous 30-day series ending on `today`, filling missing dates with zero writes.
pub fn build_30_day_series(daily_writes: &[DailyWrites], today: NaiveDate) -> Vec<DailyWrites> {
    let by_date: HashMap<NaiveDate, DailyWrites> =
        daily_writes.iter().map(|dw| (dw.date, *dw)).collect();
    let start = today - Duration::days((CHART_DAYS - 1) as i64);
    (0..CHART_DAYS)
        .map(|offset| {
            let date = start + Duration::days(offset as i64);
            by_date.get(&date).copied().unwrap_or(DailyWrites {
                date,
                active_bytes: 0,
                gap_bytes: 0,
            })
        })
        .collect()
}

/// Rounds an axis maximum up to a clean human-readable tick ceiling.
pub fn nice_axis_max(raw_max: f64) -> f64 {
    if raw_max <= 0.0 || raw_max.is_nan() {
        return 10.0;
    }
    let exp = raw_max.log10().floor();
    let base = 10.0_f64.powf(exp);
    let frac = raw_max / base;
    let nice_frac = if frac <= 1.0 {
        1.0
    } else if frac <= 2.0 {
        2.0
    } else if frac <= 5.0 {
        5.0
    } else {
        10.0
    };
    (nice_frac * base).max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_30_day_series_fills_gaps() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 26).unwrap();
        let d_prev = today - Duration::days(3);
        let input = vec![
            DailyWrites {
                date: d_prev,
                active_bytes: 5_000_000_000,
                gap_bytes: 1_000_000_000,
            },
            DailyWrites {
                date: today,
                active_bytes: 12_000_000_000,
                gap_bytes: 0,
            },
        ];

        let series = build_30_day_series(&input, today);
        assert_eq!(series.len(), 30);
        assert_eq!(series.first().unwrap().date, today - Duration::days(29));
        assert_eq!(series.last().unwrap().date, today);
        assert_eq!(series.last().unwrap().active_bytes, 12_000_000_000);
        assert_eq!(series[26].active_bytes, 5_000_000_000);
        assert_eq!(series[26].gap_bytes, 1_000_000_000);
        assert_eq!(series[0].total_bytes(), 0);
    }

    #[test]
    fn test_nice_axis_max() {
        assert_eq!(nice_axis_max(0.0), 10.0);
        assert_eq!(nice_axis_max(f64::NAN), 10.0);
        assert_eq!(nice_axis_max(7.5), 10.0);
        assert_eq!(nice_axis_max(14.0), 20.0);
        assert_eq!(nice_axis_max(42.0), 50.0);
        assert_eq!(nice_axis_max(85.0), 100.0);
        assert_eq!(nice_axis_max(135.0), 200.0);
    }
}
