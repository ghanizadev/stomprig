use super::ui::{Label, Theme, progress};
use egui::{Align, Color32, Layout, RichText};

pub(crate) fn show(
    ctx: &egui::Context,
    theme: &Theme,
    progress_value: f32,
    status: &str,
    error: Option<&str>,
) {
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(Color32::BLACK))
        .show(ctx, |ui| {
            let vertical_padding = ((ui.available_height() - 120.0) / 2.0).max(0.0);
            ui.add_space(vertical_padding);
            ui.vertical_centered(|ui| {
                ui.set_width(ui.available_width().min(360.0));
                ui.with_layout(Layout::top_down(Align::Center), |ui| {
                    Label::new(
                        RichText::new("DSP Rack Interface")
                            .color(Color32::WHITE)
                            .size(26.0)
                            .strong(),
                    )
                    .show(ui);
                    ui.add_space(18.0);
                    progress(ui, theme, progress_value);
                    ui.add_space(8.0);
                    Label::new(
                        RichText::new(error.unwrap_or(status)).color(if error.is_some() {
                            Color32::from_rgb(255, 120, 120)
                        } else {
                            Color32::from_gray(170)
                        }),
                    )
                    .show(ui);
                });
            });
        });

    if error.is_none() {
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}
