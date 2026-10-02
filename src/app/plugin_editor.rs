use eframe::Frame;
use raw_window_handle::HasWindowHandle;
use std::sync::{Arc, Mutex};
use vst3_host::{AudioHandle, EditorRect, EmbeddedEditor, Plugin, simple};

pub struct PluginEditor {
    path: String,
    plugin: Option<Arc<Mutex<Plugin>>>,
    // Keeps the plugin's audio stream alive while its editor is embedded.
    _audio: Option<AudioHandle>,
    editor: Option<EmbeddedEditor>,
    size: egui::Vec2,
    error: Option<String>,
}

impl Default for PluginEditor {
    fn default() -> Self {
        Self {
            path: std::env::var("VST3_PLUGIN_PATH").unwrap_or_default(),
            plugin: None,
            _audio: None,
            editor: None,
            size: egui::vec2(800.0, 600.0),
            error: None,
        }
    }
}

impl PluginEditor {
    pub fn ui(&mut self, ui: &mut egui::Ui, frame: &mut Frame) {
        ui.heading("VST3 Rack");
        ui.horizontal(|ui| {
            ui.label("Plugin:");
            ui.text_edit_singleline(&mut self.path);
            if ui.button("Load editor").clicked() {
                self.load();
            }
        });

        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::RED, error);
        }

        ui.add_space(12.0);
        self.show_editor(ui, frame);
    }

    fn load(&mut self) {
        self.editor = None;
        self.plugin = None;
        self._audio = None;
        self.error = None;

        if self.path.trim().is_empty() {
            self.error = Some("Set VST3_PLUGIN_PATH or enter a .vst3 path.".to_string());
            return;
        }

        let result = (|| {
            let plugin = simple::load_plugin(self.path.trim()).map_err(|e| e.to_string())?;
            let audio = simple::play(plugin).map_err(|e| e.to_string())?;
            let plugin = audio.plugin();
            let size = plugin
                .lock()
                .map_err(|_| "plugin lock poisoned".to_string())?
                .get_editor_size()
                .unwrap_or((800, 600));
            Ok::<_, String>((audio, plugin, egui::vec2(size.0 as f32, size.1 as f32)))
        })();

        match result {
            Ok((audio, plugin, size)) => {
                self._audio = Some(audio);
                self.plugin = Some(plugin);
                self.size = size;
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn show_editor(&mut self, ui: &mut egui::Ui, frame: &mut Frame) {
        let Some(plugin) = self.plugin.as_ref().cloned() else {
            ui.label("Load a VST3 plugin to show its native editor here.");
            return;
        };

        if let Some(editor) = &self.editor {
            if let Some((width, height)) = editor.take_resize_request() {
                self.size = egui::vec2(width as f32, height as f32);
            }
        }

        let (rect, _) = ui.allocate_exact_size(self.size, egui::Sense::hover());
        let editor_rect = EditorRect {
            x: rect.min.x,
            y: rect.min.y,
            width: rect.width(),
            height: rect.height(),
        };

        if let Some(editor) = &self.editor {
            editor.set_rect(editor_rect);
            return;
        }

        let handle = match frame.window_handle() {
            Ok(handle) => handle,
            Err(error) => {
                self.error = Some(format!("Could not get host window handle: {error}"));
                return;
            }
        };

        match EmbeddedEditor::embed(plugin, handle.as_raw(), editor_rect) {
            Ok(editor) => self.editor = Some(editor),
            Err(error) => self.error = Some(format!("Could not embed plugin editor: {error}")),
        }
    }
}
