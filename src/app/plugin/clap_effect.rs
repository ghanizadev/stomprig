use super::clap_host::{AudioCallbackScope, ClapHostMainThread, ClapHostShared, RackClapHost};
use super::identity::PluginIdentity;
use super::types::{PluginInfo, PluginParameter, PluginParameterValue};
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

/// Main-thread-owned CLAP instance. Only its activated processor crosses to the runtime thread.
pub(crate) struct ClapEffect {
    identity: PluginIdentity,
    instance: PluginInstance<RackClapHost>,
    parameters: Option<PluginParams>,
    gui: Option<PluginGui>,
    gui_created: bool,
    gui_visible: bool,
    parameter_producer: Producer<(u32, f64)>,
    parameter_consumer: Option<Consumer<(u32, f64)>>,
}

/// Sendable CLAP processing state, owned exclusively by the audio runtime thread.
pub(crate) struct ClapAudioProcessor {
    processor: PluginAudioProcessor<RackClapHost>,
    parameter_consumer: Consumer<(u32, f64)>,
    input_ports: AudioPorts,
    output_ports: AudioPorts,
    input_events: EventBuffer,
    output_events: EventBuffer,
    parameter_ranges: Vec<(u32, f64, f64, u32)>,
    midi_parameter_events: Vec<(u32, f64)>,
    parameters: Option<PluginParams>,
    flush_requested: std::sync::Arc<std::sync::atomic::AtomicBool>,
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
            "Rack",
            "Rack",
            "https://github.com",
            env!("CARGO_PKG_VERSION"),
        )
        .map_err(|error| format!("could not create CLAP host information: {error}"))?;
        let instance = PluginInstance::<RackClapHost>::new(
            |_| ClapHostShared::new(),
            |shared| ClapHostMainThread { shared },
            &entry,
            &plugin_id,
            &host,
        )
        .map_err(|error| format!("could not instantiate CLAP plugin: {error}"))?;
        let (parameter_producer, parameter_consumer) = RingBuffer::new(PARAMETER_QUEUE_CAPACITY);
        let mut effect = Self {
            identity,
            instance,
            parameters: None,
            gui: None,
            gui_created: false,
            gui_visible: false,
            parameter_producer,
            parameter_consumer: Some(parameter_consumer),
        };
        effect.validate_stereo_layout()?;
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
                step_count: if info
                    .flags
                    .contains(clack_extensions::params::ParamInfoFlags::IS_STEPPED)
                {
                    (info.max_value - info.min_value).round().max(0.0) as u32
                } else {
                    0
                },
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
            input_ports: AudioPorts::with_capacity(2, 1),
            output_ports: AudioPorts::with_capacity(2, 1),
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

    fn validate_stereo_layout(&mut self) -> Result<(), String> {
        let plugin = self.instance.plugin_handle();
        let Some(ports) = plugin.get_extension::<PluginAudioPorts>() else {
            return Err("CLAP plugin does not expose the audio-ports extension".to_owned());
        };
        for is_input in [true, false] {
            if ports.count(&plugin, is_input) != 1 {
                return Err("CLAP effect must expose one stereo input and output port".to_owned());
            }
            let mut buffer = AudioPortInfoBuffer::new();
            let Some(info) = ports.get(&plugin, 0, is_input, &mut buffer) else {
                return Err("CLAP effect audio-port metadata is unavailable".to_owned());
            };
            if info.channel_count != 2 {
                return Err("Rack currently supports CLAP effects with stereo ports".to_owned());
            }
        }
        Ok(())
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
        let inputs = self.input_ports.with_input_buffers([AudioPortBuffer {
            latency: 0,
            channels: AudioPortBufferType::f32_input_only(
                buffers.inputs.iter_mut().map(InputChannel::variable),
            ),
        }]);
        let mut outputs = self.output_ports.with_output_buffers([AudioPortBuffer {
            latency: 0,
            channels: AudioPortBufferType::f32_output_only(
                buffers.outputs.iter_mut().map(Vec::as_mut_slice),
            ),
        }]);
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
    }
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
    use super::midi_cc_parameter_value;

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
