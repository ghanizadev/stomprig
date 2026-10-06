use super::audio::{ChannelRouting, LevelAtomics};
use rtrb::{Consumer, Producer};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
use vst3_host::{AudioBuffers, Parameter, Plugin, PluginUnit};

const COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(2);
const COMMAND_CAPACITY: usize = 256;

#[derive(Clone, Debug)]
pub(crate) struct ParameterSnapshot {
    pub id: u32,
    pub name: String,
    pub category: String,
    pub unit: String,
    pub min: f64,
    pub max: f64,
    pub value: f64,
    pub step_count: u32,
    pub is_read_only: bool,
    pub formatted_value: String,
}

impl ParameterSnapshot {
    pub fn is_discrete(&self) -> bool {
        self.step_count > 0
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
}

enum PluginCommand {
    // Plugin instances are loaded and controlled by the eframe thread. The runtime only receives
    // shared handles so it can run the DSP loop without putting block processing in the UI frame.
    InstallPlugins {
        plugins: Vec<Arc<Mutex<Plugin>>>,
        bypass_states: Vec<Arc<AtomicBool>>,
        reply: SyncSender<Result<(), String>>,
    },
    AddPlugin {
        index: usize,
        plugin: Arc<Mutex<Plugin>>,
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
    StartAudio {
        audio: AudioStart,
        reply: SyncSender<Result<(), String>>,
    },
    StopAudio(SyncSender<()>),
    Shutdown {
        keep_plugins_loaded: bool,
        reply: SyncSender<()>,
    },
}

#[derive(Clone)]
pub(crate) struct PluginRuntimeHandle {
    sender: mpsc::SyncSender<PluginCommand>,
}

pub(crate) struct PluginRuntime {
    handle: PluginRuntimeHandle,
    worker: Option<JoinHandle<()>>,
}

impl PluginRuntime {
    pub fn spawn() -> Self {
        let (command_sender, command_receiver) = mpsc::sync_channel(COMMAND_CAPACITY);
        let worker = thread::Builder::new()
            .name("rack-plugin-runtime".to_owned())
            .spawn(move || worker_loop(command_receiver))
            .expect("could not spawn plugin runtime thread");

        Self {
            handle: PluginRuntimeHandle {
                sender: command_sender,
            },
            worker: Some(worker),
        }
    }

    pub fn handle(&self) -> PluginRuntimeHandle {
        self.handle.clone()
    }

    pub fn shutdown(&mut self, keep_plugins_loaded: bool) {
        let Some(worker) = self.worker.take() else {
            return;
        };
        let (reply_sender, reply_receiver) = mpsc::sync_channel(0);
        if self
            .handle
            .sender
            .send(PluginCommand::Shutdown {
                keep_plugins_loaded,
                reply: reply_sender,
            })
            .is_ok()
        {
            let _ = reply_receiver.recv();
        }
        let _ = worker.join();
    }
}

impl Drop for PluginRuntime {
    fn drop(&mut self) {
        self.shutdown(false);
    }
}

impl PluginRuntimeHandle {
    pub fn install_plugins(
        &self,
        plugins: Vec<Arc<Mutex<Plugin>>>,
        bypass_states: Vec<Arc<AtomicBool>>,
    ) -> Result<(), String> {
        let (reply_sender, reply_receiver) = mpsc::sync_channel(0);
        self.sender
            .send(PluginCommand::InstallPlugins {
                plugins,
                bypass_states,
                reply: reply_sender,
            })
            .map_err(|_| "plugin runtime thread stopped".to_owned())?;
        reply_receiver
            .recv()
            .map_err(|_| "plugin runtime thread stopped while installing plugins".to_owned())?
    }

    pub fn add_plugin(
        &self,
        index: usize,
        plugin: Arc<Mutex<Plugin>>,
        bypass_state: Arc<AtomicBool>,
    ) -> Result<(), String> {
        let (reply_sender, reply_receiver) = mpsc::sync_channel(0);
        self.sender
            .send(PluginCommand::AddPlugin {
                index,
                plugin,
                bypass_state,
                reply: reply_sender,
            })
            .map_err(|_| "plugin runtime thread stopped".to_owned())?;
        reply_receiver
            .recv()
            .map_err(|_| "plugin runtime thread stopped while adding plugin".to_owned())?
    }

    pub fn remove_plugin(&self, index: usize) -> Result<(), String> {
        let (reply_sender, reply_receiver) = mpsc::sync_channel(0);
        self.sender
            .send(PluginCommand::RemovePlugin {
                index,
                reply: reply_sender,
            })
            .map_err(|_| "plugin runtime thread stopped".to_owned())?;
        reply_receiver
            .recv()
            .map_err(|_| "plugin runtime thread stopped while removing plugin".to_owned())?
    }

    pub fn move_plugin(&self, from: usize, to: usize) -> Result<(), String> {
        let (reply_sender, reply_receiver) = mpsc::sync_channel(0);
        self.sender
            .send(PluginCommand::MovePlugin {
                from,
                to,
                reply: reply_sender,
            })
            .map_err(|_| "plugin runtime thread stopped".to_owned())?;
        reply_receiver
            .recv()
            .map_err(|_| "plugin runtime thread stopped while moving plugin".to_owned())?
    }

    pub fn start_audio(&self, audio: AudioStart) -> Result<(), String> {
        let (reply_sender, reply_receiver) = mpsc::sync_channel(0);
        self.sender
            .send(PluginCommand::StartAudio {
                audio,
                reply: reply_sender,
            })
            .map_err(|_| "plugin runtime thread stopped".to_owned())?;
        reply_receiver
            .recv()
            .map_err(|_| "plugin runtime thread stopped while starting audio".to_owned())?
    }

    pub fn stop_audio(&self) {
        let (reply_sender, reply_receiver) = mpsc::sync_channel(0);
        if self
            .sender
            .send(PluginCommand::StopAudio(reply_sender))
            .is_ok()
        {
            let _ = reply_receiver.recv();
        }
    }
}

struct AudioState {
    input_consumer: Consumer<f32>,
    output_producer: Producer<f32>,
    input_channels: usize,
    output_channels: usize,
    routing: ChannelRouting,
    buffers: AudioBuffers,
    output_data: Vec<f32>,
    levels: Arc<LevelAtomics>,
}

fn worker_loop(receiver: Receiver<PluginCommand>) {
    let mut plugins: Vec<Arc<Mutex<Plugin>>> = Vec::new();
    let mut bypass_states: Vec<Arc<AtomicBool>> = Vec::new();
    let mut audio = None;
    let mut running = true;

    while running {
        let mut handled_command = false;
        while let Ok(command) = receiver.try_recv() {
            handled_command = true;
            match command {
                PluginCommand::InstallPlugins {
                    plugins: installed_plugins,
                    bypass_states: installed_bypass_states,
                    reply,
                } => {
                    stop_audio(&mut audio, &plugins);
                    plugins = installed_plugins;
                    bypass_states = installed_bypass_states;
                    let _ = reply.send(Ok(()));
                }
                PluginCommand::AddPlugin {
                    index,
                    plugin,
                    bypass_state,
                    reply,
                } => {
                    let result = if index > plugins.len() {
                        Err(format!(
                            "cannot insert plugin at index {index}; stack has {} plugins",
                            plugins.len()
                        ))
                    } else if audio.is_some() {
                        match start_plugin(&plugin) {
                            Ok(()) => {
                                plugins.insert(index, plugin);
                                bypass_states.insert(index, bypass_state);
                                Ok(())
                            }
                            Err(error) => Err(error),
                        }
                    } else {
                        plugins.insert(index, plugin);
                        Ok(())
                    };
                    let _ = reply.send(result);
                }
                PluginCommand::RemovePlugin { index, reply } => {
                    let result = if index >= plugins.len() {
                        Err(format!(
                            "cannot remove plugin at index {index}; stack has {} plugins",
                            plugins.len()
                        ))
                    } else {
                        let plugin = plugins.remove(index);
                        bypass_states.remove(index);
                        if audio.is_some() {
                            stop_plugin(&plugin);
                        }
                        Ok(())
                    };
                    let _ = reply.send(result);
                }
                PluginCommand::MovePlugin { from, to, reply } => {
                    let result = move_plugin(&mut plugins, from, to);
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
                    let result = start_plugins(&plugins);
                    if result.is_ok() {
                        let output_len = start.output_channels * start.block_size;
                        audio = Some(AudioState {
                            input_consumer: start.input_consumer,
                            output_producer: start.output_producer,
                            input_channels: start.input_channels,
                            output_channels: start.output_channels,
                            routing: start.routing,
                            buffers: AudioBuffers::new(2, 2, start.block_size, start.sample_rate),
                            output_data: vec![0.0; output_len],
                            levels: start.levels,
                        });
                    }
                    let _ = reply.send(result);
                }
                PluginCommand::StopAudio(reply) => {
                    stop_audio(&mut audio, &plugins);
                    let _ = reply.send(());
                }
                PluginCommand::Shutdown {
                    keep_plugins_loaded,
                    reply,
                } => {
                    stop_audio(&mut audio, &plugins);
                    if keep_plugins_loaded {
                        std::mem::forget(std::mem::take(&mut plugins));
                    } else {
                        plugins.clear();
                    }
                    let _ = reply.send(());
                    running = false;
                    break;
                }
            }
        }

        if !running {
            break;
        }

        if let Some(audio_state) = audio.as_mut() {
            if audio_state.input_consumer.slots()
                >= audio_state.input_channels * audio_state.buffers.block_size
                && audio_state.output_producer.slots() >= audio_state.output_data.len()
            {
                process_audio_block(audio_state, &plugins, &bypass_states);
            } else if !handled_command {
                thread::sleep(Duration::from_millis(1));
            }
        } else if !handled_command {
            thread::sleep(COMMAND_POLL_INTERVAL);
        }
    }
}

fn process_audio_block(
    audio: &mut AudioState,
    plugins: &[Arc<Mutex<Plugin>>],
    bypass_states: &[Arc<AtomicBool>],
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
    };
    super::audio::processing::process_output(&mut audio.output_data, &mut context);
    for &sample in &audio.output_data {
        let _ = audio.output_producer.push(sample);
    }
}

pub(crate) fn plugin_snapshot(plugin: &Plugin) -> Result<PluginSnapshot, String> {
    // `get_units` queries the plugin's native IUnitInfo hierarchy. It is optional in VST3, so an
    // empty/error result is intentionally treated as "no native categorization" and the label
    // heuristics below take over.
    let units = plugin.get_units().unwrap_or_default();
    let parameters = plugin
        .get_parameters()
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|parameter| !is_midi_parameter(parameter) && !parameter.is_bypass)
        .map(|parameter| parameter_snapshot(plugin, parameter, &units))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PluginSnapshot {
        name: plugin.info().name.clone(),
        has_editor: plugin.has_editor(),
        latency_samples: plugin.latency_samples(),
        parameters,
    })
}

fn parameter_snapshot(
    plugin: &Plugin,
    parameter: Parameter,
    units: &[PluginUnit],
) -> Result<ParameterSnapshot, String> {
    let category = parameter_category(&parameter, units);
    let formatted_value = plugin
        .format_parameter(parameter.id, parameter.value)
        .unwrap_or_else(|_| parameter.format_value(parameter.value));
    Ok(ParameterSnapshot {
        id: parameter.id,
        name: parameter.name,
        category,
        unit: parameter.unit,
        min: parameter.min,
        max: parameter.max,
        value: parameter.value,
        step_count: parameter.step_count.max(0) as u32,
        is_read_only: parameter.is_read_only,
        formatted_value,
    })
}

fn parameter_category(parameter: &Parameter, units: &[PluginUnit]) -> String {
    native_unit_category(parameter.unit_id, units)
        .or_else(|| heuristic_parameter_category(&parameter.name))
        .unwrap_or_else(|| "General".to_owned())
}

fn native_unit_category(unit_id: i32, units: &[PluginUnit]) -> Option<String> {
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

fn heuristic_parameter_category(label: &str) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::{heuristic_parameter_category, native_unit_category, parameter_category};
    use vst3_host::{Parameter, PluginUnit};

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
}

fn is_midi_parameter(parameter: &Parameter) -> bool {
    parameter
        .name
        .split_whitespace()
        .next()
        .is_some_and(|word| word.eq_ignore_ascii_case("midi"))
}

fn start_plugins(plugins: &[Arc<Mutex<Plugin>>]) -> Result<(), String> {
    let mut started = 0;
    for plugin in plugins {
        if let Err(error) = start_plugin(plugin) {
            stop_plugins(&plugins[..started]);
            return Err(error);
        } else {
            started += 1;
        }
    }
    Ok(())
}

fn start_plugin(plugin: &Arc<Mutex<Plugin>>) -> Result<(), String> {
    match plugin.lock() {
        Ok(mut plugin) => plugin.start_processing().map_err(|error| error.to_string()),
        Err(_) => Err("plugin lock poisoned".to_owned()),
    }
}

fn stop_plugin(plugin: &Arc<Mutex<Plugin>>) {
    match plugin.lock() {
        Ok(mut plugin) => {
            if let Err(error) = plugin.stop_processing() {
                log::warn!("could not stop plugin processing: {error}");
            }
        }
        Err(_) => {
            log::warn!("could not stop plugin processing because its lock is poisoned");
        }
    }
}

fn stop_plugins(plugins: &[Arc<Mutex<Plugin>>]) {
    for plugin in plugins {
        stop_plugin(plugin);
    }
}

fn stop_audio(audio: &mut Option<AudioState>, plugins: &[Arc<Mutex<Plugin>>]) {
    if audio.take().is_some() {
        stop_plugins(plugins);
    }
}

fn move_plugin(
    plugins: &mut Vec<Arc<Mutex<Plugin>>>,
    from: usize,
    to: usize,
) -> Result<(), String> {
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
