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
        Box::new(|cc| {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "lucide".to_owned(),
                egui::FontData::from_static(lucide_icons::LUCIDE_FONT_BYTES).into(),
            );
            fonts
                .families
                .entry(egui::FontFamily::Name("lucide".into()))
                .or_default()
                .push("lucide".to_owned());
            // egui-shadcn draws widget icons (e.g. the sheet close button) with the proportional family.
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .push("lucide".to_owned());
            cc.egui_ctx.set_fonts(fonts);
            Ok(Box::new(RackApp::default()))
        }),
    )
    .map_err(|e| e.to_string())
}
