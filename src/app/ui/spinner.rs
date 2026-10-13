use super::theme::Theme;

pub(crate) fn show(ui: &mut egui::Ui, _theme: &Theme, _color: egui::Color32) {
    ui.add(egui::Spinner::new().size(20.0));
}
