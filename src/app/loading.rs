use super::ui::{Theme, spinner};
use egui::{Color32, Direction, Frame, Id, Layout, RichText};

pub(crate) fn show(ctx: &egui::Context, theme: &Theme, subtitle: &str, error: Option<&str>) {
    // Keep the rack visible underneath while dimming it by 65%.
    let backdrop = Color32::from_black_alpha(166);
    let primary_text = Color32::from_rgb(245, 247, 250);
    let secondary_text = Color32::from_rgb(220, 224, 232);

    // The plugin picker sheet uses egui's Tooltip layer for its content. Put the loading
    // modal above that layer as well, so add and remove operations have identical compositing
    // and the loading state always blocks every other UI layer.
    let modal_id = Id::new("plugin-loading-screen");
    egui::Modal::new(modal_id)
        .area(egui::Modal::default_area(modal_id).order(egui::Order::Debug))
        .frame(Frame::NONE)
        .backdrop_color(backdrop)
        .show(ctx, |ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(260.0, 110.0),
                Layout::centered_and_justified(Direction::TopDown),
                |ui| {
                    ui.vertical_centered(|ui| {
                        spinner(ui, theme, primary_text);
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new("Loading plugin rack...")
                                .color(primary_text)
                                .size(20.0)
                                .strong(),
                        );
                        ui.label(
                            RichText::new(error.unwrap_or(subtitle))
                                .color(if error.is_some() {
                                    Color32::from_rgb(255, 120, 120)
                                } else {
                                    secondary_text
                                })
                                .size(14.0),
                        );
                    });
                },
            );
        });

    if error.is_none() {
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}
