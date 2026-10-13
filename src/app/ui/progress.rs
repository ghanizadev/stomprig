use super::theme::Theme;

pub(crate) fn show(ui: &mut egui::Ui, _theme: &Theme, value: f32) {
    ui.add(egui::ProgressBar::new(value.clamp(0.0, 1.0)).desired_height(8.0));
}
