use rack::app::RackApp;

fn main() -> Result<(), String> {
    rack::app::diagnostics::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1000.0, 800.0]),
        ..Default::default()
    };

    eframe::run_native(
        "DSP Rack Interface",
        options,
        Box::new(|_cc| Ok(Box::new(RackApp::default()))),
    )
    .map_err(|e| e.to_string())
}
