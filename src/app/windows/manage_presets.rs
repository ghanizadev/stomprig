pub struct ManagePresetsWindow;

impl ManagePresetsWindow {
    pub fn show(ctx: &egui::Context, mut content: impl FnMut(&mut egui::Ui) -> bool) -> bool {
        let viewport_id = egui::ViewportId::from_hash_of("manage-presets");
        let viewport_builder = egui::ViewportBuilder::default()
            .with_title("Manage presets")
            .with_inner_size([940.0, 680.0])
            .with_min_inner_size([940.0, 680.0])
            .with_active(true)
            .with_resizable(false)
            .with_close_button(true)
            .with_always_on_top();

        ctx.show_viewport_immediate(
            viewport_id,
            viewport_builder,
            |viewport_ctx, viewport_class| {
                let mut close_requested_by_contents = false;
                if matches!(viewport_class, egui::ViewportClass::Embedded) {
                    egui::Window::new("Manage presets")
                        .collapsible(false)
                        .resizable(false)
                        .show(viewport_ctx, |ui| {
                            close_requested_by_contents = content(ui);
                        });
                    close_requested_by_contents
                        || viewport_ctx.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                        })
                } else {
                    egui::CentralPanel::default().show(viewport_ctx, |ui| {
                        close_requested_by_contents = content(ui);
                    });
                    close_requested_by_contents
                        || viewport_ctx.input(|input| input.viewport().close_requested())
                        || viewport_ctx.input_mut(|input| {
                            input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)
                        })
                }
            },
        )
    }
}
