use crate::theme::{self, Theme};
use crate::ui::{SsdWatchApp, UiSnapshot};

/// Renders the Processes view (stubbed for Agent 3).
pub fn render(ui: &mut egui::Ui, theme: &Theme, _app: &mut SsdWatchApp, _snap: &UiSnapshot) {
    theme::section_header(ui, theme, "Top Writing Processes", Some("System-wide"));
}
