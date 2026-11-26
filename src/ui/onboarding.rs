use crate::theme::Theme;
use crate::ui::{SsdWatchApp, UiSnapshot};

/// Renders onboarding / error banners (stubbed for Agent 4).
pub fn render_banner(
    _ui: &mut egui::Ui,
    _theme: &Theme,
    _app: &mut SsdWatchApp,
    _snap: &UiSnapshot,
) {
}

/// Renders skeleton loading state (stubbed for Agent 4).
pub fn render_skeleton(_ui: &mut egui::Ui, _theme: &Theme) {}

/// Renders empty state card (stubbed for Agent 4).
pub fn render_empty_state(_ui: &mut egui::Ui, _theme: &Theme, _title: &str, _subtitle: &str) {}
