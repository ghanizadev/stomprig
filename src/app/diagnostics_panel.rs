use super::diagnostics::{DiagnosticHistory, DiagnosticSeverity, format_timestamp_utc};

#[cfg(not(debug_assertions))]
pub(crate) fn show(
    ctx: &egui::Context,
    open: &mut bool,
    history: &mut DiagnosticHistory,
    log_directory: &std::path::Path,
) {
    if !*open {
        return;
    }

    egui::Window::new("Diagnostics")
        .open(open)
        .default_width(620.0)
        .default_height(420.0)
        .show(ctx, |ui| show_contents(ui, history, log_directory, true));
}

pub(crate) fn show_contents(
    ui: &mut egui::Ui,
    history: &mut DiagnosticHistory,
    log_directory: &std::path::Path,
    scroll_incidents: bool,
) {
    let clear_requested = ui
        .horizontal(|ui| {
            ui.label(format!("Log directory: {}", log_directory.display()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled(
                    history.len() > 0 || history.dropped_events() > 0,
                    egui::Button::new("Clear diagnostics"),
                )
                .clicked()
            })
            .inner
        })
        .inner;
    if clear_requested {
        history.clear();
    }

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

    let incident_count = history.len();
    let mut incident_to_remove = None;
    if scroll_incidents {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                show_incidents(ui, history, incident_count, &mut incident_to_remove);
            });
    } else {
        show_incidents(ui, history, incident_count, &mut incident_to_remove);
    }
    if let Some(index) = incident_to_remove {
        history.remove_incident(index);
    }
}

fn show_incidents(
    ui: &mut egui::Ui,
    history: &DiagnosticHistory,
    incident_count: usize,
    incident_to_remove: &mut Option<usize>,
) {
    for index in (0..incident_count).rev() {
        let Some(incident) = history.incidents().get(index).copied() else {
            continue;
        };
        ui.push_id(index, |ui| {
            let available_width = ui.available_width();
            let group_frame = egui::Frame::group(ui.style());
            let frame_margin = group_frame.total_margin();
            let horizontal_margin = frame_margin.left + frame_margin.right;
            group_frame.show(ui, |ui| {
                ui.set_min_width((available_width - horizontal_margin).max(0.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    if ui
                        .small_button("×")
                        .on_hover_text("Remove this report")
                        .clicked()
                    {
                        *incident_to_remove = Some(index);
                    }
                    ui.vertical(|ui| {
                        ui.set_min_width(ui.available_width());
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
                });
            });
        });
        ui.add_space(ui.spacing().item_spacing.y);
    }
}
