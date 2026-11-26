use egui_plot as _;

use crate::theme::{self, Theme};
use crate::ui::{SsdWatchApp, UiSnapshot};

/// Renders the Dashboard view (stubbed for Agent 2).
pub fn render(ui: &mut egui::Ui, theme: &Theme, _app: &mut SsdWatchApp, _snap: &UiSnapshot) {
    theme::section_header(ui, theme, "Dashboard", Some("SSD health & write overview"));
}
