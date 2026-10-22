use std::fs;
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use stomprig::app::StompRigApp;
use stomprig::app::updates::{apply_update, create_update_manager, initialize_velopack};
use vst3_host::{ProbeResult, SafeDiscoverySkip, Vst3Host};

const VERIFY_PACKAGED_VST3_TOOLS_ARGUMENT: &str = "--stomprig-verify-vst3-tools";

#[cfg(target_os = "macos")]
#[allow(dead_code)]
#[path = "../../vendor/vst3-host/src/bin/vst3-host-helper.rs"]
mod embedded_vst3_helper;

fn main() -> Result<(), String> {
    initialize_velopack();

    let mut arguments = std::env::args_os();
    let _ = arguments.next();
    let command = arguments.next();
    #[cfg(target_os = "macos")]
    if command.as_deref()
        == Some(std::ffi::OsStr::new(
            vst3_host::process_isolation::SELF_HOSTED_HELPER_ARGUMENT,
        ))
    {
        embedded_vst3_helper::run();
        return Ok(());
    }
    if command.as_deref() == Some(std::ffi::OsStr::new("--stomprig-clap-probe")) {
        let bundle_path = arguments
            .next()
            .ok_or_else(|| "CLAP probe helper requires a bundle path".to_owned())?;
        let result_path = arguments
            .next()
            .ok_or_else(|| "CLAP probe helper requires a result path".to_owned())?;
        return stomprig::app::run_clap_probe_helper(&bundle_path, &result_path);
    }
    if command.as_deref() == Some(std::ffi::OsStr::new("--stomprig-vst3-probe")) {
        let bundle_path = arguments
            .next()
            .ok_or_else(|| "VST3 probe helper requires a bundle path".to_owned())?;
        let result_path = arguments
            .next()
            .ok_or_else(|| "VST3 probe helper requires a result path".to_owned())?;
        return stomprig::app::run_vst3_probe_helper(&bundle_path, &result_path);
    }
    if command.as_deref() == Some(std::ffi::OsStr::new(VERIFY_PACKAGED_VST3_TOOLS_ARGUMENT)) {
        return verify_packaged_vst3_tools();
    }

    let startup_report = stomprig::app::diagnostics::init();
    let update_manager = match create_update_manager() {
        Ok(manager) => Some(manager),
        Err(error) => {
            log::debug!("Velopack updates are unavailable for this installation: {error}");
            None
        }
    };
    let app_update_manager = update_manager.clone();
    let (pending_update_sender, pending_update_receiver) = mpsc::channel();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../../assets/icon.png"))
                    .expect("assets/icon.png must be a valid PNG"),
            )
            .with_inner_size([1000.0, 800.0])
            .with_decorations(false),
        ..Default::default()
    };

    let result = eframe::run_native(
        "StompRig",
        options,
        Box::new(move |cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
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
            cc.egui_ctx.set_fonts(fonts);
            Ok(Box::new(StompRigApp::new_with_updates(
                startup_report,
                app_update_manager,
                pending_update_sender,
            )))
        }),
    );
    stomprig::app::diagnostics::mark_clean_shutdown();
    result.map_err(|e| e.to_string())?;

    if let Ok(update) = pending_update_receiver.try_recv() {
        let manager = update_manager
            .as_ref()
            .ok_or_else(|| "the Velopack update manager is unavailable".to_owned())?;
        apply_update(manager, &update)?;
    }

    Ok(())
}

fn verify_packaged_vst3_tools() -> Result<(), String> {
    // The package check must exercise the sibling executables even when the invoking shell has
    // configured development overrides. This command runs before StompRig starts any threads.
    unsafe {
        std::env::remove_var("VST3_HOST_HELPER_PATH");
        std::env::remove_var("VST3_HOST_PROBE_PATH");
    }

    let unique_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("could not create a temporary VST3 package fixture: {error}"))?
        .as_nanos();
    let fixture_root = std::env::temp_dir().join(format!(
        "stomprig-package-check-{}-{unique_id}",
        std::process::id()
    ));
    fs::create_dir(&fixture_root)
        .map_err(|error| format!("could not create package check directory: {error}"))?;

    let result = (|| {
        let malformed_bundle = fixture_root.join("Malformed.vst3");
        fs::create_dir(&malformed_bundle)
            .map_err(|error| format!("could not create malformed VST3 fixture: {error}"))?;

        let discovery_host = Vst3Host::builder()
            .add_scan_path(&fixture_root)
            .probe_timeout(Duration::from_secs(5))
            .build()
            .map_err(|error| format!("could not configure the VST3 probe check: {error}"))?;
        let report = discovery_host.discover_plugins_safe();
        if let Some(error) = report.error {
            return Err(format!(
                "packaged VST3 probe executable was not usable: {error}"
            ));
        }
        match report.skipped.as_slice() {
            [SafeDiscoverySkip::Failed { path, detail }]
                if path == &malformed_bundle && !detail.contains("failed to spawn probe") => {}
            skips => {
                return Err(format!(
                    "packaged VST3 probe did not inspect the malformed fixture as expected: {skips:?}"
                ));
            }
        }

        let runtime_host = Vst3Host::builder()
            .with_process_isolation(true)
            .response_timeout(Duration::from_secs(5))
            .slow_command_timeout(Duration::from_secs(5))
            .build()
            .map_err(|error| format!("could not configure the VST3 helper check: {error}"))?;
        match runtime_host.probe_plugin(&malformed_bundle) {
            ProbeResult::Failed(message) if !message.contains("helper unavailable") => {}
            result => {
                return Err(format!(
                    "packaged VST3 helper was not found or did not answer: {result:?}"
                ));
            }
        }

        Ok(())
    })();

    let cleanup_result = fs::remove_dir_all(&fixture_root);
    result?;
    cleanup_result.map_err(|error| format!("could not remove package check fixture: {error}"))?;
    println!("verified packaged VST3 helper and probe executables");
    Ok(())
}
