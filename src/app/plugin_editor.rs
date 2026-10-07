use super::audio::{
    AudioClip, AudioConfig, AudioDevices, AudioEngine, ChannelRouting, PlaybackState, ShutdownMode,
    SignalLevels, decode_file,
};
use super::plugin::{
    EditorWindow, PluginCache, PluginCacheEntry, RackPlugin, discover, load_plugin, load_stack,
    load_stack_with_fingerprints, plugin_fingerprint, service_run_loop,
};
use super::plugin_runtime::{ParameterSnapshot, PluginRuntime, PluginSnapshot, plugin_snapshot};
use super::presets::{Preset, preset_from_snapshots, validate_plugin_installations};
use super::settings::{AudioSettings, Settings};
use egui_shadcn::{
    BreadcrumbProps, Button, ButtonJustify, ButtonSize, ButtonVariant, Input, InputSize,
    SheetProps, SheetSide, breadcrumb, breadcrumb_item, breadcrumb_list, breadcrumb_separator,
    sheet, sheet_content, sheet_description, sheet_footer, sheet_header, sheet_title,
    sheet_trigger,
};
use lucide_icons::Icon;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

const MAX_PLUGIN_PATHS: usize = 16;
const MAX_DISCOVERY_THREADS: usize = 6;
const CUSTOM_PLUGIN_EDITOR_HEIGHT: f32 = 256.0;
const CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT: f32 = 24.0;
const CUSTOM_PLUGIN_EDITOR_COLUMN_COUNT: usize = 3;

#[derive(Clone, Debug)]
pub(super) struct DiscoveredPlugin {
    name: String,
    path: String,
    category: String,
}

fn default_plugin_paths() -> Vec<String> {
    Vec::with_capacity(MAX_PLUGIN_PATHS)
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
    #[cfg(windows)]
    roots.push(PathBuf::from(r"C:\Program Files\Steinberg\VSTPlugins"));

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
        if path.extension().is_some_and(|extension| {
            let extension = extension.to_string_lossy();
            extension.eq_ignore_ascii_case("vst3") || extension.eq_ignore_ascii_case("dll")
        }) {
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

    let metadata = match discover(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            log::warn!(
                "could not read plugin metadata for {}: {error}",
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
    Empty,
    Loading,
    Loaded,
    AudioUnavailable,
    Error,
}

enum LoadedPlugin {
    Stack(Vec<RackPlugin>),
    Single(RackPlugin),
}

pub struct PluginEditor {
    plugin_paths: Vec<String>,
    plugins: Vec<Arc<Mutex<RackPlugin>>>,
    runtime: PluginRuntime,
    engine: Option<AudioEngine>,
    active_audio_config: Option<AudioConfig>,
    editor_windows: Vec<EditorWindow>,
    plugin_names: Vec<String>,
    plugin_latencies: Vec<u32>,
    plugin_has_editors: Vec<bool>,
    plugin_parameters: Vec<Vec<RackParameter>>,
    plugin_bypass_states: Vec<Arc<AtomicBool>>,
    error: Option<String>,
    state: EditorState,
    settings: Settings,
    plugin_picker_open: bool,
    plugin_picker_search: String,
    selected_plugin_path: Option<String>,
    discovered_plugins: Vec<DiscoveredPlugin>,
    load_requested: bool,
    pending_preset: Option<Preset>,
    pending_preset_name: Option<String>,
    loaded_preset_name: Option<String>,
    load_after_picker_close: bool,
    pending_add_path: Option<String>,
    drop_zone_hovered: Option<usize>,
    playback: Arc<PlaybackState>,
    playback_clip: Option<Arc<AudioClip>>,
    playback_file: Option<PathBuf>,
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
            plugin_bypass_states: Vec::new(),
            error: None,
            state: EditorState::Unloaded,
            settings: Settings::default(),
            plugin_picker_open: false,
            plugin_picker_search: String::new(),
            selected_plugin_path: None,
            discovered_plugins: Vec::new(),
            load_requested: false,
            pending_preset: None,
            pending_preset_name: None,
            loaded_preset_name: None,
            load_after_picker_close: false,
            pending_add_path: None,
            drop_zone_hovered: None,
            playback: Arc::new(PlaybackState::new()),
            playback_clip: None,
            playback_file: None,
        }
    }
}

impl PluginEditor {
    pub fn from_settings(settings: &AudioSettings) -> Self {
        let mut editor = Self::default();
        editor.settings = Settings::from_audio_settings(settings);
        editor
    }

    pub(super) fn edit_state_label(&self) -> &str {
        self.state.label()
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
        let pending_preset = self.pending_preset.take();
        let added_path = self.pending_add_path.take();
        if let Some(preset) = pending_preset {
            let preset_name = self.pending_preset_name.take();
            self.load_preset_on_ui_thread(preset, preset_name);
        } else {
            self.load_plugins_on_ui_thread(added_path);
        }
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

    pub(crate) fn loaded_preset_name(&self) -> Option<&str> {
        self.loaded_preset_name.as_deref()
    }

    pub(crate) fn clear_loaded_preset_name(&mut self) {
        self.loaded_preset_name = None;
    }

    pub(crate) fn set_error(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
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

    pub fn has_plugin_stack(&self) -> bool {
        !self.plugins.is_empty() || self.plugin_paths.iter().any(|path| !path.trim().is_empty())
    }

    pub(crate) fn has_loaded_plugin_stack(&self) -> bool {
        !self.plugins.is_empty()
    }

    pub(crate) fn capture_preset(&mut self) -> Result<Preset, String> {
        let configured_paths = self
            .plugin_paths
            .iter()
            .filter(|path| !path.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>();
        let mut snapshots = Vec::with_capacity(self.plugins.len());
        for (index, plugin) in self.plugins.iter().enumerate() {
            let mut plugin = plugin
                .lock()
                .map_err(|_| format!("Could not read plugin {index}: plugin lock poisoned"))?;
            snapshots.push(
                plugin_snapshot(&mut plugin)
                    .map_err(|error| format!("Could not read plugin {index}: {error}"))?,
            );
        }
        let bypassed = self
            .plugin_bypass_states
            .iter()
            .map(|bypass| bypass.load(Ordering::Relaxed))
            .collect::<Vec<_>>();
        preset_from_snapshots(&configured_paths, &snapshots, &bypassed)
    }

    pub(crate) fn request_load_preset(&mut self, preset: Preset, name: String) {
        if self.is_loading() {
            return;
        }
        if preset.plugins.len() > MAX_PLUGIN_PATHS {
            self.error = Some(format!(
                "Could not load preset: it contains more than {MAX_PLUGIN_PATHS} plugins."
            ));
            return;
        }
        if let Err(error) = validate_plugin_installations(&preset) {
            self.error = Some(format!("Could not load preset: {error}"));
            return;
        }
        self.plugin_picker_open = false;
        self.load_after_picker_close = false;
        self.pending_add_path = None;
        self.selected_plugin_path = None;
        self.pending_preset = Some(preset);
        self.pending_preset_name = Some(name);
        self.load_requested = true;
    }

    pub fn clear_plugin_stack(&mut self) {
        log::info!("clearing plugin stack");
        self.load_requested = false;
        self.pending_preset = None;
        self.pending_preset_name = None;
        self.load_after_picker_close = false;
        self.pending_add_path = None;
        self.plugin_picker_open = false;
        self.selected_plugin_path = None;
        self.drop_zone_hovered = None;
        self.loaded_preset_name = None;

        while let Some(plugin_index) = self.plugins.len().checked_sub(1) {
            let plugin_count = self.plugins.len();
            self.remove_plugin(plugin_index);
            if self.plugins.len() == plugin_count {
                return;
            }
        }

        self.plugin_paths.clear();
        self.error = None;
        self.state = EditorState::Empty;
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("VST3 / VST2 Rack");
        self.show_playback(ui);
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
        ui.add_space(12.0);
        self.show_plugin_rack(ui);
        self.show_add_plugin_button(ui);
    }

    fn show_playback(&mut self, ui: &mut egui::Ui) {
        let theme = self.settings.theme.clone();
        let has_clip = self.playback_clip.is_some();
        let duration = self
            .playback_clip
            .as_ref()
            .map(|clip| clip.frames() as f64 / f64::from(clip.sample_rate))
            .unwrap_or(0.0);
        let file_name = self
            .playback_file
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned());

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.strong("Audio Playback");
                if Button::new("Choose audio")
                    .variant(ButtonVariant::Outline)
                    .show(ui, &theme)
                    .clicked()
                {
                    self.choose_playback_file();
                }
                if let Some(file_name) = &file_name {
                    ui.label(file_name);
                } else {
                    ui.weak("No audio loaded (MP3 or WAV)");
                }
            });

            ui.add_space(6.0);
            let mut position = if duration > 0.0 {
                self.playback.position_frames() as f64
                    / self
                        .playback_clip
                        .as_ref()
                        .map(|clip| f64::from(clip.sample_rate))
                        .unwrap_or(1.0)
            } else {
                0.0
            };
            position = position.clamp(0.0, duration);
            let progress = ui.add_enabled(
                has_clip,
                egui::Slider::new(&mut position, 0.0..=duration.max(0.001)).show_value(false),
            );
            if progress.changed()
                && let Some(clip) = &self.playback_clip
            {
                self.playback
                    .request_seek((position * f64::from(clip.sample_rate)) as u64);
            }

            ui.horizontal(|ui| {
                let play_label = if self.playback.is_playing() {
                    "Pause"
                } else {
                    "Play"
                };
                if Button::new(play_label)
                    .variant(ButtonVariant::Default)
                    .enabled(has_clip)
                    .show(ui, &theme)
                    .clicked()
                {
                    if !self.playback.is_playing() && position >= duration && has_clip {
                        self.playback.request_seek(0);
                    }
                    self.playback.set_playing(!self.playback.is_playing());
                }
                if Button::new("Stop")
                    .variant(ButtonVariant::Outline)
                    .enabled(has_clip)
                    .show(ui, &theme)
                    .clicked()
                {
                    self.playback.reset();
                }
                ui.add_space(8.0);
                let mut volume = self.playback.volume();
                ui.label("Volume");
                if ui
                    .add_enabled(
                        has_clip,
                        egui::Slider::new(&mut volume, 0.0..=1.0).show_value(false),
                    )
                    .changed()
                {
                    self.playback.set_volume(volume);
                }
                ui.label(format!("{:.0}%", self.playback.volume() * 100.0));
                ui.separator();
                ui.label(format_time(position));
                ui.label("/");
                ui.label(format_time(duration));
            });
        });
        ui.add_space(12.0);
        if self.playback.is_playing() {
            ui.ctx().request_repaint();
        }
    }

    fn choose_playback_file(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Audio files", &["mp3", "wav"])
            .pick_file()
        else {
            return;
        };
        match decode_file(&path) {
            Ok(clip) => {
                let clip = Arc::new(clip);
                self.playback.reset();
                match self.runtime.handle().set_playback_clip(Arc::clone(&clip)) {
                    Ok(()) => {
                        self.playback_clip = Some(clip);
                        self.playback_file = Some(path);
                        self.error = None;
                    }
                    Err(error) => {
                        self.error =
                            Some(format!("Could not load audio into the runtime: {error}"));
                    }
                }
            }
            Err(error) => self.error = Some(error),
        }
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
                    Button::new("Add an effect")
                        .variant(ButtonVariant::Default)
                        .size(egui_shadcn::ButtonSize::Default)
                        .min_width(trigger_width)
                        .show(ui, theme)
                })
                .clicked();

                let mut should_close = false;
                let _ = sheet_content(ui, theme, context, |content_ui| {
                    sheet_header(content_ui, |header_ui| {
                        sheet_title(header_ui, theme, "Add an effect");
                        sheet_description(
                            header_ui,
                            theme,
                            "Choose a VST3 or VST2 plugin from your system or user installation.",
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
                                            list_ui
                                                .label("No matching VST3 or VST2 plugins found.");
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
                                if Button::new("Add effect")
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

    fn load_preset_on_ui_thread(&mut self, preset: Preset, preset_name: Option<String>) {
        let sample_rate = self
            .settings
            .output_index
            .and_then(|index| AudioDevices::sample_rate(self.settings.backend, index).ok());
        let block_size = self.settings.buffer_size.frames() as usize;
        let paths = preset
            .plugins
            .iter()
            .map(|plugin| plugin.path.clone())
            .collect::<Vec<_>>();
        let fingerprints = preset
            .plugins
            .iter()
            .map(|plugin| plugin.fingerprint)
            .collect::<Option<Vec<_>>>();
        let result =
            load_stack_with_fingerprints(&paths, fingerprints.as_deref(), sample_rate, block_size);
        match result {
            Ok(loaded) => {
                self.plugin_paths = paths;
                self.finish_plugin_load_with_preset(loaded, &preset, preset_name.as_deref());
            }
            Err(error) => {
                self.state = if self.plugins.is_empty() {
                    EditorState::Error
                } else {
                    EditorState::Loaded
                };
                self.error = Some(format!("Could not load preset: {error}"));
            }
        }
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
                self.complete_plugin_load(loaded, None, None);
            }
        }
    }

    fn finish_plugin_load_with_preset(
        &mut self,
        loaded: Vec<RackPlugin>,
        preset: &Preset,
        preset_name: Option<&str>,
    ) {
        if let Some(engine) = self.engine.take() {
            engine.shutdown(ShutdownMode::UnloadPlugins);
        }
        self.complete_plugin_load(loaded, Some(preset), preset_name);
    }

    fn complete_incremental_add(&mut self, plugin: RackPlugin, added_path: Option<&str>) {
        let index = self.plugins.len();
        let plugin = Arc::new(Mutex::new(plugin));
        let vst2_editor = match plugin.lock() {
            Ok(mut plugin) => plugin.take_editor(),
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
        let snapshot = match plugin.lock() {
            Ok(mut plugin) => match plugin_snapshot(&mut plugin) {
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

        let bypass_state = Arc::new(AtomicBool::new(false));
        if let Err(error) =
            self.runtime
                .handle()
                .add_plugin(index, plugin.clone(), bypass_state.clone())
        {
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
        let window = plugin
            .lock()
            .map(|plugin| EditorWindow::new(&plugin, vst2_editor))
            .unwrap_or(EditorWindow::None);
        self.editor_windows.push(window);
        self.plugins.push(plugin);
        self.plugin_names.push(snapshot.name);
        self.plugin_latencies.push(snapshot.latency_samples);
        self.plugin_has_editors.push(snapshot.has_editor);
        self.plugin_parameters.push(snapshot.parameters);
        self.plugin_bypass_states.push(bypass_state);
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

    fn complete_plugin_load(
        &mut self,
        loaded: Vec<RackPlugin>,
        preset: Option<&Preset>,
        preset_name: Option<&str>,
    ) {
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
        self.plugin_bypass_states.clear();
        self.error = None;
        self.state = EditorState::Loading;

        let plugins = loaded
            .into_iter()
            .map(|plugin| Arc::new(Mutex::new(plugin)))
            .collect();
        if let Err(error) = self.install_plugins(plugins) {
            self.state = EditorState::Error;
            self.error = Some(error);
        } else {
            if let Some(preset) = preset
                && let Err(error) = self.apply_preset_state(preset)
            {
                self.error = Some(error);
            }
            if let Err(error) = self
                .runtime
                .handle()
                .install_plugins(self.plugins.clone(), self.plugin_bypass_states.clone())
            {
                self.state = EditorState::Error;
                self.error = Some(error);
            } else {
                self.start_audio();
                self.loaded_preset_name = preset_name.map(str::to_owned);
            }
        }
    }

    fn apply_preset_state(&mut self, preset: &Preset) -> Result<(), String> {
        let mut errors = Vec::new();
        for (plugin_index, (plugin, saved_plugin)) in
            self.plugins.iter().zip(&preset.plugins).enumerate()
        {
            match plugin.lock() {
                Ok(mut plugin) => {
                    for parameter in &saved_plugin.parameters {
                        if let Err(error) = plugin.set_parameter(parameter.id, parameter.value) {
                            errors.push(format!(
                                "plugin {plugin_index}, parameter {}: {error}",
                                parameter.id
                            ));
                        }
                    }
                }
                Err(_) => errors.push(format!("plugin {plugin_index}: plugin lock poisoned")),
            }
            if let Some(bypass) = self.plugin_bypass_states.get(plugin_index) {
                bypass.store(saved_plugin.bypassed, Ordering::Relaxed);
            }
        }
        self.refresh_parameters();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "Could not restore all preset parameters: {}",
                errors.join("; ")
            ))
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
        self.pending_preset = None;
        self.pending_preset_name = None;
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
        self.plugin_bypass_states.clear();
        self.state = EditorState::Unloaded;
        self.runtime
            .shutdown(matches!(mode, ShutdownMode::KeepPluginsLoaded));
    }

    fn install_plugins(&mut self, plugins: Vec<Arc<Mutex<RackPlugin>>>) -> Result<(), String> {
        let mut snapshots = Vec::with_capacity(plugins.len());
        let mut vst2_editors = Vec::with_capacity(plugins.len());
        for (index, plugin) in plugins.iter().enumerate() {
            let mut plugin = plugin
                .lock()
                .map_err(|_| format!("plugin {index} lock poisoned"))?;
            let vst2_editor = plugin.take_editor();
            snapshots.push(
                plugin_snapshot(&mut plugin)
                    .map_err(|error| format!("Could not read plugin {index}: {error}"))?,
            );
            vst2_editors.push(vst2_editor);
        }
        self.editor_windows = plugins
            .iter()
            .zip(vst2_editors)
            .map(|(plugin, vst2_editor)| {
                plugin
                    .lock()
                    .map(|plugin| EditorWindow::new(&plugin, vst2_editor))
                    .unwrap_or(EditorWindow::None)
            })
            .collect();
        self.plugins = plugins;
        self.plugin_bypass_states = (0..snapshots.len())
            .map(|_| Arc::new(AtomicBool::new(false)))
            .collect();
        self.apply_plugin_snapshots(snapshots);
        self.state = EditorState::Loaded;
        Ok(())
    }

    fn show_plugin_rack(&mut self, ui: &mut egui::Ui) {
        self.service_editor_windows();
        self.refresh_parameters();
        let mut edits = Vec::new();
        let mut remove_requested = None;
        let mut reorder_requested = None;
        let previous_drop_zone = if egui::DragAndDrop::has_payload_of_type::<usize>(ui.ctx()) {
            self.drop_zone_hovered
        } else {
            self.drop_zone_hovered = None;
            None
        };
        self.drop_zone_hovered = None;
        egui::ScrollArea::vertical()
            .id_salt("plugin-rack-scroll")
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if let Some(reorder) = self.show_plugin_drop_zone(ui, 0, previous_drop_zone) {
                    reorder_requested = Some(reorder);
                }
                for index in 0..self.plugin_names.len() {
                    self.show_plugin_container(ui, index, &mut edits, &mut remove_requested);
                    if let Some(reorder) =
                        self.show_plugin_drop_zone(ui, index + 1, previous_drop_zone)
                    {
                        reorder_requested = Some(reorder);
                    }
                }
            });
        for (plugin_index, parameter_id, value) in edits {
            self.set_parameter(plugin_index, parameter_id, value);
        }
        if let Some(plugin_index) = remove_requested {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Progress);
            self.remove_plugin(plugin_index);
        }
        if let Some((dragged_index, drop_index)) = reorder_requested {
            self.reorder_plugin(*dragged_index, drop_index);
        }
    }

    fn show_plugin_drop_zone(
        &mut self,
        ui: &mut egui::Ui,
        drop_index: usize,
        previous_drop_zone: Option<usize>,
    ) -> Option<(std::sync::Arc<usize>, usize)> {
        let is_dragging = egui::DragAndDrop::has_payload_of_type::<usize>(ui.ctx());
        let is_expanded = is_dragging && previous_drop_zone == Some(drop_index);
        let zone_height = if is_expanded { 44.0 } else { 12.0 };
        let (zone_rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), zone_height),
            egui::Sense::hover(),
        );
        let zone_response = ui.interact(
            zone_rect,
            ui.make_persistent_id(("plugin-drop-zone", drop_index)),
            egui::Sense::hover(),
        );
        let is_drop_target = is_dragging && zone_response.contains_pointer();
        if is_drop_target {
            self.drop_zone_hovered = Some(drop_index);
            ui.ctx().request_repaint();
        }
        if is_dragging {
            let is_highlighted = is_drop_target || is_expanded;
            if is_highlighted {
                ui.painter().rect_filled(
                    zone_response.rect,
                    4.0,
                    ui.visuals().selection.bg_fill.linear_multiply(0.18),
                );
                ui.painter().text(
                    zone_response.rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "Drop here",
                    egui::TextStyle::Small.resolve(ui.style()),
                    ui.visuals().selection.stroke.color,
                );
            } else {
                ui.painter().line_segment(
                    [
                        egui::pos2(zone_response.rect.left(), zone_response.rect.center().y),
                        egui::pos2(zone_response.rect.right(), zone_response.rect.center().y),
                    ],
                    egui::Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color),
                );
            }
        }
        let dropped = zone_response.dnd_release_payload::<usize>();
        dropped.map(|dragged_index| (dragged_index, drop_index))
    }

    fn show_plugin_container(
        &mut self,
        ui: &mut egui::Ui,
        plugin_index: usize,
        edits: &mut Vec<(usize, u32, f64)>,
        remove_requested: &mut Option<usize>,
    ) {
        let plugin_name = self.plugin_names[plugin_index].clone();
        let drag_id = ui.make_persistent_id(("plugin-drag-handle", plugin_index));
        let is_dragged = ui.ctx().is_being_dragged(drag_id);
        let parameter_count = self.plugin_parameters[plugin_index].len();
        let theme = self.settings.theme.clone();

        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                let (handle_rect, _) =
                    ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
                let handle_response = ui.interact(handle_rect, drag_id, egui::Sense::drag());
                let handle_dragged = handle_response.dragged();
                let handle_hovered = handle_response.hovered() || handle_dragged;
                let handle_fill = if handle_hovered {
                    ui.visuals().selection.bg_fill
                } else {
                    ui.visuals().widgets.inactive.bg_fill
                };
                ui.painter().rect_filled(handle_rect, 5.0, handle_fill);
                for row in 0..3 {
                    for column in 0..2 {
                        let center = egui::pos2(
                            handle_rect.left() + 11.0 + column as f32 * 8.0,
                            handle_rect.center().y - 8.0 + row as f32 * 8.0,
                        );
                        ui.painter().circle_filled(
                            center,
                            1.5,
                            if handle_hovered {
                                ui.visuals().selection.stroke.color
                            } else {
                                ui.visuals().text_color()
                            },
                        );
                    }
                }
                let handle_response = handle_response
                    .on_hover_cursor(if handle_dragged {
                        egui::CursorIcon::Grabbing
                    } else {
                        egui::CursorIcon::Grab
                    })
                    .on_hover_text("Drag to reorder plugin");
                handle_response.dnd_set_drag_payload(plugin_index);

                let bypassed = self.plugin_bypassed(plugin_index);
                let bypass_icon = |painter: &egui::Painter,
                                   center: egui::Pos2,
                                   size: f32,
                                   color: egui::Color32| {
                    paint_lucide_icon(painter, center, size, color, Icon::Power);
                };
                if Button::new("")
                    .variant(if bypassed {
                        ButtonVariant::Secondary
                    } else {
                        ButtonVariant::Outline
                    })
                    .size(ButtonSize::IconSm)
                    .icon(&bypass_icon)
                    .show(ui, &theme)
                    .on_hover_text(if bypassed {
                        "Enable plugin"
                    } else {
                        "Bypass plugin"
                    })
                    .clicked()
                {
                    self.toggle_plugin_bypass(plugin_index);
                }

                let editor_icon = |painter: &egui::Painter,
                                   center: egui::Pos2,
                                   size: f32,
                                   color: egui::Color32| {
                    paint_lucide_icon(painter, center, size, color, Icon::Eye);
                };
                if Button::new("")
                    .variant(ButtonVariant::Outline)
                    .size(ButtonSize::IconSm)
                    .enabled(self.plugin_has_editors[plugin_index])
                    .icon(&editor_icon)
                    .show(ui, &theme)
                    .on_hover_text(if self.plugin_has_editors[plugin_index] {
                        "Open native editor"
                    } else {
                        "No native editor"
                    })
                    .clicked()
                {
                    self.open_native_editor(plugin_index);
                }

                let remove_icon = |painter: &egui::Painter,
                                   center: egui::Pos2,
                                   size: f32,
                                   color: egui::Color32| {
                    paint_lucide_icon(painter, center, size, color, Icon::Trash2);
                };
                if Button::new("")
                    .variant(ButtonVariant::Outline)
                    .size(ButtonSize::IconSm)
                    .icon(&remove_icon)
                    .show(ui, &theme)
                    .on_hover_text("Remove plugin")
                    .clicked()
                {
                    *remove_requested = Some(plugin_index);
                }
            });

            let body_width = ui.available_width().max(0.0);
            ui.allocate_ui_with_layout(
                egui::vec2(body_width, CUSTOM_PLUGIN_EDITOR_HEIGHT),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_height(CUSTOM_PLUGIN_EDITOR_HEIGHT);
                    if is_dragged {
                        ui.set_opacity(0.35);
                    }
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        self.show_plugin_container_body(
                            ui,
                            plugin_index,
                            &plugin_name,
                            parameter_count,
                            edits,
                        );
                    });
                },
            );
            if is_dragged {
                show_plugin_drag_preview(ui.ctx(), &plugin_name, parameter_count);
            }
        });
        ui.add_space(8.0);
    }

    fn show_plugin_container_body(
        &mut self,
        ui: &mut egui::Ui,
        plugin_index: usize,
        plugin_name: &str,
        parameter_count: usize,
        edits: &mut Vec<(usize, u32, f64)>,
    ) {
        ui.horizontal(|ui| {
            ui.strong(plugin_name);
            ui.label(format!("{parameter_count} parameters"));
        });

        if self.plugin_parameters[plugin_index].is_empty() {
            ui.label("This plugin does not expose any parameters.");
            return;
        }

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                for (group_index, (unit, parameter_indices)) in
                    group_parameter_indices(&self.plugin_parameters[plugin_index])
                        .into_iter()
                        .enumerate()
                {
                    ui.add_space(4.0);
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.strong(unit);
                        ui.add_space(4.0);

                        let column_ranges = balanced_column_ranges(
                            parameter_indices.len(),
                            CUSTOM_PLUGIN_EDITOR_COLUMN_COUNT,
                        );
                        ui.columns(CUSTOM_PLUGIN_EDITOR_COLUMN_COUNT, |columns| {
                            for (column_index, (start, end)) in
                                column_ranges.iter().copied().enumerate()
                            {
                                let ui = &mut columns[column_index];
                                let parameter_indices = &parameter_indices[start..end];
                                let column_width = ui.available_width();
                                let name_width = (column_width * 0.36).max(0.0);
                                let value_width = 48.0_f32.min((column_width * 0.24).max(0.0));
                                let control_width = (column_width
                                    - name_width
                                    - value_width
                                    - ui.spacing().item_spacing.x * 2.0)
                                    .max(0.0);

                                egui::Grid::new((
                                    "plugin-parameters",
                                    plugin_index,
                                    group_index,
                                    column_index,
                                ))
                                .striped(true)
                                .num_columns(3)
                                .show(ui, |ui| {
                                    ui.label("Parameter");
                                    ui.label("Control");
                                    ui.label("Value");
                                    ui.end_row();

                                    for &parameter_index in parameter_indices {
                                        let rack_parameter = &mut self.plugin_parameters
                                            [plugin_index][parameter_index];
                                        ui.add_sized(
                                            [name_width, CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT],
                                            egui::Label::new(&rack_parameter.name).wrap(),
                                        );

                                        if is_booleanish_parameter(
                                            rack_parameter,
                                            &rack_parameter.formatted_value,
                                        ) {
                                            let mut checked = boolean_parameter_value(
                                                rack_parameter,
                                                &rack_parameter.formatted_value,
                                            );
                                            let response = ui
                                                .add_enabled_ui(
                                                    !rack_parameter.is_read_only,
                                                    |ui| {
                                                        ui.add_sized(
                                                            [
                                                                control_width,
                                                                CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT,
                                                            ],
                                                            egui::Checkbox::without_text(
                                                                &mut checked,
                                                            ),
                                                        )
                                                    },
                                                )
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
                                                let steps =
                                                    f64::from(rack_parameter.step_count) + 1.0;
                                                slider = slider.step_by(
                                                    (rack_parameter.max - rack_parameter.min)
                                                        / steps,
                                                );
                                            }
                                            let response = ui.add_enabled_ui(
                                                !rack_parameter.is_read_only,
                                                |ui| {
                                                    ui.add_sized(
                                                        [
                                                            control_width,
                                                            CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT,
                                                        ],
                                                        slider,
                                                    )
                                                },
                                            );
                                            if response.inner.changed() {
                                                rack_parameter.value = value;
                                                rack_parameter.formatted_value =
                                                    format_custom_editor_float(value);
                                                edits.push((
                                                    plugin_index,
                                                    rack_parameter.id,
                                                    value,
                                                ));
                                            }
                                        }
                                        let display_value = if is_booleanish_parameter(
                                            rack_parameter,
                                            &rack_parameter.formatted_value,
                                        ) {
                                            rack_parameter.formatted_value.clone()
                                        } else {
                                            format_custom_editor_float(rack_parameter.value)
                                        };
                                        ui.add_sized(
                                            [value_width, CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT],
                                            egui::Label::new(display_value),
                                        );
                                        ui.end_row();
                                    }
                                });
                            }
                        });
                    });
                }
            });
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
        self.plugin_bypass_states.remove(plugin_index);
        self.error = None;
        self.state = EditorState::Loaded;
    }

    fn reorder_plugin(&mut self, from: usize, to: usize) {
        if from >= self.plugins.len() || to > self.plugins.len() {
            return;
        }

        if let Err(error) = self.runtime.handle().move_plugin(from, to) {
            self.state = EditorState::Error;
            self.error = Some(format!("Could not reorder plugin stack: {error}"));
            return;
        }

        move_item(&mut self.plugins, from, to);
        move_item(&mut self.editor_windows, from, to);
        move_item(&mut self.plugin_names, from, to);
        move_item(&mut self.plugin_latencies, from, to);
        move_item(&mut self.plugin_has_editors, from, to);
        move_item(&mut self.plugin_parameters, from, to);
        move_item(&mut self.plugin_bypass_states, from, to);

        let mut configured_paths = self
            .plugin_paths
            .iter()
            .filter(|path| !path.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>();
        move_item(&mut configured_paths, from, to);
        let mut configured_paths = configured_paths.into_iter();
        for path in self
            .plugin_paths
            .iter_mut()
            .filter(|path| !path.trim().is_empty())
        {
            *path = configured_paths
                .next()
                .expect("configured plugin paths and loaded plugins must stay aligned");
        }

        log::info!("reordered plugin stack: moved {from} to {to}");
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

    fn plugin_bypassed(&self, plugin_index: usize) -> bool {
        self.plugin_bypass_states
            .get(plugin_index)
            .map(|bypass| bypass.load(Ordering::Relaxed))
            .unwrap_or(false)
    }

    fn toggle_plugin_bypass(&mut self, plugin_index: usize) {
        let bypassed = self.plugin_bypassed(plugin_index);
        if let Some(bypass) = self.plugin_bypass_states.get(plugin_index) {
            bypass.store(!bypassed, Ordering::Relaxed);
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
            let mut plugin = match self.plugins[plugin_index].try_lock() {
                Ok(plugin) => plugin,
                Err(std::sync::TryLockError::Poisoned(poison)) => poison.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => continue,
            };
            match plugin_snapshot(&mut plugin) {
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
        match AudioEngine::start(self.runtime.handle(), config, Arc::clone(&self.playback)) {
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

    pub(crate) fn audio_levels(&self) -> Option<SignalLevels> {
        self.engine.as_ref().map(AudioEngine::levels)
    }

    pub(crate) fn estimated_latency_ms(&self) -> Option<f64> {
        let engine = self.engine.as_ref()?;
        let plugin_latency_samples = self
            .plugins
            .iter()
            .enumerate()
            .map(|(index, plugin)| {
                plugin
                    .try_lock()
                    .ok()
                    .map(|plugin| f64::from(plugin.latency_samples()))
                    .or_else(|| {
                        self.plugin_latencies
                            .get(index)
                            .map(|&latency| latency as f64)
                    })
                    .unwrap_or_default()
            })
            .sum();
        engine.latency_info().estimate_ms(plugin_latency_samples)
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
            Self::Unloaded => "Unloaded",
            Self::Empty => "Empty",
            Self::Loading => "Loading",
            Self::Loaded => "Loaded",
            Self::AudioUnavailable => "Loaded (audio unavailable)",
            Self::Error => "Error",
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

fn format_time(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn move_item<T>(items: &mut Vec<T>, from: usize, to: usize) {
    if from >= items.len() || to > items.len() {
        return;
    }
    let item = items.remove(from);
    let adjusted_to = if from < to { to - 1 } else { to };
    items.insert(adjusted_to, item);
}

fn show_plugin_drag_preview(ctx: &egui::Context, plugin_name: &str, parameter_count: usize) {
    let Some(pointer) = ctx.pointer_interact_pos() else {
        return;
    };

    let visuals = &ctx.style().visuals;
    let rect = egui::Rect::from_min_size(pointer + egui::vec2(16.0, 16.0), egui::vec2(280.0, 64.0));
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("plugin-drag-preview"),
    ));
    painter.rect_filled(
        rect,
        7.0,
        visuals.widgets.active.bg_fill.gamma_multiply(0.75),
    );
    painter.rect_stroke(
        rect,
        7.0,
        egui::Stroke::new(1.0_f32, visuals.selection.stroke.color.gamma_multiply(0.8)),
        egui::StrokeKind::Outside,
    );
    painter.text(
        rect.left_top() + egui::vec2(14.0, 19.0),
        egui::Align2::LEFT_CENTER,
        plugin_name,
        egui::FontId::proportional(14.0),
        visuals.text_color().gamma_multiply(0.85),
    );
    painter.text(
        rect.left_top() + egui::vec2(14.0, 43.0),
        egui::Align2::LEFT_CENTER,
        format!("Plugin editor · {parameter_count} parameters"),
        egui::FontId::proportional(11.0),
        visuals.text_color().gamma_multiply(0.65),
    );
}

fn paint_lucide_icon(
    painter: &egui::Painter,
    center: egui::Pos2,
    size: f32,
    color: egui::Color32,
    icon: Icon,
) {
    painter.text(
        center,
        egui::Align2::CENTER_CENTER,
        icon.unicode(),
        egui::FontId::new(size, egui::FontFamily::Name("lucide".to_owned().into())),
        color,
    );
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

fn format_custom_editor_float(value: f64) -> String {
    format!("{value:.2}")
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
        let category = parameter.category.trim();
        let category = if category.is_empty() || category.eq_ignore_ascii_case("general") {
            "General".to_owned()
        } else {
            category.to_owned()
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

    groups.sort_by_key(|(category, _)| !category.eq_ignore_ascii_case("General"));
    groups
}

fn balanced_column_ranges(item_count: usize, column_count: usize) -> Vec<(usize, usize)> {
    if column_count == 0 {
        return Vec::new();
    }

    let items_per_column = item_count / column_count;
    let remainder = item_count % column_count;
    let mut start = 0;

    (0..column_count)
        .map(|column| {
            let end = start + items_per_column + usize::from(column < remainder);
            let range = (start, end);
            start = end;
            range
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        balanced_column_ranges, format_custom_editor_float, group_parameter_indices, move_item,
    };
    use crate::app::plugin_runtime::ParameterSnapshot;

    fn parameter(category: &str) -> ParameterSnapshot {
        ParameterSnapshot {
            id: 0,
            name: String::new(),
            category: category.to_owned(),
            unit: String::new(),
            min: 0.0,
            max: 1.0,
            value: 0.0,
            step_count: 0,
            is_read_only: false,
            formatted_value: String::new(),
        }
    }

    #[test]
    fn rounds_custom_editor_float_values_to_two_decimal_places() {
        assert_eq!(format_custom_editor_float(1.236), "1.24");
        assert_eq!(format_custom_editor_float(1.2), "1.20");
    }

    #[test]
    fn balances_parameters_across_three_columns() {
        assert_eq!(balanced_column_ranges(7, 3), [(0, 3), (3, 5), (5, 7)]);
        assert_eq!(balanced_column_ranges(2, 3), [(0, 1), (1, 2), (2, 2)]);
        assert_eq!(balanced_column_ranges(4, 0), []);
    }

    #[test]
    fn puts_general_parameters_in_the_first_category() {
        let groups = group_parameter_indices(&[
            parameter("Modulation"),
            parameter(""),
            parameter("general"),
            parameter("Dynamics"),
        ]);

        assert_eq!(groups[0], ("General".to_owned(), vec![1, 2]));
        assert_eq!(groups[1], ("Modulation".to_owned(), vec![0]));
        assert_eq!(groups[2], ("Dynamics".to_owned(), vec![3]));
    }

    #[test]
    fn moves_an_item_before_a_drop_index() {
        let mut items = vec!["input", "compressor", "delay", "output"];

        move_item(&mut items, 2, 1);

        assert_eq!(items, ["input", "delay", "compressor", "output"]);
    }

    #[test]
    fn moves_an_item_to_the_end_of_the_stack() {
        let mut items = vec!["input", "compressor", "delay", "output"];
        let end = items.len();

        move_item(&mut items, 0, end);

        assert_eq!(items, ["compressor", "delay", "output", "input"]);
    }

    #[test]
    fn dropping_next_to_the_same_item_is_a_noop() {
        let mut items = vec!["input", "compressor", "delay"];

        move_item(&mut items, 1, 2);

        assert_eq!(items, ["input", "compressor", "delay"]);
    }
}
