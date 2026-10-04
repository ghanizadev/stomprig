mod audio;
pub mod diagnostics;
mod plugin;
mod plugin_editor;
mod settings;
mod windows;

use eframe::Frame;
use audio::ShutdownMode;
use plugin_editor::PluginEditor;
use settings::SettingsStore;

pub struct RackApp {
    plugin_editor: PluginEditor,
    settings: SettingsStore,
}

impl Default for RackApp {
    fn default() -> Self {
        let settings = SettingsStore::load();
        Self {
            plugin_editor: PluginEditor::from_settings(&settings.values.audio),
            settings,
        }
    }
}

impl eframe::App for RackApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut Frame) {
        let main_window_enabled = !self.plugin_editor.audio_configuration_open();
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            ui.add_enabled_ui(main_window_enabled, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    ui.menu_button("File", |ui| {
                        if ui.button("Close").clicked() {
                            ui.close();
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });

                    ui.menu_button("Edit", |ui| {
                        if ui.button("Settings").clicked() {
                            ui.close();
                            self.plugin_editor.open_audio_configuration();
                        }
                    });
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_enabled_ui(main_window_enabled, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.plugin_editor.ui(ui, frame);
                });
                let values = self.current_settings();
                self.settings.save_if_changed(&values);
                if let Some(error) = &self.settings.error {
                    ui.colored_label(egui::Color32::RED, error);
                }
            });
        });
    }

    fn on_exit(&mut self, _gl: Option<&glow::Context>) {
        // eframe invokes this while the root viewport still exists. Native VST editors
        // must be detached before eframe destroys their parent window.
        log::info!("application shutdown; stopping plugin stack");
        self.plugin_editor.shutdown(ShutdownMode::KeepPluginsLoaded);
    }
}

impl RackApp {
    fn current_settings(&self) -> settings::AppSettings {
        settings::AppSettings {
            audio: self.plugin_editor.audio_settings(),
        }
    }
}
