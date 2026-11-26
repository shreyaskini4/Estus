use crate::theme::{self, Theme};
use crate::ui::{SsdWatchApp, UiSnapshot};

/// Renders the Diagnostics view (stubbed for Agent 4).
pub fn render(ui: &mut egui::Ui, theme: &Theme, _app: &mut SsdWatchApp, _snap: &UiSnapshot) {
    theme::section_header(ui, theme, "Diagnostics", Some("smartctl & 24h interval telemetry"));
}
