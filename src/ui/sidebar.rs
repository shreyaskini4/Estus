use crate::theme::{self, Theme};
use crate::ui::{SsdWatchApp, UiSnapshot};

/// Renders the collapsible left sidebar (stubbed for Agent 3).
pub fn render(ui: &mut egui::Ui, theme: &Theme, _app: &mut SsdWatchApp, _snap: &UiSnapshot) {
    theme::section_header(ui, theme, "Estus", None);
}
