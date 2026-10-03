mod audio;
mod controls;
mod plugin;
mod plugin_editor;
mod settings;
mod windows;

use controls::RackControls;
use eframe::Frame;
use egui::Ui;
use plugin_editor::PluginEditor;
use settings::SettingsStore;

pub struct RackApp {
    controls: RackControls,
    plugin_editor: PluginEditor,
    settings: SettingsStore,
}

impl Default for RackApp {
    fn default() -> Self {
        let settings = SettingsStore::load();
        Self {
            controls: RackControls::from_settings(&settings.values),
            plugin_editor: PluginEditor::from_settings(&settings.values.audio),
            settings,
        }
    }
}

impl eframe::App for RackApp {
    fn ui(&mut self, ui: &mut Ui, frame: &mut Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.plugin_editor.ui(ui, frame);
                ui.separator();
                self.controls.ui(ui);
            });
        });

        let values = self.current_settings();
        self.settings.save_if_changed(&values);
        if let Some(error) = &self.settings.error {
            ui.colored_label(egui::Color32::RED, error);
        }
    }

    fn on_exit(&mut self) {
        self.plugin_editor.shutdown();
    }
}

impl RackApp {
    fn current_settings(&self) -> settings::AppSettings {
        settings::AppSettings {
            gain: self.controls.gain(),
            tone: self.controls.tone(),
            level: self.controls.level(),
            audio: self.plugin_editor.audio_settings(),
        }
    }
}
