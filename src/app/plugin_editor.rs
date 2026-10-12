use super::audio::tuner::{
    MAX_A4_PITCH_HZ, MAX_INPUT_SENSITIVITY_DB, MAX_TRANSPOSITION_SEMITONES, MIN_A4_PITCH_HZ,
    MIN_INPUT_SENSITIVITY_DB, MIN_TRANSPOSITION_SEMITONES, TuningPreset,
};
use super::audio::{
    AudioClip, AudioConfig, AudioDevices, AudioEngine, PlaybackState, ShutdownMode, TunerReadout,
    TunerState, decode_file,
};
use super::midi::PresetMidiMapping;
#[cfg(windows)]
use super::plugin::WindowsStaApartment;
use super::plugin::{
    DiscoveredPlugin, EditorWindow, PLUGIN_LOAD_CANCELLED, PluginFormat, PluginIdentity,
    RackPlugin, load_plugin, load_stack_with_fingerprints_progress,
};
use super::plugin_runtime::{PluginRuntime, PluginRuntimeStatus, PluginSnapshot, plugin_snapshot};
use super::presets::{Preset, preset_from_snapshots, validate_plugin_installations};
use super::settings::{
    AppSettings, AudioSettings, MidiEffectSettingsRow, MidiSettings, Settings, TunerSettings,
};
use super::ui::{
    Button as UiButton, ButtonJustify, ButtonSize, ButtonType, Label, Select, SelectItem,
    SelectSize, SheetProps, SheetSide, Theme, breadcrumb, error_label, heading, label as ui_label,
    muted, paint as paint_label, search_input, sheet, sheet_content_bottom_with_height,
    sheet_description, sheet_footer, sheet_header, sheet_title, sheet_trigger, strong,
};
use lucide_icons::Icon;
use rtrb::Consumer;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use vst3_host::MidiEvent;
use vst3_host::midi_input::MidiInputConnection;

mod audio_session;
mod effect_filter;
mod formatting;
mod midi;
mod model;
mod native_editors;
mod parameter_worker;
mod view_helpers;
mod views;

use effect_filter::show_effect_type_filter;
use formatting::{
    balanced_column_ranges, boolean_parameter_value, category_segments, effect_type, format_cents,
    format_custom_editor_float, format_time, group_parameter_indices, is_booleanish_parameter,
    move_item, tuner_note,
};
use model::{PluginRack, PluginSlot};
use parameter_worker::{PluginParameterResult, PluginParameterWorker};
use view_helpers::{paint_lucide_icon, show_plugin_drag_preview, show_tall_outline_icon_button};

const MAX_PLUGIN_PATHS: usize = 16;
const CUSTOM_PLUGIN_EDITOR_HEIGHT: f32 = 256.0;
const COLLAPSED_PLUGIN_EDITOR_HEIGHT: f32 = 32.0;
const PLAYBACK_EDITOR_HEIGHT: f32 = 176.0;
const TUNER_COLLAPSED_EDITOR_HEIGHT: f32 = 60.0;
const CUSTOM_PLUGIN_EDITOR_ROW_HEIGHT: f32 = 24.0;
const CUSTOM_PLUGIN_EDITOR_COLUMN_COUNT: usize = 3;
const PARAMETER_REFRESH_INTERVAL: Duration = Duration::from_millis(200);
const PLUGIN_LOAD_STALL_TIMEOUT: Duration = Duration::from_secs(30);

fn default_plugin_paths() -> Vec<String> {
    Vec::with_capacity(MAX_PLUGIN_PATHS)
}

fn clap_plugin_snapshot(plugin: &mut super::plugin::ClapEffect) -> Result<PluginSnapshot, String> {
    let info = plugin.info();
    let has_editor = plugin.has_editor();
    let latency_samples = plugin.latency_samples();
    let parameters = plugin
        .parameters()?
        .into_iter()
        .map(|parameter| super::plugin_runtime::ParameterSnapshot {
            id: parameter.id,
            name: parameter.name,
            category: parameter.category,
            min: parameter.min,
            max: parameter.max,
            value: parameter.value,
            step_count: parameter.step_count,
            choices: parameter.choices,
            is_read_only: parameter.is_read_only,
            formatted_value: parameter.formatted_value,
        })
        .collect();
    Ok(PluginSnapshot {
        name: info.name,
        has_editor,
        latency_samples,
        parameters,
    })
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
    Stack(Vec<(RackPlugin, PluginSnapshot)>),
    Single(Box<RackPlugin>, PluginSnapshot),
}

enum LoadedEffect {
    Vst3(RackPlugin, PluginSnapshot),
    Clap(super::plugin::ClapEffect, PluginSnapshot),
}

struct PluginLoadProgress {
    current: AtomicUsize,
    total: usize,
}

struct PluginLoadTask {
    receiver: Receiver<PluginLoadCompletion>,
    cancel: Arc<AtomicBool>,
    progress: Arc<PluginLoadProgress>,
    paths: Vec<String>,
    added_path: Option<String>,
    last_progress: usize,
    last_progress_at: Instant,
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
    last_parameter_refresh: Instant,
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
    midi_learn_parameter: Option<u32>,
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
            last_parameter_refresh: Instant::now(),
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
            midi_learn_parameter: None,
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
            let result = {
                let task = self
                    .plugin_load_task
                    .as_mut()
                    .expect("plugin load task disappeared");
                let progress = task.progress.current.load(Ordering::Acquire);
                if progress != task.last_progress {
                    task.last_progress = progress;
                    task.last_progress_at = Instant::now();
                }
                task.receiver.try_recv()
            };
            match result {
                Ok(completion) => {
                    self.plugin_load_task = None;
                    self.finish_plugin_load_completion(completion);
                }
                Err(TryRecvError::Empty) => {
                    let timed_out = self.plugin_load_task.as_ref().and_then(|task| {
                        (task.last_progress_at.elapsed() >= PLUGIN_LOAD_STALL_TIMEOUT).then(|| {
                            let plugin_index = task.last_progress.saturating_sub(1);
                            let path = task
                                .paths
                                .get(plugin_index)
                                .or_else(|| task.paths.first())
                                .cloned()
                                .unwrap_or_else(|| "plugin stack".to_owned());
                            (path, task.added_path.clone())
                        })
                    });
                    if let Some((path, added_path)) = timed_out {
                        if let Some(task) = self.plugin_load_task.take() {
                            task.cancel.store(true, Ordering::Release);
                        }
                        if let Some(path) = added_path.as_deref() {
                            self.rollback_added_path(Some(path));
                        }
                        let error = format!(
                            "Loading {path} made no progress for {} seconds. Its native plug-in call may be blocked; Rack has released the loading screen but cannot safely cancel an in-process native call.",
                            PLUGIN_LOAD_STALL_TIMEOUT.as_secs()
                        );
                        log::error!("{error}");
                        self.state = if self.rack.slots.is_empty() {
                            EditorState::Error
                        } else {
                            EditorState::Loaded
                        };
                        self.error = Some(error);
                    }
                }
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

    fn load_clap_effect(&mut self, config_key: &str, added_path: Option<&str>) {
        self.state = EditorState::Loading;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let identity = PluginIdentity::from_config_key(config_key);
            let mut effect = super::plugin::ClapEffect::load(identity)?;
            let snapshot = clap_plugin_snapshot(&mut effect)?;
            Ok::<_, String>((effect, snapshot))
        }))
        .unwrap_or_else(|panic| {
            let message = panic
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown CLAP plugin loader panic");
            Err(format!("CLAP plugin loader panicked: {message}"))
        });
        match result {
            Ok((effect, snapshot)) => {
                self.complete_incremental_add_clap(effect, snapshot, added_path);
            }
            Err(error) => {
                self.rollback_added_path(added_path);
                self.finish_load_error(&format!("Could not load CLAP effect: {error}"));
            }
        }
    }

    fn load_preset_with_clap(&mut self, preset: Preset, preset_name: Option<&str>) {
        if let Err(error) = validate_plugin_installations(&preset) {
            self.finish_load_error(&format!("Could not load preset: {error}"));
            return;
        }
        let sample_rate = self
            .settings
            .output_index
            .and_then(|index| AudioDevices::sample_rate(self.settings.backend, index).ok());
        let block_size = self.settings.buffer_size.frames() as usize;
        #[cfg(windows)]
        let _ole_apartment = match WindowsStaApartment::initialize() {
            Ok(apartment) => Some(apartment),
            Err(error) => {
                self.finish_load_error(&format!(
                    "Could not initialize plugin OLE apartment: {error}"
                ));
                return;
            }
        };
        let loaded = preset
            .plugins
            .iter()
            .enumerate()
            .map(|(index, preset_plugin)| {
                let identity = preset_plugin
                    .identity
                    .clone()
                    .unwrap_or_else(|| PluginIdentity::from_config_key(&preset_plugin.path));
                match identity.format {
                    PluginFormat::Clap => {
                        let mut effect = super::plugin::ClapEffect::load(identity)?;
                        let snapshot = clap_plugin_snapshot(&mut effect)?;
                        Ok(LoadedEffect::Clap(effect, snapshot))
                    }
                    PluginFormat::Vst3 => {
                        let mut plugin =
                            load_plugin(&identity.bundle_path, sample_rate, block_size)
                                .map_err(|error| format!("plugin {index}: {error}"))?;
                        let snapshot = plugin_snapshot(&mut plugin)
                            .map_err(|error| format!("plugin {index}: {error}"))?;
                        Ok(LoadedEffect::Vst3(plugin, snapshot))
                    }
                }
            })
            .collect::<Result<Vec<_>, String>>();

        match loaded {
            Ok(loaded) => {
                if let Some(engine) = self.engine.take()
                    && let Err(error) = engine.shutdown(ShutdownMode::UnloadPlugins)
                {
                    self.quarantine_current_plugins();
                    std::mem::forget(loaded);
                    self.state = EditorState::AudioUnavailable;
                    self.error = Some(format!(
                        "Could not stop audio before loading preset: {error}"
                    ));
                    return;
                }
                self.plugin_paths = preset
                    .plugins
                    .iter()
                    .map(|plugin| plugin.path.clone())
                    .collect();
                self.complete_plugin_load(loaded, Some(&preset), preset_name);
            }
            Err(error) => self.finish_load_error(&format!("Could not load preset: {error}")),
        }
    }

    fn start_plugin_load_task(
        &mut self,
        added_path: Option<String>,
        preset: Option<Preset>,
        preset_name: Option<String>,
    ) {
        if let Some(path) = added_path.as_deref()
            && PluginIdentity::from_config_key(path).format == PluginFormat::Clap
        {
            self.load_clap_effect(path, Some(path));
            return;
        }
        if preset.as_ref().is_some_and(|preset| {
            preset.plugins.iter().any(|plugin| {
                PluginIdentity::from_config_key(&plugin.path).format == PluginFormat::Clap
            })
        }) {
            self.load_preset_with_clap(preset.expect("checked preset"), preset_name.as_deref());
            return;
        }
        if preset.is_none()
            && added_path.is_none()
            && self
                .plugin_paths
                .iter()
                .any(|path| PluginIdentity::from_config_key(path).format == PluginFormat::Clap)
        {
            let preset = Preset {
                plugins: self
                    .plugin_paths
                    .iter()
                    .map(|path| super::presets::PresetPlugin {
                        path: path.clone(),
                        identity: Some(PluginIdentity::from_config_key(path)),
                        fingerprint: None,
                        parameters: Vec::new(),
                        bypassed: false,
                        midi_mappings: Vec::new(),
                    })
                    .collect(),
            };
            self.load_preset_with_clap(preset, None);
            return;
        }
        let sample_rate = self
            .engine
            .as_ref()
            .map(|engine| engine.latency_info().sample_rate)
            .or_else(|| {
                self.settings
                    .output_index
                    .and_then(|index| AudioDevices::sample_rate(self.settings.backend, index).ok())
            });
        if let Some(sample_rate) = sample_rate {
            log::debug!("loading plugins at active sample rate {sample_rate} Hz");
        }
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
        let task_added_path = thread_added_path.clone();
        let preset_paths = thread_preset.as_ref().map(|_| paths.clone());
        let task_paths = paths
            .iter()
            .filter(|path| !path.trim().is_empty())
            .cloned()
            .collect();
        let worker = thread::Builder::new()
            .name("rack-plugin-loader".to_owned())
            .spawn(move || {
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        if thread_cancel.load(Ordering::Acquire) {
                            return Err(PLUGIN_LOAD_CANCELLED.to_owned());
                        }
                        #[cfg(windows)]
                        let _ole_apartment =
                            WindowsStaApartment::initialize().map_err(|error| {
                                format!("could not initialize plugin loader OLE apartment: {error}")
                            })?;
                        if let Some(path) = paths.first()
                            && paths.len() == 1
                            && thread_added_path.is_some()
                        {
                            thread_progress.current.store(1, Ordering::Release);
                            load_plugin(path, sample_rate, block_size).and_then(|mut plugin| {
                                let snapshot = plugin_snapshot(&mut plugin).map_err(|error| {
                                    format!("Could not read added plugin metadata: {error}")
                                })?;
                                Ok(LoadedPlugin::Single(Box::new(plugin), snapshot))
                            })
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
                            .and_then(|plugins| {
                                plugins
                                .into_iter()
                                .enumerate()
                                .map(|(index, mut plugin)| {
                                    let snapshot = plugin_snapshot(&mut plugin).map_err(|error| {
                                        format!("Could not read plugin {index} metadata: {error}")
                                    })?;
                                    Ok((plugin, snapshot))
                                })
                                .collect::<Result<Vec<_>, String>>()
                                .map(LoadedPlugin::Stack)
                            })
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
                    paths: task_paths,
                    added_path: task_added_path,
                    last_progress: 0,
                    last_progress_at: Instant::now(),
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
                Ok(LoadedPlugin::Single(_, _)) => {
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
        let snapshots = self
            .rack
            .slots
            .iter()
            .map(|slot| slot.snapshot.clone())
            .collect::<Vec<_>>();
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
            .map(|slot| {
                slot.instance
                    .as_ref()
                    .and_then(|instance| instance.try_lock().ok())
                    .map(|plugin| f64::from(plugin.latency_samples()))
                    .unwrap_or_else(|| f64::from(slot.snapshot.latency_samples))
            })
            .sum();
        let mut midi_effects = self
            .rack
            .slots
            .iter()
            .map(|slot| MidiEffectSettingsRow {
                name: slot.snapshot.name.clone(),
                mapping_label: slot
                    .midi_mappings
                    .iter()
                    .find(|mapping| matches!(mapping.action, super::midi::MidiAction::ToggleBypass))
                    .map(PresetMidiMapping::label),
                control_mode: slot
                    .midi_mappings
                    .iter()
                    .find(|mapping| matches!(mapping.action, super::midi::MidiAction::ToggleBypass))
                    .map(|mapping| mapping.control_mode)
                    .unwrap_or(self.settings.midi_controller_mode),
            })
            .collect::<Vec<_>>();
        let settings_actions = self.settings.show_configuration(
            ui,
            &mut self.error,
            plugin_latency_samples,
            &mut midi_effects,
            self.midi_learn_plugin,
        );
        let mut midi_mappings_changed = settings_actions.midi_mappings_changed;
        if midi_mappings_changed {
            for (slot, effect) in self.rack.slots.iter_mut().zip(&midi_effects) {
                for mapping in &mut slot.midi_mappings {
                    if matches!(mapping.action, super::midi::MidiAction::ToggleBypass) {
                        mapping.control_mode = effect.control_mode;
                    }
                }
            }
        }
        if let Some(plugin_index) = settings_actions.remove_midi_plugin
            && let Some(slot) = self.rack.slots.get_mut(plugin_index)
        {
            slot.midi_mappings
                .retain(|mapping| !matches!(mapping.action, super::midi::MidiAction::ToggleBypass));
            midi_mappings_changed = true;
        }
        if midi_mappings_changed {
            self.sync_midi_mappings();
        }
        if let Some(plugin_index) = settings_actions.learn_midi_plugin {
            self.start_midi_learn(plugin_index);
        }
        if settings_actions.apply_audio_devices {
            self.apply_audio_devices();
        } else if self.rack.slots.is_empty()
            && self.engine.is_none()
            && matches!(self.state, EditorState::Unloaded)
        {
            self.start_audio();
        }

        self.sync_midi_connection();
        self.poll_midi_feedback();
        self.show_midi_learn_modal(ui.ctx());

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
                        sheet_description(header_ui, theme, "Choose a discovered audio effect.");
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
                                                let config_key = plugin.config_key();
                                                let is_selected = selected.as_deref()
                                                    == Some(config_key.as_str());
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
                                                    selected = Some(config_key);
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
            LoadedPlugin::Single(plugin, snapshot) => {
                self.complete_incremental_add(*plugin, snapshot, added_path.as_deref())
            }
            LoadedPlugin::Stack(loaded) => {
                if let Some(engine) = self.engine.take()
                    && let Err(error) = engine.shutdown(ShutdownMode::UnloadPlugins)
                {
                    self.quarantine_current_plugins();
                    std::mem::forget(loaded);
                    self.state = EditorState::AudioUnavailable;
                    self.error = Some(format!(
                        "Could not stop audio before replacing plugins: {error}"
                    ));
                    return;
                }
                self.complete_plugin_load(
                    loaded
                        .into_iter()
                        .map(|(plugin, snapshot)| LoadedEffect::Vst3(plugin, snapshot))
                        .collect(),
                    None,
                    None,
                );
            }
        }
    }

    fn finish_plugin_load_with_preset(
        &mut self,
        loaded: Vec<(RackPlugin, PluginSnapshot)>,
        preset: &Preset,
        preset_name: Option<&str>,
    ) {
        if let Some(engine) = self.engine.take()
            && let Err(error) = engine.shutdown(ShutdownMode::UnloadPlugins)
        {
            self.quarantine_current_plugins();
            std::mem::forget(loaded);
            self.state = EditorState::AudioUnavailable;
            self.error = Some(format!(
                "Could not stop audio before applying preset: {error}"
            ));
            return;
        }
        self.complete_plugin_load(
            loaded
                .into_iter()
                .map(|(plugin, snapshot)| LoadedEffect::Vst3(plugin, snapshot))
                .collect(),
            Some(preset),
            preset_name,
        );
    }

    fn complete_incremental_add(
        &mut self,
        plugin: RackPlugin,
        snapshot: PluginSnapshot,
        added_path: Option<&str>,
    ) {
        let index = self.rack.slots.len();
        let plugin = Arc::new(Mutex::new(plugin));
        let parameter_worker = match PluginParameterWorker::new(Arc::clone(&plugin)) {
            Ok(worker) => worker,
            Err(error) => {
                self.rollback_added_path(added_path);
                self.state = if self.rack.slots.is_empty() {
                    EditorState::Error
                } else {
                    EditorState::Loaded
                };
                self.error = Some(error);
                return;
            }
        };
        let bypass_state = Arc::new(AtomicBool::new(false));
        let Some(audio_was_running) = self.stop_audio_for_plugin_change() else {
            return;
        };
        if let Err(error) = self.runtime.handle().add_plugin(
            index,
            super::plugin::RuntimePlugin::Vst3(plugin.clone()),
            bypass_state.clone(),
        ) {
            let _ = self.restart_audio_after_plugin_change(audio_was_running);
            self.rollback_added_path(added_path);
            self.state = if self.runtime.status() != PluginRuntimeStatus::Running {
                EditorState::AudioUnavailable
            } else if self.rack.slots.is_empty() {
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
            .map(|plugin_instance| EditorWindow::new(&plugin_instance))
            .unwrap_or(EditorWindow::None);
        self.rack.slots.push(PluginSlot {
            path: added_path.unwrap_or_default().to_owned(),
            instance: Some(plugin),
            clap: None,
            editor_window: window,
            snapshot,
            parameter_worker: Some(parameter_worker),
            bypass_state,
            midi_mappings: Vec::new(),
            collapsed: true,
        });
        self.sync_midi_mappings();
        if let Err(error) = self.restart_audio_after_plugin_change(audio_was_running) {
            self.state = EditorState::AudioUnavailable;
            self.error = Some(error);
        } else {
            self.error = None;
            self.state = EditorState::Loaded;
        }
    }

    fn complete_incremental_add_clap(
        &mut self,
        plugin: super::plugin::ClapEffect,
        snapshot: PluginSnapshot,
        added_path: Option<&str>,
    ) {
        let index = self.rack.slots.len();
        let path = added_path
            .map(str::to_owned)
            .unwrap_or_else(|| plugin.identity().config_key());
        let bypass_state = Arc::new(AtomicBool::new(false));
        let Some(audio_was_running) = self.stop_audio_for_plugin_change() else {
            return;
        };
        if let Err(error) = self.runtime.handle().add_plugin(
            index,
            super::plugin::RuntimePlugin::Clap(None),
            Arc::clone(&bypass_state),
        ) {
            let _ = self.restart_audio_after_plugin_change(audio_was_running);
            self.rollback_added_path(added_path);
            self.state = if self.runtime.status() != PluginRuntimeStatus::Running {
                EditorState::AudioUnavailable
            } else if self.rack.slots.is_empty() {
                EditorState::Error
            } else {
                EditorState::Loaded
            };
            self.error = Some(format!("Could not add CLAP effect to the runtime: {error}"));
            return;
        }

        self.rack.slots.push(PluginSlot {
            path,
            instance: None,
            clap: Some(plugin),
            editor_window: EditorWindow::None,
            snapshot,
            parameter_worker: None,
            bypass_state,
            midi_mappings: Vec::new(),
            collapsed: true,
        });
        self.sync_midi_mappings();
        if let Err(error) = self.restart_audio_after_plugin_change(audio_was_running) {
            self.state = EditorState::AudioUnavailable;
            self.error = Some(error);
        } else {
            self.error = None;
            self.state = EditorState::Loaded;
        }
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
        loaded: Vec<LoadedEffect>,
        preset: Option<&Preset>,
        preset_name: Option<&str>,
    ) {
        self.active_audio_config = None;
        self.cancel_midi_learn();
        self.deactivate_clap_plugins();
        for slot in &mut self.rack.slots {
            slot.close_editor();
        }
        self.rack.slots.clear();
        self.error = None;
        self.state = EditorState::Loading;

        if let Err(error) = self.install_plugins(loaded) {
            self.state = if self.runtime.status() == PluginRuntimeStatus::Unresponsive {
                EditorState::AudioUnavailable
            } else {
                EditorState::Error
            };
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
                .map(|slot| match &slot.instance {
                    Some(instance) => super::plugin::RuntimePlugin::Vst3(Arc::clone(instance)),
                    None => super::plugin::RuntimePlugin::Clap(None),
                })
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
            let plugin_name = slot.snapshot.name.clone();
            slot.midi_mappings = saved_plugin.midi_mappings.clone();
            let parameter_values = saved_plugin
                .parameters
                .iter()
                .map(|parameter| (parameter.id, parameter.value))
                .collect::<Vec<_>>();
            let parameter_result = if let Some(worker) = &mut slot.parameter_worker {
                worker.set_many(parameter_values)
            } else if let Some(clap) = &mut slot.clap {
                parameter_values
                    .into_iter()
                    .try_for_each(|(id, value)| clap.set_parameter(id, value))
            } else {
                Ok(())
            };
            if let Err(error) = parameter_result {
                errors.push(format!("plugin {plugin_name}: {error}"));
            }
            for parameter in &saved_plugin.parameters {
                if let Some(snapshot) = slot
                    .snapshot
                    .parameters
                    .iter_mut()
                    .find(|snapshot| snapshot.id == parameter.id)
                {
                    snapshot.value = parameter.value;
                }
            }
            slot.bypass_state
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
            slot.close_editor();
        }
        let mut shutdown_errors = Vec::new();
        if let Some(engine) = self.engine.take()
            && let Err(error) = engine.shutdown(mode)
        {
            shutdown_errors.push(error);
        }

        if let Err(error) = self
            .runtime
            .shutdown(matches!(mode, ShutdownMode::KeepPluginsLoaded))
        {
            shutdown_errors.push(error);
        }

        let quarantine_plugins = matches!(mode, ShutdownMode::KeepPluginsLoaded)
            || self.runtime.status() == PluginRuntimeStatus::Unresponsive;
        if quarantine_plugins {
            std::mem::forget(std::mem::take(&mut self.rack.slots));
        } else {
            self.deactivate_clap_plugins();
            self.rack.slots.clear();
        }
        self.active_audio_config = None;
        self.state = EditorState::Unloaded;
        if !shutdown_errors.is_empty() {
            let message = shutdown_errors.join("; ");
            log::error!("plugin editor shutdown completed with errors: {message}");
            self.error = Some(message);
        }
    }

    fn quarantine_current_plugins(&mut self) {
        for slot in &mut self.rack.slots {
            slot.close_editor();
        }
        std::mem::forget(std::mem::take(&mut self.rack.slots));
        self.active_audio_config = None;
    }

    pub(super) fn deactivate_clap_plugins(&mut self) {
        for slot in &mut self.rack.slots {
            if let Some(plugin) = &mut slot.clap
                && let Err(error) = plugin.deactivate()
            {
                log::warn!(
                    "could not deactivate CLAP plugin {}: {error}",
                    slot.snapshot.name
                );
                self.error = Some(format!(
                    "Could not deactivate {}: {error}",
                    slot.snapshot.name
                ));
            }
        }
    }

    fn install_plugins(&mut self, plugins: Vec<LoadedEffect>) -> Result<(), String> {
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
            .map(|(effect, path)| -> Result<_, String> {
                let (instance, clap, editor_window, parameter_worker, snapshot) = match effect {
                    LoadedEffect::Vst3(plugin, snapshot) => {
                        let instance = Arc::new(Mutex::new(plugin));
                        let editor_window = instance
                            .lock()
                            .map(|plugin_instance| EditorWindow::new(&plugin_instance))
                            .unwrap_or(EditorWindow::None);
                        let parameter_worker = PluginParameterWorker::new(Arc::clone(&instance))?;
                        (
                            Some(instance),
                            None,
                            editor_window,
                            Some(parameter_worker),
                            snapshot,
                        )
                    }
                    LoadedEffect::Clap(plugin, snapshot) => {
                        (None, Some(plugin), EditorWindow::None, None, snapshot)
                    }
                };
                Ok(PluginSlot {
                    path,
                    instance,
                    clap,
                    editor_window,
                    snapshot,
                    parameter_worker,
                    bypass_state: Arc::new(AtomicBool::new(false)),
                    midi_mappings: Vec::new(),
                    collapsed: true,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.state = EditorState::Loaded;
        Ok(())
    }

    fn remove_plugin(&mut self, plugin_index: usize) {
        if plugin_index >= self.rack.slots.len() {
            return;
        }

        self.cancel_midi_learn();

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

        self.rack.slots[plugin_index].close_editor();
        let Some(audio_was_running) = self.stop_audio_for_plugin_change() else {
            return;
        };
        if let Err(error) = self.runtime.handle().remove_plugin(plugin_index) {
            let _ = self.restart_audio_after_plugin_change(audio_was_running);
            self.state = if self.runtime.status() == PluginRuntimeStatus::Unresponsive {
                EditorState::AudioUnavailable
            } else {
                EditorState::Error
            };
            self.error = Some(format!("Could not remove plugin from the runtime: {error}"));
            return;
        }

        self.plugin_paths.remove(path_index);
        self.rack.slots.remove(plugin_index);
        self.sync_midi_mappings();
        if let Err(error) = self.restart_audio_after_plugin_change(audio_was_running) {
            self.state = EditorState::AudioUnavailable;
            self.error = Some(error);
            return;
        }
        self.error = None;
        self.state = EditorState::Loaded;
    }

    fn reorder_plugin(&mut self, from: usize, to: usize) {
        if from >= self.rack.slots.len() || to > self.rack.slots.len() {
            return;
        }

        if from != to {
            self.cancel_midi_learn();
        }

        let Some(audio_was_running) = self.stop_audio_for_plugin_change() else {
            return;
        };
        if let Err(error) = self.runtime.handle().move_plugin(from, to) {
            let _ = self.restart_audio_after_plugin_change(audio_was_running);
            self.state = if self.runtime.status() == PluginRuntimeStatus::Unresponsive {
                EditorState::AudioUnavailable
            } else {
                EditorState::Error
            };
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
        if let Err(error) = self.restart_audio_after_plugin_change(audio_was_running) {
            self.state = EditorState::AudioUnavailable;
            self.error = Some(error);
            return;
        }
        self.error = None;
        self.state = EditorState::Loaded;
    }

    fn open_native_editor(&mut self, plugin_index: usize) {
        let Some(slot) = self.rack.slots.get(plugin_index) else {
            return;
        };
        let plugin_name = slot.snapshot.name.clone();
        log::info!("requesting native editor open for {plugin_name}");
        log::info!(
            "calling native editor open for {plugin_name} while audio processing remains active"
        );
        let result = if let Some(plugin) = &mut self.rack.slots[plugin_index].clap {
            plugin.open_editor()
        } else {
            self.rack.slots[plugin_index].editor_window.open()
        };
        log::info!("native editor open request returned for {plugin_name}");

        if let Err(error) = result {
            self.state = EditorState::Error;
            self.error = Some(format!(
                "Could not open {} editor: {error}",
                self.rack.slots[plugin_index].snapshot.name
            ));
            log::error!("could not open {plugin_name} native editor: {error}");
        } else {
            log::info!("native editor open request accepted for {plugin_name}");
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
        let Some(slot) = self.rack.slots.get_mut(plugin_index) else {
            return;
        };
        let plugin_name = slot.snapshot.name.clone();
        if let Some(parameter) = slot
            .snapshot
            .parameters
            .iter_mut()
            .find(|parameter| parameter.id == parameter_id)
        {
            parameter.set_value(value, None);
        }
        let parameter_result = if let Some(worker) = &mut slot.parameter_worker {
            worker.set(parameter_id, value)
        } else if let Some(clap) = &mut slot.clap {
            clap.set_parameter(parameter_id, value)
        } else {
            Ok(())
        };
        if let Err(error) = parameter_result {
            self.error = Some(format!(
                "Could not queue {plugin_name} parameter {parameter_id}: {error}"
            ));
        }
    }

    fn refresh_parameters(&mut self) {
        self.poll_parameter_worker_results();
        for slot in &mut self.rack.slots {
            let parameters = slot
                .snapshot
                .parameters
                .iter()
                .map(|parameter| {
                    (
                        parameter.id,
                        is_booleanish_parameter(parameter, &parameter.formatted_value),
                    )
                })
                .collect::<Vec<_>>();
            if let Some(worker) = &mut slot.parameter_worker {
                worker.request_read(parameters);
            } else if let Some(clap) = &mut slot.clap {
                match clap.parameter_values(&parameters) {
                    Ok(values) => {
                        for value in values {
                            if let Some(parameter) = slot
                                .snapshot
                                .parameters
                                .iter_mut()
                                .find(|parameter| parameter.id == value.id)
                            {
                                parameter.set_value(value.value, value.formatted_value);
                            }
                        }
                    }
                    Err(error) => {
                        self.error = Some(format!(
                            "Could not refresh {} parameter values: {error}",
                            slot.snapshot.name
                        ));
                    }
                }
            }
        }
        self.last_parameter_refresh = Instant::now();
    }

    fn poll_parameter_worker_results(&mut self) {
        let mut errors = Vec::new();
        for slot in &mut self.rack.slots {
            let Some(worker) = &mut slot.parameter_worker else {
                continue;
            };
            while let Some(result) = worker.try_recv() {
                match result {
                    PluginParameterResult::Values {
                        generation,
                        result: Ok(values),
                    } if worker.is_current_generation(generation) => {
                        for value in values {
                            if let Some(parameter) = slot
                                .snapshot
                                .parameters
                                .iter_mut()
                                .find(|parameter| parameter.id == value.id)
                            {
                                parameter.set_value(value.value, value.formatted_value);
                            }
                        }
                    }
                    PluginParameterResult::Values { result: Ok(_), .. } => {}
                    PluginParameterResult::Values {
                        result: Err(error), ..
                    } => errors.push(format!(
                        "Could not refresh {} parameter values: {error}",
                        slot.snapshot.name
                    )),
                    PluginParameterResult::Set {
                        id,
                        result: Err(error),
                    } => errors.push(format!(
                        "Could not set {} parameter {id}: {error}",
                        slot.snapshot.name
                    )),
                    PluginParameterResult::Set { result: Ok(()), .. }
                    | PluginParameterResult::SetMany(Ok(())) => {}
                    PluginParameterResult::SetMany(Err(error)) => errors.push(format!(
                        "Could not restore {} parameters: {error}",
                        slot.snapshot.name
                    )),
                    PluginParameterResult::WorkerError(error) => errors.push(format!(
                        "{} parameter worker failed: {error}",
                        slot.snapshot.name
                    )),
                }
            }
        }
        if let Some(error) = errors.into_iter().next() {
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
            Self::Unloaded => "Unloaded",
            Self::Empty => "Empty",
            Self::Loading => "Loading",
            Self::Loaded => "Loaded",
            Self::AudioUnavailable => "Loaded (audio unavailable)",
            Self::Error => "Error",
        }
    }
}
