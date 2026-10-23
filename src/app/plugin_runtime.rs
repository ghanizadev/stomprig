use super::audio::{
    AudioClip, ChannelRouting, LevelAtomics, PlaybackState, TunerProcessor, TunerState,
};
use super::midi::PresetMidiMapping;
use super::plugin::{PluginParameter, RuntimePlugin, StompRigPlugin};
use crate::app::diagnostics::{
    DiagnosticConsumer, DiagnosticEvent, DiagnosticKind, DiagnosticProducer, DiagnosticSeverity,
    EVENT_QUEUE_CAPACITY, FixedText, SUBJECT_BYTES, channel,
};
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use vst3_host::{AudioBuffers, MidiEvent, Parameter, PluginUnit};

mod midi;

use self::midi::{MidiRuntimeState, drain_midi_events, mapping_active_states};

const COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(2);
const COMMAND_CAPACITY: usize = 256;
const MIDI_EVENT_CAPACITY: usize = 256;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const WORKER_JOIN_POLL_INTERVAL: Duration = Duration::from_millis(5);

const SHUTDOWN_NONE: u8 = 0;
const SHUTDOWN_UNLOAD_PLUGINS: u8 = 1;
const SHUTDOWN_KEEP_PLUGINS: u8 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum PluginRuntimeStatus {
    Starting = 0,
    Running = 1,
    ShutdownRequested = 2,
    Stopped = 3,
    Unresponsive = 4,
    Failed = 5,
}

impl PluginRuntimeStatus {
    fn from_raw(value: u8) -> Self {
        match value {
            0 => Self::Starting,
            1 => Self::Running,
            2 => Self::ShutdownRequested,
            3 => Self::Stopped,
            4 => Self::Unresponsive,
            _ => Self::Failed,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ParameterSnapshot {
    pub id: u32,
    pub name: String,
    pub category: String,
    pub min: f64,
    pub max: f64,
    pub value: f64,
    pub step_count: u32,
    pub choices: Vec<String>,
    pub is_read_only: bool,
    pub formatted_value: String,
}

impl ParameterSnapshot {
    pub fn is_discrete(&self) -> bool {
        self.step_count > 0
    }

    pub fn selected_choice_index(&self) -> Option<usize> {
        let interval_count = self.choices.len().checked_sub(1)?;
        let range = self.max - self.min;
        let position = if !range.is_finite() || range.abs() <= f64::EPSILON {
            0.0
        } else {
            ((self.value - self.min) / range).clamp(0.0, 1.0)
        };
        Some((position * interval_count as f64).round() as usize)
    }

    pub fn value_for_choice(&self, index: usize) -> Option<f64> {
        let interval_count = self.choices.len().checked_sub(1)?;
        if index >= self.choices.len() {
            return None;
        }
        Some(self.min + (self.max - self.min) * index as f64 / interval_count as f64)
    }

    pub fn set_value(&mut self, value: f64, formatted_value: Option<String>) {
        self.value = value;
        if let Some(formatted_value) = formatted_value {
            self.formatted_value = formatted_value;
        } else if let Some(choice_index) = self.selected_choice_index() {
            self.formatted_value = self.choices[choice_index].clone();
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PluginSnapshot {
    pub name: String,
    pub has_editor: bool,
    pub latency_samples: u32,
    pub parameters: Vec<ParameterSnapshot>,
}

pub(crate) struct AudioStart {
    pub input_consumer: Consumer<f32>,
    pub output_producer: Producer<f32>,
    pub input_channels: usize,
    pub output_channels: usize,
    pub sample_rate: f64,
    pub routing: ChannelRouting,
    pub block_size: usize,
    pub levels: Arc<LevelAtomics>,
    pub playback: Arc<PlaybackState>,
    pub tuner: Arc<TunerState>,
    pub input_queue_frames: Arc<AtomicUsize>,
    pub output_queue_frames: Arc<AtomicUsize>,
    pub output_queue_capacity_samples: usize,
    pub plugin_names: Vec<FixedText<SUBJECT_BYTES>>,
}

enum PluginCommand {
    // Plugin instances are loaded and controlled by the eframe thread. The runtime only receives
    // shared handles so it can run the DSP loop without putting block processing in the UI frame.
    // Replaced handles return to eframe so plugin destructors do not run on the runtime worker.
    InstallPlugins {
        plugins: Vec<RuntimePlugin>,
        bypass_states: Vec<Arc<AtomicBool>>,
        reply: SyncSender<Result<Vec<RuntimePlugin>, String>>,
    },
    AddPlugin {
        index: usize,
        plugin: RuntimePlugin,
        bypass_state: Arc<AtomicBool>,
        reply: SyncSender<Result<(), String>>,
    },
    RemovePlugin {
        index: usize,
        reply: SyncSender<Result<(), String>>,
    },
    MovePlugin {
        from: usize,
        to: usize,
        reply: SyncSender<Result<(), String>>,
    },
    ReplaceClapProcessor {
        index: usize,
        processor: Box<super::plugin::ClapAudioProcessor>,
        reply: SyncSender<Result<(), String>>,
    },
    StartAudio {
        audio: AudioStart,
        reply: SyncSender<Result<(), String>>,
    },
    StopAudio(SyncSender<()>),
    SetPlaybackClip {
        clip: Arc<AudioClip>,
        reply: SyncSender<()>,
    },
    SetMidiIngress {
        consumer: Option<Consumer<MidiEvent>>,
        reply: SyncSender<Result<(), String>>,
    },
    SetMidiMappings {
        mappings: Vec<Vec<PresetMidiMapping>>,
        reply: SyncSender<Result<(), String>>,
    },
    StartMidiLearn {
        plugin_index: usize,
        parameter_id: Option<u32>,
        reply: SyncSender<Result<(), String>>,
    },
    CancelMidiLearn,
}

#[derive(Clone)]
pub(crate) struct PluginRuntimeHandle {
    sender: mpsc::SyncSender<PluginCommand>,
    status: Arc<AtomicU8>,
    shutdown_signal: Arc<AtomicU8>,
}

pub(crate) struct PluginRuntime {
    handle: PluginRuntimeHandle,
    worker: Option<JoinHandle<()>>,
    audio_active: Arc<AtomicBool>,
    midi_feedback: Option<Consumer<MidiEvent>>,
    diagnostic_consumer: DiagnosticConsumer,
    startup_error: Option<String>,
}

impl PluginRuntime {
    pub fn spawn() -> Self {
        let (command_sender, command_receiver) = mpsc::sync_channel(COMMAND_CAPACITY);
        let (midi_feedback_producer, midi_feedback_consumer) = RingBuffer::new(MIDI_EVENT_CAPACITY);
        let (mut diagnostic_producer, diagnostic_consumer) = channel(EVENT_QUEUE_CAPACITY);
        let status = Arc::new(AtomicU8::new(PluginRuntimeStatus::Starting as u8));
        let shutdown_signal = Arc::new(AtomicU8::new(SHUTDOWN_NONE));
        let audio_active = Arc::new(AtomicBool::new(false));
        let worker_status = Arc::clone(&status);
        let worker_shutdown_signal = Arc::clone(&shutdown_signal);
        let worker_audio_active = Arc::clone(&audio_active);
        let worker = thread::Builder::new()
            .name("stomprig-plugin-runtime".to_owned())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    worker_loop(
                        command_receiver,
                        midi_feedback_producer,
                        &mut diagnostic_producer,
                        &worker_shutdown_signal,
                        &worker_audio_active,
                    )
                }));
                match result {
                    Ok(()) => {
                        let _ = worker_status.compare_exchange(
                            PluginRuntimeStatus::ShutdownRequested as u8,
                            PluginRuntimeStatus::Stopped as u8,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        );
                        let _ = worker_status.compare_exchange(
                            PluginRuntimeStatus::Running as u8,
                            PluginRuntimeStatus::Stopped as u8,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        );
                    }
                    Err(payload) => {
                        let message = payload
                            .downcast_ref::<&str>()
                            .copied()
                            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                            .unwrap_or("unknown panic");
                        diagnostic_producer.send(DiagnosticEvent::new(
                            DiagnosticSeverity::Error,
                            DiagnosticKind::RuntimeWorker,
                            "Plug-in runtime",
                            message,
                        ));
                        log::error!("plugin runtime worker panicked: {message}");
                        worker_status.store(PluginRuntimeStatus::Failed as u8, Ordering::Release);
                    }
                }
            });
        let (worker, startup_error) = match worker {
            Ok(worker) => {
                status.store(PluginRuntimeStatus::Running as u8, Ordering::Release);
                (Some(worker), None)
            }
            Err(error) => {
                let message = format!("could not start plugin runtime thread: {error}");
                log::error!("{message}");
                status.store(PluginRuntimeStatus::Failed as u8, Ordering::Release);
                (None, Some(message))
            }
        };

        Self {
            handle: PluginRuntimeHandle {
                sender: command_sender,
                status,
                shutdown_signal,
            },
            worker,
            audio_active,
            midi_feedback: Some(midi_feedback_consumer),
            diagnostic_consumer,
            startup_error,
        }
    }

    pub fn handle(&self) -> PluginRuntimeHandle {
        self.handle.clone()
    }

    pub fn audio_active_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.audio_active)
    }

    pub fn take_midi_feedback(&mut self) -> Consumer<MidiEvent> {
        self.midi_feedback
            .take()
            .expect("MIDI feedback consumer can only be taken once")
    }

    pub fn try_diagnostic_event(&mut self) -> Option<DiagnosticEvent> {
        self.diagnostic_consumer.try_recv()
    }

    pub fn take_dropped_diagnostic_count(&self) -> u64 {
        self.diagnostic_consumer.take_dropped()
    }

    pub fn status(&self) -> PluginRuntimeStatus {
        self.handle.status()
    }

    pub fn shutdown(&mut self, keep_plugins_loaded: bool) -> Result<(), String> {
        self.handle.request_shutdown(keep_plugins_loaded);
        let Some(worker) = self.worker.take() else {
            return match self.status() {
                PluginRuntimeStatus::Stopped => Ok(()),
                PluginRuntimeStatus::Unresponsive => Err(
                    "plugin runtime did not stop before its deadline; worker detached and plugin state quarantined until process exit".to_owned(),
                ),
                PluginRuntimeStatus::Failed => Err(self
                    .startup_error
                    .clone()
                    .unwrap_or_else(|| "plugin runtime worker failed".to_owned())),
                _ => Ok(()),
            };
        };
        wait_for_worker(worker, &self.handle.status, SHUTDOWN_TIMEOUT)
    }
}

fn wait_for_worker(
    worker: JoinHandle<()>,
    status: &AtomicU8,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while !worker.is_finished() && Instant::now() < deadline {
        thread::sleep(WORKER_JOIN_POLL_INTERVAL);
    }
    if worker.is_finished() {
        return match worker.join() {
            Ok(()) => match PluginRuntimeStatus::from_raw(status.load(Ordering::Acquire)) {
                PluginRuntimeStatus::Failed => {
                    Err("plugin runtime worker failed during shutdown".to_owned())
                }
                PluginRuntimeStatus::Unresponsive => Err(
                    "plugin runtime worker recovered after its deadline and was detached"
                        .to_owned(),
                ),
                _ => Ok(()),
            },
            Err(_) => {
                status.store(PluginRuntimeStatus::Failed as u8, Ordering::Release);
                Err("plugin runtime worker panicked during shutdown".to_owned())
            }
        };
    }

    let previous = status.swap(PluginRuntimeStatus::Unresponsive as u8, Ordering::AcqRel);
    if previous != PluginRuntimeStatus::Failed as u8 {
        log::error!("plugin runtime shutdown timed out; detaching worker and quarantining plugins");
    }
    drop(worker);
    Err("plugin runtime shutdown timed out; worker detached and plugin state quarantined until process exit".to_owned())
}

impl Drop for PluginRuntime {
    fn drop(&mut self) {
        let _ = self.shutdown(false);
    }
}

impl PluginRuntimeHandle {
    pub fn status(&self) -> PluginRuntimeStatus {
        PluginRuntimeStatus::from_raw(self.status.load(Ordering::Acquire))
    }

    fn status_error(&self) -> String {
        match self.status() {
            PluginRuntimeStatus::Starting => "plugin runtime is still starting".to_owned(),
            PluginRuntimeStatus::Running => "plugin runtime is running".to_owned(),
            PluginRuntimeStatus::ShutdownRequested => {
                "plugin runtime shutdown has been requested".to_owned()
            }
            PluginRuntimeStatus::Stopped => "plugin runtime has stopped".to_owned(),
            PluginRuntimeStatus::Unresponsive => {
                "plugin runtime is unresponsive; plugin state is quarantined until process exit"
                    .to_owned()
            }
            PluginRuntimeStatus::Failed => "plugin runtime worker failed".to_owned(),
        }
    }

    fn request_shutdown(&self, keep_plugins_loaded: bool) {
        self.shutdown_signal.store(
            if keep_plugins_loaded {
                SHUTDOWN_KEEP_PLUGINS
            } else {
                SHUTDOWN_UNLOAD_PLUGINS
            },
            Ordering::Release,
        );
        let _ = self.status.compare_exchange(
            PluginRuntimeStatus::Running as u8,
            PluginRuntimeStatus::ShutdownRequested as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    fn mark_unresponsive(&self) {
        self.shutdown_signal
            .store(SHUTDOWN_UNLOAD_PLUGINS, Ordering::Release);
        let _ = self.status.compare_exchange(
            PluginRuntimeStatus::Running as u8,
            PluginRuntimeStatus::Unresponsive as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    fn send_and_wait<T>(
        &self,
        mut command: PluginCommand,
        reply_receiver: Receiver<T>,
        operation: &str,
    ) -> Result<T, String> {
        if self.status() != PluginRuntimeStatus::Running {
            // Some commands own plugin instances; a failed or stopped runtime cannot safely
            // dispose of them on the caller thread.
            std::mem::forget(command);
            return Err(format!("{operation}: {}", self.status_error()));
        }

        let deadline = Instant::now() + COMMAND_TIMEOUT;
        loop {
            match self.sender.try_send(command) {
                Ok(()) => break,
                Err(TrySendError::Full(returned_command)) => {
                    command = returned_command;
                    if Instant::now() >= deadline {
                        self.mark_unresponsive();
                        // A command can own a plugin whose destructor may hang. Quarantine it
                        // rather than dropping it on the UI thread after the request timed out.
                        std::mem::forget(command);
                        return Err(format!(
                            "{operation}: plugin runtime command queue timed out"
                        ));
                    }
                    thread::sleep(WORKER_JOIN_POLL_INTERVAL);
                }
                Err(TrySendError::Disconnected(returned_command)) => {
                    std::mem::forget(returned_command);
                    if self.status() == PluginRuntimeStatus::Running {
                        self.status
                            .store(PluginRuntimeStatus::Failed as u8, Ordering::Release);
                    }
                    return Err(format!("{operation}: {}", self.status_error()));
                }
            }
        }

        match reply_receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(reply) => Ok(reply),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.mark_unresponsive();
                Err(format!(
                    "{operation}: plugin runtime did not respond before its deadline"
                ))
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if self.status() == PluginRuntimeStatus::Running {
                    self.status
                        .store(PluginRuntimeStatus::Failed as u8, Ordering::Release);
                }
                Err(format!("{operation}: {}", self.status_error()))
            }
        }
    }

    pub fn install_plugins(
        &self,
        plugins: Vec<RuntimePlugin>,
        bypass_states: Vec<Arc<AtomicBool>>,
    ) -> Result<(), String> {
        if plugins.len() != bypass_states.len() {
            return Err(format!(
                "plugin and bypass state counts differ ({} plugins, {} bypass states)",
                plugins.len(),
                bypass_states.len()
            ));
        }

        let (reply_sender, reply_receiver) = mpsc::sync_channel(0);
        let retired_plugins = self.send_and_wait(
            PluginCommand::InstallPlugins {
                plugins,
                bypass_states,
                reply: reply_sender,
            },
            reply_receiver,
            "installing plugins",
        )??;
        drop(retired_plugins);
        Ok(())
    }

    pub fn add_plugin(
        &self,
        index: usize,
        plugin: RuntimePlugin,
        bypass_state: Arc<AtomicBool>,
    ) -> Result<(), String> {
        let (reply_sender, reply_receiver) = mpsc::sync_channel(0);
        self.send_and_wait(
            PluginCommand::AddPlugin {
                index,
                plugin,
                bypass_state,
                reply: reply_sender,
            },
            reply_receiver,
            "adding plugin",
        )?
    }

    pub fn remove_plugin(&self, index: usize) -> Result<(), String> {
        let (reply_sender, reply_receiver) = mpsc::sync_channel(0);
        self.send_and_wait(
            PluginCommand::RemovePlugin {
                index,
                reply: reply_sender,
            },
            reply_receiver,
            "removing plugin",
        )?
    }

    pub fn move_plugin(&self, from: usize, to: usize) -> Result<(), String> {
        let (reply_sender, reply_receiver) = mpsc::sync_channel(0);
        self.send_and_wait(
            PluginCommand::MovePlugin {
                from,
                to,
                reply: reply_sender,
            },
            reply_receiver,
            "moving plugin",
        )?
    }

    pub fn replace_clap_processor(
        &self,
        index: usize,
        processor: super::plugin::ClapAudioProcessor,
    ) -> Result<(), String> {
        let (reply_sender, reply_receiver) = mpsc::sync_channel(0);
        self.send_and_wait(
            PluginCommand::ReplaceClapProcessor {
                index,
                processor: Box::new(processor),
                reply: reply_sender,
            },
            reply_receiver,
            "replacing CLAP processor",
        )?
    }

    pub fn start_audio(&self, audio: AudioStart) -> Result<(), String> {
        let (reply_sender, reply_receiver) = mpsc::sync_channel(0);
        self.send_and_wait(
            PluginCommand::StartAudio {
                audio,
                reply: reply_sender,
            },
            reply_receiver,
            "starting audio",
        )?
    }

    pub fn stop_audio(&self) -> Result<(), String> {
        let (reply_sender, reply_receiver) = mpsc::sync_channel(0);
        self.send_and_wait(
            PluginCommand::StopAudio(reply_sender),
            reply_receiver,
            "stopping audio",
        )
    }

    pub fn set_playback_clip(&self, clip: Arc<AudioClip>) -> Result<(), String> {
        let (reply_sender, reply_receiver) = mpsc::sync_channel(0);
        self.send_and_wait(
            PluginCommand::SetPlaybackClip {
                clip,
                reply: reply_sender,
            },
            reply_receiver,
            "loading playback audio",
        )
    }

    pub fn set_midi_ingress(&self, consumer: Option<Consumer<MidiEvent>>) -> Result<(), String> {
        let (reply_sender, reply_receiver) = mpsc::sync_channel(0);
        self.send_and_wait(
            PluginCommand::SetMidiIngress {
                consumer,
                reply: reply_sender,
            },
            reply_receiver,
            "setting MIDI input",
        )?
    }

    pub fn set_midi_mappings(&self, mappings: Vec<Vec<PresetMidiMapping>>) -> Result<(), String> {
        let (reply_sender, reply_receiver) = mpsc::sync_channel(0);
        self.send_and_wait(
            PluginCommand::SetMidiMappings {
                mappings,
                reply: reply_sender,
            },
            reply_receiver,
            "setting MIDI mappings",
        )?
    }

    pub fn start_midi_learn(
        &self,
        plugin_index: usize,
        parameter_id: Option<u32>,
    ) -> Result<(), String> {
        let (reply_sender, reply_receiver) = mpsc::sync_channel(0);
        self.send_and_wait(
            PluginCommand::StartMidiLearn {
                plugin_index,
                parameter_id,
                reply: reply_sender,
            },
            reply_receiver,
            "starting MIDI learn",
        )?
    }

    pub fn cancel_midi_learn(&self) {
        if self.status() == PluginRuntimeStatus::Running {
            let _ = self.sender.try_send(PluginCommand::CancelMidiLearn);
        }
    }
}

struct AudioState {
    input_consumer: Consumer<f32>,
    output_producer: Producer<f32>,
    input_channels: usize,
    output_channels: usize,
    routing: ChannelRouting,
    sample_rate: f64,
    buffers: AudioBuffers,
    output_data: Vec<f32>,
    levels: Arc<LevelAtomics>,
    playback: Arc<PlaybackState>,
    playback_clip: Option<Arc<AudioClip>>,
    tuner: Arc<TunerState>,
    tuner_processor: TunerProcessor,
    input_queue_frames: Arc<AtomicUsize>,
    output_queue_frames: Arc<AtomicUsize>,
    output_queue_capacity_samples: usize,
    plugin_names: Vec<FixedText<SUBJECT_BYTES>>,
}

fn worker_loop(
    receiver: Receiver<PluginCommand>,
    mut midi_feedback: Producer<MidiEvent>,
    diagnostics: &mut DiagnosticProducer,
    shutdown_signal: &AtomicU8,
    audio_active: &AtomicBool,
) {
    let mut plugins: Vec<RuntimePlugin> = Vec::new();
    let mut bypass_states: Vec<Arc<AtomicBool>> = Vec::new();
    let mut midi_consumer: Option<Consumer<MidiEvent>> = None;
    let mut midi_mappings: Vec<Vec<PresetMidiMapping>> = Vec::new();
    let mut midi_mapping_active: Vec<Vec<Option<bool>>> = Vec::new();
    let mut midi_cc_values = vec![None; 16 * 128];
    let mut midi_learn_target: Option<MidiLearnTarget> = None;
    let mut audio = None;
    let mut playback_clip = None;
    let mut running = true;

    while running {
        if shutdown_signal.load(Ordering::Acquire) != SHUTDOWN_NONE {
            break;
        }
        let mut handled_command = false;
        loop {
            if shutdown_signal.load(Ordering::Acquire) != SHUTDOWN_NONE {
                running = false;
                break;
            }
            let Ok(command) = receiver.try_recv() else {
                break;
            };
            handled_command = true;
            match command {
                PluginCommand::InstallPlugins {
                    plugins: installed_plugins,
                    bypass_states: installed_bypass_states,
                    reply,
                } => {
                    stop_audio(&mut audio, &mut plugins);
                    audio_active.store(false, Ordering::Release);
                    let retired_plugins = std::mem::replace(&mut plugins, installed_plugins);
                    bypass_states = installed_bypass_states;
                    midi_mappings = vec![Vec::new(); plugins.len()];
                    midi_mapping_active = vec![Vec::new(); plugins.len()];
                    if let Err(error) = reply.send(Ok(retired_plugins)) {
                        if let Ok(retired_plugins) = error.0 {
                            // If the caller timed out, quarantine retired instances instead of
                            // running plugin destructors on the runtime worker thread.
                            std::mem::forget(retired_plugins);
                        }
                    }
                }
                PluginCommand::AddPlugin {
                    index,
                    mut plugin,
                    bypass_state,
                    reply,
                } => {
                    let result = if let Some(error) = plugin_state_lengths_error(
                        plugins.len(),
                        bypass_states.len(),
                        midi_mappings.len(),
                        midi_mapping_active.len(),
                    ) {
                        Err(error)
                    } else if index > plugins.len() {
                        Err(format!(
                            "cannot insert plugin at index {index}; stack has {} plugins",
                            plugins.len()
                        ))
                    } else if audio.is_some() {
                        let audio_config = audio
                            .as_ref()
                            .map(|audio| (audio.sample_rate, audio.buffers.block_size));
                        match audio_config {
                            Some((sample_rate, block_size)) => {
                                plugin.start_processing(sample_rate, block_size).map(|()| {
                                    plugins.insert(index, plugin);
                                    bypass_states.insert(index, bypass_state);
                                    midi_mappings.insert(index, Vec::new());
                                    midi_mapping_active.insert(index, Vec::new());
                                })
                            }
                            None => unreachable!("audio must exist in this branch"),
                        }
                    } else {
                        plugins.insert(index, plugin);
                        bypass_states.insert(index, bypass_state);
                        midi_mappings.insert(index, Vec::new());
                        midi_mapping_active.insert(index, Vec::new());
                        Ok(())
                    };
                    let _ = reply.send(result);
                }
                PluginCommand::RemovePlugin { index, reply } => {
                    let result = if let Some(error) = plugin_state_lengths_error(
                        plugins.len(),
                        bypass_states.len(),
                        midi_mappings.len(),
                        midi_mapping_active.len(),
                    ) {
                        Err(error)
                    } else if index >= plugins.len() {
                        Err(format!(
                            "cannot remove plugin at index {index}; stack has {} plugins",
                            plugins.len()
                        ))
                    } else {
                        let mut plugin = plugins.remove(index);
                        bypass_states.remove(index);
                        midi_mappings.remove(index);
                        midi_mapping_active.remove(index);
                        if audio.is_some() {
                            plugin.stop_processing();
                        }
                        Ok(())
                    };
                    let _ = reply.send(result);
                }
                PluginCommand::MovePlugin { from, to, reply } => {
                    let result = plugin_state_lengths_error(
                        plugins.len(),
                        bypass_states.len(),
                        midi_mappings.len(),
                        midi_mapping_active.len(),
                    )
                    .map_or_else(|| move_plugin(&mut plugins, from, to), Err);
                    if result.is_ok() {
                        move_item(&mut bypass_states, from, to);
                        move_item(&mut midi_mappings, from, to);
                        move_item(&mut midi_mapping_active, from, to);
                    }
                    let _ = reply.send(result);
                }
                PluginCommand::ReplaceClapProcessor {
                    index,
                    processor,
                    reply,
                } => {
                    let result = match plugins.get_mut(index) {
                        Some(RuntimePlugin::Clap(current)) if current.is_none() => {
                            *current = Some(processor);
                            Ok(())
                        }
                        Some(RuntimePlugin::Clap(_)) => {
                            Err("CLAP processor must be stopped before replacement".to_owned())
                        }
                        Some(RuntimePlugin::Vst3 { .. }) => {
                            Err("plugin slot is not CLAP".to_owned())
                        }
                        None => Err(format!("plugin index {index} is out of range")),
                    };
                    let _ = reply.send(result);
                }
                PluginCommand::StartAudio {
                    audio: start,
                    reply,
                } => {
                    if audio.is_some() {
                        let _ = reply.send(Err("audio is already running".to_owned()));
                        continue;
                    }
                    let result = start_plugins(&mut plugins, start.sample_rate, start.block_size);
                    if result.is_ok() {
                        let output_len = start.output_channels * start.block_size;
                        audio = Some(AudioState {
                            input_consumer: start.input_consumer,
                            output_producer: start.output_producer,
                            input_channels: start.input_channels,
                            output_channels: start.output_channels,
                            routing: start.routing,
                            sample_rate: start.sample_rate,
                            buffers: AudioBuffers::new(2, 2, start.block_size, start.sample_rate),
                            output_data: vec![0.0; output_len],
                            levels: start.levels,
                            playback: start.playback,
                            playback_clip: playback_clip.clone(),
                            tuner: start.tuner,
                            tuner_processor: TunerProcessor::new(start.sample_rate),
                            input_queue_frames: start.input_queue_frames,
                            output_queue_frames: start.output_queue_frames,
                            output_queue_capacity_samples: start.output_queue_capacity_samples,
                            plugin_names: start.plugin_names,
                        });
                    }
                    audio_active.store(result.is_ok(), Ordering::Release);
                    let _ = reply.send(result);
                }
                PluginCommand::StopAudio(reply) => {
                    stop_audio(&mut audio, &mut plugins);
                    audio_active.store(false, Ordering::Release);
                    let _ = reply.send(());
                }
                PluginCommand::SetPlaybackClip { clip, reply } => {
                    playback_clip = Some(clip.clone());
                    if let Some(audio) = audio.as_mut() {
                        audio.playback_clip = Some(clip);
                    }
                    let _ = reply.send(());
                }
                PluginCommand::SetMidiIngress { consumer, reply } => {
                    midi_consumer = consumer;
                    midi_cc_values.fill(None);
                    midi_mapping_active = mapping_active_states(&midi_mappings, &midi_cc_values);
                    let _ = reply.send(Ok(()));
                }
                PluginCommand::SetMidiMappings { mappings, reply } => {
                    if mappings.len() != plugins.len() {
                        let _ = reply.send(Err(
                            "MIDI mapping count does not match the plugin stack".to_owned(),
                        ));
                    } else {
                        midi_mappings = mappings;
                        midi_mapping_active =
                            mapping_active_states(&midi_mappings, &midi_cc_values);
                        let _ = reply.send(Ok(()));
                    }
                }
                PluginCommand::StartMidiLearn {
                    plugin_index,
                    parameter_id,
                    reply,
                } => {
                    if plugin_index >= plugins.len() {
                        let _ = reply.send(Err(format!(
                            "cannot learn MIDI for plugin {plugin_index}; stack has {} plugins",
                            plugins.len()
                        )));
                    } else {
                        midi_learn_target = Some(MidiLearnTarget { parameter_id });
                        let _ = reply.send(Ok(()));
                    }
                }
                PluginCommand::CancelMidiLearn => {
                    midi_learn_target = None;
                }
            }
        }

        if !running {
            break;
        }

        drain_midi_events(
            &mut midi_consumer,
            &mut midi_feedback,
            MidiRuntimeState {
                learn_target: &mut midi_learn_target,
                mappings: &mut midi_mappings,
                mapping_active: &mut midi_mapping_active,
                cc_values: &mut midi_cc_values,
                bypass_states: &bypass_states,
                plugins: &mut plugins,
            },
        );

        if let Some(audio_state) = audio.as_mut() {
            if audio_state.input_consumer.slots()
                >= audio_state.input_channels * audio_state.buffers.block_size
                && audio_state.output_producer.slots() >= audio_state.output_data.len()
            {
                process_audio_block(audio_state, &mut plugins, &bypass_states, diagnostics);
            } else if !handled_command {
                thread::sleep(Duration::from_millis(1));
            }
        } else if !handled_command {
            thread::sleep(COMMAND_POLL_INTERVAL);
        }
    }

    stop_audio(&mut audio, &mut plugins);
    audio_active.store(false, Ordering::Release);
    if shutdown_signal.load(Ordering::Acquire) == SHUTDOWN_KEEP_PLUGINS {
        std::mem::forget(std::mem::take(&mut plugins));
    } else {
        // Plugin destructors run here, off the UI thread. The owner detaches this worker if
        // destruction exceeds the shutdown deadline.
        plugins.clear();
    }
}

#[derive(Clone, Copy)]
struct MidiLearnTarget {
    parameter_id: Option<u32>,
}
fn process_audio_block(
    audio: &mut AudioState,
    plugins: &mut [RuntimePlugin],
    bypass_states: &[Arc<AtomicBool>],
    diagnostics: &mut DiagnosticProducer,
) {
    let mut context = super::audio::processing::ProcessContext {
        input_channels: audio.input_channels,
        output_channels: audio.output_channels,
        input_left: audio.routing.input_left,
        input_right: audio.routing.input_right,
        output_left: audio.routing.output_left,
        output_right: audio.routing.output_right,
        input_consumer: &mut audio.input_consumer,
        plugins,
        bypass_states,
        buffers: &mut audio.buffers,
        levels: &audio.levels,
        sample_rate: audio.sample_rate,
        tuner: &mut audio.tuner_processor,
        tuner_state: &audio.tuner,
        diagnostics,
        plugin_names: &audio.plugin_names,
    };
    super::audio::processing::process_output(
        &mut audio.output_data,
        &mut context,
        Some(audio.playback.as_ref()),
        audio.playback_clip.as_deref(),
    );
    for &sample in &audio.output_data {
        let _ = audio.output_producer.push(sample);
    }
    audio.input_queue_frames.store(
        audio.input_consumer.slots() / audio.input_channels.max(1),
        Ordering::Release,
    );
    audio.output_queue_frames.store(
        audio
            .output_queue_capacity_samples
            .saturating_sub(audio.output_producer.slots())
            / audio.output_channels.max(1),
        Ordering::Release,
    );
}

pub(crate) fn plugin_snapshot(plugin: &mut StompRigPlugin) -> Result<PluginSnapshot, String> {
    let info = plugin.info();
    let parameters = plugin
        .parameters()?
        .into_iter()
        .map(parameter_snapshot)
        .collect();
    Ok(PluginSnapshot {
        name: info.name,
        has_editor: plugin.has_editor(),
        latency_samples: plugin.latency_samples(),
        parameters,
    })
}

fn parameter_snapshot(parameter: PluginParameter) -> ParameterSnapshot {
    ParameterSnapshot {
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
    }
}

#[allow(dead_code)]
fn parameter_category(parameter: &Parameter, units: &[PluginUnit]) -> String {
    native_unit_category(parameter.unit_id, units)
        .or_else(|| heuristic_parameter_category(&parameter.name))
        .unwrap_or_else(|| "General".to_owned())
}

pub(crate) fn native_unit_category(unit_id: i32, units: &[PluginUnit]) -> Option<String> {
    let mut current_id = unit_id;
    let mut visited = Vec::new();
    let mut names = Vec::new();

    while !visited.contains(&current_id) {
        visited.push(current_id);
        let unit = units.iter().find(|unit| unit.id == current_id)?;
        let name = unit.name.trim();
        if !name.is_empty() {
            names.push(name.to_owned());
        }

        if unit.parent_id < 0 || unit.parent_id == current_id {
            break;
        }
        current_id = unit.parent_id;
    }

    if names.is_empty() {
        None
    } else {
        names.reverse();
        Some(names.join(" / "))
    }
}

pub(crate) fn heuristic_parameter_category(label: &str) -> Option<String> {
    let words: Vec<_> = label
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(|word| word.to_ascii_lowercase())
        .collect();

    let has = |candidates: &[&str]| {
        candidates
            .iter()
            .any(|candidate| words.iter().any(|word| word == candidate))
    };
    let has_phrase = |first: &str, second: &str| {
        words
            .windows(2)
            .any(|pair| pair[0] == first && pair[1] == second)
    };

    if has(&[
        "eq", "bass", "mid", "middle", "treble", "presence", "notch", "bell", "peaking",
    ]) || has_phrase("low", "shelf")
        || has_phrase("high", "shelf")
        || has_phrase("low", "cut")
        || has_phrase("high", "cut")
    {
        return Some("EQ".to_owned());
    }
    if has(&[
        "compressor",
        "compress",
        "threshold",
        "ratio",
        "attack",
        "release",
        "knee",
        "gate",
        "expander",
        "limiter",
        "ducking",
    ]) {
        return Some("Dynamics".to_owned());
    }
    if has(&["delay", "echo", "feedback", "pingpong"]) {
        return Some("Delay".to_owned());
    }
    if has(&["reverb", "room", "decay", "damping", "predelay"]) {
        return Some("Reverb".to_owned());
    }
    if has(&[
        "chorus",
        "flanger",
        "phaser",
        "lfo",
        "modulation",
        "rate",
        "depth",
    ]) {
        return Some("Modulation".to_owned());
    }
    if has(&[
        "distortion",
        "drive",
        "saturation",
        "fuzz",
        "grit",
        "bitcrush",
    ]) {
        return Some("Distortion".to_owned());
    }
    if has(&["cutoff", "resonance", "filter"]) {
        return Some("Filter".to_owned());
    }
    if has(&["mix", "blend", "drywet", "wetdry"]) {
        return Some("Mix".to_owned());
    }
    if has(&["output", "volume", "level", "gain", "loudness"]) {
        return Some("Output".to_owned());
    }

    None
}

fn start_plugins(
    plugins: &mut [RuntimePlugin],
    sample_rate: f64,
    block_size: usize,
) -> Result<(), String> {
    let mut started = 0;
    for plugin in plugins.iter_mut() {
        if let Err(error) = plugin.start_processing(sample_rate, block_size) {
            stop_plugins(&mut plugins[..started]);
            return Err(error);
        } else {
            started += 1;
        }
    }
    Ok(())
}

fn stop_plugins(plugins: &mut [RuntimePlugin]) {
    for plugin in plugins {
        plugin.stop_processing();
    }
}

fn stop_audio(audio: &mut Option<AudioState>, plugins: &mut [RuntimePlugin]) {
    if audio.take().is_some() {
        stop_plugins(plugins);
    } else {
        for plugin in plugins {
            if matches!(plugin, RuntimePlugin::Clap(_)) {
                plugin.stop_processing();
            }
        }
    }
}

fn plugin_state_lengths_error(
    plugin_count: usize,
    bypass_count: usize,
    mapping_count: usize,
    active_mapping_count: usize,
) -> Option<String> {
    if plugin_count == bypass_count
        && plugin_count == mapping_count
        && plugin_count == active_mapping_count
    {
        None
    } else {
        Some(format!(
            "plugin runtime state is out of sync (plugins: {plugin_count}, bypass states: {bypass_count}, mappings: {mapping_count}, active mappings: {active_mapping_count})"
        ))
    }
}

fn move_plugin(plugins: &mut Vec<RuntimePlugin>, from: usize, to: usize) -> Result<(), String> {
    if from >= plugins.len() {
        return Err(format!(
            "cannot move plugin from index {from}; stack has {} plugins",
            plugins.len()
        ));
    }
    if to > plugins.len() {
        return Err(format!(
            "cannot move plugin to index {to}; stack has {} plugins",
            plugins.len()
        ));
    }

    move_item(plugins, from, to);
    Ok(())
}

fn move_item<T>(items: &mut Vec<T>, from: usize, to: usize) {
    if from >= items.len() || to > items.len() {
        return;
    }
    let item = items.remove(from);
    let adjusted_to = if from < to { to - 1 } else { to };
    items.insert(adjusted_to, item);
}

#[cfg(test)]
mod tests {
    use super::{
        AudioStart, ParameterSnapshot, PluginRuntime, PluginRuntimeStatus, RuntimePlugin,
        heuristic_parameter_category, native_unit_category, parameter_category,
        plugin_state_lengths_error, wait_for_worker,
    };
    use crate::app::audio::{ChannelRouting, LevelAtomics, PlaybackState, TunerState};
    use rtrb::RingBuffer;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
    use std::thread;
    use std::time::Duration;
    use vst3_host::{Parameter, PluginUnit};

    fn audio_start() -> AudioStart {
        let (_, input_consumer) = RingBuffer::new(64);
        let (output_producer, _) = RingBuffer::new(64);
        AudioStart {
            input_consumer,
            output_producer,
            input_channels: 2,
            output_channels: 2,
            sample_rate: 48_000.0,
            routing: ChannelRouting {
                input_left: 0,
                input_right: 1,
                output_left: 0,
                output_right: 1,
            },
            block_size: 16,
            levels: Arc::new(LevelAtomics::new()),
            playback: Arc::new(PlaybackState::new()),
            tuner: Arc::new(TunerState::new()),
            input_queue_frames: Arc::new(AtomicUsize::new(0)),
            output_queue_frames: Arc::new(AtomicUsize::new(0)),
            output_queue_capacity_samples: 64,
            plugin_names: Vec::new(),
        }
    }

    #[test]
    fn maps_select_choices_to_and_from_parameter_values() {
        let mut parameter = ParameterSnapshot {
            id: 1,
            name: "Model".to_owned(),
            category: "General".to_owned(),
            min: 0.0,
            max: 1.0,
            value: 0.5,
            step_count: 6,
            choices: (1..=7).map(|index| format!("Model {index}")).collect(),
            is_read_only: false,
            formatted_value: "Model 4".to_owned(),
        };

        assert_eq!(parameter.selected_choice_index(), Some(3));
        assert_eq!(parameter.value_for_choice(6), Some(1.0));
        assert_eq!(parameter.value_for_choice(7), None);

        parameter.set_value(0.0, None);

        assert_eq!(parameter.formatted_value, "Model 1");
    }

    fn parameter(name: &str, unit_id: i32) -> Parameter {
        Parameter {
            id: 1,
            unit_id,
            name: name.to_owned(),
            value: 0.0,
            min: 0.0,
            max: 1.0,
            default: 0.0,
            unit: String::new(),
            step_count: 0,
            can_automate: true,
            is_read_only: false,
            is_bypass: false,
            flags: 0,
        }
    }

    #[test]
    fn native_unit_hierarchy_becomes_a_category_path() {
        let units = vec![
            PluginUnit {
                id: 0,
                parent_id: -1,
                name: "Synth".to_owned(),
                program_list_id: None,
                programs: Vec::new(),
            },
            PluginUnit {
                id: 7,
                parent_id: 0,
                name: "Oscillator".to_owned(),
                program_list_id: None,
                programs: Vec::new(),
            },
        ];

        assert_eq!(
            native_unit_category(7, &units),
            Some("Synth / Oscillator".to_owned())
        );
        assert_eq!(
            parameter_category(&parameter("Bass", 7), &units),
            "Synth / Oscillator"
        );
    }

    #[test]
    fn common_tone_labels_fall_back_to_eq() {
        for label in ["Bass", "Mid", "Treble"] {
            assert_eq!(heuristic_parameter_category(label), Some("EQ".to_owned()));
        }
        assert_eq!(parameter_category(&parameter("Treble", 99), &[]), "EQ");
    }

    #[test]
    fn detects_mismatched_plugin_state_vectors() {
        assert!(plugin_state_lengths_error(1, 1, 1, 1).is_none());
        assert_eq!(
            plugin_state_lengths_error(1, 0, 1, 1),
            Some(
                "plugin runtime state is out of sync (plugins: 1, bypass states: 0, mappings: 1, active mappings: 1)"
                    .to_owned()
            )
        );
    }

    #[test]
    fn repeated_start_stop_is_bounded_and_idempotent() {
        let runtime = PluginRuntime::spawn();
        let handle = runtime.handle();

        for _ in 0..4 {
            handle.start_audio(audio_start()).unwrap();
            handle.stop_audio().unwrap();
            handle.stop_audio().unwrap();
        }

        assert_eq!(runtime.status(), PluginRuntimeStatus::Running);
    }

    #[test]
    fn device_restart_stops_and_restarts_runtime_audio() {
        let runtime = PluginRuntime::spawn();
        let handle = runtime.handle();

        handle.start_audio(audio_start()).unwrap();
        handle.stop_audio().unwrap();
        handle.start_audio(audio_start()).unwrap();
        handle.stop_audio().unwrap();
    }

    #[test]
    fn failed_plugin_startup_leaves_runtime_usable_for_cleanup() {
        let runtime = PluginRuntime::spawn();
        let handle = runtime.handle();
        handle
            .install_plugins(
                vec![RuntimePlugin::Clap(None)],
                vec![Arc::new(std::sync::atomic::AtomicBool::new(false))],
            )
            .unwrap();

        let error = handle.start_audio(audio_start()).unwrap_err();
        assert!(error.contains("CLAP audio processor was not activated"));
        handle.stop_audio().unwrap();
        handle.remove_plugin(0).unwrap();
        assert_eq!(runtime.status(), PluginRuntimeStatus::Running);
    }

    #[test]
    fn plugin_removal_completes_cleanly() {
        let runtime = PluginRuntime::spawn();
        let handle = runtime.handle();
        handle
            .add_plugin(
                0,
                RuntimePlugin::Clap(None),
                Arc::new(std::sync::atomic::AtomicBool::new(false)),
            )
            .unwrap();
        handle.remove_plugin(0).unwrap();
    }

    #[test]
    fn editor_closure_uses_keep_loaded_shutdown() {
        let mut runtime = PluginRuntime::spawn();
        runtime
            .handle()
            .install_plugins(
                vec![RuntimePlugin::Clap(None)],
                vec![Arc::new(std::sync::atomic::AtomicBool::new(false))],
            )
            .unwrap();
        runtime.shutdown(true).unwrap();
        assert_eq!(runtime.status(), PluginRuntimeStatus::Stopped);
        runtime.shutdown(true).unwrap();
    }

    #[test]
    fn application_exit_drop_stops_runtime_worker() {
        let runtime = PluginRuntime::spawn();
        let handle = runtime.handle();

        drop(runtime);

        assert_eq!(handle.status(), PluginRuntimeStatus::Stopped);
    }

    #[test]
    fn shutdown_detaches_worker_stalled_in_plugin_processing() {
        let worker = thread::spawn(|| thread::sleep(Duration::from_millis(150)));
        assert_shutdown_times_out(worker);
    }

    #[test]
    fn shutdown_detaches_worker_stalled_in_plugin_destruction() {
        struct SlowDrop;
        impl Drop for SlowDrop {
            fn drop(&mut self) {
                thread::sleep(Duration::from_millis(150));
            }
        }

        let worker = thread::spawn(|| drop(SlowDrop));
        assert_shutdown_times_out(worker);
    }

    fn assert_shutdown_times_out(worker: thread::JoinHandle<()>) {
        let status = AtomicU8::new(PluginRuntimeStatus::ShutdownRequested as u8);
        let started = std::time::Instant::now();
        let result = wait_for_worker(worker, &status, Duration::from_millis(20));

        assert!(result.unwrap_err().contains("timed out"));
        assert!(started.elapsed() < Duration::from_millis(100));
        assert_eq!(
            PluginRuntimeStatus::from_raw(status.load(Ordering::Acquire)),
            PluginRuntimeStatus::Unresponsive
        );
    }
}
