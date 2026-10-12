use super::diagnostics::{DiagnosticHistory, DiagnosticSeverity, format_timestamp_utc};

pub(crate) fn show(
    ctx: &egui::Context,
    open: &mut bool,
    history: &DiagnosticHistory,
    log_directory: &std::path::Path,
) {
    if !*open {
        return;
    }

    egui::Window::new("Diagnostics")
        .open(open)
        .default_width(620.0)
        .default_height(420.0)
        .show(ctx, |ui| {
            ui.label(format!("Log directory: {}", log_directory.display()));
            if history.dropped_events() > 0 {
                ui.colored_label(
                    ui.visuals().error_fg_color,
                    format!(
                        "{} diagnostic event(s) could not be queued.",
                        history.dropped_events()
                    ),
                );
            }
            ui.separator();

            if history.incidents().is_empty() {
                ui.label("No incidents recorded.");
                return;
            }

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for incident in history.incidents().iter().rev() {
                        ui.group(|ui| {
                            ui.horizontal_wrapped(|ui| {
                                let color = match incident.event.severity {
                                    DiagnosticSeverity::Warning => ui.visuals().warn_fg_color,
                                    DiagnosticSeverity::Error => ui.visuals().error_fg_color,
                                };
                                ui.colored_label(color, incident.event.severity.label());
                                ui.label(format_timestamp_utc(incident.event.timestamp_ms));
                                if incident.occurrences > 1 {
                                    ui.weak(format!("{} occurrences", incident.occurrences));
                                }
                            });
                            ui.strong(format!(
                                "{} · {}",
                                incident.event.kind.label(),
                                incident.event.subject.as_str()
                            ));
                            ui.label(incident.event.message());
                        });
                    }
                });
        });
}
