use rack::app::RackApp;

fn main() -> Result<(), String> {
    let mut arguments = std::env::args_os();
    let _ = arguments.next();
    if arguments.next().as_deref() == Some(std::ffi::OsStr::new("--rack-clap-probe")) {
        let bundle_path = arguments
            .next()
            .ok_or_else(|| "CLAP probe helper requires a bundle path".to_owned())?;
        let result_path = arguments
            .next()
            .ok_or_else(|| "CLAP probe helper requires a result path".to_owned())?;
        return rack::app::run_clap_probe_helper(&bundle_path, &result_path);
    }

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
