use super::audio::{AudioConfig, AudioDevices, AudioEngine, ChannelRouting, ShutdownMode};
use super::plugin::{
    PluginCache, PluginCacheEntry, load_plugin, load_stack, plugin_fingerprint, service_run_loop,
};
use super::plugin_runtime::{ParameterSnapshot, PluginRuntime, PluginSnapshot, plugin_snapshot};
use super::settings::{AudioSettings, Settings};
use super::windows::AudioMonitor;
use egui_shadcn::{
    BreadcrumbProps, Button, ButtonJustify, ButtonVariant, Input, InputSize, SheetProps, SheetSide,
    breadcrumb, breadcrumb_item, breadcrumb_list, breadcrumb_separator, sheet, sheet_content,
    sheet_description, sheet_footer, sheet_header, sheet_title, sheet_trigger,
};
use lucide_icons::Icon;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use vst3_host::{Plugin, PluginWindow, get_detailed_plugin_info};

const MAX_PLUGIN_PATHS: usize = 16;
const MAX_DISCOVERY_THREADS: usize = 6;

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
    mut report_progress: impl FnMut(f32, usize, usize),
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

    let mut plugin_paths = Vec::new();
    for root in &roots {
        collect_plugin_paths(root, &mut plugin_paths);
    }

    let plugin_count = plugin_paths.len();
    report_progress(20.0, 0, plugin_count);
    let plugin_paths = Arc::new(plugin_paths);
    let cache = Arc::new(Mutex::new(PluginCache::load()));
    let mut plugins = HashMap::new();
    let (sender, receiver) = mpsc::channel();
    let next_plugin = Arc::new(AtomicUsize::new(0));
    let worker_count = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(MAX_DISCOVERY_THREADS)
        .min(plugin_count);
    let mut workers = Vec::with_capacity(worker_count);

    for _ in 0..worker_count {
        let plugin_paths = Arc::clone(&plugin_paths);
        let cache = Arc::clone(&cache);
        let next_plugin = Arc::clone(&next_plugin);
        let sender = sender.clone();
        workers.push(thread::spawn(move || {
            loop {
                let index = next_plugin.fetch_add(1, Ordering::Relaxed);
                let Some(path) = plugin_paths.get(index) else {
                    break;
                };
                let discovered = discover_plugin(path, &cache);
                if sender.send(discovered).is_err() {
                    break;
                }
            }
        }));
    }
    drop(sender);

    for (index, discovered) in receiver.into_iter().enumerate() {
        if let Some((identity, plugin)) = discovered {
            plugins.entry(identity).or_insert(plugin);
        }
        let progress = if plugin_count == 0 {
            95.0
        } else {
            20.0 + 75.0 * (index + 1) as f32 / plugin_count as f32
        };
        report_progress(progress, plugins.len(), plugin_count);
    }
    for worker in workers {
        worker
            .join()
            .expect("plugin discovery worker thread panicked");
    }
    if let Ok(cache) = cache.lock() {
        cache.save();
    } else {
        log::warn!("could not save plugin cache because its lock was poisoned");
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
    report_progress(95.0, plugins.len(), plugin_count);
    plugins
}

fn collect_plugin_paths(directory: &Path, plugin_paths: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("vst3"))
        {
            plugin_paths.push(path);
        } else if path.is_dir() {
            collect_plugin_paths(&path, plugin_paths);
        }
    }
}

fn discover_plugin(
    path: &Path,
    cache: &Arc<Mutex<PluginCache>>,
) -> Option<(String, DiscoveredPlugin)> {
    let fingerprint = plugin_fingerprint(path).ok();
    if let Some(fingerprint) = fingerprint
        && let Ok(cache) = cache.lock()
        && let Some(entry) = cache.entry(path, fingerprint)
    {
        log::debug!("plugin cache hit for {} ({})", path.display(), entry.name);
        return Some(discovered_plugin_from_cache(entry));
    }

    if fingerprint.is_some()
        && let Ok(cache) = cache.lock()
        && cache.has_path(path)
    {
        log::info!(
            "plugin cache fingerprint changed; refreshing {}",
            path.display()
        );
    } else {
        log::debug!("plugin cache miss for {}", path.display());
    }

    let metadata = match get_detailed_plugin_info(path) {
        Ok(metadata) => metadata.info,
        Err(error) => {
            log::warn!(
                "could not read VST3 metadata for {}: {error}",
                path.display()
            );
            return None;
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

    if let Some(fingerprint) = fingerprint
        && let Ok(mut cache) = cache.lock()
    {
        cache.update(
            Path::new(&path),
            fingerprint,
            &name,
            &category,
            &metadata.uid,
        );
    }

    Some((
        identity,
        DiscoveredPlugin {
            name,
            path,
            category,
        },
    ))
}

fn discovered_plugin_from_cache(entry: PluginCacheEntry) -> (String, DiscoveredPlugin) {
    let identity = if entry.uid.trim().is_empty() {
        entry.path.to_ascii_lowercase()
    } else {
        entry.uid.to_ascii_lowercase()
    };
    (
        identity,
        DiscoveredPlugin {
            name: entry.name,
            path: entry.path,
            category: entry.category,
        },
    )
}

type RackParameter = ParameterSnapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
enum EditorState {
    Unloaded,
    Loading,
    Loaded,
    AudioUnavailable,
    Error,
}

enum LoadedPlugin {
    Stack(Vec<Plugin>),
    Single(Plugin),
}

pub struct PluginEditor {
    plugin_paths: Vec<String>,
    plugins: Vec<Arc<Mutex<Plugin>>>,
    runtime: PluginRuntime,
    engine: Option<AudioEngine>,
    active_audio_config: Option<AudioConfig>,
    editor_windows: Vec<PluginWindow>,
    plugin_names: Vec<String>,
    plugin_latencies: Vec<u32>,
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
    load_requested: bool,
    load_after_picker_close: bool,
    pending_add_path: Option<String>,
}

impl Default for PluginEditor {
    fn default() -> Self {
        Self {
            plugin_paths: default_plugin_paths(),
            plugins: Vec::new(),
            runtime: PluginRuntime::spawn(),
            engine: None,
            active_audio_config: None,
            editor_windows: Vec::new(),
            plugin_names: Vec::new(),
            plugin_latencies: Vec::new(),
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
            load_requested: false,
            load_after_picker_close: false,
            pending_add_path: None,
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

    fn apply_plugin_snapshots(&mut self, plugins: Vec<PluginSnapshot>) {
        self.plugin_names = plugins.iter().map(|plugin| plugin.name.clone()).collect();
        self.plugin_latencies = plugins
            .iter()
            .map(|plugin| plugin.latency_samples)
            .collect();
        self.plugin_has_editors = plugins.iter().map(|plugin| plugin.has_editor).collect();
        self.plugin_parameters = plugins
            .into_iter()
            .map(|plugin| plugin.parameters)
            .collect();
    }

    pub fn audio_settings(&self) -> AudioSettings {
        self.settings.audio_settings()
    }

    pub(super) fn process_pending_load(&mut self) {
        if !self.load_requested {
            return;
        }

        self.load_requested = false;
        self.state = EditorState::Loading;
        let added_path = self.pending_add_path.take();
        self.load_plugins_on_ui_thread(added_path);
    }

    pub(super) fn is_loading(&self) -> bool {
        matches!(self.state, EditorState::Loading) || self.load_requested
    }

    pub(super) fn show_loading(&self, ctx: &egui::Context) {
        super::loading::show(ctx, &self.settings.theme, "Please wait", None);
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
            .filter_map(|plugin| plugin.try_lock().ok())
            .map(|plugin| f64::from(plugin.latency_samples()))
            .sum();
        if self
            .settings
            .show_configuration(ui, &mut self.error, plugin_latency_samples)
        {
            self.apply_audio_devices();
        } else if self.plugin_names.is_empty()
            && self.engine.is_none()
            && matches!(self.state, EditorState::Unloaded)
        {
            self.start_audio();
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
                    let inner_width = content_ui.available_width() - 32.0;
                    egui::Frame::NONE
                        .inner_margin(egui::Margin::symmetric(16, 0))
                        .show(content_ui, |input_ui| {
                            Input::new(search_id)
                                .size(InputSize::Size2)
                                .width(inner_width - 8.0)
                                .placeholder("Search by plugin name or category")
                                .left_slot(|painter, rect, color| {
                                    let icon_size = 14.0;
                                    painter.text(
                                        egui::pos2(
                                            rect.left() + 4.0 + icon_size * 0.5,
                                            rect.center().y,
                                        ),
                                        egui::Align2::CENTER_CENTER,
                                        Icon::Search.unicode().to_string(),
                                        egui::FontId::new(
                                            icon_size,
                                            egui::FontFamily::Name("lucide".into()),
                                        ),
                                        color,
                                    );
                                })
                                .show(input_ui, theme, &mut search);
                        });

                    let query = search.trim().to_ascii_lowercase();
                    let mut categories: Vec<(&str, Vec<&DiscoveredPlugin>)> = Vec::new();
                    for plugin in &self.discovered_plugins {
                        if !query.is_empty()
                            && !plugin.name.to_ascii_lowercase().contains(&query)
                            && !plugin.path.to_ascii_lowercase().contains(&query)
                            && !category_segments(&plugin.category)
                                .join(" ")
                                .to_ascii_lowercase()
                                .contains(&query)
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
                    categories.sort_by_cached_key(|(category, _)| {
                        category_segments(category).join(" > ").to_lowercase()
                    });
                    for (_, plugins) in &mut categories {
                        plugins.sort_by_cached_key(|plugin| plugin.name.to_lowercase());
                    }

                    egui::Frame::NONE
                        .inner_margin(egui::Margin::symmetric(16, 0))
                        .show(content_ui, |frame_ui| {
                            frame_ui.group(|group_ui| {
                                let directory_width = group_ui.available_width() - 8.0;
                                egui::ScrollArea::vertical()
                                    .auto_shrink([false, false])
                                    .max_width(directory_width)
                                    .min_scrolled_width(directory_width)
                                    .max_height(190.0)
                                    .show(group_ui, |list_ui| {
                                        if categories.is_empty() {
                                            list_ui.label("No matching VST3 plugins found.");
                                        }
                                        for (category, plugins) in categories {
                                            list_ui.add_space(4.0);
                                            show_category_breadcrumb(list_ui, theme, category);
                                            for plugin in plugins {
                                                let is_selected = selected.as_deref()
                                                    == Some(plugin.path.as_str());
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
                            });
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
            // Commit the sheet close before requesting the next-frame plugin load.
            open = false;
            self.plugin_picker_open = false;
            selected = None;
            if !self.plugin_paths.iter().any(|existing| existing == &path)
                && self.push_plugin_path(path)
            {
                self.pending_add_path = self.plugin_paths.last().cloned();
                self.load_after_picker_close = true;
            }
        }
        self.plugin_picker_open = open;
        self.plugin_picker_search = search;
        self.selected_plugin_path = selected;

        if self.load_after_picker_close
            && !open
            && ui.ctx().animate_bool(sheet_id.with("open-anim"), false) <= 0.0
        {
            self.load_after_picker_close = false;
            if self.engine.is_some() || !self.plugins.is_empty() {
                self.request_incremental_add();
            } else {
                self.pending_add_path = None;
                self.request_load();
            }
        }
    }

    fn request_incremental_add(&mut self) {
        if self.is_loading() || self.load_requested {
            return;
        }
        self.plugin_picker_open = false;
        self.selected_plugin_path = None;
        self.load_requested = true;
    }

    fn request_load(&mut self) {
        if self.is_loading() || self.load_requested {
            return;
        }
        self.plugin_picker_open = false;
        self.selected_plugin_path = None;
        self.pending_add_path = None;
        self.load_requested = true;
    }

    fn load_plugins_on_ui_thread(&mut self, added_path: Option<String>) {
        let sample_rate = self
            .settings
            .output_index
            .and_then(|index| AudioDevices::sample_rate(self.settings.backend, index).ok());
        let block_size = self.settings.buffer_size.frames() as usize;
        // vst3-host binds the plugin's controller/editor connection to the thread that creates
        // the instance. Native editors are opened and serviced by eframe's UI thread, so plugin
        // instances must be created here as well. Loading them on a worker thread produces blank
        // windows and can make plugins deadlock or crash when their HWND is attached later.
        let result = match added_path.as_deref() {
            Some(path) => load_plugin(path, sample_rate, block_size).map(LoadedPlugin::Single),
            None => {
                load_stack(&self.plugin_paths, sample_rate, block_size).map(LoadedPlugin::Stack)
            }
        };
        self.finish_plugin_load(result, added_path);
    }

    fn finish_plugin_load(
        &mut self,
        result: Result<LoadedPlugin, String>,
        added_path: Option<String>,
    ) {
        let loaded = match result {
            Ok(plugins) => plugins,
            Err(error) => {
                self.rollback_added_path(added_path.as_deref());
                self.state = if self.plugins.is_empty() {
                    EditorState::Error
                } else {
                    EditorState::Loaded
                };
                self.error = Some(format!("Could not load editor: {error}"));
                return;
            }
        };

        match loaded {
            LoadedPlugin::Single(plugin) => {
                self.complete_incremental_add(plugin, added_path.as_deref())
            }
            LoadedPlugin::Stack(loaded) => {
                if let Some(engine) = self.engine.take() {
                    engine.shutdown(ShutdownMode::UnloadPlugins);
                }
                self.complete_plugin_load(loaded);
            }
        }
    }

    fn complete_incremental_add(&mut self, plugin: Plugin, added_path: Option<&str>) {
        let index = self.plugins.len();
        let plugin = Arc::new(Mutex::new(plugin));
        let snapshot = match plugin.lock() {
            Ok(plugin) => match plugin_snapshot(&plugin) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    self.rollback_added_path(added_path);
                    self.state = if self.plugins.is_empty() {
                        EditorState::Error
                    } else {
                        EditorState::Loaded
                    };
                    self.error = Some(format!("Could not read added plugin: {error}"));
                    return;
                }
            },
            Err(_) => {
                self.rollback_added_path(added_path);
                self.state = if self.plugins.is_empty() {
                    EditorState::Error
                } else {
                    EditorState::Loaded
                };
                self.error = Some("Could not read added plugin: plugin lock poisoned".to_owned());
                return;
            }
        };

        if let Err(error) = self.runtime.handle().add_plugin(index, plugin.clone()) {
            self.rollback_added_path(added_path);
            self.state = if self.plugins.is_empty() {
                EditorState::Error
            } else {
                EditorState::Loaded
            };
            self.error = Some(format!("Could not add plugin to the runtime: {error}"));
            return;
        }

        log::info!("adding plugin {} to the stack", snapshot.name);
        self.editor_windows.push(PluginWindow::new(plugin.clone()));
        self.plugins.push(plugin);
        self.plugin_names.push(snapshot.name);
        self.plugin_latencies.push(snapshot.latency_samples);
        self.plugin_has_editors.push(snapshot.has_editor);
        self.plugin_parameters.push(snapshot.parameters);
        self.error = None;
        self.state = EditorState::Loaded;
    }

    fn rollback_added_path(&mut self, added_path: Option<&str>) {
        let Some(added_path) = added_path else {
            return;
        };
        if let Some(index) = self.plugin_paths.iter().position(|path| path == added_path) {
            self.plugin_paths.remove(index);
        }
        self.pending_add_path = None;
    }

    fn complete_plugin_load(&mut self, loaded: Vec<Plugin>) {
        self.active_audio_config = None;
        for window in &mut self.editor_windows {
            window.close();
        }
        self.editor_windows.clear();
        self.plugins.clear();
        self.plugin_names.clear();
        self.plugin_latencies.clear();
        self.plugin_has_editors.clear();
        self.plugin_parameters.clear();
        self.error = None;
        self.state = EditorState::Loading;

        let plugins = loaded
            .into_iter()
            .map(|plugin| Arc::new(Mutex::new(plugin)))
            .collect();
        if let Err(error) = self.install_plugins(plugins) {
            self.state = EditorState::Error;
            self.error = Some(error);
        } else if let Err(error) = self.runtime.handle().install_plugins(self.plugins.clone()) {
            self.state = EditorState::Error;
            self.error = Some(error);
        } else {
            self.start_audio();
        }
    }

    fn apply_audio_devices(&mut self) {
        if self.plugin_names.is_empty() {
            self.request_load();
        } else {
            self.restart_audio();
        }
    }

    pub fn shutdown(&mut self, mode: ShutdownMode) {
        self.load_requested = false;
        self.pending_add_path = None;
        for window in &mut self.editor_windows {
            window.close();
        }
        if matches!(mode, ShutdownMode::KeepPluginsLoaded) {
            std::mem::forget(std::mem::take(&mut self.editor_windows));
        } else {
            self.editor_windows.clear();
        }
        if let Some(engine) = self.engine.take() {
            engine.shutdown(mode);
        }
        self.active_audio_config = None;
        if matches!(mode, ShutdownMode::KeepPluginsLoaded) {
            std::mem::forget(std::mem::take(&mut self.plugins));
        } else {
            self.plugins.clear();
        }
        self.plugin_names.clear();
        self.plugin_latencies.clear();
        self.plugin_has_editors.clear();
        self.plugin_parameters.clear();
        self.state = EditorState::Unloaded;
        self.runtime
            .shutdown(matches!(mode, ShutdownMode::KeepPluginsLoaded));
    }

    fn install_plugins(&mut self, plugins: Vec<Arc<Mutex<Plugin>>>) -> Result<(), String> {
        let mut snapshots = Vec::with_capacity(plugins.len());
        for (index, plugin) in plugins.iter().enumerate() {
            let plugin = plugin
                .lock()
                .map_err(|_| format!("plugin {index} lock poisoned"))?;
            snapshots.push(
                plugin_snapshot(&plugin)
                    .map_err(|error| format!("Could not read plugin {index}: {error}"))?,
            );
        }
        self.editor_windows = plugins
            .iter()
            .map(|plugin| PluginWindow::new(plugin.clone()))
            .collect();
        self.plugins = plugins;
        self.apply_plugin_snapshots(snapshots);
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
                for index in 0..self.plugin_names.len() {
                    self.show_plugin_container(ui, index, &mut edits, &mut remove_requested);
                }
            });
        for (plugin_index, parameter_id, value) in edits {
            self.set_parameter(plugin_index, parameter_id, value);
        }
        if let Some(plugin_index) = remove_requested {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Progress);
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
                            ui.label(&rack_parameter.name);

                            if is_booleanish_parameter(
                                rack_parameter,
                                &rack_parameter.formatted_value,
                            ) {
                                let mut checked = boolean_parameter_value(
                                    rack_parameter,
                                    &rack_parameter.formatted_value,
                                );
                                let response = ui
                                    .add_enabled_ui(!rack_parameter.is_read_only, |ui| {
                                        ui.toggle_value(&mut checked, "")
                                    })
                                    .inner;
                                if response.changed() {
                                    rack_parameter.value = if checked {
                                        rack_parameter.max
                                    } else {
                                        rack_parameter.min
                                    };
                                    rack_parameter.formatted_value =
                                        rack_parameter.value.to_string();
                                    edits.push((
                                        plugin_index,
                                        rack_parameter.id,
                                        rack_parameter.value,
                                    ));
                                }
                            } else {
                                let mut value = rack_parameter.value;
                                let mut slider = egui::Slider::new(
                                    &mut value,
                                    rack_parameter.min..=rack_parameter.max,
                                )
                                .show_value(false);
                                if rack_parameter.is_discrete() {
                                    let steps = f64::from(rack_parameter.step_count) + 1.0;
                                    slider = slider
                                        .step_by((rack_parameter.max - rack_parameter.min) / steps);
                                }
                                let response = ui.add_enabled(!rack_parameter.is_read_only, slider);
                                if response.changed() {
                                    rack_parameter.value = value;
                                    rack_parameter.formatted_value = value.to_string();
                                    edits.push((plugin_index, rack_parameter.id, value));
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
        if plugin_index >= self.plugin_names.len() || plugin_index >= self.plugins.len() {
            return;
        }

        let plugin_name = self.plugin_names[plugin_index].clone();
        log::info!("removing plugin {plugin_name} from the stack");

        let Some(path_index) = self
            .plugin_paths
            .iter()
            .enumerate()
            .filter(|(_, path)| !path.trim().is_empty())
            .nth(plugin_index)
            .map(|(index, _)| index)
        else {
            return;
        };

        if let Some(window) = self.editor_windows.get_mut(plugin_index) {
            window.close();
        }
        if let Err(error) = self.runtime.handle().remove_plugin(plugin_index) {
            self.state = EditorState::Error;
            self.error = Some(format!("Could not remove plugin from the runtime: {error}"));
            return;
        }

        self.plugin_paths.remove(path_index);
        self.plugins.remove(plugin_index);
        self.editor_windows.remove(plugin_index);
        self.plugin_names.remove(plugin_index);
        self.plugin_latencies.remove(plugin_index);
        self.plugin_has_editors.remove(plugin_index);
        self.plugin_parameters.remove(plugin_index);
        self.error = None;
        self.state = EditorState::Loaded;
    }

    fn open_native_editor(&mut self, plugin_index: usize) {
        let Some(window) = self.editor_windows.get_mut(plugin_index) else {
            return;
        };
        if let Err(error) = window.open() {
            self.state = EditorState::Error;
            self.error = Some(format!(
                "Could not open {} editor: {error}",
                self.plugin_names[plugin_index]
            ));
        }
    }

    fn set_parameter(&mut self, plugin_index: usize, parameter_id: u32, value: f64) {
        let result = self
            .plugins
            .get(plugin_index)
            .ok_or_else(|| format!("plugin {plugin_index} is not available"))
            .and_then(|plugin| {
                plugin
                    .lock()
                    .map_err(|_| "plugin lock poisoned".to_owned())
                    .and_then(|mut plugin| {
                        plugin
                            .set_parameter(parameter_id, value)
                            .map_err(|error| error.to_string())
                    })
            });
        if let Err(error) = result {
            self.state = EditorState::Error;
            self.error = Some(format!(
                "Could not set {} parameter {parameter_id}: {error}",
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
            match plugin_snapshot(&plugin) {
                Ok(snapshot) => {
                    self.plugin_parameters[plugin_index] = snapshot.parameters;
                }
                Err(error) => {
                    self.error = Some(format!(
                        "Could not read {} parameters: {error}",
                        self.plugin_names[plugin_index]
                    ));
                }
            }
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
            if let Some(plugin) = self.plugins.get(index) {
                service_run_loop(plugin);
            }
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
        match AudioEngine::start(self.runtime.handle(), config) {
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
            Self::Loading => "loading",
            Self::Loaded => "loaded",
            Self::AudioUnavailable => "loaded (audio unavailable)",
            Self::Error => "error",
        }
    }
}

fn category_segments(category: &str) -> Vec<String> {
    category
        .split('|')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(|segment| match segment {
            "Fx" => "Effects".to_owned(),
            other => other.to_owned(),
        })
        .collect()
}

fn show_category_breadcrumb(ui: &mut egui::Ui, theme: &egui_shadcn::Theme, category: &str) {
    let segments = category_segments(category);
    breadcrumb(
        ui,
        theme,
        BreadcrumbProps::new().text_size(13.0),
        |ui, ctx| {
            breadcrumb_list(ui, ctx, |ui, ctx| {
                let last = segments.len().saturating_sub(1);
                for (index, segment) in segments.iter().enumerate() {
                    breadcrumb_item(ui, ctx, |ui| {
                        let faded = theme.palette.muted_foreground.gamma_multiply(0.7);
                        ui.label(egui::RichText::new(segment.as_str()).color(faded));
                    });
                    if index != last {
                        breadcrumb_separator(ui, ctx, None);
                    }
                }
            });
        },
    );
}

fn is_booleanish_parameter(parameter: &RackParameter, formatted_value: &str) -> bool {
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

fn boolean_parameter_value(parameter: &RackParameter, formatted_value: &str) -> bool {
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
        let category = if parameter.unit.trim().is_empty() {
            "General".to_string()
        } else {
            parameter.unit.trim().to_string()
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
