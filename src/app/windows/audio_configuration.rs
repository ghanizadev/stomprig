pub struct AudioConfigurationWindow;

impl AudioConfigurationWindow {
    pub fn show(ctx: &egui::Context, open: &mut bool, content: impl FnOnce(&mut egui::Ui)) {
        if !*open {
            return;
        }
        let mut visible = *open;
        egui::Window::new("Audio Configuration")
            .open(&mut visible)
            .resizable(false)
            .default_width(420.0)
            .order(egui::Order::Foreground)
            .show(ctx, content);
        *open = visible;
    }
}
