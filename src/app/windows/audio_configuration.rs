pub struct AudioConfigurationWindow;

impl AudioConfigurationWindow {
    pub fn show(ctx: &egui::Context, open: &mut bool, mut content: impl FnMut(&mut egui::Ui)) {
        if !*open {
            return;
        }

        let viewport_id = egui::ViewportId::from_hash_of("audio-configuration");
        let viewport_builder = egui::ViewportBuilder::default()
            .with_title("Audio Configuration")
            .with_inner_size([520.0, 420.0])
            .with_min_inner_size([520.0, 420.0])
            .with_max_inner_size([520.0, 420.0])
            .with_resizable(false)
            .with_maximize_button(false)
            .with_always_on_top();

        let close_requested = ctx.show_viewport_immediate(
            viewport_id,
            viewport_builder,
            |viewport_ctx, viewport_class| {
                if matches!(viewport_class, egui::ViewportClass::Embedded) {
                    let mut visible = true;
                    egui::Window::new("Audio Configuration")
                        .open(&mut visible)
                        .resizable(false)
                        .default_width(420.0)
                        .order(egui::Order::Foreground)
                        .show(viewport_ctx, |ui| content(ui));
                    !visible
                } else {
                    egui::CentralPanel::default()
                        .show(viewport_ctx, |ui| content(ui));
                    viewport_ctx.input(|input| input.viewport().close_requested())
                }
            },
        );

        if close_requested {
            *open = false;
        }
    }
}
