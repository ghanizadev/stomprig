use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};

use super::ClapAudioProcessor;
#[cfg(windows)]
use super::windows_vst3_host::WindowsVst3EditorHost;
use super::{
    PARAMETER_CHANGE_QUEUE_CAPACITY, ParameterChangeConsumer, ParameterRuntimeFeedbackProducer,
};
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
    pub(crate) choices: Vec<String>,
    pub(crate) is_read_only: bool,
    pub(crate) formatted_value: String,
}

const MAX_PARAMETER_CHOICE_COUNT: u32 = 128;
const MAX_INFERRED_PARAMETER_CHOICE_STEPS: u32 = 16;

pub(crate) fn parameter_choice_labels(
    min: f64,
    max: f64,
    step_count: u32,
    mut format_value: impl FnMut(f64) -> String,
) -> Vec<String> {
    if step_count == 0 || step_count >= MAX_PARAMETER_CHOICE_COUNT {
        return Vec::new();
    }

    let range = max - min;
    if !min.is_finite() || !max.is_finite() || !range.is_finite() {
        return Vec::new();
    }

    (0..=step_count)
        .map(|index| {
            let value = min + range * (f64::from(index) / f64::from(step_count));
            format_value(value)
        })
        .collect()
}

fn has_named_parameter_choices(choices: &[String]) -> bool {
    choices.len() > 1
        && choices.iter().skip(1).any(|choice| choice != &choices[0])
        && choices
            .iter()
            .any(|choice| !starts_with_numeric_parameter_value(choice))
}

fn prefers_parameter_choice_control(name: &str) -> bool {
    name.split(|character: char| !character.is_alphanumeric())
        .any(|word| {
            matches!(
                word.to_ascii_lowercase().as_str(),
                "circuit"
                    | "channel"
                    | "model"
                    | "mode"
                    | "shape"
                    | "style"
                    | "topology"
                    | "type"
                    | "waveform"
            )
        })
}

fn starts_with_numeric_parameter_value(label: &str) -> bool {
    let label = label.trim().trim_start_matches(['+', '-', '−']);
    let mut has_digit = false;
    for character in label.chars() {
        if character.is_ascii_digit() {
            has_digit = true;
        } else if character != '.' {
            break;
        }
    }
    has_digit
}

pub(crate) struct PluginParameterValue {
    pub(crate) id: u32,
    pub(crate) value: f64,
    pub(crate) formatted_value: Option<String>,
}

pub(crate) enum StompRigPlugin {
    Vst3 {
        plugin: Arc<Mutex<vst3_host::Plugin>>,
        #[cfg(windows)]
        editor_host: WindowsVst3EditorHost,
    },
}

pub(crate) enum RuntimePlugin {
    Vst3 {
        plugin: Arc<Mutex<StompRigPlugin>>,
        parameter_changes: ParameterChangeConsumer,
        parameter_feedback: ParameterRuntimeFeedbackProducer,
        pending_parameter_changes: Vec<(u32, f64)>,
        dropped_midi_updates: usize,
    },
    Clap(Option<Box<ClapAudioProcessor>>),
}

const MIDI_PARAMETER_CHANGE_CAPACITY: usize = 256;

pub(crate) enum AudioProcessError {
    Busy,
    Poisoned,
    Inactive,
    PluginFailure,
}

impl AudioProcessError {
    pub(crate) const fn diagnostic_message(&self) -> &'static str {
        match self {
            Self::Busy => "Plug-in access remained contended; a crossfaded dry fallback was used.",
            Self::Poisoned => "Plug-in state is unavailable; the dry signal was passed through.",
            Self::Inactive => "Plug-in is inactive; the dry signal was passed through.",
            Self::PluginFailure => "Plug-in processing failed; the dry signal was passed through.",
        }
    }
}

fn try_audio_lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, AudioProcessError> {
    match mutex.try_lock() {
        Ok(guard) => Ok(guard),
        Err(TryLockError::WouldBlock) => Err(AudioProcessError::Busy),
        Err(TryLockError::Poisoned(_)) => Err(AudioProcessError::Poisoned),
    }
}

impl RuntimePlugin {
    pub(crate) fn vst3(
        plugin: Arc<Mutex<StompRigPlugin>>,
        parameter_changes: ParameterChangeConsumer,
        parameter_feedback: ParameterRuntimeFeedbackProducer,
    ) -> Self {
        Self::Vst3 {
            plugin,
            parameter_changes,
            parameter_feedback,
            pending_parameter_changes: Vec::with_capacity(
                PARAMETER_CHANGE_QUEUE_CAPACITY + MIDI_PARAMETER_CHANGE_CAPACITY,
            ),
            dropped_midi_updates: 0,
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
                .start_processing(sample_rate, block_size),
            Self::Clap(Some(plugin)) => plugin.start_processing(),
            Self::Clap(None) => Err("CLAP audio processor was not activated".to_owned()),
        }
    }

    pub(crate) fn stop_processing(&mut self) {
        match self {
            Self::Vst3 { plugin, .. } => match plugin.lock() {
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
            Self::Vst3 {
                plugin,
                parameter_changes,
                parameter_feedback,
                pending_parameter_changes,
                dropped_midi_updates,
            } => {
                // Release the wrapper lock before attempting the VST3 host lock.
                let plugin_handle = {
                    let plugin_wrapper = try_audio_lock(plugin)?;
                    let StompRigPlugin::Vst3 { plugin, .. } = &*plugin_wrapper;
                    Arc::clone(plugin)
                };
                StompRigPlugin::process_audio_with_parameter_changes(
                    &plugin_handle,
                    parameter_changes,
                    pending_parameter_changes,
                    dropped_midi_updates,
                    parameter_feedback,
                    buffers,
                )
            }
            Self::Clap(Some(plugin)) => plugin
                .process_audio(buffers)
                .map_err(|_| AudioProcessError::PluginFailure),
            Self::Clap(None) => Err(AudioProcessError::Inactive),
        }
    }

    pub(crate) fn set_parameter_from_midi(&mut self, parameter_id: u32, value: u8) {
        match self {
            Self::Vst3 {
                pending_parameter_changes,
                dropped_midi_updates,
                ..
            } => {
                let value = f64::from(value) / 127.0;
                if let Some((_, pending_value)) = pending_parameter_changes
                    .iter_mut()
                    .rev()
                    .find(|(pending_id, _)| *pending_id == parameter_id)
                {
                    *pending_value = value;
                } else if pending_parameter_changes.len() < MIDI_PARAMETER_CHANGE_CAPACITY {
                    pending_parameter_changes.push((parameter_id, value));
                } else {
                    *dropped_midi_updates = dropped_midi_updates.saturating_add(1);
                }
            }
            Self::Clap(Some(plugin)) => plugin.set_parameter_from_midi(parameter_id, value),
            Self::Clap(None) => {}
        }
    }
}

impl StompRigPlugin {
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

    #[cfg(target_os = "macos")]
    pub(crate) fn try_recovery_count(&self) -> Option<u64> {
        let Self::Vst3 { plugin, .. } = self;
        plugin.try_lock().ok().map(|plugin| plugin.recovery_count())
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
        let current_values = plugin
            .get_parameters()
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|parameter| (parameter.id, parameter.value))
            .collect::<HashMap<_, _>>();
        parameters
            .iter()
            .map(|(id, needs_formatted_value)| {
                let value = current_values
                    .get(id)
                    .copied()
                    .ok_or_else(|| format!("plugin parameter {id} was not found"))?;
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

    fn process_audio_with_parameter_changes(
        plugin: &Arc<Mutex<vst3_host::Plugin>>,
        parameter_changes: &mut ParameterChangeConsumer,
        pending_parameter_changes: &mut Vec<(u32, f64)>,
        dropped_midi_updates: &mut usize,
        parameter_feedback: &mut ParameterRuntimeFeedbackProducer,
        buffers: &mut vst3_host::AudioBuffers,
    ) -> Result<(), AudioProcessError> {
        let mut plugin = try_audio_lock(plugin)?;

        parameter_changes.drain_into(pending_parameter_changes);
        if !pending_parameter_changes.is_empty() {
            if let Err(error) =
                plugin.queue_parameter_changes_for_next_block(pending_parameter_changes)
            {
                log::error!("could not queue VST3 parameter changes: {error}");
            }
            pending_parameter_changes.clear();
        }

        if *dropped_midi_updates > 0 {
            let dropped = std::mem::take(dropped_midi_updates);
            log::warn!(
                "dropped {dropped} MIDI parameter update(s) because the runtime queue was full"
            );
        }

        match plugin.process_audio_and_get_parameter_changes(buffers) {
            Ok(changes) => {
                for (parameter_id, value) in changes {
                    parameter_feedback.push(parameter_id, value);
                }
                Ok(())
            }
            Err(_) => Err(AudioProcessError::PluginFailure),
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
    let step_count = parameter.step_count.max(0) as u32;
    let is_list = parameter.is_list();
    let should_probe_choices =
        is_list || (step_count > 0 && step_count <= MAX_INFERRED_PARAMETER_CHOICE_STEPS);
    let choices = if should_probe_choices {
        let choices = parameter_choice_labels(parameter.min, parameter.max, step_count, |value| {
            plugin
                .format_parameter(parameter.id, value)
                .ok()
                .filter(|label| !label.trim().is_empty())
                .unwrap_or_else(|| parameter.format_value(value))
        });
        if is_list
            || has_named_parameter_choices(&choices)
            || prefers_parameter_choice_control(&parameter.name)
        {
            choices
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    PluginParameter {
        id: parameter.id,
        name: parameter.name,
        category,
        min: parameter.min,
        max: parameter.max,
        value: parameter.value,
        step_count,
        choices,
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
    use super::{
        AudioProcessError, has_named_parameter_choices, parameter_choice_labels,
        prefers_parameter_choice_control, try_audio_lock,
    };
    use std::sync::Mutex;

    #[test]
    fn builds_plugin_formatted_labels_for_each_discrete_value() {
        let labels = parameter_choice_labels(0.0, 1.0, 6, |value| {
            format!("Model {}", (value * 6.0).round() as usize + 1)
        });

        assert_eq!(
            labels,
            [
                "Model 1", "Model 2", "Model 3", "Model 4", "Model 5", "Model 6", "Model 7"
            ]
        );
    }

    #[test]
    fn skips_choice_lists_that_exceed_the_supported_size() {
        assert!(parameter_choice_labels(0.0, 128.0, 128, |value| value.to_string()).is_empty());
    }

    #[test]
    fn infers_named_choices_when_a_plugin_omits_the_list_flag() {
        let model_names = [
            "Classic Clean LGCY".to_owned(),
            "Classic Drive LGCY".to_owned(),
            "Modern Drive LGCY".to_owned(),
            "Classic Clean".to_owned(),
            "Classic Drive".to_owned(),
            "Modern Drive".to_owned(),
            "Classic Bass".to_owned(),
        ];
        let numeric_values = [
            "-12.0 dB".to_owned(),
            "-8.0 dB".to_owned(),
            "-4.0 dB".to_owned(),
            "0.0 dB".to_owned(),
            "4.0 dB".to_owned(),
            "8.0 dB".to_owned(),
            "12.0 dB".to_owned(),
        ];

        assert!(has_named_parameter_choices(&model_names));
        assert!(!has_named_parameter_choices(&numeric_values));
    }

    #[test]
    fn recognizes_selector_names_when_a_plugin_formats_choices_as_numbers() {
        assert!(prefers_parameter_choice_control("Circuit Type"));
        assert!(prefers_parameter_choice_control("Amp Mode"));
        assert!(!prefers_parameter_choice_control("Output Level"));
    }

    #[test]
    fn audio_processing_reports_busy_without_blocking_on_control_thread_lock() {
        let plugin = Mutex::new(());
        let _control_guard = plugin.lock().unwrap();

        assert!(matches!(
            try_audio_lock(&plugin),
            Err(AudioProcessError::Busy)
        ));
    }
}
