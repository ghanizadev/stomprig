use super::ui::{Label, Theme, spinner};
use egui::{Color32, Direction, Frame, Id, Layout, RichText};

pub(crate) fn show(
    ctx: &egui::Context,
    theme: &Theme,
    subtitle: &str,
    current: usize,
    total: usize,
    can_cancel: bool,
) -> bool {
    // Keep the rack visible underneath while dimming it by 65%.
    let backdrop = Color32::from_black_alpha(166);
    let primary_text = Color32::from_rgb(245, 247, 250);
    let secondary_text = Color32::from_rgb(220, 224, 232);
    let mut cancel_requested = false;

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
                egui::vec2(320.0, if can_cancel { 150.0 } else { 120.0 }),
                Layout::centered_and_justified(Direction::TopDown),
                |ui| {
                    ui.vertical_centered(|ui| {
                        spinner(ui, theme, primary_text);
                        ui.add_space(12.0);
                        Label::new(
                            RichText::new("Loading plugin rack...")
                                .color(primary_text)
                                .size(20.0)
                                .strong(),
                        )
                        .show(ui);
                        Label::new(RichText::new(subtitle).color(secondary_text).size(14.0))
                            .show(ui);
                        if total > 0 {
                            ui.add_space(8.0);
                            ui.label(format!("{current}/{total}"));
                        }
                        if can_cancel {
                            ui.add_space(8.0);
                            if ui.button("Cancel").clicked() {
                                cancel_requested = true;
                            }
                        }
                    });
                },
            );
        });

    ctx.request_repaint_after(std::time::Duration::from_millis(33));
    cancel_requested
}
