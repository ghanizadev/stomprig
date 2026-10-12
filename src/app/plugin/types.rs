use std::sync::{Arc, Mutex, MutexGuard, TryLockError};

use super::ClapAudioProcessor;
#[cfg(windows)]
use super::windows_vst3_host::WindowsVst3EditorHost;
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

pub(crate) enum RackPlugin {
    Vst3 {
        plugin: Arc<Mutex<vst3_host::Plugin>>,
        #[cfg(windows)]
        editor_host: WindowsVst3EditorHost,
    },
}

pub(crate) enum RuntimePlugin {
    Vst3(Arc<Mutex<RackPlugin>>),
    Clap(Option<Box<ClapAudioProcessor>>),
}

pub(crate) enum AudioProcessError {
    Busy,
    Poisoned,
    Inactive,
    PluginFailure,
}

fn try_audio_lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, AudioProcessError> {
    match mutex.try_lock() {
        Ok(guard) => Ok(guard),
        Err(TryLockError::WouldBlock) => Err(AudioProcessError::Busy),
        Err(TryLockError::Poisoned(_)) => Err(AudioProcessError::Poisoned),
    }
}

impl RuntimePlugin {
    pub(crate) fn start_processing(
        &mut self,
        sample_rate: f64,
        block_size: usize,
    ) -> Result<(), String> {
        match self {
            Self::Vst3(plugin) => plugin
                .lock()
                .map_err(|_| "plugin lock poisoned".to_owned())?
                .start_processing(sample_rate, block_size),
            Self::Clap(Some(plugin)) => plugin.start_processing(),
            Self::Clap(None) => Err("CLAP audio processor was not activated".to_owned()),
        }
    }

    pub(crate) fn stop_processing(&mut self) {
        match self {
            Self::Vst3(plugin) => match plugin.lock() {
                Ok(mut plugin) => {
                    if let Err(error) = plugin.stop_processing() {
                        log::warn!("could not stop plugin processing: {error}");
                    }
                }
                Err(_) => {
                    log::warn!("could not stop plugin processing because its lock is poisoned")
                }
            },
            Self::Clap(processor) => {
                if let Some(mut processor) = processor.take() {
                    processor.stop_processing();
                    // The activated processor handle is dropped on the runtime thread. The
                    // UI then calls PluginInstance::try_deactivate before reconfiguration.
                    drop(processor);
                }
            }
        }
    }

    pub(crate) fn process_audio(
        &mut self,
        buffers: &mut vst3_host::AudioBuffers,
    ) -> Result<(), AudioProcessError> {
        match self {
            Self::Vst3(plugin) => try_audio_lock(plugin)?.process_audio(buffers),
            Self::Clap(Some(plugin)) => plugin
                .process_audio(buffers)
                .map_err(|_| AudioProcessError::PluginFailure),
            Self::Clap(None) => Err(AudioProcessError::Inactive),
        }
    }

    pub(crate) fn set_parameter_from_midi(&mut self, parameter_id: u32, value: u8) {
        match self {
            Self::Vst3(plugin) => {
                if let Ok(mut plugin) = plugin.lock() {
                    let _ = plugin.set_parameter(parameter_id, f64::from(value) / 127.0);
                }
            }
            Self::Clap(Some(plugin)) => plugin.set_parameter_from_midi(parameter_id, value),
            Self::Clap(None) => {}
        }
    }
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

    pub(crate) fn info(&self) -> PluginInfo {
        let Self::Vst3 { plugin, .. } = self;
        plugin
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
            })
    }

    pub(crate) fn latency_samples(&self) -> u32 {
        let Self::Vst3 { plugin, .. } = self;
        plugin
            .lock()
            .map(|plugin| plugin.latency_samples())
            .unwrap_or_default()
    }

    pub(crate) fn has_editor(&self) -> bool {
        let Self::Vst3 { plugin, .. } = self;
        plugin
            .lock()
            .map(|plugin| plugin.has_editor())
            .unwrap_or(false)
    }

    #[cfg(not(windows))]
    pub(crate) fn vst3_window(&self) -> Option<vst3_host::PluginWindow> {
        let Self::Vst3 { plugin, .. } = self;
        Some(vst3_host::PluginWindow::new(Arc::clone(plugin)))
    }

    #[cfg(windows)]
    pub(crate) fn vst3_editor_host(&self) -> Option<WindowsVst3EditorHost> {
        let Self::Vst3 { editor_host, .. } = self;
        Some(editor_host.clone())
    }

    pub(crate) fn parameters(&mut self) -> Result<Vec<PluginParameter>, String> {
        let Self::Vst3 { plugin, .. } = self;
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
            .collect();
        Ok(parameters)
    }

    pub(crate) fn parameter_values(
        &mut self,
        parameters: &[(u32, bool)],
    ) -> Result<Vec<PluginParameterValue>, String> {
        let Self::Vst3 { plugin, .. } = self;
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

    pub(crate) fn set_parameter(&mut self, id: u32, value: f64) -> Result<(), String> {
        let Self::Vst3 { plugin, .. } = self;
        plugin
            .lock()
            .map_err(|_| "plugin lock poisoned".to_owned())?
            .set_parameter(id, value)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn start_processing(
        &mut self,
        _sample_rate: f64,
        _block_size: usize,
    ) -> Result<(), String> {
        let Self::Vst3 { plugin, .. } = self;
        plugin
            .lock()
            .map_err(|_| "plugin lock poisoned".to_owned())?
            .start_processing()
            .map_err(|error| error.to_string())
    }

    pub(crate) fn stop_processing(&mut self) -> Result<(), String> {
        let Self::Vst3 { plugin, .. } = self;
        plugin
            .lock()
            .map_err(|_| "plugin lock poisoned".to_owned())?
            .stop_processing()
            .map_err(|error| error.to_string())
    }

    pub(crate) fn process_audio(
        &mut self,
        buffers: &mut vst3_host::AudioBuffers,
    ) -> Result<(), AudioProcessError> {
        let Self::Vst3 { plugin, .. } = self;
        try_audio_lock(plugin)?
            .process_audio(buffers)
            .map_err(|_| AudioProcessError::PluginFailure)
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

#[cfg(test)]
mod tests {
    use super::{AudioProcessError, try_audio_lock};
    use std::sync::Mutex;

    #[test]
    fn audio_lock_returns_busy_when_control_thread_holds_plugin() {
        let plugin = Mutex::new(());
        let _control_guard = plugin.lock().unwrap();

        assert!(matches!(
            try_audio_lock(&plugin),
            Err(AudioProcessError::Busy)
        ));
    }
}
