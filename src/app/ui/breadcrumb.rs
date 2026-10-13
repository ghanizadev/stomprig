use super::{Label, theme::Theme};

pub(crate) fn show(ui: &mut egui::Ui, _theme: &Theme, segments: &[String]) {
    ui.horizontal(|ui| {
        let last = segments.len().saturating_sub(1);
        for (index, segment) in segments.iter().enumerate() {
            Label::new(segment).show(ui);
            if index != last {
                ui.label("/");
            }
        }
    });
}
