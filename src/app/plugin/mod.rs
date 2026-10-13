mod cache;
mod clap;
mod clap_effect;
mod clap_host;
mod discovery;
#[cfg(target_os = "linux")]
mod hosting;
mod identity;
mod loader;
mod parameter_queue;
mod types;
mod vst3_probe;
mod windows;
#[cfg(windows)]
mod windows_sta;
#[cfg(windows)]
mod windows_vst3_host;

pub(crate) use cache::{PluginCache, fingerprint as plugin_fingerprint};
pub(crate) use clap::run_probe_helper as run_clap_probe_helper;
pub(crate) use clap_effect::{ClapAudioProcessor, ClapEffect};
pub(crate) use discovery::{DiscoveredPlugin, discover_plugins_with_progress};
#[cfg(target_os = "linux")]
pub(crate) use hosting::service_run_loop;
pub(crate) use identity::{PluginFormat, PluginIdentity};
pub(crate) use loader::{
    PLUGIN_LOAD_CANCELLED, load as load_plugin, load_stack_with_fingerprints_progress,
    validate_plugin_stack,
};
pub(crate) use parameter_queue::{
    PARAMETER_CHANGE_QUEUE_CAPACITY, ParameterChangeConsumer, ParameterChangeProducer,
    ParameterRuntimeFeedbackConsumer, ParameterRuntimeFeedbackProducer, parameter_change_queue,
    parameter_runtime_feedback_queue,
};
pub(crate) use types::{
    AudioProcessError, PluginInfo, PluginParameter, PluginParameterValue, RuntimePlugin,
    StompRigPlugin,
};
pub(crate) use vst3_probe::run_probe_helper as run_vst3_probe_helper;
pub(crate) use windows::EditorWindow;
#[cfg(windows)]
pub(crate) use windows_sta::WindowsStaApartment;
