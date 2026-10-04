use super::audio::{AudioConfig, AudioDevices, AudioEngine, ChannelRouting, ShutdownMode};
use super::plugin::{PLUGIN_PATHS, load_stack, service_run_loop};
use super::settings::{AudioSettings, Settings};
use super::windows::AudioMonitor;
use std::sync::{Arc, Mutex};
use vst3_host::{Parameter, Plugin, PluginWindow};

struct RackParameter {
    parameter: Parameter,
    formatted_value: String,
}

#[derive(Debug, Clone)]
enum EditorState {
    Unloaded,
    Loaded,
    AudioUnavailable,
    Error,
}

pub struct PluginEditor {
    plugins: Vec<Arc<Mutex<Plugin>>>,
    engine: Option<AudioEngine>,
    active_audio_config: Option<AudioConfig>,
    editor_windows: Vec<PluginWindow>,
    plugin_names: Vec<String>,
    plugin_has_editors: Vec<bool>,
    plugin_parameters: Vec<Vec<RackParameter>>,
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
            editor_windows: Vec::new(),
            plugin_names: Vec::new(),
            plugin_has_editors: Vec::new(),
            plugin_parameters: Vec::new(),
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

    pub fn ui(&mut self, ui: &mut egui::Ui) {
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
        self.show_plugin_rack(ui);
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
                } else {
                    self.start_audio();
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
        if !self.editor_windows.is_empty() {
            if matches!(mode, ShutdownMode::KeepPluginsLoaded) {
                log::debug!(
                    "leaving {} plugin editor(s) for process teardown",
                    self.editor_windows.len()
                );
                for window in &mut self.editor_windows {
                    window.close();
                }
                std::mem::forget(std::mem::take(&mut self.editor_windows));
            } else {
                log::debug!("detaching {} plugin editor(s)", self.editor_windows.len());
                self.editor_windows.clear();
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
        self.plugin_names.clear();
        self.plugin_has_editors.clear();
        self.plugin_parameters.clear();
        self.state = EditorState::Unloaded;
    }

    fn install_plugins(&mut self, plugins: Vec<Arc<Mutex<Plugin>>>) -> Result<(), String> {
        let mut names = Vec::with_capacity(plugins.len());
        let mut has_editors = Vec::with_capacity(plugins.len());
        let mut parameters = Vec::with_capacity(plugins.len());
        for (index, plugin) in plugins.iter().enumerate() {
            let plugin_guard = plugin
                .lock()
                .map_err(|_| format!("plugin {index} lock poisoned"))?;
            names.push(plugin_guard.info().name.clone());
            has_editors.push(plugin_guard.has_editor());
            let plugin_parameters = read_rack_parameters(&plugin_guard)
                .map_err(|error| format!("Could not read plugin {index} parameters: {error}"))?;
            parameters.push(plugin_parameters);
        }
        self.editor_windows = plugins
            .iter()
            .map(|plugin| PluginWindow::new(plugin.clone()))
            .collect();
        self.plugins = plugins;
        self.plugin_names = names;
        self.plugin_has_editors = has_editors;
        self.plugin_parameters = parameters;
        self.state = EditorState::Loaded;
        Ok(())
    }

    fn show_plugin_rack(&mut self, ui: &mut egui::Ui) {
        if self.plugins.is_empty() {
            ui.label("Load the stack to show plugin controls.");
            return;
        }
        self.service_editor_windows();
        self.refresh_parameters();

        let mut edits = Vec::new();
        egui::ScrollArea::vertical()
            .id_salt("plugin-rack-scroll")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for index in 0..self.plugins.len() {
                    self.show_plugin_container(ui, index, &mut edits);
                }
            });
        for (plugin_index, parameter_id, value) in edits {
            self.set_parameter(plugin_index, parameter_id, value);
        }
    }

    fn show_plugin_container(
        &mut self,
        ui: &mut egui::Ui,
        plugin_index: usize,
        edits: &mut Vec<(usize, u32, f64)>,
    ) {
        let plugin_name = self.plugin_names[plugin_index].clone();
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.strong(plugin_name);
                ui.label(format!(
                    "{} parameters",
                    self.plugin_parameters[plugin_index].len()
                ));
                if self.plugin_has_editors[plugin_index] {
                    if ui.button("Open native editor").clicked() {
                        self.open_native_editor(plugin_index);
                    }
                } else {
                    ui.label("No native editor");
                }
            });

            if self.plugin_parameters[plugin_index].is_empty() {
                ui.label("This plugin does not expose any parameters.");
                return;
            }

            for (group_index, (unit, parameter_indices)) in
                group_parameter_indices(&self.plugin_parameters[plugin_index])
                    .into_iter()
                    .enumerate()
            {
                ui.add_space(4.0);
                ui.strong(unit);
                egui::Grid::new(("plugin-parameters", plugin_index, group_index))
                    .striped(true)
                    .num_columns(3)
                    .show(ui, |ui| {
                        ui.label("Parameter");
                        ui.label("Control");
                        ui.label("Value");
                        ui.end_row();

                        for parameter_index in parameter_indices {
                            let rack_parameter =
                                &mut self.plugin_parameters[plugin_index][parameter_index];
                            let parameter = &mut rack_parameter.parameter;
                            ui.label(&parameter.name);

                            if is_booleanish_parameter(parameter, &rack_parameter.formatted_value) {
                                let mut checked = boolean_parameter_value(
                                    parameter,
                                    &rack_parameter.formatted_value,
                                );
                                let response = ui
                                    .add_enabled_ui(!parameter.is_read_only, |ui| {
                                        ui.toggle_value(&mut checked, "")
                                    })
                                    .inner;
                                if response.changed() {
                                    parameter.value = if checked {
                                        parameter.max
                                    } else {
                                        parameter.min
                                    };
                                    rack_parameter.formatted_value =
                                        parameter.format_value(parameter.value);
                                    edits.push((plugin_index, parameter.id, parameter.value));
                                }
                            } else {
                                let mut value = parameter.value;
                                let mut slider =
                                    egui::Slider::new(&mut value, parameter.min..=parameter.max)
                                        .show_value(false);
                                if parameter.is_discrete() {
                                    let steps = f64::from(parameter.step_count) + 1.0;
                                    slider =
                                        slider.step_by((parameter.max - parameter.min) / steps);
                                }
                                let response = ui.add_enabled(!parameter.is_read_only, slider);
                                if response.changed() {
                                    parameter.value = value;
                                    rack_parameter.formatted_value = parameter.format_value(value);
                                    edits.push((plugin_index, parameter.id, value));
                                }
                            }
                            ui.label(&rack_parameter.formatted_value);
                            ui.end_row();
                        }
                    });
            }
        });
        ui.add_space(8.0);
    }

    fn open_native_editor(&mut self, plugin_index: usize) {
        if let Err(error) = self.editor_windows[plugin_index].open() {
            self.state = EditorState::Error;
            self.error = Some(format!(
                "Could not open {} editor: {error}",
                self.plugin_names[plugin_index]
            ));
        }
    }

    fn refresh_parameters(&mut self) {
        for plugin_index in 0..self.plugins.len() {
            let plugin = match self.plugins[plugin_index].try_lock() {
                Ok(plugin) => plugin,
                Err(std::sync::TryLockError::Poisoned(poison)) => poison.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => continue,
            };
            let parameters = match read_rack_parameters(&plugin) {
                Ok(parameters) => parameters,
                Err(error) => {
                    self.error = Some(format!(
                        "Could not read {} parameters: {error}",
                        self.plugin_names[plugin_index]
                    ));
                    continue;
                }
            };
            self.plugin_parameters[plugin_index] = parameters;
        }
    }

    fn set_parameter(&mut self, plugin_index: usize, parameter_id: u32, value: f64) {
        let result = self.plugins[plugin_index]
            .lock()
            .map_err(|_| "plugin lock poisoned".to_string())
            .and_then(|mut plugin| {
                plugin
                    .set_parameter(parameter_id, value)
                    .map_err(|error| error.to_string())
            });
        if let Err(error) = result {
            self.state = EditorState::Error;
            self.error = Some(format!(
                "Could not set {} parameter {parameter_id}: {error}",
                self.plugin_names[plugin_index]
            ));
        }
    }

    fn service_editor_windows(&mut self) {
        for (index, window) in self.editor_windows.iter_mut().enumerate() {
            if let Err(error) = window.service_platform_events() {
                self.state = EditorState::Error;
                self.error = Some(format!(
                    "Could not service {} editor: {error}",
                    self.plugin_names[index]
                ));
            }
            if window.closed_by_user() {
                window.close();
            }
            service_run_loop(&self.plugins[index]);
        }
    }

    fn start_audio(&mut self) {
        if self.engine.is_some() {
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

    pub(crate) fn audio_ui(&mut self, ui: &mut egui::Ui) {
        let Some(engine) = self.engine.as_mut() else {
            self.monitor.idle_ui(ui);
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
            Self::Loaded => "loaded",
            Self::AudioUnavailable => "loaded (audio unavailable)",
            Self::Error => "error",
        }
    }
}

fn read_rack_parameters(plugin: &Plugin) -> Result<Vec<RackParameter>, String> {
    plugin
        .get_parameters()
        .map_err(|error| error.to_string())
        .map(|parameters| {
            parameters
                .into_iter()
                .filter(|parameter| !is_midi_parameter(parameter))
                .map(|parameter| {
                    let formatted_value = plugin
                        .format_parameter(parameter.id, parameter.value)
                        .unwrap_or_else(|_| parameter.format_value(parameter.value));
                    RackParameter {
                        parameter,
                        formatted_value,
                    }
                })
                .collect()
        })
}

fn is_midi_parameter(parameter: &Parameter) -> bool {
    parameter
        .name
        .split_whitespace()
        .next()
        .is_some_and(|word| word.eq_ignore_ascii_case("midi"))
}

fn is_booleanish_parameter(parameter: &Parameter, formatted_value: &str) -> bool {
    parameter.step_count == 1
        || parameter.is_bypass
        || boolean_display_value(formatted_value).is_some()
        || parameter
            .name
            .split(|character: char| !character.is_alphanumeric())
            .any(|word| {
                matches!(
                    word.to_ascii_lowercase().as_str(),
                    "active"
                        | "bypass"
                        | "disable"
                        | "disabled"
                        | "enable"
                        | "enabled"
                        | "mute"
                        | "on"
                        | "off"
                        | "solo"
                )
            })
}

fn boolean_parameter_value(parameter: &Parameter, formatted_value: &str) -> bool {
    boolean_display_value(formatted_value).unwrap_or_else(|| {
        let midpoint = (parameter.min + parameter.max) / 2.0;
        parameter.value >= midpoint
    })
}

fn boolean_display_value(formatted_value: &str) -> Option<bool> {
    match formatted_value.trim().to_ascii_lowercase().as_str() {
        "active" | "enabled" | "on" | "true" | "yes" => Some(true),
        "disabled" | "inactive" | "no" | "off" | "false" => Some(false),
        _ => None,
    }
}

fn group_parameter_indices(parameters: &[RackParameter]) -> Vec<(String, Vec<usize>)> {
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();

    for (index, parameter) in parameters.iter().enumerate() {
        let category = if parameter.parameter.unit.trim().is_empty() {
            "General".to_string()
        } else {
            parameter.parameter.unit.trim().to_string()
        };

        if let Some((_, indices)) = groups
            .iter_mut()
            .find(|(existing_category, _)| existing_category == &category)
        {
            indices.push(index);
        } else {
            groups.push((category, vec![index]));
        }
    }

    groups
}
