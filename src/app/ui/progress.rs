use super::theme::Theme;

pub(crate) fn show(ui: &mut egui::Ui, theme: &Theme, value: f32) {
    egui_shadcn::progress(
        ui,
        theme.renderer_theme(),
        egui_shadcn::ProgressProps::new(Some(value))
            .size(egui_shadcn::ProgressSize::Size2)
            .variant(egui_shadcn::ProgressVariant::Classic),
    );
}
