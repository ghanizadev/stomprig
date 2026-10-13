use super::clap_host::{AudioCallbackScope, ClapHostMainThread, ClapHostShared, StompRigClapHost};
use super::identity::PluginIdentity;
use super::types::{PluginInfo, PluginParameter, PluginParameterValue, parameter_choice_labels};
use clack_extensions::audio_ports::{AudioPortInfoBuffer, PluginAudioPorts};
use clack_extensions::gui::{GuiApiType, GuiConfiguration, PluginGui};
use clack_extensions::latency::PluginLatency;
use clack_extensions::params::{ParamInfoBuffer, PluginParams};
use clack_host::events::event_types::ParamValueEvent;
use clack_host::events::io::{EventBuffer, InputEvents, OutputEvents};
use clack_host::events::{Event, EventFlags};
use clack_host::prelude::{
    AudioPortBuffer, AudioPortBufferType, AudioPorts, HostInfo, InputChannel,
    PluginAudioConfiguration, PluginAudioProcessor, PluginEntry, PluginInstance,
};
use rtrb::{Consumer, Producer, RingBuffer};
use std::ffi::CString;
use std::path::Path;
use vst3_host::AudioBuffers;

const PARAMETER_QUEUE_CAPACITY: usize = 256;
const MIDI_PARAMETER_CAPACITY: usize = 256;
const EVENT_BUFFER_CAPACITY: usize = PARAMETER_QUEUE_CAPACITY + MIDI_PARAMETER_CAPACITY;
const MAX_LOGGED_CLAP_AUDIO_PORTS: u32 = 32;

/// Main-thread-owned CLAP instance. Only its activated processor crosses to the runtime thread.
pub(crate) struct ClapEffect {
    identity: PluginIdentity,
    instance: PluginInstance<StompRigClapHost>,
    audio_layout: ClapAudioLayout,
    parameters: Option<PluginParams>,
    gui: Option<PluginGui>,
    gui_created: bool,
    gui_visible: bool,
    parameter_producer: Producer<(u32, f64)>,
    parameter_consumer: Option<Consumer<(u32, f64)>>,
}

/// Sendable CLAP processing state, owned exclusively by the audio runtime thread.
pub(crate) struct ClapAudioProcessor {
    processor: PluginAudioProcessor<StompRigClapHost>,
    parameter_consumer: Consumer<(u32, f64)>,
    audio_layout: ClapAudioLayout,
    mono_input: Vec<f32>,
    mono_output: Vec<f32>,
    silent_sidechain_input: [Vec<f32>; 2],
    input_ports: AudioPorts,
    output_ports: AudioPorts,
    input_events: EventBuffer,
    output_events: EventBuffer,
    parameter_ranges: Vec<(u32, f64, f64, u32)>,
    midi_parameter_events: Vec<(u32, f64)>,
    parameters: Option<PluginParams>,
    flush_requested: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Clone, Copy)]
struct ClapAudioLayout {
    input: ClapPortLayout,
    output: ClapPortLayout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ClapPortLayout {
    Mono,
    Stereo,
    DualMono,
    StereoWithSidechain { main_port_index: usize },
}

#[derive(Debug)]
struct ClapPortDescription {
    name: String,
    channel_count: u32,
    is_main: bool,
    port_type: Option<String>,
}

impl ClapEffect {
    pub(crate) fn load(identity: PluginIdentity) -> Result<Self, String> {
        let plugin_id = identity
            .plugin_id
            .as_deref()
            .ok_or_else(|| "CLAP plugin identity is missing its plugin ID".to_owned())?;
        let plugin_id =
            CString::new(plugin_id).map_err(|_| "CLAP plugin ID contains a NUL byte".to_owned())?;
        let entry = unsafe { PluginEntry::load(Path::new(&identity.bundle_path)) }
            .map_err(|error| format!("could not load CLAP bundle: {error}"))?;
        let host = HostInfo::new(
            "StompRig",
            "StompRig",
            "https://github.com",
            env!("CARGO_PKG_VERSION"),
        )
        .map_err(|error| format!("could not create CLAP host information: {error}"))?;
        let mut instance = PluginInstance::<StompRigClapHost>::new(
            |_| ClapHostShared::new(),
            |shared| ClapHostMainThread { shared },
            &entry,
            &plugin_id,
            &host,
        )
        .map_err(|error| format!("could not instantiate CLAP plugin: {error}"))?;
        let audio_layout = Self::validate_audio_layout(&mut instance, &identity)?;
        let (parameter_producer, parameter_consumer) = RingBuffer::new(PARAMETER_QUEUE_CAPACITY);
        let mut effect = Self {
            identity,
            instance,
            audio_layout,
            parameters: None,
            gui: None,
            gui_created: false,
            gui_visible: false,
            parameter_producer,
            parameter_consumer: Some(parameter_consumer),
        };
        let handle = effect.instance.plugin_handle();
        effect.parameters = handle.get_extension::<PluginParams>();
        effect.gui = handle.get_extension::<PluginGui>();
        Ok(effect)
    }

    pub(crate) fn identity(&self) -> &PluginIdentity {
        &self.identity
    }

    pub(crate) fn info(&self) -> PluginInfo {
        let descriptor = self
            .instance
            .entry()
            .get_plugin_factory()
            .and_then(|factory| {
                factory.plugin_descriptors().find(|descriptor| {
                    descriptor.id().is_some_and(|id| {
                        id.to_bytes()
                            == self
                                .identity
                                .plugin_id
                                .as_deref()
                                .unwrap_or_default()
                                .as_bytes()
                    })
                })
            });
        PluginInfo {
            name: descriptor
                .and_then(|descriptor| descriptor.name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| self.identity.plugin_id.clone().unwrap_or_default()),
            category: "Effect".to_owned(),
            uid: self.identity.plugin_id.clone().unwrap_or_default(),
        }
    }

    pub(crate) fn has_editor(&mut self) -> bool {
        let Some(gui) = self.gui else {
            return false;
        };
        let Some(api_type) = GuiApiType::default_for_current_platform() else {
            return false;
        };
        gui.is_api_supported(
            &self.instance.plugin_handle(),
            GuiConfiguration {
                api_type,
                is_floating: true,
            },
        )
    }

    pub(crate) fn latency_samples(&mut self) -> u32 {
        let plugin = self.instance.plugin_handle();
        plugin
            .get_extension::<PluginLatency>()
            .map(|latency| latency.get(&plugin))
            .unwrap_or_default()
    }

    pub(crate) fn open_editor(&mut self) -> Result<(), String> {
        let gui = self
            .gui
            .ok_or_else(|| "CLAP plugin does not expose a native editor".to_owned())?;
        let api_type = GuiApiType::default_for_current_platform()
            .ok_or_else(|| "native CLAP editors are not supported on this platform".to_owned())?;
        let configuration = GuiConfiguration {
            api_type,
            is_floating: true,
        };
        let handle = self.instance.plugin_handle();
        if !gui.is_api_supported(&handle, configuration) {
            return Err(format!(
                "CLAP editor does not support the {} API",
                api_type.0.to_string_lossy()
            ));
        }
        if !self.gui_created {
            gui.create(&handle, configuration)
                .map_err(|error| format!("could not create CLAP editor: {error}"))?;
            self.gui_created = true;
        }
        gui.show(&handle)
            .map_err(|error| format!("could not show CLAP editor: {error}"))?;
        self.gui_visible = true;
        Ok(())
    }

    pub(crate) fn close_editor(&mut self) {
        let (Some(gui), true) = (self.gui, self.gui_created) else {
            return;
        };
        let handle = self.instance.plugin_handle();
        if self.gui_visible {
            if let Err(error) = gui.hide(&handle) {
                log::warn!("could not hide CLAP editor: {error}");
            }
            self.gui_visible = false;
        }
        gui.destroy(&handle);
        self.gui_created = false;
    }

    pub(crate) fn service_main_thread_callbacks(&mut self) -> (bool, bool, bool) {
        let (
            callback_requested,
            restart_requested,
            gui_action,
            gui_closed,
            gui_destroyed,
            latency_changed,
            parameter_rescan_requested,
        ) = self.instance.access_shared_handler(|shared| {
            (
                shared.take_callback_requested(),
                shared.take_restart_requested(),
                shared.take_gui_action(),
                shared.take_gui_closed(),
                shared.take_gui_destroyed(),
                shared.take_latency_changed(),
                shared.take_parameter_rescan_requested(),
            )
        });
        if callback_requested {
            self.instance.call_on_main_thread_callback();
        }
        if gui_closed {
            if let Some(gui) = self.gui {
                let handle = self.instance.plugin_handle();
                if !gui_destroyed && self.gui_visible {
                    let _ = gui.hide(&handle);
                }
                gui.destroy(&handle);
            }
            self.gui_created = false;
            self.gui_visible = false;
        } else {
            match gui_action {
                1 => {
                    if let Err(error) = self.open_editor() {
                        log::debug!("CLAP plugin requested its editor to open: {error}");
                    }
                }
                2 => {
                    if let Some(gui) = self.gui
                        && self.gui_created
                    {
                        if let Err(error) = gui.hide(&self.instance.plugin_handle()) {
                            log::debug!("CLAP plugin requested its editor to hide: {error}");
                        }
                        self.gui_visible = false;
                    }
                }
                _ => {}
            }
        }
        (
            restart_requested,
            latency_changed,
            parameter_rescan_requested,
        )
    }

    pub(crate) fn parameters(&mut self) -> Result<Vec<PluginParameter>, String> {
        let Some(extension) = self.parameters else {
            return Ok(Vec::new());
        };
        let plugin = self.instance.plugin_handle();
        let mut parameters = Vec::new();
        for index in 0..extension.count(&plugin) {
            let mut buffer = ParamInfoBuffer::new();
            let Some(info) = extension.get_info(&plugin, index, &mut buffer) else {
                continue;
            };
            if info
                .flags
                .contains(clack_extensions::params::ParamInfoFlags::IS_HIDDEN)
                || info
                    .flags
                    .contains(clack_extensions::params::ParamInfoFlags::IS_BYPASS)
            {
                continue;
            }
            let id = info.id.get();
            let value = extension
                .get_value(&plugin, info.id)
                .unwrap_or(info.default_value);
            let mut formatted = [0u8; 128];
            let formatted_value = extension
                .value_to_text(&plugin, info.id, value, &mut formatted)
                .map(|text| String::from_utf8_lossy(text).into_owned())
                .unwrap_or_else(|_| format!("{value:.3}"));
            let step_count = if info
                .flags
                .contains(clack_extensions::params::ParamInfoFlags::IS_STEPPED)
            {
                (info.max_value - info.min_value).round().max(0.0) as u32
            } else {
                0
            };
            let choices = if info
                .flags
                .contains(clack_extensions::params::ParamInfoFlags::IS_ENUM)
            {
                parameter_choice_labels(
                    info.min_value,
                    info.max_value,
                    step_count,
                    |choice_value| {
                        let mut buffer = [0u8; 128];
                        extension
                            .value_to_text(&plugin, info.id, choice_value, &mut buffer)
                            .ok()
                            .map(|text| String::from_utf8_lossy(text).into_owned())
                            .filter(|label| !label.trim().is_empty())
                            .unwrap_or_else(|| format!("{choice_value:.3}"))
                    },
                )
            } else {
                Vec::new()
            };
            parameters.push(PluginParameter {
                id,
                name: String::from_utf8_lossy(info.name).trim().to_owned(),
                category: if info.module.is_empty() {
                    "General".to_owned()
                } else {
                    String::from_utf8_lossy(info.module).replace('/', " / ")
                },
                min: info.min_value,
                max: info.max_value,
                value,
                step_count,
                choices,
                is_read_only: info
                    .flags
                    .contains(clack_extensions::params::ParamInfoFlags::IS_READONLY),
                formatted_value,
            });
        }
        Ok(parameters)
    }

    pub(crate) fn parameter_values(
        &mut self,
        requested: &[(u32, bool)],
    ) -> Result<Vec<PluginParameterValue>, String> {
        let Some(extension) = self.parameters else {
            return Ok(Vec::new());
        };
        let plugin = self.instance.plugin_handle();
        Ok(requested
            .iter()
            .filter_map(|(id, needs_formatted_value)| {
                let clap_id = clack_host::utils::ClapId::from_raw(*id)?;
                let value = extension.get_value(&plugin, clap_id)?;
                let formatted_value = needs_formatted_value.then(|| {
                    let mut buffer = [0u8; 128];
                    extension
                        .value_to_text(&plugin, clap_id, value, &mut buffer)
                        .map(|text| String::from_utf8_lossy(text).into_owned())
                        .unwrap_or_else(|_| format!("{value:.3}"))
                });
                Some(PluginParameterValue {
                    id: *id,
                    value,
                    formatted_value,
                })
            })
            .collect::<Vec<_>>())
    }

    pub(crate) fn set_parameter(&mut self, id: u32, value: f64) -> Result<(), String> {
        let parameter = self
            .parameters()?
            .into_iter()
            .find(|parameter| parameter.id == id)
            .ok_or_else(|| format!("CLAP parameter {id} was not found"))?;
        if parameter.is_read_only {
            return Err(format!("CLAP parameter {id} is read-only"));
        }
        self.parameter_producer
            .push((id, value.clamp(parameter.min, parameter.max)))
            .map_err(|_| "CLAP parameter update queue is full".to_owned())
    }

    pub(crate) fn activate(
        &mut self,
        sample_rate: f64,
        block_size: usize,
    ) -> Result<ClapAudioProcessor, String> {
        let parameter_ranges = self
            .parameters()?
            .into_iter()
            .map(|parameter| {
                (
                    parameter.id,
                    parameter.min,
                    parameter.max,
                    parameter.step_count,
                )
            })
            .collect();
        let consumer = self.parameter_consumer.take().ok_or_else(|| {
            "CLAP parameter queue is unavailable; stop and restart the audio engine".to_owned()
        })?;
        let processor = self
            .instance
            .activate(
                |_, _| (),
                PluginAudioConfiguration {
                    sample_rate,
                    min_frames_count: 1,
                    max_frames_count: block_size.max(1) as u32,
                },
            )
            .map_err(|error| format!("could not activate CLAP plugin: {error}"))?;
        let flush_requested = self
            .instance
            .access_shared_handler(ClapHostShared::flush_request_flag);
        Ok(ClapAudioProcessor {
            processor: processor.into(),
            parameter_consumer: consumer,
            audio_layout: self.audio_layout,
            mono_input: vec![0.0; block_size.max(1)],
            mono_output: vec![0.0; block_size.max(1)],
            silent_sidechain_input: std::array::from_fn(|_| vec![0.0; block_size.max(1)]),
            input_ports: AudioPorts::with_capacity(2, 2),
            output_ports: AudioPorts::with_capacity(2, 2),
            input_events: EventBuffer::with_capacity(EVENT_BUFFER_CAPACITY),
            output_events: EventBuffer::with_capacity(EVENT_BUFFER_CAPACITY),
            parameter_ranges,
            midi_parameter_events: Vec::with_capacity(MIDI_PARAMETER_CAPACITY),
            parameters: self.parameters,
            flush_requested,
        })
    }

    pub(crate) fn deactivate(&mut self) -> Result<(), String> {
        if self.instance.is_active() {
            self.instance
                .try_deactivate()
                .map_err(|error| format!("could not deactivate CLAP plugin: {error}"))?;
        }
        if self.parameter_consumer.is_none() {
            let (producer, consumer) = RingBuffer::new(PARAMETER_QUEUE_CAPACITY);
            self.parameter_producer = producer;
            self.parameter_consumer = Some(consumer);
        }
        Ok(())
    }

    fn validate_audio_layout(
        instance: &mut PluginInstance<StompRigClapHost>,
        identity: &PluginIdentity,
    ) -> Result<ClapAudioLayout, String> {
        let plugin = instance.plugin_handle();
        let Some(ports) = plugin.get_extension::<PluginAudioPorts>() else {
            return Err("CLAP plugin does not expose the audio-ports extension".to_owned());
        };
        let mut port_counts = [0; 2];
        let mut descriptions: [Vec<ClapPortDescription>; 2] = std::array::from_fn(|_| Vec::new());
        for is_input in [true, false] {
            let direction_index = usize::from(!is_input);
            let port_count = ports.count(&plugin, is_input);
            port_counts[direction_index] = port_count;
            for port_index in 0..port_count.min(MAX_LOGGED_CLAP_AUDIO_PORTS) {
                let mut buffer = AudioPortInfoBuffer::new();
                let Some(info) = ports.get(&plugin, port_index, is_input, &mut buffer) else {
                    return Err("CLAP effect audio-port metadata is unavailable".to_owned());
                };
                descriptions[direction_index].push(ClapPortDescription {
                    name: String::from_utf8_lossy(info.name).into_owned(),
                    channel_count: info.channel_count,
                    is_main: info
                        .flags
                        .contains(clack_extensions::audio_ports::AudioPortFlags::IS_MAIN),
                    port_type: info
                        .port_type
                        .map(|port_type| port_type.0.to_string_lossy().into_owned()),
                });
            }
        }

        let plugin_id = identity.plugin_id.as_deref().unwrap_or("unknown");
        let input_omitted = port_counts[0].saturating_sub(descriptions[0].len() as u32);
        let output_omitted = port_counts[1].saturating_sub(descriptions[1].len() as u32);
        let _input_description = descriptions[0]
            .iter()
            .enumerate()
            .map(|(index, port)| {
                format!(
                    "{index}: name={:?}, channels={}, main={}, type={:?}",
                    port.name, port.channel_count, port.is_main, port.port_type
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        let _output_description = descriptions[1]
            .iter()
            .enumerate()
            .map(|(index, port)| {
                format!(
                    "{index}: name={:?}, channels={}, main={}, type={:?}",
                    port.name, port.channel_count, port.is_main, port.port_type
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        log::info!(
            "CLAP plugin '{}' audio-port configuration: StompRig accepts one mono port, one stereo port, or two mono ports; it also accepts a stereo main input with a stereo sidechain input (sidechain fed silence); inputs ({} declared, {} omitted) = [{}]; outputs ({} declared, {} omitted) = [{}]",
            plugin_id,
            port_counts[0],
            input_omitted,
            _input_description,
            port_counts[1],
            output_omitted,
            _output_description,
        );

        let mut layouts = [ClapPortLayout::Mono; 2];
        for is_input in [true, false] {
            let direction_index = usize::from(!is_input);
            let direction = if is_input { "input" } else { "output" };
            let port_count = usize::try_from(port_counts[direction_index])
                .map_err(|_| "CLAP audio-port count is unavailable".to_owned())?;
            let mut channel_counts = [0; 2];
            let mut main_ports = [false; 2];
            for (port_index, description) in
                descriptions[direction_index].iter().take(2).enumerate()
            {
                channel_counts[port_index] = supported_port_channels(description.channel_count)
                    .map_err(|error| format!("CLAP {direction} {error}"))?;
                main_ports[port_index] = description.is_main;
            }
            layouts[direction_index] = if is_input {
                validate_input_port_layout(port_count, channel_counts, main_ports)
            } else {
                validate_port_layout(port_count, channel_counts)
            }
            .map_err(|error| format!("CLAP {direction} {error}"))?;
        }
        Ok(ClapAudioLayout {
            input: layouts[0],
            output: layouts[1],
        })
    }
}

impl ClapAudioProcessor {
    pub(crate) fn set_parameter_from_midi(&mut self, parameter_id: u32, value: u8) {
        let Some((_, min, max, step_count)) = self
            .parameter_ranges
            .iter()
            .find(|(id, _, _, _)| *id == parameter_id)
        else {
            return;
        };
        let parameter_value = midi_cc_parameter_value(value, *min, *max, *step_count);
        if let Some((_, queued_value)) = self
            .midi_parameter_events
            .iter_mut()
            .find(|(id, _)| *id == parameter_id)
        {
            *queued_value = parameter_value;
        } else if self.midi_parameter_events.len() < MIDI_PARAMETER_CAPACITY {
            self.midi_parameter_events
                .push((parameter_id, parameter_value));
        }
    }

    pub(crate) fn start_processing(&mut self) -> Result<(), String> {
        let _audio_callback_scope = AudioCallbackScope::enter();
        self.processor
            .ensure_processing_started()
            .map(|_| ())
            .map_err(|error| format!("could not start CLAP processing: {error}"))
    }

    pub(crate) fn stop_processing(&mut self) {
        let _audio_callback_scope = AudioCallbackScope::enter();
        self.processor.ensure_processing_stopped();
    }

    pub(crate) fn process_audio(&mut self, buffers: &mut AudioBuffers) -> Result<(), String> {
        let _audio_callback_scope = AudioCallbackScope::enter();
        let frames = buffers.block_size;
        if buffers.inputs.len() < 2
            || buffers.outputs.len() < 2
            || buffers.inputs[0].len() < frames
            || buffers.inputs[1].len() < frames
            || buffers.outputs[0].len() < frames
            || buffers.outputs[1].len() < frames
        {
            return Err("CLAP effects require two rack input and output channels".to_owned());
        }
        if self.audio_layout.input == ClapPortLayout::Mono {
            if frames > self.mono_input.len() {
                return Err("CLAP mono input block exceeds the activated frame capacity".to_owned());
            }
            downmix_stereo_to_mono(
                &buffers.inputs[0][..frames],
                &buffers.inputs[1][..frames],
                &mut self.mono_input[..frames],
            );
        }
        if self.audio_layout.output == ClapPortLayout::Mono {
            if frames > self.mono_output.len() {
                return Err(
                    "CLAP mono output block exceeds the activated frame capacity".to_owned(),
                );
            }
            self.mono_output[..frames].fill(0.0);
        }
        if matches!(
            self.audio_layout.input,
            ClapPortLayout::StereoWithSidechain { .. }
        ) {
            clear_silent_sidechain_input(&mut self.silent_sidechain_input, frames)?;
        }
        self.input_events.clear();
        self.output_events.clear();
        while let Ok((id, value)) = self.parameter_consumer.pop() {
            if let Some(id) = clack_host::utils::ClapId::from_raw(id) {
                let event =
                    ParamValueEvent::new(0, id, clack_host::events::Pckn::match_all(), value);
                self.input_events.push(&event);
            }
        }
        for &(raw_id, value) in &self.midi_parameter_events {
            if let Some(id) = clack_host::utils::ClapId::from_raw(raw_id) {
                let event =
                    ParamValueEvent::new(0, id, clack_host::events::Pckn::match_all(), value)
                        .with_flags(EventFlags::DONT_RECORD);
                self.input_events.push(&event);
            }
        }
        self.midi_parameter_events.clear();
        let input_events = InputEvents::from_buffer(&self.input_events);
        let mut output_events = OutputEvents::from_buffer(&mut self.output_events);

        macro_rules! mono_input_port {
            ($channel:expr) => {
                AudioPortBuffer {
                    latency: 0,
                    channels: AudioPortBufferType::f32_input_only(std::iter::once(
                        InputChannel::variable($channel),
                    )),
                }
            };
        }

        macro_rules! stereo_input_port {
            ($channels:expr) => {
                AudioPortBuffer {
                    latency: 0,
                    channels: AudioPortBufferType::f32_input_only($channels),
                }
            };
        }

        macro_rules! dual_mono_input_ports {
            () => {{
                let (left, right) = buffers.inputs.split_at_mut(1);
                [
                    mono_input_port!(&mut left[0][..frames]),
                    mono_input_port!(&mut right[0][..frames]),
                ]
            }};
        }

        macro_rules! mono_output_port {
            ($channel:expr) => {
                AudioPortBuffer {
                    latency: 0,
                    channels: AudioPortBufferType::f32_output_only(std::iter::once($channel)),
                }
            };
        }

        macro_rules! stereo_output_port {
            ($channels:expr) => {
                AudioPortBuffer {
                    latency: 0,
                    channels: AudioPortBufferType::f32_output_only($channels),
                }
            };
        }

        macro_rules! dual_mono_output_ports {
            () => {{
                let (left, right) = buffers.outputs.split_at_mut(1);
                [
                    mono_output_port!(&mut left[0][..frames]),
                    mono_output_port!(&mut right[0][..frames]),
                ]
            }};
        }

        macro_rules! process_with_ports {
            ($input_ports:expr, $output_ports:expr) => {{
                let inputs = self.input_ports.with_input_buffers($input_ports);
                let mut outputs = self.output_ports.with_output_buffers($output_ports);
                self.processor
                    .ensure_processing_started()
                    .and_then(|processor| {
                        if self
                            .flush_requested
                            .swap(false, std::sync::atomic::Ordering::AcqRel)
                            && let Some(parameters) = self.parameters
                        {
                            {
                                let mut plugin_handle = processor.plugin_handle();
                                parameters.flush_active(
                                    &mut plugin_handle,
                                    &InputEvents::empty(),
                                    &mut output_events,
                                );
                            }
                        }
                        processor.process(
                            &inputs,
                            &mut outputs,
                            &input_events,
                            &mut output_events,
                            None,
                            None,
                        )
                    })
                    .map(|_| ())
                    .map_err(|error| format!("CLAP audio processing failed: {error}"))
            }};
        }

        let process_result = match (self.audio_layout.input, self.audio_layout.output) {
            (ClapPortLayout::Mono, ClapPortLayout::Mono) => process_with_ports!(
                [mono_input_port!(&mut self.mono_input[..frames])],
                [mono_output_port!(&mut self.mono_output[..frames])]
            ),
            (ClapPortLayout::Mono, ClapPortLayout::Stereo) => process_with_ports!(
                [mono_input_port!(&mut self.mono_input[..frames])],
                [stereo_output_port!(
                    buffers.outputs.iter_mut().map(Vec::as_mut_slice)
                )]
            ),
            (ClapPortLayout::Mono, ClapPortLayout::DualMono) => process_with_ports!(
                [mono_input_port!(&mut self.mono_input[..frames])],
                dual_mono_output_ports!()
            ),
            (ClapPortLayout::Stereo, ClapPortLayout::Mono) => process_with_ports!(
                [stereo_input_port!(
                    buffers.inputs.iter_mut().map(InputChannel::variable)
                )],
                [mono_output_port!(&mut self.mono_output[..frames])]
            ),
            (ClapPortLayout::Stereo, ClapPortLayout::Stereo) => process_with_ports!(
                [stereo_input_port!(
                    buffers.inputs.iter_mut().map(InputChannel::variable)
                )],
                [stereo_output_port!(
                    buffers.outputs.iter_mut().map(Vec::as_mut_slice)
                )]
            ),
            (ClapPortLayout::Stereo, ClapPortLayout::DualMono) => process_with_ports!(
                [stereo_input_port!(
                    buffers.inputs.iter_mut().map(InputChannel::variable)
                )],
                dual_mono_output_ports!()
            ),
            (
                ClapPortLayout::StereoWithSidechain { main_port_index: 0 },
                ClapPortLayout::Stereo,
            ) => process_with_ports!(
                [
                    stereo_input_port!(buffers.inputs.iter_mut().map(InputChannel::variable)),
                    stereo_input_port!(
                        self.silent_sidechain_input
                            .iter_mut()
                            .map(InputChannel::variable)
                    ),
                ],
                [stereo_output_port!(
                    buffers.outputs.iter_mut().map(Vec::as_mut_slice)
                )]
            ),
            (
                ClapPortLayout::StereoWithSidechain { main_port_index: 1 },
                ClapPortLayout::Stereo,
            ) => process_with_ports!(
                [
                    stereo_input_port!(
                        self.silent_sidechain_input
                            .iter_mut()
                            .map(InputChannel::variable)
                    ),
                    stereo_input_port!(buffers.inputs.iter_mut().map(InputChannel::variable)),
                ],
                [stereo_output_port!(
                    buffers.outputs.iter_mut().map(Vec::as_mut_slice)
                )]
            ),
            (ClapPortLayout::StereoWithSidechain { .. }, _) => {
                Err("CLAP stereo sidechain input requires one stereo output port".to_owned())
            }
            (_, ClapPortLayout::StereoWithSidechain { .. }) => {
                Err("CLAP stereo sidechain is only supported on plugin inputs".to_owned())
            }
            (ClapPortLayout::DualMono, ClapPortLayout::Mono) => process_with_ports!(
                dual_mono_input_ports!(),
                [mono_output_port!(&mut self.mono_output[..frames])]
            ),
            (ClapPortLayout::DualMono, ClapPortLayout::Stereo) => process_with_ports!(
                dual_mono_input_ports!(),
                [stereo_output_port!(
                    buffers.outputs.iter_mut().map(Vec::as_mut_slice)
                )]
            ),
            (ClapPortLayout::DualMono, ClapPortLayout::DualMono) => {
                process_with_ports!(dual_mono_input_ports!(), dual_mono_output_ports!())
            }
        };
        process_result?;

        if self.audio_layout.output == ClapPortLayout::Mono {
            let (left_output, right_output) = buffers.outputs.split_at_mut(1);
            mirror_mono_to_stereo(
                &self.mono_output[..frames],
                &mut left_output[0][..frames],
                &mut right_output[0][..frames],
            );
        }
        Ok(())
    }
}

fn supported_port_channels(channel_count: u32) -> Result<usize, String> {
    match channel_count {
        1 | 2 => Ok(channel_count as usize),
        _ => Err(format!(
            "StompRig supports CLAP audio ports with one or two channels; found {channel_count}"
        )),
    }
}

fn validate_port_layout(
    port_count: usize,
    channel_counts: [usize; 2],
) -> Result<ClapPortLayout, String> {
    match (port_count, channel_counts) {
        (1, [1, _]) => Ok(ClapPortLayout::Mono),
        (1, [2, _]) => Ok(ClapPortLayout::Stereo),
        (2, [1, 1]) => Ok(ClapPortLayout::DualMono),
        _ => Err(format!(
            "port layout has {port_count} ports with channel counts {channel_counts:?}; supported layouts are one mono port, one stereo port, or two mono ports"
        )),
    }
}

fn validate_input_port_layout(
    port_count: usize,
    channel_counts: [usize; 2],
    main_ports: [bool; 2],
) -> Result<ClapPortLayout, String> {
    match (port_count, channel_counts, main_ports) {
        (2, [2, 2], [true, false]) => {
            Ok(ClapPortLayout::StereoWithSidechain { main_port_index: 0 })
        }
        (2, [2, 2], [false, true]) => {
            Ok(ClapPortLayout::StereoWithSidechain { main_port_index: 1 })
        }
        (2, [2, 2], _) => Err(
            "stereo sidechain inputs must declare exactly one main port and one sidechain port"
                .to_owned(),
        ),
        _ => validate_port_layout(port_count, channel_counts),
    }
}

fn downmix_stereo_to_mono(left: &[f32], right: &[f32], mono: &mut [f32]) {
    for ((mono_sample, &left_sample), &right_sample) in mono.iter_mut().zip(left).zip(right) {
        *mono_sample = left_sample * 0.5 + right_sample * 0.5;
    }
}

fn mirror_mono_to_stereo(mono: &[f32], left: &mut [f32], right: &mut [f32]) {
    left.copy_from_slice(mono);
    right.copy_from_slice(mono);
}

fn clear_silent_sidechain_input(
    sidechain_input: &mut [Vec<f32>; 2],
    frames: usize,
) -> Result<(), String> {
    if sidechain_input.iter().any(|channel| channel.len() < frames) {
        return Err("CLAP sidechain block exceeds the activated frame capacity".to_owned());
    }
    for channel in sidechain_input {
        channel[..frames].fill(0.0);
    }
    Ok(())
}

fn midi_cc_parameter_value(value: u8, min: f64, max: f64, step_count: u32) -> f64 {
    let range = max - min;
    let normalized = f64::from(value.min(127)) / 127.0;
    let mut parameter_value = min + range * normalized;
    if step_count > 0 && range > 0.0 {
        let steps = f64::from(step_count);
        let step = ((parameter_value - min) / range * steps)
            .round()
            .clamp(0.0, steps);
        parameter_value = min + range * step / steps;
    }
    parameter_value
}

#[cfg(test)]
mod tests {
    use super::{
        ClapPortLayout, clear_silent_sidechain_input, downmix_stereo_to_mono,
        midi_cc_parameter_value, mirror_mono_to_stereo, supported_port_channels,
        validate_input_port_layout, validate_port_layout,
    };

    #[test]
    fn downmixes_stereo_clap_input_for_mono_processing() {
        let left = [0.75, -0.5];
        let right = [0.25, 0.5];
        let mut mono = [0.0; 2];

        downmix_stereo_to_mono(&left, &right, &mut mono);

        assert_eq!(mono, [0.5, 0.0]);
    }

    #[test]
    fn mirrors_mono_clap_output_into_both_stereo_channels() {
        let mono = [0.25, -0.75];
        let mut left = [0.0; 2];
        let mut right = [0.0; 2];

        mirror_mono_to_stereo(&mono, &mut left, &mut right);

        assert_eq!(left, mono);
        assert_eq!(right, mono);
    }

    #[test]
    fn accepts_mono_stereo_and_two_mono_clap_port_layouts() {
        assert_eq!(validate_port_layout(1, [1, 0]), Ok(ClapPortLayout::Mono));
        assert_eq!(validate_port_layout(1, [2, 0]), Ok(ClapPortLayout::Stereo));
        assert_eq!(
            validate_port_layout(2, [1, 1]),
            Ok(ClapPortLayout::DualMono)
        );
    }

    #[test]
    fn accepts_stereo_main_and_stereo_sidechain_clap_inputs() {
        assert_eq!(
            validate_input_port_layout(2, [2, 2], [true, false]),
            Ok(ClapPortLayout::StereoWithSidechain { main_port_index: 0 })
        );
        assert_eq!(
            validate_input_port_layout(2, [2, 2], [false, true]),
            Ok(ClapPortLayout::StereoWithSidechain { main_port_index: 1 })
        );
        assert!(validate_input_port_layout(2, [2, 2], [false, false]).is_err());
        assert!(validate_port_layout(2, [2, 2]).is_err());
    }

    #[test]
    fn silences_each_stereo_sidechain_block() {
        let mut sidechain = [vec![0.5, -0.5, 0.25], vec![-0.25, 0.75, -0.5]];

        clear_silent_sidechain_input(&mut sidechain, 2).unwrap();

        assert_eq!(sidechain[0], [0.0, 0.0, 0.25]);
        assert_eq!(sidechain[1], [0.0, 0.0, -0.5]);
        assert!(clear_silent_sidechain_input(&mut sidechain, 4).is_err());
    }

    #[test]
    fn rejects_clap_port_layouts_with_more_than_two_channels() {
        assert!(supported_port_channels(3).is_err());
        assert!(validate_port_layout(2, [2, 1]).is_err());
    }

    #[test]
    fn midi_cc_maps_the_full_controller_range_to_the_parameter_range() {
        assert_eq!(midi_cc_parameter_value(0, -12.0, 12.0, 0), -12.0);
        assert_eq!(midi_cc_parameter_value(127, -12.0, 12.0, 0), 12.0);
    }

    #[test]
    fn midi_cc_quantizes_stepped_parameter_values() {
        assert_eq!(midi_cc_parameter_value(64, 0.0, 5.0, 5), 3.0);
    }

    #[test]
    fn midi_cc_handles_constant_parameter_ranges() {
        assert_eq!(midi_cc_parameter_value(127, 2.0, 2.0, 1), 2.0);
    }
}
