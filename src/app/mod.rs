mod audio;
mod controls;
mod plugin;
mod plugin_editor;
mod windows;

use controls::RackControls;
use eframe::Frame;
use egui::Ui;
use plugin_editor::PluginEditor;

pub struct RackApp {
    controls: RackControls,
    plugin_editor: PluginEditor,
}

impl Default for RackApp {
    fn default() -> Self {
        Self {
            controls: RackControls::new(),
            plugin_editor: PluginEditor::default(),
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
    }
}
