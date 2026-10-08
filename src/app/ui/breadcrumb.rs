use super::theme::Theme;

pub(crate) fn show(ui: &mut egui::Ui, theme: &Theme, segments: &[String]) {
    egui_shadcn::breadcrumb(
        ui,
        theme.renderer_theme(),
        egui_shadcn::BreadcrumbProps::new().text_size(13.0),
        |ui, context| {
            egui_shadcn::breadcrumb_list(ui, context, |ui, context| {
                let last = segments.len().saturating_sub(1);
                for (index, segment) in segments.iter().enumerate() {
                    egui_shadcn::breadcrumb_item(ui, context, |ui| {
                        let faded = theme.colors().muted_foreground.gamma_multiply(0.7);
                        ui.label(egui::RichText::new(segment).color(faded));
                    });
                    if index != last {
                        egui_shadcn::breadcrumb_separator(ui, context, None);
                    }
                }
            });
        },
    );
}
