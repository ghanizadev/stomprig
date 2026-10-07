#![allow(deprecated)]

use std::sync::{Arc, Mutex};

use vst::editor::Editor;
use vst::host::{Host as Vst2Host, HostBuffer, PluginInstance, PluginLoader};
use vst::plugin::Plugin as Vst2PluginTrait;
use vst3_host::{Parameter, PluginUnit};

#[derive(Clone, Debug)]
pub(crate) struct PluginInfo {
    pub(crate) name: String,
    pub(crate) category: String,
    pub(crate) uid: String,
}

#[derive(Clone, Debug)]
pub(crate) struct PluginParameter {
    pub(crate) id: u32,
    pub(crate) name: String,
    pub(crate) category: String,
    pub(crate) unit: String,
    pub(crate) min: f64,
    pub(crate) max: f64,
    pub(crate) value: f64,
    pub(crate) step_count: u32,
    pub(crate) is_read_only: bool,
    pub(crate) formatted_value: String,
}

pub(crate) struct Vst2Plugin {
    instance: PluginInstance,
    host_buffer: HostBuffer<f32>,
    info: vst::plugin::Info,
    editor_requested: bool,
}

// `HostBuffer` owns only its pointer arrays; the pointers are rebound to the current audio
// buffers immediately before every process call while the surrounding plugin mutex is held.
unsafe impl Send for Vst2Plugin {}

pub(crate) enum RackPlugin {
    Vst3 {
        plugin: Arc<Mutex<vst3_host::Plugin>>,
    },
    Vst2(Vst2Plugin),
}

impl RackPlugin {
    pub(crate) fn from_vst3(plugin: vst3_host::Plugin) -> Self {
        Self::Vst3 {
            plugin: Arc::new(Mutex::new(plugin)),
        }
    }

    pub(crate) fn from_vst2(path: &std::path::Path) -> Result<Self, String> {
        let host = Arc::new(Mutex::new(RackVst2Host));
        let mut loader = PluginLoader::load(path, host).map_err(|error| error.to_string())?;
        let instance = loader.instance().map_err(|error| error.to_string())?;
        let info = instance.get_info();
        let input_count = usize::try_from(info.inputs.max(0)).unwrap_or_default();
        let output_count = usize::try_from(info.outputs.max(0)).unwrap_or_default();
        if input_count > 2 || output_count > 2 {
            return Err(format!(
                "VST2 plugin exposes {input_count} inputs and {output_count} outputs; rack supports at most stereo I/O"
            ));
        }
        let mut instance = instance;
        instance.init();
        Ok(Self::Vst2(Vst2Plugin {
            instance,
            host_buffer: HostBuffer::new(input_count.min(2), output_count.min(2)),
            info,
            editor_requested: false,
        }))
    }

    pub(crate) fn info(&self) -> PluginInfo {
        match self {
            Self::Vst3 { plugin } => plugin
                .lock()
                .map(|plugin| PluginInfo {
                    name: plugin.info().name.clone(),
                    category: plugin.info().category.clone(),
                    uid: plugin.info().uid.clone(),
                })
                .unwrap_or_else(|_| PluginInfo {
                    name: "Unknown plugin".to_owned(),
                    category: "Uncategorized".to_owned(),
                    uid: String::new(),
                }),
            Self::Vst2(plugin) => PluginInfo {
                name: plugin.info.name.clone(),
                category: format!("{:?}", plugin.info.category),
                uid: format!("vst2:{:08x}", plugin.info.unique_id as u32),
            },
        }
    }

    pub(crate) fn latency_samples(&self) -> u32 {
        match self {
            Self::Vst3 { plugin } => plugin
                .lock()
                .map(|plugin| plugin.latency_samples())
                .unwrap_or_default(),
            Self::Vst2(plugin) => {
                u32::try_from(plugin.info.initial_delay.max(0)).unwrap_or_default()
            }
        }
    }

    pub(crate) fn has_editor(&self) -> bool {
        match self {
            Self::Vst3 { plugin } => plugin
                .lock()
                .map(|plugin| plugin.has_editor())
                .unwrap_or(false),
            Self::Vst2(plugin) => plugin.editor_requested,
        }
    }

    pub(crate) fn take_editor(&mut self) -> Option<Box<dyn Editor>> {
        let Self::Vst2(plugin) = self else {
            return None;
        };
        if plugin.editor_requested {
            return None;
        }
        let editor = plugin.instance.get_editor();
        plugin.editor_requested = editor.is_some();
        editor
    }

    pub(crate) fn vst3_window(&self) -> Option<vst3_host::PluginWindow> {
        match self {
            Self::Vst3 { plugin } => Some(vst3_host::PluginWindow::new(plugin.clone())),
            Self::Vst2(_) => None,
        }
    }

    pub(crate) fn parameters(&mut self) -> Result<Vec<PluginParameter>, String> {
        match self {
            Self::Vst3 { plugin } => {
                let plugin = plugin
                    .lock()
                    .map_err(|_| "plugin lock poisoned".to_owned())?;
                let units = plugin.get_units().unwrap_or_default();
                let parameters = plugin
                    .get_parameters()
                    .map_err(|error| error.to_string())?
                    .into_iter()
                    .filter(|parameter| !is_midi_parameter(parameter) && !parameter.is_bypass)
                    .map(|parameter| vst3_parameter(&plugin, parameter, &units))
                    .collect::<Vec<_>>();
                Ok(parameters)
            }
            Self::Vst2(plugin) => {
                let parameters = plugin.instance.get_parameter_object();
                let mut result = Vec::with_capacity(plugin.info.parameters.max(0) as usize);
                for index in 0..plugin.info.parameters.max(0) {
                    let name = parameters.get_parameter_name(index);
                    let formatted_value = parameters.get_parameter_text(index);
                    result.push(PluginParameter {
                        id: index as u32,
                        category: crate::app::plugin_runtime::heuristic_parameter_category(&name)
                            .unwrap_or_else(|| "General".to_owned()),
                        unit: parameters.get_parameter_label(index),
                        min: 0.0,
                        max: 1.0,
                        value: f64::from(parameters.get_parameter(index)),
                        step_count: 0,
                        is_read_only: !parameters.can_be_automated(index),
                        name,
                        formatted_value,
                    });
                }
                Ok(result)
            }
        }
    }

    pub(crate) fn set_parameter(&mut self, id: u32, value: f64) -> Result<(), String> {
        match self {
            Self::Vst3 { plugin } => plugin
                .lock()
                .map_err(|_| "plugin lock poisoned".to_owned())?
                .set_parameter(id, value)
                .map_err(|error| error.to_string()),
            Self::Vst2(plugin) => {
                if id >= plugin.info.parameters.max(0) as u32 {
                    return Err(format!("parameter {id} is out of range"));
                }
                plugin
                    .instance
                    .get_parameter_object()
                    .set_parameter(id as i32, value.clamp(0.0, 1.0) as f32);
                Ok(())
            }
        }
    }

    pub(crate) fn start_processing(
        &mut self,
        sample_rate: f64,
        block_size: usize,
    ) -> Result<(), String> {
        match self {
            Self::Vst3 { plugin } => plugin
                .lock()
                .map_err(|_| "plugin lock poisoned".to_owned())?
                .start_processing()
                .map_err(|error| error.to_string()),
            Self::Vst2(plugin) => {
                plugin.instance.set_sample_rate(sample_rate as f32);
                plugin.instance.set_block_size(block_size as i64);
                plugin.instance.resume();
                Ok(())
            }
        }
    }

    pub(crate) fn stop_processing(&mut self) -> Result<(), String> {
        match self {
            Self::Vst3 { plugin } => plugin
                .lock()
                .map_err(|_| "plugin lock poisoned".to_owned())?
                .stop_processing()
                .map_err(|error| error.to_string()),
            Self::Vst2(plugin) => {
                plugin.instance.suspend();
                Ok(())
            }
        }
    }

    pub(crate) fn process_audio(
        &mut self,
        buffers: &mut vst3_host::AudioBuffers,
    ) -> Result<(), String> {
        match self {
            Self::Vst3 { plugin } => plugin
                .lock()
                .map_err(|_| "plugin lock poisoned".to_owned())?
                .process_audio(buffers)
                .map_err(|error| error.to_string()),
            Self::Vst2(plugin) => {
                let input_count = plugin.host_buffer.input_count().min(buffers.inputs.len());
                let output_count = plugin.host_buffer.output_count().min(buffers.outputs.len());
                let inputs = [&buffers.inputs[0][..], &buffers.inputs[1][..]];
                let (output_left, output_right) = buffers.outputs.split_at_mut(1);
                let mut outputs = [&mut output_left[0][..], &mut output_right[0][..]];
                let mut audio_buffer = plugin
                    .host_buffer
                    .bind(&inputs[..input_count], &mut outputs[..output_count]);
                plugin.instance.process(&mut audio_buffer);
                Ok(())
            }
        }
    }
}

fn vst3_parameter(
    plugin: &vst3_host::Plugin,
    parameter: Parameter,
    units: &[PluginUnit],
) -> PluginParameter {
    let category = crate::app::plugin_runtime::native_unit_category(parameter.unit_id, units)
        .or_else(|| crate::app::plugin_runtime::heuristic_parameter_category(&parameter.name))
        .unwrap_or_else(|| "General".to_owned());
    let formatted_value = plugin
        .format_parameter(parameter.id, parameter.value)
        .unwrap_or_else(|_| parameter.format_value(parameter.value));
    PluginParameter {
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
    }
}

fn is_midi_parameter(parameter: &Parameter) -> bool {
    parameter
        .name
        .split_whitespace()
        .next()
        .is_some_and(|word| word.eq_ignore_ascii_case("midi"))
}

#[derive(Default)]
struct RackVst2Host;

impl Vst2Host for RackVst2Host {}
