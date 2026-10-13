use super::theme::Theme;

pub(crate) fn search(
    ui: &mut egui::Ui,
    _theme: &Theme,
    id: egui::Id,
    width: f32,
    placeholder: &str,
    left_slot: impl Fn(&egui::Painter, egui::Rect, egui::Color32),
    value: &mut String,
) {
    let width = width.min(ui.available_width()).max(0.0);
    ui.allocate_ui_with_layout(
        egui::vec2(width, 34.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.horizontal(|ui| {
                let (icon_rect, _) =
                    ui.allocate_exact_size(egui::vec2(18.0, 22.0), egui::Sense::hover());
                left_slot(ui.painter(), icon_rect, ui.visuals().text_color());
                ui.add_sized(
                    [ui.available_width(), 26.0],
                    egui::TextEdit::singleline(value)
                        .id(id)
                        .hint_text(placeholder)
                        .margin(egui::vec2(0.0, 3.0)),
                );
            });
        },
    );
}
