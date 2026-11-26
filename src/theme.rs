use std::path::PathBuf;

use egui::epaint::PathShape;
use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, TextStyle, Ui, Vec2};
use serde::{Deserialize, Serialize};

/// Spacing scale tokens (4px base grid).
pub const SPACE_XS: f32 = 4.0;
pub const SPACE_SM: f32 = 8.0;
pub const SPACE_MD: f32 = 12.0;
pub const SPACE_LG: f32 = 16.0;
pub const SPACE_XL: f32 = 24.0;
pub const SPACE_2XL: f32 = 32.0;

/// Corner radius scale tokens.
pub const RADIUS_XS: f32 = 4.0;
pub const RADIUS_SM: f32 = 6.0;
pub const RADIUS_MD: f32 = 10.0;
pub const RADIUS_LG: f32 = 12.0;
pub const RADIUS_PILL: f32 = 999.0;

/// Typography size tokens.
pub const FONT_HERO: f32 = 36.0;
pub const FONT_METRIC: f32 = 22.0;
pub const FONT_HEADING: f32 = 16.0;
pub const FONT_BODY: f32 = 13.0;
pub const FONT_LABEL: f32 = 11.0;
pub const FONT_MONO: f32 = 12.0;

/// User-selectable color theme mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
    System,
}

impl std::fmt::Display for ThemeMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dark => write!(f, "Dark"),
            Self::Light => write!(f, "Light"),
            Self::System => write!(f, "System"),
        }
    }
}

/// Resolved semantic color palette for the active theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub is_dark: bool,
    pub bg_base: Color32,
    pub bg_sidebar: Color32,
    pub bg_surface: Color32,
    pub bg_elevated: Color32,
    pub border_subtle: Color32,
    pub border_strong: Color32,
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub text_muted: Color32,
    pub accent: Color32,
    pub accent_text: Color32,
    pub accent_muted: Color32,
    pub accent_on_fill: Color32,
    pub health_green: Color32,
    pub health_amber: Color32,
    pub health_red: Color32,
}

impl Theme {
    /// Returns the dark theme token palette.
    pub const fn dark() -> Self {
        Self {
            is_dark: true,
            bg_base: Color32::from_rgb(15, 17, 21),
            bg_sidebar: Color32::from_rgb(18, 21, 27),
            bg_surface: Color32::from_rgb(22, 25, 32),
            bg_elevated: Color32::from_rgb(28, 32, 41),
            border_subtle: Color32::from_rgb(37, 42, 54),
            border_strong: Color32::from_rgb(52, 59, 76),
            text_primary: Color32::from_rgb(241, 245, 249),
            text_secondary: Color32::from_rgb(148, 163, 184),
            text_muted: Color32::from_rgb(124, 139, 158),
            accent: Color32::from_rgb(34, 211, 238),
            accent_text: Color32::from_rgb(34, 211, 238),
            accent_muted: Color32::from_rgb(22, 78, 99),
            accent_on_fill: Color32::from_rgb(8, 18, 22),
            health_green: Color32::from_rgb(52, 211, 153),
            health_amber: Color32::from_rgb(251, 191, 36),
            health_red: Color32::from_rgb(248, 113, 113),
        }
    }

    /// Returns the light theme token palette.
    pub const fn light() -> Self {
        Self {
            is_dark: false,
            bg_base: Color32::from_rgb(248, 250, 252),
            bg_sidebar: Color32::from_rgb(241, 245, 249),
            bg_surface: Color32::from_rgb(255, 255, 255),
            bg_elevated: Color32::from_rgb(241, 245, 249),
            border_subtle: Color32::from_rgb(226, 232, 240),
            border_strong: Color32::from_rgb(203, 213, 225),
            text_primary: Color32::from_rgb(15, 23, 42),
            text_secondary: Color32::from_rgb(71, 85, 105),
            text_muted: Color32::from_rgb(91, 107, 130),
            accent: Color32::from_rgb(8, 145, 178),
            accent_text: Color32::from_rgb(14, 116, 144),
            accent_muted: Color32::from_rgb(236, 254, 255),
            accent_on_fill: Color32::from_rgb(255, 255, 255),
            health_green: Color32::from_rgb(4, 120, 87),
            health_amber: Color32::from_rgb(180, 83, 9),
            health_red: Color32::from_rgb(220, 38, 38),
        }
    }

    /// Returns the semantic health color for a given wear percentage (`<50` green, `50..80` amber, `>=80` red).
    pub fn health_color(&self, wear_pct: u8) -> Color32 {
        if wear_pct < 50 {
            self.health_green
        } else if wear_pct < 80 {
            self.health_amber
        } else {
            self.health_red
        }
    }
}

/// Resolves a `ThemeMode` into a concrete `Theme` and applies `egui::Style` / `egui::Visuals`.
pub fn apply_theme(ctx: &egui::Context, mode: ThemeMode) -> Theme {
    let use_dark = match mode {
        ThemeMode::Dark => true,
        ThemeMode::Light => false,
        ThemeMode::System => !matches!(ctx.system_theme(), Some(egui::Theme::Light)),
    };

    let theme = if use_dark {
        Theme::dark()
    } else {
        Theme::light()
    };

    let mut visuals = if use_dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    visuals.dark_mode = use_dark;
    visuals.override_text_color = Some(theme.text_primary);
    visuals.panel_fill = theme.bg_base;
    visuals.window_fill = theme.bg_surface;
    visuals.extreme_bg_color = theme.bg_elevated;
    visuals.faint_bg_color = theme.bg_elevated;
    visuals.code_bg_color = theme.bg_elevated;

    visuals.window_corner_radius = CornerRadius::same(RADIUS_LG as u8);
    visuals.window_stroke = Stroke::new(1.0, theme.border_subtle);
    visuals.menu_corner_radius = CornerRadius::same(RADIUS_MD as u8);

    visuals.selection.bg_fill = theme.accent_muted;
    visuals.selection.stroke = Stroke::new(1.0, theme.accent);

    visuals.widgets.noninteractive.bg_fill = theme.bg_surface;
    visuals.widgets.noninteractive.weak_bg_fill = theme.bg_surface;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, theme.border_subtle);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, theme.text_secondary);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(RADIUS_SM as u8);

    visuals.widgets.inactive.bg_fill = theme.bg_elevated;
    visuals.widgets.inactive.weak_bg_fill = theme.bg_elevated;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, theme.border_subtle);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, theme.text_primary);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(RADIUS_SM as u8);

    visuals.widgets.hovered.bg_fill = theme.bg_elevated;
    visuals.widgets.hovered.weak_bg_fill = theme.bg_elevated;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, theme.border_strong);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.2, theme.text_primary);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(RADIUS_SM as u8);

    visuals.widgets.active.bg_fill = theme.accent_muted;
    visuals.widgets.active.weak_bg_fill = theme.accent_muted;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, theme.accent);
    visuals.widgets.active.fg_stroke = Stroke::new(1.5, theme.text_primary);
    visuals.widgets.active.corner_radius = CornerRadius::same(RADIUS_SM as u8);

    visuals.widgets.open.bg_fill = theme.bg_elevated;
    visuals.widgets.open.weak_bg_fill = theme.bg_elevated;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, theme.border_strong);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0, theme.text_primary);
    visuals.widgets.open.corner_radius = CornerRadius::same(RADIUS_SM as u8);

    visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 4],
        blur: 12,
        spread: 0,
        color: Color32::from_black_alpha(if use_dark { 90 } else { 25 }),
    };

    let egui_theme = if use_dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    ctx.set_theme(egui_theme);

    let mut style = (*ctx.style_of(egui_theme)).clone();
    style.visuals = visuals;
    style.spacing.item_spacing = Vec2::new(SPACE_SM, SPACE_SM);
    style.spacing.button_padding = Vec2::new(SPACE_MD, SPACE_XS + 2.0);
    style.spacing.window_margin = egui::Margin::same(SPACE_LG as i8);
    style.text_styles.insert(
        TextStyle::Heading,
        FontId::new(FONT_HEADING, FontFamily::Proportional),
    );
    style
        .text_styles
        .insert(TextStyle::Body, FontId::new(FONT_BODY, FontFamily::Proportional));
    style.text_styles.insert(
        TextStyle::Button,
        FontId::new(FONT_BODY, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Small,
        FontId::new(FONT_LABEL, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Monospace,
        FontId::new(FONT_MONO, FontFamily::Monospace),
    );

    ctx.set_style_of(egui_theme, style);
    theme
}

/// Loads Windows system fonts (`SegUIVar.ttf` / `segoeui.ttf` and `consola.ttf`) if present,
/// falling back gracefully to `egui`'s built-in fonts when unavailable.
pub fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();

    let Some(windir) = std::env::var_os("WINDIR") else {
        return;
    };
    let fonts_dir = PathBuf::from(windir).join("Fonts");

    let prop_candidates = ["SegUIVar.ttf", "segoeui.ttf"];
    for candidate in prop_candidates {
        let path = fonts_dir.join(candidate);
        if let Ok(bytes) = std::fs::read(&path) {
            let key = "estus_ui".to_string();
            fonts
                .font_data
                .insert(key.clone(), FontData::from_owned(bytes).into());
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .insert(0, key);
            break;
        }
    }

    let mono_path = fonts_dir.join("consola.ttf");
    if let Ok(bytes) = std::fs::read(&mono_path) {
        let key = "estus_mono".to_string();
        fonts
            .font_data
            .insert(key.clone(), FontData::from_owned(bytes).into());
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .insert(0, key);
    }

    ctx.set_fonts(fonts);
}

/// Geometric vector icons rendered via `egui::Painter` (zero Unicode emojis).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppIcon {
    DriveAll,
    DriveNvme,
    DriveSata,
    Dashboard,
    Processes,
    Settings,
    Diagnostics,
    Refresh,
    Rescan,
    Folder,
    Search,
    Pencil,
    Check,
    Close,
    Warning,
    ChevronLeft,
    ChevronRight,
    Export,
    Sun,
    Moon,
    Monitor,
    Flame,
}

/// Paints a crisp geometric vector icon inside `rect`.
pub fn paint_icon(painter: &Painter, rect: Rect, icon: AppIcon, color: Color32) {
    let size = rect.size().min_elem();
    if size <= 1.0 {
        return;
    }
    let c = rect.center();
    let r = size * 0.4;
    let sw = (size * 0.1).clamp(1.2, 2.2);
    let stroke = Stroke::new(sw, color);

    match icon {
        AppIcon::DriveAll => {
            let top = Rect::from_center_size(Pos2::new(c.x, c.y - r * 0.45), Vec2::new(r * 1.7, r * 0.65));
            let bot = Rect::from_center_size(Pos2::new(c.x, c.y + r * 0.45), Vec2::new(r * 1.7, r * 0.65));
            painter.rect_stroke(top, CornerRadius::same(2), stroke, StrokeKind::Inside);
            painter.rect_stroke(bot, CornerRadius::same(2), stroke, StrokeKind::Inside);
            painter.circle_filled(Pos2::new(top.right() - r * 0.35, top.center().y), sw * 0.8, color);
            painter.circle_filled(Pos2::new(bot.right() - r * 0.35, bot.center().y), sw * 0.8, color);
        }
        AppIcon::DriveNvme => {
            let body = Rect::from_center_size(c, Vec2::new(r * 1.8, r * 0.95));
            painter.rect_stroke(body, CornerRadius::same(2), stroke, StrokeKind::Inside);
            let chip = Rect::from_center_size(Pos2::new(c.x - r * 0.25, c.y), Vec2::new(r * 0.6, r * 0.45));
            painter.rect_filled(chip, CornerRadius::same(1), color);
            painter.line_segment(
                [Pos2::new(body.right() - r * 0.35, body.top()), Pos2::new(body.right() - r * 0.35, body.bottom())],
                stroke,
            );
        }
        AppIcon::DriveSata => {
            let body = Rect::from_center_size(c, Vec2::new(r * 1.4, r * 1.75));
            painter.rect_stroke(body, CornerRadius::same(2), stroke, StrokeKind::Inside);
            painter.circle_stroke(Pos2::new(c.x, c.y - r * 0.2), r * 0.38, stroke);
            painter.line_segment(
                [Pos2::new(c.x - r * 0.35, c.y + r * 0.55), Pos2::new(c.x + r * 0.35, c.y + r * 0.55)],
                stroke,
            );
        }
        AppIcon::Dashboard => {
            let gap = r * 0.18;
            let cell = r * 0.72;
            let offsets = [
                Vec2::new(-gap - cell * 0.5, -gap - cell * 0.5),
                Vec2::new(gap + cell * 0.5, -gap - cell * 0.5),
                Vec2::new(-gap - cell * 0.5, gap + cell * 0.5),
                Vec2::new(gap + cell * 0.5, gap + cell * 0.5),
            ];
            for off in offsets {
                let tile = Rect::from_center_size(c + off, Vec2::splat(cell));
                painter.rect_stroke(tile, CornerRadius::same(2), stroke, StrokeKind::Inside);
            }
        }
        AppIcon::Processes => {
            let pts = [
                Pos2::new(c.x - r, c.y + r * 0.1),
                Pos2::new(c.x - r * 0.45, c.y + r * 0.1),
                Pos2::new(c.x - r * 0.15, c.y - r * 0.75),
                Pos2::new(c.x + r * 0.25, c.y + r * 0.75),
                Pos2::new(c.x + r * 0.55, c.y - r * 0.1),
                Pos2::new(c.x + r, c.y - r * 0.1),
            ];
            painter.add(Shape::Path(PathShape::line(pts.to_vec(), stroke)));
        }
        AppIcon::Settings => {
            painter.circle_stroke(c, r * 0.52, stroke);
            for i in 0..6 {
                let angle = (i as f32) * std::f32::consts::FRAC_PI_3;
                let dir = Vec2::new(angle.cos(), angle.sin());
                painter.line_segment([c + dir * (r * 0.65), c + dir * (r * 0.98)], stroke);
            }
        }
        AppIcon::Diagnostics => {
            let box_rect = Rect::from_center_size(c, Vec2::new(r * 1.75, r * 1.55));
            painter.rect_stroke(box_rect, CornerRadius::same(2), stroke, StrokeKind::Inside);
            painter.line_segment(
                [Pos2::new(c.x - r * 0.45, c.y - r * 0.2), Pos2::new(c.x - r * 0.15, c.y + r * 0.05)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(c.x - r * 0.45, c.y + r * 0.3), Pos2::new(c.x - r * 0.15, c.y + r * 0.05)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(c.x + r * 0.05, c.y + r * 0.3), Pos2::new(c.x + r * 0.5, c.y + r * 0.3)],
                stroke,
            );
        }
        AppIcon::Refresh | AppIcon::Rescan => {
            let pts: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let t = (i as f32) / 16.0;
                    let angle = -0.3 + t * std::f32::consts::PI * 1.55;
                    c + Vec2::new(angle.cos(), angle.sin()) * (r * 0.8)
                })
                .collect();
            if let Some(&end) = pts.last() {
                painter.add(Shape::Path(PathShape::line(pts, stroke)));
                painter.line_segment([end, end + Vec2::new(-r * 0.35, -r * 0.15)], stroke);
                painter.line_segment([end, end + Vec2::new(r * 0.1, -r * 0.38)], stroke);
            }
        }
        AppIcon::Folder => {
            let pts = [
                Pos2::new(c.x - r * 0.9, c.y - r * 0.65),
                Pos2::new(c.x - r * 0.25, c.y - r * 0.65),
                Pos2::new(c.x - r * 0.05, c.y - r * 0.35),
                Pos2::new(c.x + r * 0.9, c.y - r * 0.35),
                Pos2::new(c.x + r * 0.9, c.y + r * 0.65),
                Pos2::new(c.x - r * 0.9, c.y + r * 0.65),
            ];
            painter.add(Shape::Path(PathShape::closed_line(pts.to_vec(), stroke)));
        }
        AppIcon::Search => {
            let center = Pos2::new(c.x - r * 0.18, c.y - r * 0.18);
            painter.circle_stroke(center, r * 0.58, stroke);
            painter.line_segment(
                [center + Vec2::splat(r * 0.42), Pos2::new(c.x + r * 0.85, c.y + r * 0.85)],
                stroke,
            );
        }
        AppIcon::Pencil => {
            painter.line_segment(
                [Pos2::new(c.x - r * 0.75, c.y + r * 0.75), Pos2::new(c.x + r * 0.55, c.y - r * 0.55)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(c.x + r * 0.3, c.y - r * 0.8), Pos2::new(c.x + r * 0.8, c.y - r * 0.3)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(c.x - r * 0.75, c.y + r * 0.75), Pos2::new(c.x - r * 0.9, c.y + r * 0.9)],
                stroke,
            );
        }
        AppIcon::Check => {
            let pts = [
                Pos2::new(c.x - r * 0.75, c.y + r * 0.05),
                Pos2::new(c.x - r * 0.15, c.y + r * 0.65),
                Pos2::new(c.x + r * 0.8, c.y - r * 0.55),
            ];
            painter.add(Shape::Path(PathShape::line(pts.to_vec(), stroke)));
        }
        AppIcon::Close => {
            painter.line_segment(
                [Pos2::new(c.x - r * 0.7, c.y - r * 0.7), Pos2::new(c.x + r * 0.7, c.y + r * 0.7)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(c.x + r * 0.7, c.y - r * 0.7), Pos2::new(c.x - r * 0.7, c.y + r * 0.7)],
                stroke,
            );
        }
        AppIcon::Warning => {
            let pts = [
                Pos2::new(c.x, c.y - r * 0.85),
                Pos2::new(c.x + r * 0.9, c.y + r * 0.75),
                Pos2::new(c.x - r * 0.9, c.y + r * 0.75),
            ];
            painter.add(Shape::Path(PathShape::closed_line(pts.to_vec(), stroke)));
            painter.line_segment(
                [Pos2::new(c.x, c.y - r * 0.28), Pos2::new(c.x, c.y + r * 0.25)],
                stroke,
            );
            painter.circle_filled(Pos2::new(c.x, c.y + r * 0.5), sw * 0.75, color);
        }
        AppIcon::ChevronLeft => {
            let pts = [
                Pos2::new(c.x + r * 0.35, c.y - r * 0.7),
                Pos2::new(c.x - r * 0.35, c.y),
                Pos2::new(c.x + r * 0.35, c.y + r * 0.7),
            ];
            painter.add(Shape::Path(PathShape::line(pts.to_vec(), stroke)));
        }
        AppIcon::ChevronRight => {
            let pts = [
                Pos2::new(c.x - r * 0.35, c.y - r * 0.7),
                Pos2::new(c.x + r * 0.35, c.y),
                Pos2::new(c.x - r * 0.35, c.y + r * 0.7),
            ];
            painter.add(Shape::Path(PathShape::line(pts.to_vec(), stroke)));
        }
        AppIcon::Export => {
            painter.line_segment(
                [Pos2::new(c.x, c.y + r * 0.3), Pos2::new(c.x, c.y - r * 0.8)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(c.x - r * 0.4, c.y - r * 0.4), Pos2::new(c.x, c.y - r * 0.8)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(c.x + r * 0.4, c.y - r * 0.4), Pos2::new(c.x, c.y - r * 0.8)],
                stroke,
            );
            let tray = [
                Pos2::new(c.x - r * 0.85, c.y + r * 0.3),
                Pos2::new(c.x - r * 0.85, c.y + r * 0.8),
                Pos2::new(c.x + r * 0.85, c.y + r * 0.8),
                Pos2::new(c.x + r * 0.85, c.y + r * 0.3),
            ];
            painter.add(Shape::Path(PathShape::line(tray.to_vec(), stroke)));
        }
        AppIcon::Sun => {
            painter.circle_stroke(c, r * 0.42, stroke);
            for i in 0..8 {
                let angle = (i as f32) * std::f32::consts::FRAC_PI_4;
                let dir = Vec2::new(angle.cos(), angle.sin());
                painter.line_segment([c + dir * (r * 0.65), c + dir * (r * 0.95)], stroke);
            }
        }
        AppIcon::Moon => {
            let pts: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let t = (i as f32) / 16.0;
                    let angle = 0.4 + t * std::f32::consts::PI * 1.35;
                    c + Vec2::new(angle.cos(), angle.sin()) * (r * 0.78)
                })
                .collect();
            painter.add(Shape::Path(PathShape::line(pts, stroke)));
        }
        AppIcon::Monitor => {
            let screen = Rect::from_center_size(Pos2::new(c.x, c.y - r * 0.15), Vec2::new(r * 1.8, r * 1.2));
            painter.rect_stroke(screen, CornerRadius::same(2), stroke, StrokeKind::Inside);
            painter.line_segment(
                [Pos2::new(c.x, screen.bottom()), Pos2::new(c.x, c.y + r * 0.78)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(c.x - r * 0.45, c.y + r * 0.78), Pos2::new(c.x + r * 0.45, c.y + r * 0.78)],
                stroke,
            );
        }
        AppIcon::Flame => {
            let pts = [
                Pos2::new(c.x, c.y - r * 0.95),
                Pos2::new(c.x + r * 0.65, c.y + r * 0.05),
                Pos2::new(c.x + r * 0.45, c.y + r * 0.75),
                Pos2::new(c.x, c.y + r * 0.92),
                Pos2::new(c.x - r * 0.45, c.y + r * 0.75),
                Pos2::new(c.x - r * 0.65, c.y + r * 0.05),
            ];
            painter.add(Shape::Path(PathShape::closed_line(pts.to_vec(), stroke)));
            let inner = [
                Pos2::new(c.x, c.y - r * 0.15),
                Pos2::new(c.x + r * 0.25, c.y + r * 0.35),
                Pos2::new(c.x, c.y + r * 0.65),
                Pos2::new(c.x - r * 0.25, c.y + r * 0.35),
            ];
            painter.add(Shape::Path(PathShape::closed_line(inner.to_vec(), stroke)));
        }
    }
}

/// Renders a standard surface card container with 1px hairline border and `RADIUS_LG` corners.
pub fn card(ui: &mut Ui, theme: &Theme, add_contents: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(theme.bg_surface)
        .stroke(Stroke::new(1.0, theme.border_subtle))
        .corner_radius(CornerRadius::same(RADIUS_LG as u8))
        .inner_margin(egui::Margin::same(SPACE_LG as i8))
        .show(ui, add_contents);
}

/// Renders a compact elevated stat tile with label, metric value, subtext, and optional progress bar.
pub fn stat_tile(
    ui: &mut Ui,
    theme: &Theme,
    label: &str,
    value: &str,
    subtext: &str,
    accent_color: Color32,
    progress: Option<f32>,
) {
    egui::Frame::new()
        .fill(theme.bg_elevated)
        .stroke(Stroke::new(1.0, theme.border_subtle))
        .corner_radius(CornerRadius::same(RADIUS_MD as u8))
        .inner_margin(egui::Margin::same(SPACE_MD as i8))
        .show(ui, |ui| {
            ui.set_min_width(140.0);
            ui.label(
                egui::RichText::new(label.to_uppercase())
                    .size(FONT_LABEL)
                    .strong()
                    .color(theme.text_muted),
            );
            ui.add_space(SPACE_XS);
            ui.label(
                egui::RichText::new(value)
                    .size(FONT_METRIC)
                    .strong()
                    .color(accent_color),
            );
            ui.add_space(SPACE_XS);
            ui.label(
                egui::RichText::new(subtext)
                    .size(FONT_LABEL)
                    .color(theme.text_secondary),
            );
            if let Some(frac) = progress {
                ui.add_space(SPACE_SM);
                let (bar_rect, _) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 4.0), Sense::hover());
                ui.painter().rect_filled(
                    bar_rect,
                    CornerRadius::same(RADIUS_XS as u8),
                    theme.border_subtle,
                );
                let fill_w = (bar_rect.width() * frac.clamp(0.0, 1.0)).max(0.0);
                if fill_w > 0.5 {
                    let fill_rect = Rect::from_min_size(bar_rect.min, Vec2::new(fill_w, bar_rect.height()));
                    ui.painter().rect_filled(
                        fill_rect,
                        CornerRadius::same(RADIUS_XS as u8),
                        accent_color,
                    );
                }
            }
        });
}

/// Renders a styled pill button with optional leading `AppIcon`.
pub fn pill_button(
    ui: &mut Ui,
    theme: &Theme,
    label: &str,
    icon: Option<AppIcon>,
    primary: bool,
) -> Response {
    let icon_space = if icon.is_some() { 18.0 } else { 0.0 };
    let font_id = FontId::new(FONT_BODY, FontFamily::Proportional);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font_id, Color32::WHITE);
    let desired_size = Vec2::new(
        galley.size().x + icon_space + SPACE_LG * 1.5,
        30.0,
    );

    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click());
    if ui.is_rect_visible(rect) {
        let (fill, fg, border) = if primary {
            let base = if theme.is_dark {
                theme.accent
            } else {
                theme.accent_text
            };
            let bg = if response.hovered() {
                base.gamma_multiply(0.9)
            } else {
                base
            };
            (bg, theme.accent_on_fill, bg)
        } else {
            let bg = if response.hovered() {
                theme.bg_elevated
            } else {
                theme.bg_surface
            };
            let stroke_col = if response.hovered() {
                theme.border_strong
            } else {
                theme.border_subtle
            };
            (bg, theme.text_primary, stroke_col)
        };

        ui.painter()
            .rect_filled(rect, CornerRadius::same(RADIUS_SM as u8), fill);
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(RADIUS_SM as u8),
            Stroke::new(1.0, border),
            StrokeKind::Inside,
        );

        let content_width = galley.size().x + icon_space;
        let mut cursor_x = rect.center().x - content_width * 0.5;
        if let Some(ic) = icon {
            let icon_rect = Rect::from_center_size(
                Pos2::new(cursor_x + 7.0, rect.center().y),
                Vec2::splat(14.0),
            );
            paint_icon(ui.painter(), icon_rect, ic, fg);
            cursor_x += icon_space;
        }
        let text_pos = Pos2::new(cursor_x, rect.center().y - galley.size().y * 0.5);
        ui.painter().galley(text_pos, galley, fg);
    }
    response
}

/// Renders a horizontal segmented control. Returns `true` if the active selection changed.
pub fn segmented_control<T: Copy + PartialEq>(
    ui: &mut Ui,
    theme: &Theme,
    id: &str,
    current: &mut T,
    options: &[(T, &str)],
) -> bool {
    let mut changed = false;
    ui.push_id(id, |ui| {
        egui::Frame::new()
            .fill(theme.bg_elevated)
            .stroke(Stroke::new(1.0, theme.border_subtle))
            .corner_radius(CornerRadius::same(RADIUS_SM as u8))
            .inner_margin(egui::Margin::same(2))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for &(val, text) in options {
                        let selected = *current == val;
                        let font_id = FontId::new(FONT_LABEL, FontFamily::Proportional);
                        let galley = ui
                            .painter()
                            .layout_no_wrap(text.to_string(), font_id, Color32::WHITE);
                        let desired = Vec2::new(galley.size().x + SPACE_MD * 1.5, 24.0);
                        let (rect, resp) = ui.allocate_exact_size(desired, Sense::click());
                        if ui.is_rect_visible(rect) {
                            let bg = if selected {
                                theme.accent_muted
                            } else if resp.hovered() {
                                theme.bg_surface
                            } else {
                                Color32::TRANSPARENT
                            };
                            let fg = if selected {
                                theme.accent_text
                            } else {
                                theme.text_secondary
                            };
                            ui.painter()
                                .rect_filled(rect, CornerRadius::same(RADIUS_XS as u8), bg);
                            if selected {
                                ui.painter().rect_stroke(
                                    rect,
                                    CornerRadius::same(RADIUS_XS as u8),
                                    Stroke::new(1.0, theme.accent),
                                    StrokeKind::Inside,
                                );
                            }
                            let pos = rect.center() - galley.size() * 0.5;
                            ui.painter().galley(pos, galley, fg);
                        }
                        if resp.clicked() && !selected {
                            *current = val;
                            changed = true;
                        }
                    }
                });
            });
    });
    changed
}

/// Renders a consistent section title with optional subtitle.
pub fn section_header(ui: &mut Ui, theme: &Theme, title: &str, subtitle: Option<&str>) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(
            egui::RichText::new(title)
                .size(FONT_HEADING)
                .strong()
                .color(theme.text_primary),
        );
        if let Some(sub) = subtitle {
            ui.label(
                egui::RichText::new(sub)
                    .size(FONT_BODY)
                    .color(theme.text_secondary),
            );
        }
    });
}

/// Renders a compact pill badge with tinted background and colored text.
pub fn badge(ui: &mut Ui, _theme: &Theme, text: &str, color: Color32) {
    let font_id = FontId::new(FONT_LABEL, FontFamily::Proportional);
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font_id, color);
    let desired = Vec2::new(galley.size().x + SPACE_MD, 20.0);
    let (rect, _) = ui.allocate_exact_size(desired, Sense::hover());
    if ui.is_rect_visible(rect) {
        let bg = color.gamma_multiply(0.16);
        ui.painter().rect_filled(rect, CornerRadius::same(255), bg);
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(255),
            Stroke::new(1.0, color.gamma_multiply(0.4)),
            StrokeKind::Inside,
        );
        let pos = rect.center() - galley.size() * 0.5;
        ui.painter().galley(pos, galley, color);
    }
}

/// Renders a square icon-only button with hover highlight and tooltip.
pub fn icon_button(ui: &mut Ui, theme: &Theme, icon: AppIcon, tooltip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
    if ui.is_rect_visible(rect) {
        if resp.hovered() {
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(RADIUS_SM as u8),
                theme.bg_elevated,
            );
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(RADIUS_SM as u8),
                Stroke::new(1.0, theme.border_subtle),
                StrokeKind::Inside,
            );
        }
        let icon_color = if resp.hovered() {
            theme.text_primary
        } else {
            theme.text_secondary
        };
        paint_icon(ui.painter(), rect.shrink(6.0), icon, icon_color);
    }
    resp.on_hover_text(tooltip)
}

/// Renders a pill toggle switch. Returns the `egui::Response` (toggled on click).
pub fn toggle_switch(ui: &mut Ui, theme: &Theme, on: &mut bool) -> Response {
    let desired_size = Vec2::new(38.0, 20.0);
    let (rect, mut response) = ui.allocate_exact_size(desired_size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let track_color = if *on {
            if theme.is_dark {
                theme.accent
            } else {
                theme.accent_text
            }
        } else {
            theme.bg_elevated
        };
        let border_color = if *on {
            track_color
        } else if response.hovered() {
            theme.border_strong
        } else {
            theme.border_subtle
        };

        ui.painter()
            .rect_filled(rect, CornerRadius::same(255), track_color);
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(255),
            Stroke::new(1.0, border_color),
            StrokeKind::Inside,
        );

        let radius = rect.height() * 0.38;
        let knob_x = if *on {
            rect.right() - rect.height() * 0.5
        } else {
            rect.left() + rect.height() * 0.5
        };
        let knob_color = if *on {
            theme.accent_on_fill
        } else {
            theme.text_secondary
        };
        ui.painter()
            .circle_filled(Pos2::new(knob_x, rect.center().y), radius, knob_color);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_mode_default_and_display() {
        assert_eq!(ThemeMode::default(), ThemeMode::Dark);
        assert_eq!(ThemeMode::Dark.to_string(), "Dark");
        assert_eq!(ThemeMode::Light.to_string(), "Light");
        assert_eq!(ThemeMode::System.to_string(), "System");
    }

    #[test]
    fn test_theme_health_color_thresholds() {
        let dark = Theme::dark();
        assert_eq!(dark.health_color(0), dark.health_green);
        assert_eq!(dark.health_color(49), dark.health_green);
        assert_eq!(dark.health_color(50), dark.health_amber);
        assert_eq!(dark.health_color(79), dark.health_amber);
        assert_eq!(dark.health_color(80), dark.health_red);
        assert_eq!(dark.health_color(100), dark.health_red);

        let light = Theme::light();
        assert_eq!(light.health_color(10), light.health_green);
        assert_eq!(light.health_color(60), light.health_amber);
        assert_eq!(light.health_color(95), light.health_red);
    }
}
