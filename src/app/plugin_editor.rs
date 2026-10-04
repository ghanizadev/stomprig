use super::audio::{AudioConfig, AudioDevices, AudioEngine, ChannelRouting, ShutdownMode};
use super::plugin::{load_stack, service_run_loop};
use super::settings::{AudioSettings, Settings};
use super::windows::AudioMonitor;
use egui_shadcn::{
    Button, ButtonJustify, ButtonVariant, Input, InputSize, SheetProps, SheetSide, sheet,
    sheet_content, sheet_description, sheet_footer, sheet_header, sheet_title, sheet_trigger,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use vst3_host::{Parameter, Plugin, PluginWindow, get_detailed_plugin_info};

const MAX_PLUGIN_PATHS: usize = 16;

#[derive(Clone, Debug)]
pub(super) struct DiscoveredPlugin {
    name: String,
    path: String,
    category: String,
}

fn default_plugin_paths() -> Vec<String> {
    let paths = Vec::with_capacity(MAX_PLUGIN_PATHS);
    // paths.extend([
    //     r"C:\Program Files\Common Files\VST3\InstFx.vst3".to_owned(),
    //     r"C:\Program Files\Common Files\VST3\Kilohearts\kHs Reverb.vst3".to_owned(),
    //     r"C:\Program Files\Common Files\VST3\Kilohearts\kHs Delay.vst3".to_owned(),
    // ]);
    paths
}

pub(super) fn discover_plugins_with_progress(
    mut report_progress: impl FnMut(f32),
) -> Vec<DiscoveredPlugin> {
    let mut roots = Vec::new();
    if let Some(program_files) = std::env::var_os("ProgramFiles") {
        roots.push(PathBuf::from(program_files).join("Common Files\\VST3"));
    }
    if let Some(program_files_x86) = std::env::var_os("ProgramFiles(x86)") {
        roots.push(PathBuf::from(program_files_x86).join("Common Files\\VST3"));
    }
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        let local_app_data = PathBuf::from(local_app_data);
        roots.push(local_app_data.join("Programs\\Common Files\\VST3"));
        roots.push(local_app_data.join("VST3"));
    }
    if let Some(app_data) = std::env::var_os("APPDATA") {
        roots.push(PathBuf::from(app_data).join("VST3"));
    }

    let mut plugins = HashMap::new();
    let root_count = roots.len().max(1);
    for (index, root) in roots.into_iter().enumerate() {
        discover_plugins_in(&root, &mut plugins);
        let progress = 20.0 + 70.0 * (index + 1) as f32 / root_count as f32;
        report_progress(progress);
    }

    let mut plugins: Vec<_> = plugins.into_values().collect();
    plugins.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            })
            .then_with(|| left.path.cmp(&right.path))
    });
    report_progress(95.0);
    plugins
}

fn discover_plugins_in(directory: &Path, plugins: &mut HashMap<String, DiscoveredPlugin>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("vst3"))
        {
            add_discovered_plugin(&path, plugins);
        } else if path.is_dir() {
            discover_plugins_in(&path, plugins);
        }
    }
}

fn add_discovered_plugin(path: &Path, plugins: &mut HashMap<String, DiscoveredPlugin>) {
    let metadata = match get_detailed_plugin_info(path) {
        Ok(metadata) => metadata.info,
        Err(error) => {
            log::warn!(
                "could not read VST3 metadata for {}: {error}",
                path.display()
            );
            return;
        }
    };

    let name = if metadata.name.trim().is_empty() {
        path.file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Unknown plugin".to_owned())
    } else {
        metadata.name
    };
    let category = if metadata.category.trim().is_empty() {
        "Uncategorized".to_owned()
    } else {
        metadata.category.trim().to_owned()
    };
    let path = path.to_string_lossy().into_owned();
    let identity = if metadata.uid.trim().is_empty() {
        path.to_ascii_lowercase()
    } else {
        metadata.uid.to_ascii_lowercase()
    };

    plugins.entry(identity).or_insert(DiscoveredPlugin {
        name,
        path,
        category,
    });
}

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
    plugin_paths: Vec<String>,
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
    plugin_picker_open: bool,
    plugin_picker_search: String,
    selected_plugin_path: Option<String>,
    discovered_plugins: Vec<DiscoveredPlugin>,
}

impl Default for PluginEditor {
    fn default() -> Self {
        Self {
            plugin_paths: default_plugin_paths(),
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
            plugin_picker_open: false,
            plugin_picker_search: String::new(),
            selected_plugin_path: None,
            discovered_plugins: Vec::new(),
        }
    }
}

impl PluginEditor {
    pub fn from_settings(settings: &AudioSettings) -> Self {
        let mut editor = Self::default();
        editor.settings = Settings::from_audio_settings(settings);
        editor
    }

    pub(super) fn set_discovered_plugins(&mut self, plugins: Vec<DiscoveredPlugin>) {
        self.discovered_plugins = plugins;
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

    pub fn push_plugin_path(&mut self, path: impl Into<String>) -> bool {
        if self.plugin_paths.len() >= MAX_PLUGIN_PATHS {
            return false;
        }
        self.plugin_paths.push(path.into());
        true
    }

    pub fn remove_plugin_path(&mut self, index: usize) -> Option<String> {
        (index < self.plugin_paths.len()).then(|| self.plugin_paths.remove(index))
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("VST3 Rack");
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
        self.show_add_plugin_button(ui);
    }

    fn show_add_plugin_button(&mut self, ui: &mut egui::Ui) {
        let mut open = self.plugin_picker_open;
        let mut search = self.plugin_picker_search.clone();
        let mut selected = self.selected_plugin_path.clone();
        let mut path_to_add = None;
        let theme = &self.settings.theme;
        let sheet_id = ui.make_persistent_id("plugin-picker-sheet");

        sheet(
            ui,
            SheetProps::new(sheet_id, &mut open).side(SheetSide::Bottom),
            |ui, context| {
                let trigger_width = ui.available_width();
                sheet_trigger(ui, context, |ui| {
                    Button::new("Add a plugin")
                        .variant(ButtonVariant::Default)
                        .size(egui_shadcn::ButtonSize::Default)
                        .min_width(trigger_width)
                        .show(ui, theme)
                })
                .clicked();

                let mut should_close = false;
                let _ = sheet_content(ui, theme, context, |content_ui| {
                    sheet_header(content_ui, |header_ui| {
                        sheet_title(header_ui, theme, "Add a plugin");
                        sheet_description(
                            header_ui,
                            theme,
                            "Choose a VST3 plugin from your system or user installation.",
                        );
                    });

                    let search_id = content_ui.make_persistent_id("plugin-picker-search");
                    Input::new(search_id)
                        .size(InputSize::Size2)
                        .width(content_ui.available_width())
                        .show(content_ui, theme, &mut search);

                    let query = search.trim().to_ascii_lowercase();
                    let mut categories: Vec<(&str, Vec<&DiscoveredPlugin>)> = Vec::new();
                    for plugin in &self.discovered_plugins {
                        if !query.is_empty()
                            && !plugin.name.to_ascii_lowercase().contains(&query)
                            && !plugin.path.to_ascii_lowercase().contains(&query)
                        {
                            continue;
                        }
                        if let Some((_, plugins)) = categories
                            .iter_mut()
                            .find(|(category, _)| *category == plugin.category.as_str())
                        {
                            plugins.push(plugin);
                        } else {
                            categories.push((plugin.category.as_str(), vec![plugin]));
                        }
                    }

                    let directory_width = content_ui.available_width();
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .max_width(directory_width)
                        .min_scrolled_width(directory_width)
                        .max_height(190.0)
                        .show(content_ui, |list_ui| {
                            if categories.is_empty() {
                                list_ui.label("No matching VST3 plugins found.");
                            }
                            for (category, plugins) in categories {
                                list_ui.add_space(4.0);
                                list_ui.strong(category);
                                for plugin in plugins {
                                    let is_selected =
                                        selected.as_deref() == Some(plugin.path.as_str());
                                    let variant = if is_selected {
                                        ButtonVariant::Default
                                    } else {
                                        ButtonVariant::Ghost
                                    };
                                    if Button::new(plugin.name.clone())
                                        .variant(variant)
                                        .justify(ButtonJustify::Start)
                                        .min_width(directory_width)
                                        .show(list_ui, theme)
                                        .clicked()
                                    {
                                        selected = Some(plugin.path.clone());
                                    }
                                }
                            }
                        });

                    sheet_footer(content_ui, |footer_ui| {
                        footer_ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |footer_ui| {
                                if Button::new("Add plugin")
                                    .variant(ButtonVariant::Default)
                                    .size(egui_shadcn::ButtonSize::Sm)
                                    .enabled(selected.is_some())
                                    .show(footer_ui, theme)
                                    .clicked()
                                {
                                    path_to_add = selected.clone();
                                    should_close = true;
                                }
                            },
                        );
                    });
                });
                if should_close {
                    *context.open = false;
                }
            },
        );

        if let Some(path) = path_to_add {
            if !self.plugin_paths.iter().any(|existing| existing == &path)
                && self.push_plugin_path(path)
            {
                self.load();
            }
            selected = None;
        }
        self.plugin_picker_open = open;
        self.plugin_picker_search = search;
        self.selected_plugin_path = selected;
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
                &self.plugin_paths,
                Some(sample_rate),
                self.settings.buffer_size.frames() as usize,
            ),
            None => load_stack(
                &self.plugin_paths,
                None,
                self.settings.buffer_size.frames() as usize,
            ),
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
        self.service_editor_windows();
        self.refresh_parameters();

        let mut edits = Vec::new();
        let mut remove_requested = None;
        egui::ScrollArea::vertical()
            .id_salt("plugin-rack-scroll")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for index in 0..self.plugins.len() {
                    self.show_plugin_container(ui, index, &mut edits, &mut remove_requested);
                }
            });
        for (plugin_index, parameter_id, value) in edits {
            self.set_parameter(plugin_index, parameter_id, value);
        }
        if let Some(plugin_index) = remove_requested {
            self.remove_plugin(plugin_index);
        }
    }

    fn show_plugin_container(
        &mut self,
        ui: &mut egui::Ui,
        plugin_index: usize,
        edits: &mut Vec<(usize, u32, f64)>,
        remove_requested: &mut Option<usize>,
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
                if ui.button("Remove").clicked() {
                    *remove_requested = Some(plugin_index);
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

    fn remove_plugin(&mut self, plugin_index: usize) {
        if plugin_index >= self.plugins.len() {
            return;
        }

        let plugin_name = self.plugin_names[plugin_index].clone();
        log::info!("removing plugin {plugin_name} from the stack");

        if let Some(engine) = self.engine.take() {
            engine.shutdown(ShutdownMode::UnloadPlugins);
        }
        self.active_audio_config = None;

        // Detach the native editor before dropping the plugin Arc it uses.
        self.editor_windows[plugin_index].close();
        self.editor_windows.remove(plugin_index);
        self.plugins.remove(plugin_index);
        self.plugin_names.remove(plugin_index);
        self.plugin_has_editors.remove(plugin_index);
        self.plugin_parameters.remove(plugin_index);

        if let Some(path_index) = self
            .plugin_paths
            .iter()
            .enumerate()
            .filter(|(_, path)| !path.trim().is_empty())
            .nth(plugin_index)
            .map(|(index, _)| index)
        {
            self.plugin_paths.remove(path_index);
        }

        self.error = None;
        if self.plugins.is_empty() {
            self.state = EditorState::Unloaded;
        } else {
            self.state = EditorState::Loaded;
            self.start_audio();
        }
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
