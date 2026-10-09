use super::audio::tuner::{
    MAX_A4_PITCH_HZ, MAX_INPUT_SENSITIVITY_DB, MAX_TRANSPOSITION_SEMITONES, MIN_A4_PITCH_HZ,
    MIN_INPUT_SENSITIVITY_DB, MIN_TRANSPOSITION_SEMITONES, TuningPreset,
};
use super::audio::{
    AudioClip, AudioConfig, AudioDevices, AudioEngine, PlaybackState, ShutdownMode, TunerReadout,
    TunerState, decode_file,
};
use super::midi::PresetMidiMapping;
use super::plugin::{
    DiscoveredPlugin, EditorWindow, PLUGIN_LOAD_CANCELLED, PluginFormat, RackPlugin, load_plugin,
    load_stack_with_fingerprints_progress,
};
use super::plugin_runtime::{PluginRuntime, plugin_snapshot};
use super::presets::{Preset, preset_from_snapshots, validate_plugin_installations};
use super::settings::{AppSettings, AudioSettings, MidiSettings, Settings, TunerSettings};
use super::ui::{
    Button as UiButton, ButtonJustify, ButtonSize, ButtonType, Label, Select, SelectItem,
    SheetProps, SheetSide, Theme, breadcrumb, error_label, heading, label as ui_label, muted,
    paint as paint_label, search_input, sheet, sheet_content_bottom_with_height, sheet_description,
    sheet_footer, sheet_header, sheet_title, sheet_trigger, strong,
};
use lucide_icons::Icon;
use rtrb::Consumer;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use vst3_host::MidiEvent;
use vst3_host::midi_input::MidiInputConnection;

mod audio_session;
mod effect_filter;
mod formatting;
mod midi;
mod model;
mod native_editors;
mod view_helpers;
mod views;

use effect_filter::show_effect_type_filter;
use formatting::{
    balanced_column_ranges, boolean_parameter_value, category_segments, effect_type, format_cents,
    format_custom_editor_float, format_time, group_parameter_indices, is_booleanish_parameter,
    move_item, tuner_note,
};
use model::{PluginRack, PluginSlot};
use view_helpers::{paint_lucide_icon, show_plugin_drag_preview, show_tall_outline_icon_button};

const MAX_PLUGIN_PATHS: usize = 16;
const CUSTOM_PLUGIN_EDITOR_HEIGHT: f32 = 256.0;
const COLLAPSED_PLUGIN_EDITOR_HEIGHT: f32 = 32.0;
const PLAYBACK_EDITOR_HEIGHT: f32 = 176.0;
const TUNER_COLLAPSED_EDITOR_HEIGHT: f32 = 60.0;
const CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT: f32 = 24.0;
const CUSTOM_PLUGIN_EDITOR_COLUMN_COUNT: usize = 3;

fn default_plugin_paths() -> Vec<String> {
    Vec::with_capacity(MAX_PLUGIN_PATHS)
}

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
    Single(Box<RackPlugin>),
}

struct PluginLoadProgress {
    current: AtomicUsize,
    total: usize,
}

struct PluginLoadTask {
    receiver: Receiver<PluginLoadCompletion>,
    cancel: Arc<AtomicBool>,
    progress: Arc<PluginLoadProgress>,
}

struct PluginLoadCompletion {
    result: Result<LoadedPlugin, String>,
    added_path: Option<String>,
    preset: Option<Preset>,
    preset_name: Option<String>,
    preset_paths: Option<Vec<String>>,
}

#[derive(Clone, Debug)]
pub(crate) struct PluginEffectState {
    pub(crate) name: String,
    pub(crate) bypassed: bool,
}

pub struct PluginEditor {
    plugin_paths: Vec<String>,
    rack: PluginRack,
    runtime: PluginRuntime,
    engine: Option<AudioEngine>,
    active_audio_config: Option<AudioConfig>,
    error: Option<String>,
    state: EditorState,
    settings: Settings,
    plugin_picker_open: bool,
    plugin_picker_search: String,
    plugin_picker_effect_types: Vec<String>,
    selected_plugin_path: Option<String>,
    discovered_plugins: Vec<DiscoveredPlugin>,
    load_requested: bool,
    pending_preset: Option<Preset>,
    pending_preset_name: Option<String>,
    loaded_preset_name: Option<String>,
    load_after_picker_close: bool,
    pending_add_path: Option<String>,
    plugin_load_task: Option<PluginLoadTask>,
    drop_zone_hovered: Option<usize>,
    playback: Arc<PlaybackState>,
    playback_clip: Option<Arc<AudioClip>>,
    playback_file: Option<PathBuf>,
    playback_collapsed: bool,
    show_playback: bool,
    tuner: Arc<TunerState>,
    tuner_settings: TunerSettings,
    tuner_collapsed: bool,
    show_tuner: bool,
    midi_feedback: Consumer<MidiEvent>,
    midi_connection: Option<MidiInputConnection>,
    midi_connection_target: Option<String>,
    midi_connection_error: Option<String>,
    midi_device_snapshot: Vec<String>,
    midi_devices_revision: u64,
    midi_learn_plugin: Option<usize>,
    pending_program_changes: Vec<(u8, u8)>,
}

impl Default for PluginEditor {
    fn default() -> Self {
        let mut runtime = PluginRuntime::spawn();
        Self {
            plugin_paths: default_plugin_paths(),
            rack: PluginRack::default(),
            midi_feedback: runtime.take_midi_feedback(),
            runtime,
            engine: None,
            active_audio_config: None,
            error: None,
            state: EditorState::Unloaded,
            settings: Settings::default(),
            plugin_picker_open: false,
            plugin_picker_search: String::new(),
            plugin_picker_effect_types: Vec::new(),
            selected_plugin_path: None,
            discovered_plugins: Vec::new(),
            load_requested: false,
            pending_preset: None,
            pending_preset_name: None,
            loaded_preset_name: None,
            load_after_picker_close: false,
            pending_add_path: None,
            plugin_load_task: None,
            drop_zone_hovered: None,
            playback: Arc::new(PlaybackState::new()),
            playback_clip: None,
            playback_file: None,
            playback_collapsed: true,
            show_playback: true,
            tuner: Arc::new(TunerState::new()),
            tuner_settings: TunerSettings::default(),
            tuner_collapsed: true,
            show_tuner: true,
            midi_connection: None,
            midi_connection_target: None,
            midi_connection_error: None,
            midi_device_snapshot: Vec::new(),
            midi_devices_revision: 0,
            midi_learn_plugin: None,
            pending_program_changes: Vec::new(),
        }
    }
}

impl PluginEditor {
    pub fn from_settings(settings: &AppSettings) -> Self {
        let mut editor = Self::default();
        editor.settings = Settings::from_app_settings(settings);
        editor.tuner_settings = settings.tuner.clone().normalized();
        editor.apply_tuner_settings();
        editor.show_playback = settings.show_playback;
        editor.show_tuner = settings.show_tuner;
        editor
    }

    pub(super) fn edit_state_label(&self) -> &str {
        self.state.label()
    }

    pub(super) fn set_discovered_plugins(&mut self, plugins: Vec<DiscoveredPlugin>) {
        self.discovered_plugins = plugins;
    }

    pub fn audio_settings(&self) -> AudioSettings {
        self.settings.audio_settings()
    }

    pub fn midi_settings(&self) -> MidiSettings {
        self.settings.midi_settings()
    }

    pub fn plugin_discovery_settings(&self) -> super::settings::PluginDiscoverySettings {
        self.settings.plugin_discovery_settings()
    }

    pub(super) fn theme(&self) -> &Theme {
        &self.settings.theme
    }

    pub fn tuner_settings(&self) -> TunerSettings {
        self.tuner_settings.clone()
    }

    fn apply_tuner_settings(&self) {
        self.tuner
            .set_input_sensitivity_db(self.tuner_settings.input_sensitivity_db);
        self.tuner.set_a4_pitch_hz(self.tuner_settings.a4_pitch_hz);
        self.tuner
            .set_transposition_semitones(self.tuner_settings.transposition_semitones);
        self.tuner
            .set_tuning_preset(self.tuner_settings.tuning_preset);
    }

    pub(crate) fn is_playback_visible(&self) -> bool {
        self.show_playback
    }

    pub(crate) fn set_show_playback(&mut self, show_playback: bool) {
        self.show_playback = show_playback;
    }

    pub(crate) fn is_tuner_visible(&self) -> bool {
        self.show_tuner
    }

    pub(crate) fn set_show_tuner(&mut self, show_tuner: bool) {
        self.show_tuner = show_tuner;
    }

    pub(super) fn process_pending_load(&mut self) {
        if self.plugin_load_task.is_some() {
            let result = self
                .plugin_load_task
                .as_ref()
                .expect("plugin load task disappeared")
                .receiver
                .try_recv();
            match result {
                Ok(completion) => {
                    self.plugin_load_task = None;
                    self.finish_plugin_load_completion(completion);
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.plugin_load_task = None;
                    self.state = if self.rack.slots.is_empty() {
                        EditorState::Error
                    } else {
                        EditorState::Loaded
                    };
                    self.error = Some("Plugin loading stopped unexpectedly.".to_owned());
                }
            }
            return;
        }

        if !self.load_requested {
            return;
        }

        self.load_requested = false;
        self.state = EditorState::Loading;
        let pending_preset = self.pending_preset.take();
        let added_path = self.pending_add_path.take();
        let preset_name = self.pending_preset_name.take();
        self.start_plugin_load_task(added_path, pending_preset, preset_name);
    }

    pub(super) fn is_loading(&self) -> bool {
        matches!(self.state, EditorState::Loading)
            || self.load_requested
            || self.plugin_load_task.is_some()
    }

    pub(super) fn show_loading(&mut self, ctx: &egui::Context) {
        let (current, total, cancelling) = self
            .plugin_load_task
            .as_ref()
            .map(|task| {
                (
                    task.progress.current.load(Ordering::Acquire),
                    task.progress.total,
                    task.cancel.load(Ordering::Acquire),
                )
            })
            .unwrap_or((0, 0, false));
        let subtitle = if cancelling {
            "Cancelling plugin load...".to_owned()
        } else if total == 0 {
            "Preparing plugin load...".to_owned()
        } else {
            format!("Loading plugin {current} of {total}...")
        };
        if super::loading::show(
            ctx,
            &self.settings.theme,
            &subtitle,
            current,
            total,
            !cancelling,
        ) {
            self.cancel_plugin_load();
        }
    }

    fn cancel_plugin_load(&self) {
        if let Some(task) = &self.plugin_load_task {
            log::info!("cancelling plugin load");
            task.cancel.store(true, Ordering::Release);
        }
    }

    fn start_plugin_load_task(
        &mut self,
        added_path: Option<String>,
        preset: Option<Preset>,
        preset_name: Option<String>,
    ) {
        let sample_rate = self
            .settings
            .output_index
            .and_then(|index| AudioDevices::sample_rate(self.settings.backend, index).ok());
        let block_size = self.settings.buffer_size.frames() as usize;
        let (paths, expected_fingerprints) = if let Some(preset) = &preset {
            (
                preset
                    .plugins
                    .iter()
                    .map(|plugin| plugin.path.clone())
                    .collect::<Vec<_>>(),
                preset
                    .plugins
                    .iter()
                    .map(|plugin| plugin.fingerprint)
                    .collect::<Option<Vec<_>>>(),
            )
        } else if let Some(path) = &added_path {
            (vec![path.clone()], None)
        } else {
            (self.plugin_paths.clone(), None)
        };
        let total = if added_path.is_some() {
            1
        } else {
            paths.iter().filter(|path| !path.trim().is_empty()).count()
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(PluginLoadProgress {
            current: AtomicUsize::new(0),
            total,
        });
        let (sender, receiver) = mpsc::channel();
        let thread_cancel = Arc::clone(&cancel);
        let thread_progress = Arc::clone(&progress);
        let thread_preset = preset;
        let thread_preset_name = preset_name;
        let thread_added_path = added_path;
        let preset_paths = thread_preset.as_ref().map(|_| paths.clone());
        let worker = thread::Builder::new()
            .name("rack-plugin-loader".to_owned())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if thread_cancel.load(Ordering::Acquire) {
                        return Err(PLUGIN_LOAD_CANCELLED.to_owned());
                    }
                    if let Some(path) = paths.first()
                        && paths.len() == 1
                        && thread_added_path.is_some()
                    {
                        thread_progress.current.store(1, Ordering::Release);
                        load_plugin(path, sample_rate, block_size)
                            .map(|plugin| LoadedPlugin::Single(Box::new(plugin)))
                    } else {
                        load_stack_with_fingerprints_progress(
                            &paths,
                            expected_fingerprints.as_deref(),
                            sample_rate,
                            block_size,
                            || thread_cancel.load(Ordering::Acquire),
                            |current, _total| {
                                thread_progress.current.store(current, Ordering::Release);
                            },
                        )
                        .map(LoadedPlugin::Stack)
                    }
                }))
                .unwrap_or_else(|panic| {
                    let message = panic
                        .downcast_ref::<&str>()
                        .copied()
                        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                        .unwrap_or("unknown plugin loader panic");
                    Err(format!("plugin loader panicked: {message}"))
                });
                let _ = sender.send(PluginLoadCompletion {
                    result,
                    added_path: thread_added_path,
                    preset: thread_preset,
                    preset_name: thread_preset_name,
                    preset_paths,
                });
            });
        match worker {
            Ok(_) => {
                self.plugin_load_task = Some(PluginLoadTask {
                    receiver,
                    cancel,
                    progress,
                });
            }
            Err(error) => {
                self.state = if self.rack.slots.is_empty() {
                    EditorState::Error
                } else {
                    EditorState::Loaded
                };
                self.error = Some(format!("Could not start plugin loader: {error}"));
            }
        }
    }

    fn finish_plugin_load_completion(&mut self, completion: PluginLoadCompletion) {
        if let Some(paths) = completion.preset_paths {
            match completion.result {
                Ok(LoadedPlugin::Stack(loaded)) => {
                    self.plugin_paths = paths;
                    self.finish_plugin_load_with_preset(
                        loaded,
                        completion.preset.as_ref().expect("preset missing"),
                        completion.preset_name.as_deref(),
                    );
                }
                Ok(LoadedPlugin::Single(_)) => {
                    self.finish_load_error("plugin loader returned an invalid preset result");
                }
                Err(error) if error == PLUGIN_LOAD_CANCELLED => self.finish_load_error(&error),
                Err(error) => self.finish_load_error(&format!("Could not load preset: {error}")),
            }
        } else {
            self.finish_plugin_load(completion.result, completion.added_path);
        }
    }

    fn finish_load_error(&mut self, error: &str) {
        let cancelled = error == PLUGIN_LOAD_CANCELLED;
        self.state = if self.rack.slots.is_empty() {
            if cancelled {
                EditorState::Empty
            } else {
                EditorState::Error
            }
        } else {
            EditorState::Loaded
        };
        self.error = if cancelled {
            Some("Plugin load cancelled.".to_owned())
        } else {
            Some(error.to_owned())
        };
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

    pub(crate) fn set_loaded_preset_name(&mut self, name: impl Into<String>) {
        self.loaded_preset_name = Some(name.into());
    }

    pub(crate) fn clear_loaded_preset_name(&mut self) {
        self.loaded_preset_name = None;
    }

    pub(crate) fn live_effects(&self) -> Vec<PluginEffectState> {
        self.rack
            .slots
            .iter()
            .enumerate()
            .map(|(index, slot)| PluginEffectState {
                name: slot.snapshot.name.clone(),
                bypassed: self.plugin_bypassed(index),
            })
            .collect()
    }

    pub(crate) fn live_tuner_readout(&self) -> TunerReadout {
        self.tuner.read()
    }

    pub(crate) fn live_tuner_bypassed(&self) -> bool {
        self.tuner.is_bypassed()
    }

    pub(crate) fn set_error(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
    }

    pub(crate) fn take_program_changes(&mut self) -> Vec<(u8, u8)> {
        std::mem::take(&mut self.pending_program_changes)
    }

    pub fn push_plugin_path(&mut self, path: impl Into<String>) -> bool {
        if self.plugin_paths.len() >= MAX_PLUGIN_PATHS {
            return false;
        }
        self.plugin_paths.push(path.into());
        true
    }

    pub fn has_plugin_stack(&self) -> bool {
        !self.rack.slots.is_empty() || self.plugin_paths.iter().any(|path| !path.trim().is_empty())
    }

    pub(crate) fn has_loaded_plugin_stack(&self) -> bool {
        !self.rack.slots.is_empty()
    }

    pub(crate) fn capture_preset(&mut self) -> Result<Preset, String> {
        let configured_paths = self
            .plugin_paths
            .iter()
            .filter(|path| !path.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>();
        let mut snapshots = Vec::with_capacity(self.rack.slots.len());
        for (index, slot) in self.rack.slots.iter().enumerate() {
            let mut plugin = slot
                .instance
                .lock()
                .map_err(|_| format!("Could not read plugin {index}: plugin lock poisoned"))?;
            snapshots.push(
                plugin_snapshot(&mut plugin)
                    .map_err(|error| format!("Could not read plugin {index}: {error}"))?,
            );
        }
        let bypassed = self
            .rack
            .slots
            .iter()
            .map(|slot| slot.bypass_state.load(Ordering::Relaxed))
            .collect::<Vec<_>>();
        preset_from_snapshots(
            &configured_paths,
            &snapshots,
            &bypassed,
            &self
                .rack
                .slots
                .iter()
                .map(|slot| slot.midi_mappings.clone())
                .collect::<Vec<_>>(),
        )
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
        self.cancel_plugin_load();
        self.load_requested = false;
        self.pending_preset = None;
        self.pending_preset_name = None;
        self.load_after_picker_close = false;
        self.pending_add_path = None;
        self.plugin_picker_open = false;
        self.selected_plugin_path = None;
        self.drop_zone_hovered = None;
        self.loaded_preset_name = None;

        while let Some(plugin_index) = self.rack.slots.len().checked_sub(1) {
            let plugin_count = self.rack.slots.len();
            self.remove_plugin(plugin_index);
            if self.rack.slots.len() == plugin_count {
                return;
            }
        }

        self.plugin_paths.clear();
        self.error = None;
        self.state = EditorState::Empty;
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        heading(ui, "VST Plugin Rack");
        let plugin_latency_samples = self
            .rack
            .slots
            .iter()
            .filter_map(|slot| slot.instance.try_lock().ok())
            .map(|plugin| f64::from(plugin.latency_samples()))
            .sum();
        if self
            .settings
            .show_configuration(ui, &mut self.error, plugin_latency_samples)
        {
            self.apply_audio_devices();
        } else if self.rack.slots.is_empty()
            && self.engine.is_none()
            && matches!(self.state, EditorState::Unloaded)
        {
            self.start_audio();
        }

        self.sync_midi_connection();
        self.poll_midi_feedback();

        if let Some(error) = &self.error {
            error_label(ui, error);
        }
        if let Some(error) = &self.midi_connection_error {
            error_label(ui, error);
        }
        if self.settings.midi_enabled && self.midi_connection_error.is_none() {
            muted(
                ui,
                egui::RichText::new(if self.midi_connection.is_some() {
                    "MIDI input connected"
                } else {
                    "MIDI input is enabled but not connected"
                })
                .small(),
            );
        }
        ui.add_space(12.0);
        self.show_plugin_rack(ui);
    }

    fn show_add_plugin_button(&mut self, ui: &mut egui::Ui) {
        let mut open = self.plugin_picker_open;
        let mut search = self.plugin_picker_search.clone();
        let mut selected_effect_types = self.plugin_picker_effect_types.clone();
        let mut selected = self.selected_plugin_path.clone();
        let mut path_to_add = None;
        let theme = &self.settings.theme;
        let sheet_id = ui.make_persistent_id("plugin-picker-sheet");

        sheet(
            ui,
            SheetProps::new(sheet_id, &mut open).side(SheetSide::Bottom),
            |ui, context| {
                let trigger_width = ui.available_width();
                let add_icon = |painter: &egui::Painter,
                                center: egui::Pos2,
                                size: f32,
                                color: egui::Color32| {
                    paint_lucide_icon(painter, center, size, color, Icon::Plus);
                };
                sheet_trigger(ui, context, |ui| {
                    show_tall_outline_icon_button(ui, theme, trigger_width, &add_icon)
                })
                .clicked();

                let mut should_close = false;
                let _ = sheet_content_bottom_with_height(ui, theme, context, 520.0, |content_ui| {
                    content_ui.spacing_mut().item_spacing.y = 8.0;
                    sheet_header(content_ui, |header_ui| {
                        sheet_title(header_ui, theme, "Add an effect");
                        sheet_description(
                            header_ui,
                            theme,
                            "Choose a discovered VST3 or VST2 plugin.",
                        );
                    });

                    let search_id = content_ui.make_persistent_id("plugin-picker-search");
                    let inner_width = content_ui.available_width() - 32.0;
                    egui::Frame::NONE
                        .inner_margin(egui::Margin::symmetric(16, 0))
                        .show(content_ui, |input_ui| {
                            search_input(
                                input_ui,
                                theme,
                                search_id,
                                inner_width - 8.0,
                                "Search by plugin name or category",
                                |painter, rect, color| {
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
                                },
                                &mut search,
                            );
                        });

                    let mut available_effect_types = self
                        .discovered_plugins
                        .iter()
                        .map(|plugin| effect_type(&plugin.category))
                        .collect::<Vec<_>>();
                    available_effect_types
                        .sort_by_cached_key(|effect_type| effect_type.to_ascii_lowercase());
                    available_effect_types.dedup();
                    selected_effect_types.retain(|effect_type| {
                        available_effect_types
                            .iter()
                            .any(|available| available == effect_type)
                    });

                    show_effect_type_filter(
                        content_ui,
                        theme,
                        &available_effect_types,
                        &mut selected_effect_types,
                    );

                    let query = search.trim().to_ascii_lowercase();
                    let mut categories: Vec<(PluginFormat, &str, Vec<&DiscoveredPlugin>)> =
                        Vec::new();
                    for plugin in &self.discovered_plugins {
                        if !selected_effect_types.is_empty()
                            && !selected_effect_types.contains(&effect_type(&plugin.category))
                        {
                            continue;
                        }
                        if !query.is_empty()
                            && !plugin.name.to_ascii_lowercase().contains(&query)
                            && !plugin.path.to_ascii_lowercase().contains(&query)
                            && !plugin.format.label().to_ascii_lowercase().contains(&query)
                            && !category_segments(&plugin.category)
                                .join(" ")
                                .to_ascii_lowercase()
                                .contains(&query)
                        {
                            continue;
                        }
                        if let Some((_, _, plugins)) =
                            categories.iter_mut().find(|(format, category, _)| {
                                *format == plugin.format && *category == plugin.category.as_str()
                            })
                        {
                            plugins.push(plugin);
                        } else {
                            categories.push((
                                plugin.format,
                                plugin.category.as_str(),
                                vec![plugin],
                            ));
                        }
                    }
                    categories.sort_by_cached_key(|(format, category, _)| {
                        (
                            *format,
                            category_segments(category).join(" > ").to_lowercase(),
                        )
                    });
                    for (_, _, plugins) in &mut categories {
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
                                    .max_height(360.0)
                                    .show(group_ui, |list_ui| {
                                        if categories.is_empty() {
                                            list_ui.label("No matching plugins found.");
                                        }
                                        for (format, category, plugins) in categories {
                                            list_ui.add_space(4.0);
                                            let mut category_path = vec![format.label().to_owned()];
                                            category_path.extend(category_segments(category));
                                            breadcrumb(list_ui, theme, &category_path);
                                            for plugin in plugins {
                                                let is_selected = selected.as_deref()
                                                    == Some(plugin.path.as_str());
                                                let variant = if is_selected {
                                                    ButtonType::Primary
                                                } else {
                                                    ButtonType::Ghost
                                                };
                                                if UiButton::new(plugin.name.clone())
                                                    .button_type(variant)
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
                                if UiButton::new("Add effect")
                                    .button_type(ButtonType::Primary)
                                    .size(ButtonSize::Sm)
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
                    context.set_open(false);
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
        self.plugin_picker_effect_types = selected_effect_types;
        self.selected_plugin_path = selected;

        if self.load_after_picker_close
            && !open
            && ui.ctx().animate_bool(sheet_id.with("open-anim"), false) <= 0.0
        {
            self.load_after_picker_close = false;
            if self.engine.is_some() || !self.rack.slots.is_empty() {
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

    fn finish_plugin_load(
        &mut self,
        result: Result<LoadedPlugin, String>,
        added_path: Option<String>,
    ) {
        let loaded = match result {
            Ok(plugins) => plugins,
            Err(error) => {
                self.rollback_added_path(added_path.as_deref());
                if error == PLUGIN_LOAD_CANCELLED {
                    self.finish_load_error(&error);
                    return;
                }
                self.state = if self.rack.slots.is_empty() {
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
                self.complete_incremental_add(*plugin, added_path.as_deref())
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
        let index = self.rack.slots.len();
        let plugin = Arc::new(Mutex::new(plugin));
        let vst2_editor = match plugin.lock() {
            Ok(mut plugin) => plugin.take_editor(),
            Err(_) => {
                self.rollback_added_path(added_path);
                self.state = if self.rack.slots.is_empty() {
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
                    self.state = if self.rack.slots.is_empty() {
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
                self.state = if self.rack.slots.is_empty() {
                    EditorState::Error
                } else {
                    EditorState::Loaded
                };
                self.error = Some("Could not read added plugin: plugin lock poisoned".to_owned());
                return;
            }
        };

        let bypass_state = Arc::new(AtomicBool::new(false));
        let audio_was_running = self.stop_audio_for_plugin_change();
        if let Err(error) =
            self.runtime
                .handle()
                .add_plugin(index, plugin.clone(), bypass_state.clone())
        {
            self.restart_audio_after_plugin_change(audio_was_running);
            self.rollback_added_path(added_path);
            self.state = if self.rack.slots.is_empty() {
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
        self.rack.slots.push(PluginSlot {
            path: added_path.unwrap_or_default().to_owned(),
            instance: plugin,
            editor_window: window,
            snapshot,
            bypass_state,
            midi_mappings: Vec::new(),
            collapsed: true,
        });
        self.sync_midi_mappings();
        self.restart_audio_after_plugin_change(audio_was_running);
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
        self.cancel_midi_learn();
        for slot in &mut self.rack.slots {
            slot.editor_window.close();
        }
        self.rack.slots.clear();
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
            let instances = self
                .rack
                .slots
                .iter()
                .map(|slot| Arc::clone(&slot.instance))
                .collect();
            let bypass_states = self
                .rack
                .slots
                .iter()
                .map(|slot| Arc::clone(&slot.bypass_state))
                .collect();
            if let Err(error) = self
                .runtime
                .handle()
                .install_plugins(instances, bypass_states)
            {
                self.state = EditorState::Error;
                self.error = Some(error);
            } else {
                self.sync_midi_mappings();
                self.start_audio();
                self.loaded_preset_name = preset_name.map(str::to_owned);
            }
        }
    }

    fn apply_preset_state(&mut self, preset: &Preset) -> Result<(), String> {
        let mut errors = Vec::new();
        for (slot, saved_plugin) in self.rack.slots.iter_mut().zip(&preset.plugins) {
            slot.midi_mappings = saved_plugin.midi_mappings.clone();
        }
        for (plugin_index, (plugin, saved_plugin)) in
            self.rack.slots.iter().zip(&preset.plugins).enumerate()
        {
            match plugin.instance.lock() {
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
            plugin
                .bypass_state
                .store(saved_plugin.bypassed, Ordering::Relaxed);
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
        if self.rack.slots.is_empty() {
            self.request_load();
        } else {
            self.restart_audio();
        }
    }

    pub fn shutdown(&mut self, mode: ShutdownMode) {
        self.cancel_plugin_load();
        self.load_requested = false;
        self.pending_preset = None;
        self.pending_preset_name = None;
        self.pending_add_path = None;
        self.midi_connection.take();
        self.midi_learn_plugin = None;
        for slot in &mut self.rack.slots {
            slot.editor_window.close();
        }
        if matches!(mode, ShutdownMode::KeepPluginsLoaded) {
            std::mem::forget(std::mem::take(&mut self.rack.slots));
        } else {
            self.rack.slots.clear();
        }
        if let Some(engine) = self.engine.take() {
            engine.shutdown(mode);
        }
        self.active_audio_config = None;
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
        let paths = self
            .plugin_paths
            .iter()
            .filter(|path| !path.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>();
        if paths.len() != plugins.len() {
            return Err("configured plugin paths and loaded plugins are out of sync".to_owned());
        }
        self.rack.slots = plugins
            .into_iter()
            .zip(paths)
            .zip(vst2_editors)
            .zip(snapshots)
            .map(|(((instance, path), vst2_editor), snapshot)| {
                let editor_window = instance
                    .lock()
                    .map(|plugin| EditorWindow::new(&plugin, vst2_editor))
                    .unwrap_or(EditorWindow::None);
                PluginSlot {
                    path,
                    instance,
                    editor_window,
                    snapshot,
                    bypass_state: Arc::new(AtomicBool::new(false)),
                    midi_mappings: Vec::new(),
                    collapsed: true,
                }
            })
            .collect();
        self.state = EditorState::Loaded;
        Ok(())
    }

    fn remove_plugin(&mut self, plugin_index: usize) {
        if plugin_index >= self.rack.slots.len() {
            return;
        }

        let plugin_name = self.rack.slots[plugin_index].snapshot.name.clone();
        log::info!("removing plugin {plugin_name} from the stack");

        let slot_path = self.rack.slots[plugin_index].path.clone();
        let Some(path_index) = self
            .plugin_paths
            .iter()
            .enumerate()
            .find(|(_, path)| *path == &slot_path)
            .map(|(index, _)| index)
        else {
            return;
        };

        self.rack.slots[plugin_index].editor_window.close();
        let audio_was_running = self.stop_audio_for_plugin_change();
        if let Err(error) = self.runtime.handle().remove_plugin(plugin_index) {
            self.restart_audio_after_plugin_change(audio_was_running);
            self.state = EditorState::Error;
            self.error = Some(format!("Could not remove plugin from the runtime: {error}"));
            return;
        }

        self.plugin_paths.remove(path_index);
        self.rack.slots.remove(plugin_index);
        self.sync_midi_mappings();
        self.restart_audio_after_plugin_change(audio_was_running);
        self.error = None;
        self.state = EditorState::Loaded;
    }

    fn reorder_plugin(&mut self, from: usize, to: usize) {
        if from >= self.rack.slots.len() || to > self.rack.slots.len() {
            return;
        }

        let audio_was_running = self.stop_audio_for_plugin_change();
        if let Err(error) = self.runtime.handle().move_plugin(from, to) {
            self.restart_audio_after_plugin_change(audio_was_running);
            self.state = EditorState::Error;
            self.error = Some(format!("Could not reorder plugin stack: {error}"));
            return;
        }

        move_item(&mut self.rack.slots, from, to);
        self.sync_midi_mappings();

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
        self.restart_audio_after_plugin_change(audio_was_running);
        self.error = None;
        self.state = EditorState::Loaded;
    }

    fn open_native_editor(&mut self, plugin_index: usize) {
        let Some(slot) = self.rack.slots.get_mut(plugin_index) else {
            return;
        };
        if let Err(error) = slot.editor_window.open() {
            self.state = EditorState::Error;
            self.error = Some(format!(
                "Could not open {} editor: {error}",
                slot.snapshot.name
            ));
        }
    }

    fn plugin_bypassed(&self, plugin_index: usize) -> bool {
        self.rack
            .slots
            .get(plugin_index)
            .map(|slot| slot.bypass_state.load(Ordering::Relaxed))
            .unwrap_or(false)
    }

    fn toggle_plugin_bypass(&mut self, plugin_index: usize) {
        let bypassed = self.plugin_bypassed(plugin_index);
        if let Some(slot) = self.rack.slots.get(plugin_index) {
            slot.bypass_state.store(!bypassed, Ordering::Relaxed);
        }
    }

    fn set_parameter(&mut self, plugin_index: usize, parameter_id: u32, value: f64) {
        let result = self
            .rack
            .slots
            .get(plugin_index)
            .ok_or_else(|| format!("plugin {plugin_index} is not available"))
            .and_then(|slot| {
                slot.instance
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
                self.rack.slots[plugin_index].snapshot.name
            ));
        }
    }

    fn refresh_parameters(&mut self) {
        for plugin_index in 0..self.rack.slots.len() {
            let mut plugin = match self.rack.slots[plugin_index].instance.try_lock() {
                Ok(plugin) => plugin,
                Err(std::sync::TryLockError::Poisoned(poison)) => poison.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => continue,
            };
            let snapshot = plugin_snapshot(&mut plugin);
            drop(plugin);
            match snapshot {
                Ok(snapshot) => {
                    self.rack.slots[plugin_index].snapshot.parameters = snapshot.parameters;
                }
                Err(error) => {
                    self.error = Some(format!(
                        "Could not read {} parameters: {error}",
                        self.rack.slots[plugin_index].snapshot.name
                    ));
                }
            }
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
            Self::Unloaded => "Unloaded",
            Self::Empty => "Empty",
            Self::Loading => "Loading",
            Self::Loaded => "Loaded",
            Self::AudioUnavailable => "Loaded (audio unavailable)",
            Self::Error => "Error",
        }
    }
}
