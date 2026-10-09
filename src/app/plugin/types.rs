use std::sync::{Arc, Mutex};

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
    ) -> Result<(), String> {
        let Self::Vst3 { plugin, .. } = self;
        plugin
            .lock()
            .map_err(|_| "plugin lock poisoned".to_owned())?
            .process_audio(buffers)
            .map_err(|error| error.to_string())
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
