use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;

use super::audio::{AudioBackend, AudioBufferSize, AudioDevices};
use super::windows::AudioConfigurationWindow;
use egui_shadcn::Theme;
use egui_shadcn::*;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioSettings {
    pub backend: AudioBackend,
    #[serde(default)]
    pub buffer_size: AudioBufferSize,
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub input_left_channel: Option<usize>,
    pub input_right_channel: Option<usize>,
    pub output_left_channel: Option<usize>,
    pub output_right_channel: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct AppSettings {
    pub audio: AudioSettings,
}

pub struct SettingsStore {
    pub values: AppSettings,
    path: PathBuf,
    last_saved: AppSettings,
    pub error: Option<String>,
}

#[derive(Clone, Copy)]
struct PendingChannelRouting {
    input_left: Option<usize>,
    input_right: Option<usize>,
    output_left: Option<usize>,
    output_right: Option<usize>,
}

pub struct Settings {
    theme: Theme,
    pub(crate) input_devices: Vec<super::audio::DeviceInfo>,
    pub(crate) output_devices: Vec<super::audio::DeviceInfo>,
    pub(crate) input_index: Option<usize>,
    pub(crate) output_index: Option<usize>,
    pub(crate) input_left_channel: Option<usize>,
    pub(crate) input_right_channel: Option<usize>,
    pub(crate) output_left_channel: Option<usize>,
    pub(crate) output_right_channel: Option<usize>,
    pub(crate) backend: AudioBackend,
    pub(crate) buffer_size: AudioBufferSize,
    audio_config_open: bool,
    preferred_input_name: Option<String>,
    preferred_output_name: Option<String>,
    pending_channel_routing: Option<PendingChannelRouting>,
    input_channel_items: Vec<SelectItem>,
    output_channel_items: Vec<SelectItem>,
    settings_tab: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
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
            settings_tab: "general".to_owned(),
        }
    }
}

impl Settings {
    pub fn from_audio_settings(values: &AudioSettings) -> Self {
        let mut settings = Self::default();
        settings.backend = values.backend;
        settings.buffer_size = values.buffer_size;
        settings.input_left_channel = values.input_left_channel;
        settings.input_right_channel = values.input_right_channel;
        settings.output_left_channel = values.output_left_channel;
        settings.output_right_channel = values.output_right_channel;
        settings.preferred_input_name = values.input_device.clone();
        settings.preferred_output_name = values.output_device.clone();
        settings.pending_channel_routing = Some(PendingChannelRouting {
            input_left: values.input_left_channel,
            input_right: values.input_right_channel,
            output_left: values.output_left_channel,
            output_right: values.output_right_channel,
        });
        settings
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

    pub fn open_audio_configuration(&mut self) {
        self.audio_config_open = true;
    }

    pub fn refresh_devices(&mut self, error: &mut Option<String>) {
        let old_input = self
            .input_index
            .and_then(|index| {
                self.input_devices
                    .iter()
                    .find(|device| device.index == index)
            })
            .map(|device| device.name.clone())
            .or_else(|| self.preferred_input_name.clone());
        let old_output = self
            .output_index
            .and_then(|index| {
                self.output_devices
                    .iter()
                    .find(|device| device.index == index)
            })
            .map(|device| device.name.clone())
            .or_else(|| self.preferred_output_name.clone());
        match AudioDevices::input_devices(self.backend) {
            Ok(devices) => self.input_devices = devices,
            Err(message) => *error = Some(message),
        }
        match AudioDevices::output_devices(self.backend) {
            Ok(devices) => self.output_devices = devices,
            Err(message) => *error = Some(message),
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

    pub fn show_configuration(
        &mut self,
        ui: &mut egui::Ui,
        error: &mut Option<String>,
        plugin_latency_samples: f64,
    ) -> bool {
        if self.input_devices.is_empty() && self.output_devices.is_empty() {
            self.refresh_devices(error);
        }
        let mut apply = false;
        let mut open = self.audio_config_open;
        AudioConfigurationWindow::show(ui.ctx(), &mut open, |ui| {
            self.show_device_controls(ui, error, plugin_latency_samples, &mut apply)
        });
        self.audio_config_open = open;
        apply
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
        let input_channels = self
            .input_index
            .and_then(|index| {
                self.input_devices
                    .iter()
                    .find(|device| device.index == index)
            })
            .map(|device| device.channels);
        match input_channels {
            Some(channels) if channels > 0 => {
                self.input_left_channel =
                    Some(self.input_left_channel.unwrap_or(0).min(channels - 1));
                self.input_right_channel = Some(
                    self.input_right_channel
                        .unwrap_or((channels > 1) as usize)
                        .min(channels - 1),
                );
            }
            _ => {
                self.input_left_channel = None;
                self.input_right_channel = None;
            }
        }
        let output_channels = self
            .output_index
            .and_then(|index| {
                self.output_devices
                    .iter()
                    .find(|device| device.index == index)
            })
            .map(|device| device.channels);
        match output_channels {
            Some(channels) if channels > 0 => {
                self.output_left_channel =
                    Some(self.output_left_channel.unwrap_or(0).min(channels - 1));
                self.output_right_channel = Some(
                    self.output_right_channel
                        .unwrap_or((channels > 1) as usize)
                        .min(channels - 1),
                );
            }
            _ => {
                self.output_left_channel = None;
                self.output_right_channel = None;
            }
        }
    }

    fn show_device_controls(
        &mut self,
        ui: &mut egui::Ui,
        error: &mut Option<String>,
        plugin_latency_samples: f64,
        apply: &mut bool,
    ) {
        let mut backend_changed = false;
        let previous_input_index = self.input_index;
        let previous_output_index = self.output_index;
        let items = [
            TabItem::new("general", "General"),
            TabItem::new("audio", "Audio"),
        ];
        let mut active_tab = self.settings_tab.clone();
        let theme = self.theme.clone();
        ui.vertical(|content| {
            let _ = tabs(
                content,
                &theme,
                TabsProps::new(egui::Id::new("preview-tabs"), &items, &mut active_tab)
                    .variant(TabsVariant::Soft)
                    .scrollable(true),
                |tab_content, active| {
                    if active.id == "general" {
                        let _ = text(tab_content, &self.theme, TextProps::new("General details"));
                    } else {
                        self.show_audio_settings(
                            tab_content,
                            error,
                            plugin_latency_samples,
                            &mut backend_changed,
                            apply,
                        );
                    }
                },
            );
        });
        self.settings_tab = active_tab;
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
            self.refresh_devices(error);
        } else if self.input_index != previous_input_index
            || self.output_index != previous_output_index
        {
            self.normalize_channel_selections();
            self.rebuild_channel_items();
        }
    }

    fn show_audio_settings(
        &mut self,
        ui: &mut egui::Ui,
        error: &mut Option<String>,
        plugin_latency_samples: f64,
        backend_changed: &mut bool,
        apply: &mut bool,
    ) {
        ui.vertical(|ui| {
            ui.label("Audio backend");
            let mut selected_backend = Some(self.backend.label().to_owned());
            let backend_items: Vec<SelectItem> = AudioDevices::available_backends()
                .into_iter()
                .map(|backend| SelectItem::option(backend.label(), backend.label()))
                .collect();
            if select_with_items(
                ui,
                &self.theme,
                SelectProps::new("audio-backend", &mut selected_backend)
                    .placeholder("Unavailable")
                    .width(300.0),
                &backend_items,
            )
            .changed()
                && let Some(selected) = selected_backend.as_deref()
                && let Some(backend) = AudioDevices::available_backends()
                    .into_iter()
                    .find(|backend| backend.label() == selected)
            {
                self.backend = backend;
                *backend_changed = true;
            }
            ui.add_space(8.0);
            ui.label("Input device");
            let mut selected_input = self.input_index.map(|index| index.to_string());
            let input_items: Vec<SelectItem> = self
                .input_devices
                .iter()
                .map(|device| SelectItem::option(device.index.to_string(), device.name.clone()))
                .collect();
            if select_with_items(
                ui,
                &self.theme,
                SelectProps::new("input-device", &mut selected_input)
                    .placeholder("Unavailable")
                    .width(300.0),
                &input_items,
            )
            .changed()
            {
                self.input_index = selected_input.and_then(|index| index.parse().ok());
            }
            ui.add_space(8.0);
            ui.label("Output device");
            let mut selected_output = self.output_index.map(|index| index.to_string());
            let output_items: Vec<SelectItem> = self
                .output_devices
                .iter()
                .map(|device| SelectItem::option(device.index.to_string(), device.name.clone()))
                .collect();
            if select_with_items(
                ui,
                &self.theme,
                SelectProps::new("output-device", &mut selected_output)
                    .placeholder("Unavailable")
                    .width(300.0),
                &output_items,
            )
            .changed()
            {
                self.output_index = selected_output.and_then(|index| index.parse().ok());
            }
            ui.add_space(8.0);
            ui.label("Audio buffer size");
            let sample_rate = self
                .output_index
                .and_then(|index| AudioDevices::sample_rate(self.backend, index).ok());
            let mut selected_buffer_size = Some(self.buffer_size.label().to_owned());
            let buffer_size_items: Vec<SelectItem> = AudioBufferSize::ALL
                .into_iter()
                .map(|size| SelectItem::option(size.label(), size.label()))
                .collect();
            if select_with_items(
                ui,
                &self.theme,
                SelectProps::new("audio-buffer-size", &mut selected_buffer_size)
                    .placeholder("Unavailable")
                    .width(300.0),
                &buffer_size_items,
            )
            .changed()
                && let Some(selected) = selected_buffer_size.as_deref()
                && let Some(size) = AudioBufferSize::ALL
                    .into_iter()
                    .find(|size| size.label() == selected)
            {
                self.buffer_size = size;
            }
            if let Some(sample_rate) = sample_rate {
                ui.label(format!(
                    "Estimated round-trip latency: {:.2} ms",
                    2.0 * f64::from(self.buffer_size.frames()) / sample_rate * 1000.0
                        + plugin_latency_samples / sample_rate * 1000.0
                ));
            } else {
                ui.label("Estimated round-trip latency: unavailable");
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if Button::new("Refresh devices")
                    .variant(ButtonVariant::Outline)
                    .size(ButtonSize::Default)
                    .show(ui, &self.theme)
                    .clicked()
                {
                    self.refresh_devices(error);
                }
                if Button::new("Apply audio devices")
                    .variant(ButtonVariant::Default)
                    .size(ButtonSize::Default)
                    .show(ui, &self.theme)
                    .clicked()
                {
                    self.normalize_channel_selections();
                    if self.input_index.is_none()
                        || self.output_index.is_none()
                        || self.input_left_channel.is_none()
                        || self.input_right_channel.is_none()
                        || self.output_left_channel.is_none()
                        || self.output_right_channel.is_none()
                    {
                        *error = Some("Select input and output devices and channels.".to_string());
                    } else {
                        *apply = true;
                    }
                }
            });
            ui.add_space(16.0);
            self.show_channel_routing(ui, apply);
        });
    }

    fn show_channel_routing(&mut self, ui: &mut egui::Ui, apply: &mut bool) {
        self.normalize_channel_selections();
        ui.group(|ui| {
            ui.label("Track channel routing");
            let mut input_left = self.input_left_channel.map(|v| v.to_string());
            let mut input_right = self.input_right_channel.map(|v| v.to_string());
            let mut output_left = self.output_left_channel.map(|v| v.to_string());
            let mut output_right = self.output_right_channel.map(|v| v.to_string());
            let select = |ui: &mut egui::Ui,
                          id: &str,
                          value: &mut Option<String>,
                          items: &Vec<SelectItem>| {
                select_with_items(
                    ui,
                    &self.theme,
                    SelectProps::new(id, value)
                        .placeholder("Unavailable")
                        .width(220.0),
                    items,
                )
                .changed()
            };
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label("Input left");
                    if select(
                        ui,
                        "input-left-channel",
                        &mut input_left,
                        &self.input_channel_items,
                    ) {
                        self.input_left_channel =
                            input_left.as_deref().and_then(|v| v.parse().ok());
                    }
                });
                ui.vertical(|ui| {
                    ui.label("Input right");
                    if select(
                        ui,
                        "input-right-channel",
                        &mut input_right,
                        &self.input_channel_items,
                    ) {
                        self.input_right_channel =
                            input_right.as_deref().and_then(|v| v.parse().ok());
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label("Output left");
                    if select(
                        ui,
                        "output-left-channel",
                        &mut output_left,
                        &self.output_channel_items,
                    ) {
                        self.output_left_channel =
                            output_left.as_deref().and_then(|v| v.parse().ok());
                    }
                });
                ui.vertical(|ui| {
                    ui.label("Output right");
                    if select(
                        ui,
                        "output-right-channel",
                        &mut output_right,
                        &self.output_channel_items,
                    ) {
                        self.output_right_channel =
                            output_right.as_deref().and_then(|v| v.parse().ok());
                    }
                });
            });
            if Button::new("Apply channel routing")
                .variant(ButtonVariant::Default)
                .size(ButtonSize::Default)
                .show(ui, &self.theme)
                .clicked()
            {
                self.normalize_channel_selections();
                *apply = if self.input_index.is_none()
                    || self.output_index.is_none()
                    || self.input_left_channel.is_none()
                    || self.input_right_channel.is_none()
                    || self.output_left_channel.is_none()
                    || self.output_right_channel.is_none()
                {
                    false
                } else {
                    true
                }
            }
        });
    }
}

impl SettingsStore {
    pub fn load() -> Self {
        let path = settings_path();
        match fs::read_to_string(&path) {
            Ok(contents) => match toml::from_str::<AppSettings>(&contents) {
                Ok(values) => Self {
                    last_saved: values.clone(),
                    values,
                    path,
                    error: None,
                },
                Err(error) => Self {
                    values: AppSettings::default(),
                    last_saved: AppSettings::default(),
                    path,
                    error: Some(format!("Could not read settings: {error}")),
                },
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self {
                values: AppSettings::default(),
                last_saved: AppSettings::default(),
                path,
                error: None,
            },
            Err(error) => Self {
                values: AppSettings::default(),
                last_saved: AppSettings::default(),
                path,
                error: Some(format!("Could not open settings: {error}")),
            },
        }
    }

    pub fn save_if_changed(&mut self, values: &AppSettings) {
        if *values == self.last_saved {
            return;
        }

        let result = (|| {
            let contents = toml::to_string_pretty(values)
                .map_err(|error| format!("Could not serialize settings: {error}"))?;
            if let Some(parent) = self.path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("Could not create settings directory: {error}"))?;
            }
            fs::write(&self.path, contents)
                .map_err(|error| format!("Could not save settings: {error}"))
        })();

        match result {
            Ok(()) => {
                self.last_saved = values.clone();
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
}

fn settings_path() -> PathBuf {
    #[cfg(windows)]
    let base = env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    #[cfg(target_os = "macos")]
    let base = env::var_os("HOME")
        .map(PathBuf::from)
        .map(|path| path.join("Library").join("Application Support"))
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    #[cfg(all(unix, not(target_os = "macos")))]
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|path| PathBuf::from(path).join(".config")))
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    base.join("rack").join("settings.toml")
}
