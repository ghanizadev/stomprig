use super::super::diagnostics::DiagnosticHistory;
use super::super::ui::section;

pub(super) fn show(
    ui: &mut egui::Ui,
    history: &mut DiagnosticHistory,
    log_directory: &std::path::Path,
) {
    ui.group(|ui| {
        section(ui, "Runtime diagnostics");
        ui.add_space(4.0);
        super::super::diagnostics_panel::show_contents(ui, history, log_directory, false);
    });
}
