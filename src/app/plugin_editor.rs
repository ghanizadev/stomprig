use super::audio::{
    AudioBackend, AudioBufferSize, AudioConfig, AudioDevices, AudioEngine, ChannelRouting,
    DeviceInfo,
};
use super::plugin::{PLUGIN_PATHS, load_stack, service_run_loop};
use super::settings::AudioSettings;
use super::windows::{AudioConfigurationWindow, AudioMonitor};
use eframe::Frame;
use egui_shadcn::*;
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

#[derive(Clone, Copy)]
struct PendingChannelRouting {
    input_left: Option<usize>,
    input_right: Option<usize>,
    output_left: Option<usize>,
    output_right: Option<usize>,
}

pub struct PluginEditor {
    theme: Theme,
    plugins: Vec<Arc<Mutex<Plugin>>>,
    engine: Option<AudioEngine>,
    active_audio_config: Option<AudioConfig>,
    editors: Vec<EmbeddedEditor>,
    sizes: Vec<egui::Vec2>,
    error: Option<String>,
    monitor: AudioMonitor,
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
    buffer_size: AudioBufferSize,
    audio_config_open: bool,
    preferred_input_name: Option<String>,
    preferred_output_name: Option<String>,
    pending_channel_routing: Option<PendingChannelRouting>,
    input_channel_items: Vec<SelectItem>,
    output_channel_items: Vec<SelectItem>,
}

impl Default for PluginEditor {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            plugins: Vec::new(),
            engine: None,
            active_audio_config: None,
            editors: Vec::new(),
            sizes: Vec::new(),
            error: None,
            monitor: AudioMonitor::default(),
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
            buffer_size: AudioBufferSize::default(),
            audio_config_open: false,
            preferred_input_name: None,
            preferred_output_name: None,
            pending_channel_routing: None,
            input_channel_items: Vec::new(),
            output_channel_items: Vec::new(),
        }
    }
}

impl PluginEditor {
    pub fn from_settings(settings: &AudioSettings) -> Self {
        let mut editor = Self::default();
        editor.backend = settings.backend;
        editor.buffer_size = settings.buffer_size;
        editor.input_left_channel = settings.input_left_channel;
        editor.input_right_channel = settings.input_right_channel;
        editor.output_left_channel = settings.output_left_channel;
        editor.output_right_channel = settings.output_right_channel;
        editor.preferred_input_name = settings.input_device.clone();
        editor.preferred_output_name = settings.output_device.clone();
        editor.pending_channel_routing = Some(PendingChannelRouting {
            input_left: settings.input_left_channel,
            input_right: settings.input_right_channel,
            output_left: settings.output_left_channel,
            output_right: settings.output_right_channel,
        });
        editor
    }

    pub fn audio_settings(&self) -> AudioSettings {
        AudioSettings {
            backend: self.backend,
            buffer_size: self.buffer_size,
            input_device: self.input_index.and_then(|index| {
                self.input_devices
                    .iter()
                    .find(|device| device.index == index)
                    .map(|device| device.name.clone())
            }),
            output_device: self.output_index.and_then(|index| {
                self.output_devices
                    .iter()
                    .find(|device| device.index == index)
                    .map(|device| device.name.clone())
            }),
            input_left_channel: self.input_left_channel,
            input_right_channel: self.input_right_channel,
            output_left_channel: self.output_left_channel,
            output_right_channel: self.output_right_channel,
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, frame: &mut Frame) {
        if self.input_devices.is_empty() && self.output_devices.is_empty() {
            self.refresh_devices();
        }
        ui.heading("VST3 Rack");
        ui.horizontal(|ui| {
            ui.label(format!("Plugin stack ({} entries)", PLUGIN_PATHS.len()));
            if ui.button("Load stack").clicked() {
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
        self.shutdown();
        self.error = None;
        self.state = EditorState::Unloaded;

        let plugin_result = match self
            .output_index
            .and_then(|index| AudioDevices::sample_rate(self.backend, index).ok())
        {
            Some(sample_rate) => load_stack(Some(sample_rate), self.buffer_size.frames() as usize),
            None => load_stack(None, self.buffer_size.frames() as usize),
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

    fn refresh_devices(&mut self) {
        let old_input = self
            .input_index
            .and_then(|index| {
                self.input_devices
                    .iter()
                    .find(|device| device.index == index)
            })
            .map(|device| device.name.clone());
        let old_input = old_input.or_else(|| self.preferred_input_name.clone());
        let old_output = self
            .output_index
            .and_then(|index| {
                self.output_devices
                    .iter()
                    .find(|device| device.index == index)
            })
            .map(|device| device.name.clone());
        let old_output = old_output.or_else(|| self.preferred_output_name.clone());

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
        self.restore_pending_channel_routing();
        self.normalize_channel_selections();
        self.rebuild_channel_items();
    }

    fn rebuild_channel_items(&mut self) {
        self.input_channel_items = self
            .input_index
            .and_then(|index| {
                self.input_devices
                    .iter()
                    .find(|device| device.index == index)
            })
            .into_iter()
            .flat_map(|device| {
                (0..device.channels).map(|channel| {
                    SelectItem::option(channel.to_string(), format!("Channel {}", channel + 1))
                })
            })
            .collect();

        self.output_channel_items = self
            .output_index
            .and_then(|index| {
                self.output_devices
                    .iter()
                    .find(|device| device.index == index)
            })
            .into_iter()
            .flat_map(|device| {
                (0..device.channels).map(|channel| {
                    SelectItem::option(channel.to_string(), format!("Channel {}", channel + 1))
                })
            })
            .collect();
    }

    fn restore_pending_channel_routing(&mut self) {
        let Some(routing) = self.pending_channel_routing.take() else {
            return;
        };
        if routing.input_left.is_some() {
            self.input_left_channel = routing.input_left;
        }
        if routing.input_right.is_some() {
            self.input_right_channel = routing.input_right;
        }
        if routing.output_left.is_some() {
            self.output_left_channel = routing.output_left;
        }
        if routing.output_right.is_some() {
            self.output_right_channel = routing.output_right;
        }
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

            let (mut selected_input_left_channel, mut selected_input_right_channel) = (
                self.input_left_channel.map(|channel| channel.to_string()),
                self.input_right_channel.map(|channel| channel.to_string()),
            );

            let (mut selected_output_left_channel, mut selected_output_right_channel) = (
                self.output_left_channel.map(|channel| channel.to_string()),
                self.output_right_channel.map(|channel| channel.to_string()),
            );

            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label("Input left");
                    let select = select_with_items(
                        ui,
                        &self.theme,
                        SelectProps::new("input-left-channel", &mut selected_input_left_channel)
                            .placeholder("Unavailable")
                            .width(220.0),
                        &self.input_channel_items,
                    );

                    if select.changed() {
                        self.input_left_channel = selected_input_left_channel
                            .and_then(|channel| channel.parse::<usize>().ok());
                    }
                });

                ui.vertical(|ui| {
                    ui.label("Input right");
                    let select = select_with_items(
                        ui,
                        &self.theme,
                        SelectProps::new("input-right-channel", &mut selected_input_right_channel)
                            .placeholder("Unavailable")
                            .width(220.0),
                        &self.input_channel_items,
                    );

                    if select.changed() {
                        self.input_right_channel = selected_input_right_channel
                            .and_then(|channel| channel.parse::<usize>().ok());
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label("Output left");
                    let select = select_with_items(
                        ui,
                        &self.theme,
                        SelectProps::new("output-left-channel", &mut selected_output_left_channel)
                            .placeholder("Unavailable")
                            .width(220.0),
                        &self.output_channel_items,
                    );

                    if select.changed() {
                        self.output_left_channel = selected_output_left_channel
                            .and_then(|channel| channel.parse::<usize>().ok());
                    }
                });

                ui.vertical(|ui| {
                    ui.label("Output right");
                    let select = select_with_items(
                        ui,
                        &self.theme,
                        SelectProps::new(
                            "output-right-channel",
                            &mut selected_output_right_channel,
                        )
                        .placeholder("Unavailable")
                        .width(220.0),
                        &self.output_channel_items,
                    );

                    if select.changed() {
                        self.output_right_channel = selected_output_right_channel
                            .and_then(|channel| channel.parse::<usize>().ok());
                    }
                });
            });

            let btn = Button::new("Apply channel routing")
                .variant(ButtonVariant::Default)
                .size(ButtonSize::Default)
                .show(ui, &self.theme);

            if btn.clicked() {
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
        let previous_input_index = self.input_index;
        let previous_output_index = self.output_index;
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

            ui.add_space(8.0);
            ui.label("Audio buffer size");
            let sample_rate = self
                .output_index
                .and_then(|index| AudioDevices::sample_rate(self.backend, index).ok());
            egui::ComboBox::from_id_salt("audio-buffer-size")
                .selected_text(self.buffer_size.label())
                .width(300.0)
                .show_ui(ui, |ui| {
                    for size in AudioBufferSize::ALL {
                        ui.selectable_value(&mut self.buffer_size, size, size.label());
                    }
                });
            if let Some(sample_rate) = sample_rate {
                ui.label(format!(
                    "Estimated round-trip latency: {:.2} ms",
                    self.estimated_latency_ms(sample_rate)
                ));
            } else {
                ui.label("Estimated round-trip latency: unavailable");
            }

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
            self.preferred_input_name = None;
            self.preferred_output_name = None;
            self.pending_channel_routing = None;
            self.input_devices.clear();
            self.output_devices.clear();
            self.input_index = None;
            self.output_index = None;
            self.input_left_channel = None;
            self.input_right_channel = None;
            self.output_left_channel = None;
            self.output_right_channel = None;
            self.refresh_devices();
        } else if self.input_index != previous_input_index
            || self.output_index != previous_output_index
        {
            self.normalize_channel_selections();
            self.rebuild_channel_items();
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
        if self.plugins.is_empty() {
            self.load();
        } else {
            self.restart_audio();
        }
    }

    pub fn shutdown(&mut self) {
        if let Some(engine) = self.engine.take() {
            engine.shutdown();
        }
        self.active_audio_config = None;
        self.editors.clear();
        self.plugins.clear();
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
            if let Some(editor) = self.editors.get_mut(index) {
                if let Some((width, height)) = editor.take_resize_request() {
                    self.sizes[index] = egui::vec2(width as f32, height as f32);
                }
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
            engine.shutdown();
        }
        self.active_audio_config = None;
        self.start_audio();
    }

    fn selected_audio_config(&self) -> Option<AudioConfig> {
        Some(AudioConfig {
            backend: self.backend,
            input_index: self.input_index?,
            output_index: self.output_index?,
            buffer_size: self.buffer_size,
            routing: ChannelRouting {
                input_left: self.input_left_channel?,
                input_right: self.input_right_channel?,
                output_left: self.output_left_channel?,
                output_right: self.output_right_channel?,
            },
        })
    }

    fn estimated_latency_ms(&self, sample_rate: f64) -> f64 {
        let buffer_latency = 2.0 * f64::from(self.buffer_size.frames()) / sample_rate * 1000.0;
        let plugin_latency_samples = self
            .plugins
            .iter()
            .filter_map(|plugin| plugin.lock().ok())
            .map(|plugin| f64::from(plugin.latency_samples()))
            .sum::<f64>();
        buffer_latency + plugin_latency_samples / sample_rate * 1000.0
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
        self.shutdown();
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
