use super::audio::{AudioConfig, AudioDevices, AudioEngine, ChannelRouting, ShutdownMode};
use super::plugin::{PLUGIN_PATHS, load_stack, service_run_loop};
use super::settings::{AudioSettings, Settings};
use super::windows::AudioMonitor;
use eframe::Frame;
use raw_window_handle::HasWindowHandle;
use std::sync::{Arc, Mutex};
use vst3_host::{EditorRect, EmbeddedEditor, Plugin};

#[derive(Debug, Clone)]
enum EditorState {
    Unloaded,
    ReadyToEmbed,
    Embedded,
    AudioUnavailable,
    Error,
}

pub struct PluginEditor {
    plugins: Vec<Arc<Mutex<Plugin>>>,
    engine: Option<AudioEngine>,
    active_audio_config: Option<AudioConfig>,
    editors: Vec<EmbeddedEditor>,
    sizes: Vec<egui::Vec2>,
    error: Option<String>,
    monitor: AudioMonitor,
    state: EditorState,
    settings: Settings,
}

impl Default for PluginEditor {
    fn default() -> Self {
        Self {
            plugins: Vec::new(),
            engine: None,
            active_audio_config: None,
            editors: Vec::new(),
            sizes: Vec::new(),
            error: None,
            monitor: AudioMonitor::default(),
            state: EditorState::Unloaded,
            settings: Settings::default(),
        }
    }
}

impl PluginEditor {
    pub fn from_settings(settings: &AudioSettings) -> Self {
        let mut editor = Self::default();
        editor.settings = Settings::from_audio_settings(settings);
        editor
    }

    pub fn audio_settings(&self) -> AudioSettings {
        self.settings.audio_settings()
    }

    pub fn open_audio_configuration(&mut self) {
        self.settings.open_audio_configuration();
    }

    pub fn audio_configuration_open(&self) -> bool {
        self.settings.audio_configuration_open()
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, frame: &mut Frame) {
        ui.heading("VST3 Rack");
        ui.horizontal(|ui| {
            ui.label(format!("Plugin stack ({} entries)", PLUGIN_PATHS.len()));
            if ui.button("Load stack").clicked() {
                self.load();
            }
        });
        let plugin_latency_samples = self
            .plugins
            .iter()
            .filter_map(|plugin| plugin.lock().ok())
            .map(|plugin| f64::from(plugin.latency_samples()))
            .sum();
        if self
            .settings
            .show_configuration(ui, &mut self.error, plugin_latency_samples)
        {
            self.apply_audio_devices();
        }

        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::RED, error);
        }
        ui.label(format!("Editor: {}", self.state.label()));

        ui.add_space(12.0);
        self.show_editor(ui, frame);
        self.show_audio(ui);
    }

    fn load(&mut self) {
        self.shutdown(ShutdownMode::UnloadPlugins);
        self.error = None;
        self.state = EditorState::Unloaded;

        let plugin_result = match self
            .settings
            .output_index
            .and_then(|index| AudioDevices::sample_rate(self.settings.backend, index).ok())
        {
            Some(sample_rate) => load_stack(
                Some(sample_rate),
                self.settings.buffer_size.frames() as usize,
            ),
            None => load_stack(None, self.settings.buffer_size.frames() as usize),
        };

        match plugin_result {
            Ok(plugins) => {
                let plugins = plugins
                    .into_iter()
                    .map(|plugin| Arc::new(Mutex::new(plugin)))
                    .collect();
                if let Err(error) = self.install_plugins(plugins) {
                    self.error = Some(error);
                }
            }
            Err(error) => {
                self.error = Some(format!("Could not load editor: {error}"));
            }
        }
    }

    fn apply_audio_devices(&mut self) {
        if self.plugins.is_empty() {
            self.load();
        } else {
            self.restart_audio();
        }
    }

    pub fn shutdown(&mut self, mode: ShutdownMode) {
        // An embedded editor owns a child window in the main viewport. Detach it while
        // that viewport is still alive, before deactivating or dropping its plugin.
        if !self.editors.is_empty() {
            if matches!(mode, ShutdownMode::KeepPluginsLoaded) {
                log::debug!(
                    "leaving {} plugin editor(s) for process teardown",
                    self.editors.len()
                );
                // Do not invoke close_editor during final process teardown. The parent viewport
                // is about to disappear and the operating system will reclaim these child views.
                std::mem::forget(std::mem::take(&mut self.editors));
            } else {
                log::debug!("detaching {} plugin editor(s)", self.editors.len());
                self.editors.clear();
            }
        }
        if let Some(engine) = self.engine.take() {
            engine.shutdown(mode);
        }
        self.active_audio_config = None;
        if !self.plugins.is_empty() {
            if matches!(mode, ShutdownMode::KeepPluginsLoaded) {
                log::debug!(
                    "leaving {} plugin(s) loaded for process teardown",
                    self.plugins.len()
                );
                // Keep the Arc references alive so Plugin::drop is never entered during exit.
                std::mem::forget(std::mem::take(&mut self.plugins));
            } else {
                log::debug!("releasing {} plugin(s)", self.plugins.len());
                self.plugins.clear();
            }
        }
        self.sizes.clear();
        self.state = EditorState::Unloaded;
    }

    fn install_plugins(&mut self, plugins: Vec<Arc<Mutex<Plugin>>>) -> Result<(), String> {
        let mut sizes = Vec::with_capacity(plugins.len());
        for (index, plugin) in plugins.iter().enumerate() {
            let plugin_guard = plugin
                .lock()
                .map_err(|_| format!("plugin {index} lock poisoned"))?;
            if !plugin_guard.has_editor() {
                return Err(format!("Plugin {index} does not provide an editor"));
            }
            let size = plugin_guard
                .get_editor_size()
                .map_err(|error| format!("Could not query plugin {index} editor size: {error}"))?;
            sizes.push(egui::vec2(size.0 as f32, size.1 as f32));
        }
        self.plugins = plugins;
        self.sizes = sizes;
        self.state = EditorState::ReadyToEmbed;
        Ok(())
    }

    fn show_editor(&mut self, ui: &mut egui::Ui, frame: &mut Frame) {
        if self.plugins.is_empty() {
            ui.label("Load a VST3 plugin to show its native editor here.");
            return;
        }
        let handle = match frame.window_handle() {
            Ok(handle) => handle,
            Err(error) => {
                self.error = Some(format!("Could not get host window handle: {error}"));
                return;
            }
        };
        for index in 0..self.plugins.len() {
            if let Some(editor) = self.editors.get_mut(index)
                && let Some((width, height)) = editor.take_resize_request()
            {
                self.sizes[index] = egui::vec2(width as f32, height as f32);
            }
            let size = self.sizes[index];
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            let editor_rect = EditorRect {
                x: rect.min.x,
                y: rect.min.y,
                width: rect.width(),
                height: rect.height(),
            };
            if let Some(editor) = self.editors.get_mut(index) {
                match editor.try_set_rect(editor_rect) {
                    Ok(accepted) => {
                        self.sizes[index] = egui::vec2(accepted.width, accepted.height);
                    }
                    Err(error) => {
                        self.state = EditorState::Error;
                        self.error =
                            Some(format!("Could not resize plugin {index} editor: {error}"));
                    }
                }
            } else {
                match EmbeddedEditor::embed(
                    self.plugins[index].clone(),
                    handle.as_raw(),
                    editor_rect,
                ) {
                    Ok(editor) => self.editors.push(editor),
                    Err(error) => {
                        self.state = EditorState::Error;
                        self.error =
                            Some(format!("Could not embed plugin {index} editor: {error}"));
                    }
                }
            }
            service_run_loop(&self.plugins[index]);
        }
        if self.editors.len() == self.plugins.len() {
            self.state = EditorState::Embedded;
            self.start_audio();
        }
    }

    fn start_audio(&mut self) {
        if self.engine.is_some() || self.editors.len() != self.plugins.len() {
            return;
        }
        let Some(config) = self.selected_audio_config() else {
            self.state = EditorState::AudioUnavailable;
            self.error = Some("Editors loaded, but no audio devices are selected.".to_string());
            return;
        };
        match AudioEngine::start(Arc::new(self.plugins.clone()), config) {
            Ok(engine) => {
                self.engine = Some(engine);
                self.active_audio_config = Some(config);
                self.error = None;
            }
            Err(error) => {
                self.state = EditorState::AudioUnavailable;
                self.error = Some(format!("Editors loaded, but audio is unavailable: {error}"));
            }
        }
    }

    fn restart_audio(&mut self) {
        let Some(config) = self.selected_audio_config() else {
            self.error = Some("Select input and output devices and channels.".to_string());
            return;
        };
        if self.engine.is_some() && self.active_audio_config == Some(config) {
            return;
        }
        if let Some(engine) = self.engine.take() {
            engine.shutdown(ShutdownMode::UnloadPlugins);
        }
        self.active_audio_config = None;
        self.start_audio();
    }

    fn selected_audio_config(&self) -> Option<AudioConfig> {
        Some(AudioConfig {
            backend: self.settings.backend,
            input_index: self.settings.input_index?,
            output_index: self.settings.output_index?,
            buffer_size: self.settings.buffer_size,
            routing: ChannelRouting {
                input_left: self.settings.input_left_channel?,
                input_right: self.settings.input_right_channel?,
                output_left: self.settings.output_left_channel?,
                output_right: self.settings.output_right_channel?,
            },
        })
    }

    fn show_audio(&mut self, ui: &mut egui::Ui) {
        let Some(engine) = self.engine.as_mut() else {
            return;
        };
        if let Some(error) = self.monitor.ui(ui, engine) {
            self.error = Some(error);
        }
    }
}

impl Drop for PluginEditor {
    fn drop(&mut self) {
        self.shutdown(ShutdownMode::UnloadPlugins);
    }
}

impl EditorState {
    fn label(&self) -> &str {
        match self {
            Self::Unloaded => "unloaded",
            Self::ReadyToEmbed => "ready",
            Self::Embedded => "embedded",
            Self::AudioUnavailable => "embedded (audio unavailable)",
            Self::Error => "error",
        }
    }
}
