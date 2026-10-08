use super::theme::Theme;

pub(crate) fn show(ui: &mut egui::Ui, theme: &Theme, color: egui::Color32) {
    let _ = egui_shadcn::spinner(
        ui,
        theme.renderer_theme(),
        egui_shadcn::SpinnerProps::default()
            .size(egui_shadcn::SpinnerSize::Size3)
            .color(color)
            .opacity(1.0)
            .variant(egui_shadcn::SpinnerVariant::RadixLeaf),
    );
}
