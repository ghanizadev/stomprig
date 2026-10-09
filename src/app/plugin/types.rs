#![allow(deprecated)]

use std::cell::Cell;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(windows)]
use super::windows_vst3_host::WindowsVst3EditorHost;
use vst::api::ProcessLevel;
use vst::editor::Editor;
use vst::host::{Host as Vst2Host, HostBuffer, PluginInstance, PluginLoader};
use vst::plugin::Plugin as Vst2PluginTrait;
use vst3_host::{Parameter, PluginUnit};

std::thread_local! {
    static VST2_PROCESS_LEVEL: Cell<isize> = const { Cell::new(ProcessLevel::User as isize) };
}

struct Vst2ProcessLevelGuard {
    previous_level: isize,
}

impl Vst2ProcessLevelGuard {
    fn enter(level: ProcessLevel) -> Self {
        let previous_level = VST2_PROCESS_LEVEL.with(|current| current.replace(level as isize));
        Self { previous_level }
    }
}

impl Drop for Vst2ProcessLevelGuard {
    fn drop(&mut self) {
        VST2_PROCESS_LEVEL.with(|current| current.set(self.previous_level));
    }
}

fn current_vst2_process_level() -> isize {
    VST2_PROCESS_LEVEL.with(Cell::get)
}

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
    pub(crate) min: f64,
    pub(crate) max: f64,
    pub(crate) value: f64,
    pub(crate) step_count: u32,
    pub(crate) is_read_only: bool,
    pub(crate) formatted_value: String,
}

pub(crate) struct PluginParameterValue {
    pub(crate) id: u32,
    pub(crate) value: f64,
    pub(crate) formatted_value: Option<String>,
}

pub(crate) struct Vst2Plugin {
    instance: PluginInstance,
    host_buffer: HostBuffer<f32>,
    host_sample_rate: Arc<AtomicIsize>,
    host_block_size: Arc<AtomicIsize>,
    info: vst::plugin::Info,
    editor_requested: bool,
}

// `HostBuffer` owns only its pointer arrays; the pointers are rebound to the current audio
// buffers immediately before every process call while the surrounding plugin mutex is held.
unsafe impl Send for Vst2Plugin {}

#[allow(clippy::large_enum_variant)]
pub(crate) enum RackPlugin {
    Vst3 {
        plugin: Arc<Mutex<vst3_host::Plugin>>,
        #[cfg(windows)]
        editor_host: WindowsVst3EditorHost,
    },
    Vst2(Vst2Plugin),
}

impl RackPlugin {
    #[cfg(not(windows))]
    pub(crate) fn from_vst3(plugin: vst3_host::Plugin) -> Self {
        Self::Vst3 {
            plugin: Arc::new(Mutex::new(plugin)),
        }
    }

    #[cfg(windows)]
    pub(crate) fn from_vst3(
        plugin: Arc<Mutex<vst3_host::Plugin>>,
        editor_host: WindowsVst3EditorHost,
    ) -> Self {
        Self::Vst3 {
            plugin,
            editor_host,
        }
    }

    pub(crate) fn from_vst2(
        path: &std::path::Path,
        sample_rate: f64,
        block_size: usize,
    ) -> Result<Self, String> {
        let host_sample_rate = Arc::new(AtomicIsize::new(sample_rate as isize));
        let host_block_size = Arc::new(AtomicIsize::new(block_size.max(1) as isize));
        let host = Arc::new(Mutex::new(RackVst2Host {
            sample_rate: Arc::clone(&host_sample_rate),
            block_size: Arc::clone(&host_block_size),
            last_logged_process_level: AtomicIsize::new(ProcessLevel::Unknown as isize),
        }));
        log::debug!(
            "initializing VST2 host callbacks for {} (sample_rate={}, block_size={})",
            path.display(),
            sample_rate as isize,
            block_size.max(1)
        );
        let loader_started = std::time::Instant::now();
        log::info!(
            "VST2 load phase: loading module and locating VSTPluginMain for {}",
            path.display()
        );
        let mut loader = PluginLoader::load(path, host).map_err(|error| error.to_string())?;
        log::info!(
            "VST2 load phase: module load returned for {} after {:?}",
            path.display(),
            loader_started.elapsed()
        );
        let instance_started = std::time::Instant::now();
        log::info!(
            "VST2 load phase: creating effect instance for {}",
            path.display()
        );
        let instance = loader.instance().map_err(|error| error.to_string())?;
        log::info!(
            "VST2 load phase: effect instance created for {} after {:?}",
            path.display(),
            instance_started.elapsed()
        );
        let info = instance.get_info();
        log::info!(
            "VST2 plugin '{}' reports {} parameters and {}x{} audio channels",
            info.name,
            info.parameters,
            info.inputs,
            info.outputs
        );
        let input_count = usize::try_from(info.inputs.max(0)).unwrap_or_default();
        let output_count = usize::try_from(info.outputs.max(0)).unwrap_or_default();
        if input_count > 2 || output_count > 2 {
            return Err(format!(
                "VST2 plugin exposes {input_count} inputs and {output_count} outputs; rack supports at most stereo I/O"
            ));
        }
        let mut instance = instance;
        log::info!("VST2 load phase: sending effOpen to {}", info.name);
        let init_started = std::time::Instant::now();
        instance.init();
        log::info!(
            "VST2 load phase: effOpen returned for '{}' after {:?}",
            info.name,
            init_started.elapsed()
        );
        Ok(Self::Vst2(Vst2Plugin {
            instance,
            host_buffer: HostBuffer::new(input_count.min(2), output_count.min(2)),
            host_sample_rate,
            host_block_size,
            info,
            editor_requested: false,
        }))
    }

    pub(crate) fn info(&self) -> PluginInfo {
        match self {
            Self::Vst3 { plugin, .. } => plugin
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
            Self::Vst3 { plugin, .. } => plugin
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
            Self::Vst3 { plugin, .. } => plugin
                .lock()
                .map(|plugin| plugin.has_editor())
                .unwrap_or(false),
            Self::Vst2(plugin) => plugin.editor_requested,
        }
    }

    pub(crate) fn take_editor(&mut self) -> Option<Box<dyn Editor + Send>> {
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

    #[cfg(not(windows))]
    pub(crate) fn vst3_window(&self) -> Option<vst3_host::PluginWindow> {
        match self {
            Self::Vst3 { plugin, .. } => Some(vst3_host::PluginWindow::new(plugin.clone())),
            Self::Vst2(_) => None,
        }
    }

    #[cfg(windows)]
    pub(crate) fn vst3_editor_host(&self) -> Option<WindowsVst3EditorHost> {
        match self {
            Self::Vst3 { editor_host, .. } => Some(editor_host.clone()),
            Self::Vst2(_) => None,
        }
    }

    pub(crate) fn parameters(&mut self) -> Result<Vec<PluginParameter>, String> {
        match self {
            Self::Vst3 { plugin, .. } => {
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
                let parameter_count = plugin.info.parameters.max(0);
                log::debug!(
                    "reading VST2 parameter metadata for '{}' ({parameter_count} parameters) on thread {:?}",
                    plugin.info.name,
                    std::thread::current().id()
                );
                let mut result = Vec::with_capacity(parameter_count as usize);
                for index in 0..parameter_count {
                    let name = parameters.get_parameter_name(index);
                    let formatted_value = parameters.get_parameter_text(index);
                    let can_be_automated = parameters.can_be_automated(index);
                    result.push(PluginParameter {
                        id: index as u32,
                        category: crate::app::plugin_runtime::heuristic_parameter_category(&name)
                            .unwrap_or_else(|| "General".to_owned()),
                        min: 0.0,
                        max: 1.0,
                        value: f64::from(parameters.get_parameter(index)),
                        step_count: 0,
                        is_read_only: !can_be_automated,
                        name,
                        formatted_value,
                    });
                    if (index + 1) % 32 == 0 || index + 1 == parameter_count {
                        log::debug!(
                            "read VST2 parameter metadata for '{}' through {}/{}",
                            plugin.info.name,
                            index + 1,
                            parameter_count
                        );
                    }
                }
                Ok(result)
            }
        }
    }

    pub(crate) fn parameter_values(
        &mut self,
        parameters: &[(u32, bool)],
    ) -> Result<Vec<PluginParameterValue>, String> {
        match self {
            Self::Vst3 { plugin, .. } => {
                let plugin = plugin
                    .lock()
                    .map_err(|_| "plugin lock poisoned".to_owned())?;
                parameters
                    .iter()
                    .map(|(id, needs_formatted_value)| {
                        let value = plugin
                            .get_parameter(*id)
                            .map_err(|error| error.to_string())?;
                        let formatted_value = if *needs_formatted_value {
                            Some(
                                plugin
                                    .format_parameter(*id, value)
                                    .map_err(|error| error.to_string())?,
                            )
                        } else {
                            None
                        };
                        Ok(PluginParameterValue {
                            id: *id,
                            value,
                            formatted_value,
                        })
                    })
                    .collect()
            }
            Self::Vst2(plugin) => {
                let parameter_object = plugin.instance.get_parameter_object();
                parameters
                    .iter()
                    .map(|(id, needs_formatted_value)| {
                        let index = i32::try_from(*id)
                            .map_err(|_| format!("parameter {id} is out of range"))?;
                        Ok(PluginParameterValue {
                            id: *id,
                            value: f64::from(parameter_object.get_parameter(index)),
                            formatted_value: needs_formatted_value
                                .then(|| parameter_object.get_parameter_text(index)),
                        })
                    })
                    .collect()
            }
        }
    }

    pub(crate) fn set_parameter(&mut self, id: u32, value: f64) -> Result<(), String> {
        match self {
            Self::Vst3 { plugin, .. } => plugin
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
            Self::Vst3 { plugin, .. } => plugin
                .lock()
                .map_err(|_| "plugin lock poisoned".to_owned())?
                .start_processing()
                .map_err(|error| error.to_string()),
            Self::Vst2(plugin) => {
                plugin
                    .host_sample_rate
                    .store(sample_rate as isize, Ordering::Relaxed);
                plugin.host_block_size.store(
                    block_size.max(1).min(isize::MAX as usize) as isize,
                    Ordering::Relaxed,
                );
                log::debug!(
                    "starting VST2 plugin processing at {} Hz with block size {}",
                    sample_rate as isize,
                    block_size
                );
                plugin.instance.set_sample_rate(sample_rate as f32);
                plugin.instance.set_block_size(block_size as i64);
                plugin.instance.resume();
                Ok(())
            }
        }
    }

    pub(crate) fn stop_processing(&mut self) -> Result<(), String> {
        match self {
            Self::Vst3 { plugin, .. } => plugin
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
            Self::Vst3 { plugin, .. } => plugin
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
                let _process_level = Vst2ProcessLevelGuard::enter(ProcessLevel::Realtime);
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
struct RackVst2Host {
    sample_rate: Arc<AtomicIsize>,
    block_size: Arc<AtomicIsize>,
    last_logged_process_level: AtomicIsize,
}

impl Vst2Host for RackVst2Host {
    fn get_sample_rate(&self) -> isize {
        let sample_rate = self.sample_rate.load(Ordering::Relaxed);
        log::trace!("VST2 audioMasterGetSampleRate -> {sample_rate}");
        sample_rate
    }

    fn get_block_size(&self) -> isize {
        let block_size = self.block_size.load(Ordering::Relaxed);
        log::trace!("VST2 audioMasterGetBlockSize -> {block_size}");
        block_size
    }

    fn get_current_process_level(&self) -> isize {
        let process_level = current_vst2_process_level();
        let previous_level = self
            .last_logged_process_level
            .swap(process_level, Ordering::Relaxed);
        if previous_level != process_level {
            log::debug!("VST2 audioMasterGetCurrentProcessLevel -> {process_level}");
        }
        process_level
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vst2_process_level_defaults_to_user() {
        assert_eq!(current_vst2_process_level(), ProcessLevel::User as isize);
    }

    #[test]
    fn vst2_process_level_guard_restores_previous_level() {
        {
            let _guard = Vst2ProcessLevelGuard::enter(ProcessLevel::Realtime);
            assert_eq!(
                current_vst2_process_level(),
                ProcessLevel::Realtime as isize
            );
        }

        assert_eq!(current_vst2_process_level(), ProcessLevel::User as isize);
    }
}
