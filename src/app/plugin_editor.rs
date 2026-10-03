use super::audio::{
    AudioBackend, AudioConfig, AudioDevices, AudioEngine, ChannelRouting, DeviceInfo,
};
use super::plugin::{load as load_plugin, service_run_loop};
use super::windows::{AudioConfigurationWindow, AudioMonitor};
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
    path: String,
    plugin: Option<Arc<Mutex<Plugin>>>,
    engine: Option<AudioEngine>,
    editor: Option<EmbeddedEditor>,
    size: egui::Vec2,
    error: Option<String>,
    monitor: AudioMonitor,
    native_size: Option<egui::Vec2>,
    state: EditorState,
    input_devices: Vec<DeviceInfo>,
    output_devices: Vec<DeviceInfo>,
    input_index: Option<usize>,
    output_index: Option<usize>,
    input_left_channel: Option<usize>,
    input_right_channel: Option<usize>,
    output_left_channel: Option<usize>,
    output_right_channel: Option<usize>,
    backend: AudioBackend,
    audio_config_open: bool,
}

impl Default for PluginEditor {
    fn default() -> Self {
        Self {
            path: std::env::var("VST3_PLUGIN_PATH").unwrap_or_default(),
            plugin: None,
            engine: None,
            editor: None,
            size: egui::vec2(800.0, 600.0),
            error: None,
            monitor: AudioMonitor::default(),
            native_size: None,
            state: EditorState::Unloaded,
            input_devices: Vec::new(),
            output_devices: Vec::new(),
            input_index: None,
            output_index: None,
            input_left_channel: None,
            input_right_channel: None,
            output_left_channel: None,
            output_right_channel: None,
            backend: AudioBackend::Wasapi,
            audio_config_open: false,
        }
    }
}

impl PluginEditor {
    pub fn ui(&mut self, ui: &mut egui::Ui, frame: &mut Frame) {
        if self.input_devices.is_empty() && self.output_devices.is_empty() {
            self.refresh_devices();
        }
        ui.heading("VST3 Rack");
        ui.horizontal(|ui| {
            ui.label("Plugin:");
            ui.text_edit_singleline(&mut self.path);
            if ui.button("Load editor").clicked() {
                self.load();
            }
        });
        if ui.button("Audio Configuration").clicked() {
            self.audio_config_open = true;
        }
        self.show_audio_configuration(ui);
        self.show_channel_routing(ui);

        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::RED, error);
        }
        ui.label(format!("Editor: {}", self.state.label()));

        ui.add_space(12.0);
        self.show_editor(ui, frame);
        self.show_audio(ui);
    }

    fn load(&mut self) {
        self.editor = None;
        self.plugin = None;
        self.engine = None;
        self.error = None;
        self.native_size = None;
        self.state = EditorState::Unloaded;

        if self.path.trim().is_empty() {
            self.error = Some("Set VST3_PLUGIN_PATH or enter a .vst3 path.".to_string());
            return;
        }

        let plugin_result = match self
            .output_index
            .and_then(|index| AudioDevices::sample_rate(self.backend, index).ok())
        {
            Some(sample_rate) => load_plugin(self.path.trim(), Some(sample_rate)),
            None => load_plugin(self.path.trim(), None),
        };

        match plugin_result {
            Ok(plugin) => {
                if let Err(error) = self.install_plugin(Arc::new(Mutex::new(plugin))) {
                    self.error = Some(error);
                }
            }
            Err(error) => {
                self.error = Some(format!("Could not load editor: {error}"));
            }
        }
    }

    fn refresh_devices(&mut self) {
        let old_input = self
            .input_index
            .and_then(|index| {
                self.input_devices
                    .iter()
                    .find(|device| device.index == index)
            })
            .map(|device| device.name.clone());
        let old_output = self
            .output_index
            .and_then(|index| {
                self.output_devices
                    .iter()
                    .find(|device| device.index == index)
            })
            .map(|device| device.name.clone());

        match AudioDevices::input_devices(self.backend) {
            Ok(devices) => self.input_devices = devices,
            Err(error) => self.error = Some(error),
        }
        match AudioDevices::output_devices(self.backend) {
            Ok(devices) => self.output_devices = devices,
            Err(error) => self.error = Some(error),
        }

        let (default_input, default_output) = AudioDevices::default_device_indices(self.backend);
        self.input_index = old_input
            .and_then(|name| self.input_devices.iter().find(|device| device.name == name))
            .map(|device| device.index)
            .or(default_input)
            .or_else(|| self.input_devices.first().map(|device| device.index));
        self.output_index = old_output
            .and_then(|name| {
                self.output_devices
                    .iter()
                    .find(|device| device.name == name)
            })
            .map(|device| device.index)
            .or(default_output)
            .or_else(|| self.output_devices.first().map(|device| device.index));
        self.normalize_channel_selections();
    }

    fn normalize_channel_selections(&mut self) {
        if let Some(channels) = self
            .input_index
            .and_then(|index| {
                self.input_devices
                    .iter()
                    .find(|device| device.index == index)
            })
            .map(|device| device.channels)
        {
            if channels > 0 {
                self.input_left_channel =
                    Some(self.input_left_channel.unwrap_or(0).min(channels - 1));
                self.input_right_channel = Some(
                    self.input_right_channel
                        .unwrap_or((channels > 1) as usize)
                        .min(channels - 1),
                );
            }
        } else {
            self.input_left_channel = None;
            self.input_right_channel = None;
        }

        if let Some(channels) = self
            .output_index
            .and_then(|index| {
                self.output_devices
                    .iter()
                    .find(|device| device.index == index)
            })
            .map(|device| device.channels)
        {
            if channels > 0 {
                self.output_left_channel =
                    Some(self.output_left_channel.unwrap_or(0).min(channels - 1));
                self.output_right_channel = Some(
                    self.output_right_channel
                        .unwrap_or((channels > 1) as usize)
                        .min(channels - 1),
                );
            }
        } else {
            self.output_left_channel = None;
            self.output_right_channel = None;
        }
    }

    fn show_channel_routing(&mut self, ui: &mut egui::Ui) {
        self.normalize_channel_selections();
        ui.group(|ui| {
            ui.label("Track channel routing");
            ui.horizontal(|ui| {
                Self::show_channel_selector(
                    ui,
                    "Input left",
                    "input-left-channel",
                    &mut self.input_left_channel,
                    self.input_index.and_then(|index| {
                        self.input_devices
                            .iter()
                            .find(|device| device.index == index)
                    }),
                );
                Self::show_channel_selector(
                    ui,
                    "Input right",
                    "input-right-channel",
                    &mut self.input_right_channel,
                    self.input_index.and_then(|index| {
                        self.input_devices
                            .iter()
                            .find(|device| device.index == index)
                    }),
                );
            });
            ui.horizontal(|ui| {
                Self::show_channel_selector(
                    ui,
                    "Output left",
                    "output-left-channel",
                    &mut self.output_left_channel,
                    self.output_index.and_then(|index| {
                        self.output_devices
                            .iter()
                            .find(|device| device.index == index)
                    }),
                );
                Self::show_channel_selector(
                    ui,
                    "Output right",
                    "output-right-channel",
                    &mut self.output_right_channel,
                    self.output_index.and_then(|index| {
                        self.output_devices
                            .iter()
                            .find(|device| device.index == index)
                    }),
                );
            });
            if ui.button("Apply channel routing").clicked() {
                self.apply_audio_devices();
            }
        });
    }

    fn show_channel_selector(
        ui: &mut egui::Ui,
        label: &str,
        id: &'static str,
        selected: &mut Option<usize>,
        device: Option<&DeviceInfo>,
    ) {
        ui.vertical(|ui| {
            ui.label(label);
            let selected_text = selected
                .map(|channel| format!("Channel {}", channel + 1))
                .unwrap_or_else(|| "Unavailable".to_string());
            egui::ComboBox::from_id_salt(id)
                .selected_text(selected_text)
                .show_ui(ui, |ui| {
                    if let Some(device) = device {
                        for channel in 0..device.channels {
                            ui.selectable_value(
                                selected,
                                Some(channel),
                                format!("Channel {}", channel + 1),
                            );
                        }
                    }
                });
        });
    }

    fn show_audio_configuration(&mut self, ui: &mut egui::Ui) {
        let mut open = self.audio_config_open;
        AudioConfigurationWindow::show(ui.ctx(), &mut open, |ui| {
            self.show_device_controls(ui);
        });
        self.audio_config_open = open;
    }

    fn show_device_controls(&mut self, ui: &mut egui::Ui) {
        let mut backend_changed = false;
        ui.vertical(|ui| {
            ui.label("Audio backend");
            egui::ComboBox::from_id_salt("audio-backend")
                .selected_text(self.backend.label())
                .width(300.0)
                .show_ui(ui, |ui| {
                    for backend in AudioDevices::available_backends() {
                        if ui
                            .selectable_value(&mut self.backend, backend, backend.label())
                            .changed()
                        {
                            backend_changed = true;
                        }
                    }
                });

            ui.add_space(8.0);
            ui.label("Input device");
            let input_name = self
                .input_index
                .and_then(|index| {
                    self.input_devices
                        .iter()
                        .find(|device| device.index == index)
                })
                .map(|device| device.name.as_str())
                .unwrap_or("Unavailable");
            egui::ComboBox::from_id_salt("input-device")
                .selected_text(input_name)
                .width(300.0)
                .show_ui(ui, |ui| {
                    for device in &self.input_devices {
                        ui.selectable_value(
                            &mut self.input_index,
                            Some(device.index),
                            &device.name,
                        );
                    }
                });

            ui.add_space(8.0);
            ui.label("Output device");
            let output_name = self
                .output_index
                .and_then(|index| {
                    self.output_devices
                        .iter()
                        .find(|device| device.index == index)
                })
                .map(|device| device.name.as_str())
                .unwrap_or("Unavailable");
            egui::ComboBox::from_id_salt("output-device")
                .selected_text(output_name)
                .width(300.0)
                .show_ui(ui, |ui| {
                    for device in &self.output_devices {
                        ui.selectable_value(
                            &mut self.output_index,
                            Some(device.index),
                            &device.name,
                        );
                    }
                });

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button("Refresh devices").clicked() {
                    self.refresh_devices();
                }
                if ui.button("Apply audio devices").clicked() {
                    self.apply_audio_devices();
                }
            });
        });
        if backend_changed {
            self.input_devices.clear();
            self.output_devices.clear();
            self.input_index = None;
            self.output_index = None;
            self.input_left_channel = None;
            self.input_right_channel = None;
            self.output_left_channel = None;
            self.output_right_channel = None;
            self.refresh_devices();
        }
    }

    fn apply_audio_devices(&mut self) {
        self.normalize_channel_selections();
        if self.input_index.is_none()
            || self.output_index.is_none()
            || self.input_left_channel.is_none()
            || self.input_right_channel.is_none()
            || self.output_left_channel.is_none()
            || self.output_right_channel.is_none()
        {
            self.error = Some("Select input and output devices and channels.".to_string());
            return;
        }
        if let Some(engine) = self.engine.take() {
            engine.shutdown();
        }
        self.load();
    }

    fn install_plugin(&mut self, plugin: Arc<Mutex<Plugin>>) -> Result<(), String> {
        let plugin_guard = plugin
            .lock()
            .map_err(|_| "plugin lock poisoned".to_string())?;
        if !plugin_guard.has_editor() {
            return Err("The plugin does not provide an editor".to_string());
        }
        let size = plugin_guard
            .get_editor_size()
            .map_err(|error| format!("Could not query plugin editor size: {error}"))?;
        drop(plugin_guard);
        self.plugin = Some(plugin);
        self.native_size = Some(egui::vec2(size.0 as f32, size.1 as f32));
        self.size = egui::vec2(size.0 as f32, size.1 as f32);
        self.state = EditorState::ReadyToEmbed;
        Ok(())
    }

    fn show_editor(&mut self, ui: &mut egui::Ui, frame: &mut Frame) {
        let Some(plugin) = self.plugin.as_ref().cloned() else {
            ui.label("Load a VST3 plugin to show its native editor here.");
            return;
        };

        if let Some(native_size) = self.native_size {
            self.size = native_size;
        }

        if let Some(editor) = &self.editor {
            if let Some((width, height)) = editor.take_resize_request() {
                self.native_size = Some(egui::vec2(width as f32, height as f32));
                self.size = self.native_size.unwrap();
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
            match editor.try_set_rect(editor_rect) {
                Ok(accepted) => {
                    self.native_size = Some(egui::vec2(accepted.width, accepted.height));
                    self.state = EditorState::Embedded;
                }
                Err(error) => {
                    self.state = EditorState::Error;
                    self.error = Some(format!("Could not resize embedded editor: {error}"));
                }
            }
            service_run_loop(&plugin);
            return;
        }

        let handle = match frame.window_handle() {
            Ok(handle) => handle,
            Err(error) => {
                self.error = Some(format!("Could not get host window handle: {error}"));
                return;
            }
        };

        match EmbeddedEditor::embed(plugin.clone(), handle.as_raw(), editor_rect) {
            Ok(editor) => {
                self.editor = Some(editor);
                self.state = EditorState::Embedded;
                match (self.input_index, self.output_index) {
                    (Some(input_index), Some(output_index)) => {
                        let config = AudioConfig {
                            backend: self.backend,
                            input_index,
                            output_index,
                            routing: ChannelRouting {
                                input_left: self.input_left_channel.unwrap(),
                                input_right: self.input_right_channel.unwrap(),
                                output_left: self.output_left_channel.unwrap(),
                                output_right: self.output_right_channel.unwrap(),
                            },
                        };
                        match AudioEngine::start(plugin.clone(), config) {
                            Ok(engine) => self.engine = Some(engine),
                            Err(error) => {
                                self.state = EditorState::AudioUnavailable;
                                self.error = Some(format!(
                                    "Editor loaded, but audio is unavailable: {error}"
                                ));
                            }
                        }
                    }
                    _ => {
                        self.state = EditorState::AudioUnavailable;
                        self.error =
                            Some("Editor loaded, but no audio devices are selected.".to_string());
                    }
                }
                service_run_loop(&plugin);
            }
            Err(error) => {
                self.state = EditorState::Error;
                self.error = Some(format!("Could not embed plugin editor: {error}"));
            }
        }
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
