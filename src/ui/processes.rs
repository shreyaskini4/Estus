use egui::{CornerRadius, FontFamily, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

use crate::theme::{
    self, AppIcon, Theme, FONT_BODY, FONT_LABEL, FONT_MONO, RADIUS_SM, RADIUS_XS, SPACE_LG,
    SPACE_MD, SPACE_SM, SPACE_XS,
};
use crate::ui::{format_bytes, SsdWatchApp, TimeFilter, UiSnapshot};

/// A single filtered process row with precomputed rank and share percentage.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessRow {
    pub rank: usize,
    pub name: String,
    pub bytes: u64,
    pub share_pct: f64,
}

/// Filters process write entries by case-insensitive substring while preserving original rank
/// and computing share percentage against the unfiltered period total.
pub fn filter_process_rows(entries: &[(String, u64)], query: &str) -> (Vec<ProcessRow>, u64) {
    let total_bytes: u64 = entries.iter().map(|(_, b)| *b).sum();
    let needle = query.trim().to_ascii_lowercase();

    let rows = entries
        .iter()
        .enumerate()
        .filter(|(_, (name, _))| {
            needle.is_empty() || name.to_ascii_lowercase().contains(&needle)
        })
        .map(|(idx, (name, bytes))| {
            let share_pct = if total_bytes > 0 {
                (*bytes as f64 / total_bytes as f64) * 100.0
            } else {
                0.0
            };
            ProcessRow {
                rank: idx + 1,
                name: name.clone(),
                bytes: *bytes,
                share_pct,
            }
        })
        .collect();

    (rows, total_bytes)
}

/// Renders the Processes view with time-range filter, search input, and proportional write bars.
pub fn render(ui: &mut egui::Ui, theme: &Theme, app: &mut SsdWatchApp, snap: &UiSnapshot) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = SPACE_LG;

        // 1. Header & Controls
        ui.horizontal_wrapped(|ui| {
            theme::section_header(
                ui,
                theme,
                "Top Writing Processes",
                Some("Estimated share of SSD writes (system-wide)"),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                theme::segmented_control(
                    ui,
                    theme,
                    "processes_time_filter",
                    &mut app.time_filter,
                    &[
                        (TimeFilter::Today, "Today"),
                        (TimeFilter::SevenDays, "7 Days"),
                        (TimeFilter::ThirtyDays, "30 Days"),
                    ],
                );
            });
        });

        // Search / filter bar
        render_search_bar(ui, theme, app);

        // 2. Process List
        let source = match app.time_filter {
            TimeFilter::Today => &snap.top_today,
            TimeFilter::SevenDays => &snap.top_7d,
            TimeFilter::ThirtyDays => &snap.top_30d,
        };
        let (rows, total_bytes) = filter_process_rows(source, &app.process_filter);

        egui::ScrollArea::vertical()
            .id_salt("processes_list_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                theme::card(ui, theme, |ui| {
                    if source.is_empty() {
                        render_empty_state(
                            ui,
                            theme,
                            "No process write activity recorded yet",
                            "Process I/O deltas will appear here after the next background polling cycle.",
                        );
                        return;
                    }

                    if rows.is_empty() {
                        render_empty_state(
                            ui,
                            theme,
                            "No matching processes",
                            "Try clearing or adjusting your search filter.",
                        );
                        return;
                    }

                    render_table_header(ui, theme);
                    paint_row_divider(ui, theme);

                    for (idx, row) in rows.iter().enumerate() {
                        render_process_row(ui, theme, row, total_bytes);
                        if idx + 1 < rows.len() {
                            paint_row_divider(ui, theme);
                        }
                    }
                });
            });
    });
}

fn render_search_bar(ui: &mut egui::Ui, theme: &Theme, app: &mut SsdWatchApp) {
    egui::Frame::new()
        .fill(theme.bg_surface)
        .stroke(Stroke::new(1.0, theme.border_subtle))
        .corner_radius(CornerRadius::same(RADIUS_SM as u8))
        .inner_margin(egui::Margin::symmetric(SPACE_MD as i8, SPACE_XS as i8 + 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_SM;
                let (search_rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                theme::paint_icon(ui.painter(), search_rect, AppIcon::Search, theme.text_muted);

                let clear_reserve = if app.process_filter.is_empty() {
                    0.0
                } else {
                    32.0
                };
                let edit_w = (ui.available_width() - clear_reserve).max(120.0);
                ui.add_sized(
                    Vec2::new(edit_w, 24.0),
                    egui::TextEdit::singleline(&mut app.process_filter)
                        .hint_text("Filter processes...")
                        .frame(egui::Frame::NONE),
                );

                if !app.process_filter.is_empty()
                    && theme::icon_button(ui, theme, AppIcon::Close, "Clear filter").clicked()
                {
                    app.process_filter.clear();
                }
            });
        });
}

fn render_table_header(ui: &mut egui::Ui, theme: &Theme) {
    let available_w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(available_w, 24.0), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }

    let layout = RowColumns::compute(rect);
    let font = FontId::new(FONT_LABEL, FontFamily::Proportional);

    ui.painter().text(
        Pos2::new(layout.rank_left, rect.center().y),
        egui::Align2::LEFT_CENTER,
        "#",
        font.clone(),
        theme.text_muted,
    );
    ui.painter().text(
        Pos2::new(layout.name_left, rect.center().y),
        egui::Align2::LEFT_CENTER,
        "PROCESS",
        font.clone(),
        theme.text_muted,
    );
    if layout.bar_width > 24.0 {
        ui.painter().text(
            Pos2::new(layout.bar_left, rect.center().y),
            egui::Align2::LEFT_CENTER,
            "Share of writes (estimated)",
            font.clone(),
            theme.text_muted,
        );
    }
    ui.painter().text(
        Pos2::new(layout.pct_right, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        "SHARE",
        font.clone(),
        theme.text_muted,
    );
    ui.painter().text(
        Pos2::new(layout.size_right, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        "WRITES",
        font,
        theme.text_muted,
    );
}

fn render_process_row(
    ui: &mut egui::Ui,
    theme: &Theme,
    row: &ProcessRow,
    total_bytes: u64,
) {
    let available_w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(available_w, 34.0), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }

    if resp.hovered() {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(RADIUS_XS as u8),
            theme.bg_elevated,
        );
    }

    let layout = RowColumns::compute(rect);
    let mono_font = FontId::new(FONT_MONO, FontFamily::Monospace);
    let body_font = FontId::new(FONT_BODY, FontFamily::Proportional);

    // Rank
    ui.painter().text(
        Pos2::new(layout.rank_left, rect.center().y),
        egui::Align2::LEFT_CENTER,
        format!("#{}", row.rank),
        mono_font.clone(),
        theme.text_muted,
    );

    // Executable name (clipped cleanly to its column)
    let name_clip = Rect::from_min_max(
        Pos2::new(layout.name_left, rect.top()),
        Pos2::new((layout.name_left + layout.name_width).max(layout.name_left), rect.bottom()),
    );
    ui.painter().with_clip_rect(name_clip).text(
        Pos2::new(layout.name_left, rect.center().y),
        egui::Align2::LEFT_CENTER,
        &row.name,
        body_font,
        theme.text_primary,
    );

    // Horizontal proportional bar
    if layout.bar_width > 24.0 {
        let bar_rect = Rect::from_min_size(
            Pos2::new(layout.bar_left, rect.center().y - 4.0),
            Vec2::new(layout.bar_width, 8.0),
        );
        ui.painter().rect_filled(
            bar_rect,
            CornerRadius::same(RADIUS_XS as u8),
            theme.bg_elevated,
        );
        ui.painter().rect_stroke(
            bar_rect,
            CornerRadius::same(RADIUS_XS as u8),
            Stroke::new(1.0, theme.border_subtle),
            StrokeKind::Inside,
        );

        let frac = if total_bytes > 0 {
            (row.bytes as f32 / total_bytes as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let fill_w = (bar_rect.width() * frac).max(if row.bytes > 0 { 2.0 } else { 0.0 });
        if fill_w > 0.5 {
            let fill_rect =
                Rect::from_min_size(bar_rect.min, Vec2::new(fill_w, bar_rect.height()));
            ui.painter().rect_filled(
                fill_rect,
                CornerRadius::same(RADIUS_XS as u8),
                theme.accent,
            );
        }
    }

    // Right-aligned Share %
    ui.painter().text(
        Pos2::new(layout.pct_right, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        format!("{:.1}%", row.share_pct),
        mono_font.clone(),
        theme.text_secondary,
    );

    // Right-aligned Estimated size
    ui.painter().text(
        Pos2::new(layout.size_right, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        format_bytes(row.bytes),
        mono_font,
        theme.text_primary,
    );
}

struct RowColumns {
    rank_left: f32,
    name_left: f32,
    name_width: f32,
    bar_left: f32,
    bar_width: f32,
    pct_right: f32,
    size_right: f32,
}

impl RowColumns {
    fn compute(rect: Rect) -> Self {
        let pad = 6.0;
        let rank_w = 38.0;
        let pct_w = 64.0;
        let size_w = 88.0;

        let rank_left = rect.left() + pad;
        let name_left = rank_left + rank_w;
        let size_right = rect.right() - pad;
        let pct_right = size_right - size_w - 12.0;

        let middle_right = pct_right - pct_w - 12.0;
        let middle_w = (middle_right - name_left).max(80.0);

        let (name_width, bar_left, bar_width) = if middle_w >= 260.0 {
            let nw = (middle_w * 0.45).min(240.0);
            let bl = name_left + nw + 12.0;
            let bw = (middle_right - bl).max(0.0);
            (nw, bl, bw)
        } else {
            (middle_w, middle_right, 0.0)
        };

        Self {
            rank_left,
            name_left,
            name_width,
            bar_left,
            bar_width,
            pct_right,
            size_right,
        }
    }
}

fn paint_row_divider(ui: &mut egui::Ui, theme: &Theme) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().line_segment(
            [Pos2::new(rect.left(), rect.center().y), Pos2::new(rect.right(), rect.center().y)],
            Stroke::new(1.0, theme.border_subtle),
        );
    }
}

fn render_empty_state(ui: &mut egui::Ui, theme: &Theme, title: &str, subtitle: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(SPACE_LG);
        let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::hover());
        theme::paint_icon(ui.painter(), icon_rect, AppIcon::Processes, theme.text_muted);
        ui.add_space(SPACE_SM);
        ui.label(
            egui::RichText::new(title)
                .size(FONT_BODY)
                .strong()
                .color(theme.text_primary),
        );
        ui.label(
            egui::RichText::new(subtitle)
                .size(FONT_LABEL)
                .color(theme.text_secondary),
        );
        ui.add_space(SPACE_LG);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filter_process_rows_preserves_rank_and_total_share() {
        let entries = vec![
            ("chrome.exe".to_string(), 750),
            ("Code.exe".to_string(), 250),
        ];

        let (all, total) = filter_process_rows(&entries, "");
        assert_eq!(total, 1000);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].rank, 1);
        assert!((all[0].share_pct - 75.0).abs() < 1e-6);
        assert_eq!(all[1].rank, 2);
        assert!((all[1].share_pct - 25.0).abs() < 1e-6);

        let (filtered, filtered_total) = filter_process_rows(&entries, "CODE");
        assert_eq!(filtered_total, 1000);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].rank, 2);
        assert_eq!(filtered[0].name, "Code.exe");
        assert!((filtered[0].share_pct - 25.0).abs() < 1e-6);
    }

    #[test]
    fn test_filter_process_rows_empty_and_zero_bytes() {
        let (empty_rows, empty_total) = filter_process_rows(&[], "abc");
        assert!(empty_rows.is_empty());
        assert_eq!(empty_total, 0);

        let zero_entries = vec![("idle.exe".to_string(), 0)];
        let (zero_rows, zero_total) = filter_process_rows(&zero_entries, "");
        assert_eq!(zero_total, 0);
        assert_eq!(zero_rows.len(), 1);
        assert!((zero_rows[0].share_pct - 0.0).abs() < 1e-6);
    }
}
