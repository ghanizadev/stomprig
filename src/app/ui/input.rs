use super::theme::Theme;

pub(crate) fn search(
    ui: &mut egui::Ui,
    theme: &Theme,
    id: egui::Id,
    width: f32,
    placeholder: &str,
    left_slot: impl Fn(&egui::Painter, egui::Rect, egui::Color32),
    value: &mut String,
) {
    egui_shadcn::Input::new(id)
        .size(egui_shadcn::InputSize::Size2)
        .width(width)
        .placeholder(placeholder)
        .left_slot(left_slot)
        .show(ui, theme.renderer_theme(), value);
}
