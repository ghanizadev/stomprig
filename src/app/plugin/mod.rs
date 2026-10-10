mod cache;
mod clap;
mod clap_effect;
mod clap_host;
mod discovery;
mod hosting;
mod identity;
mod loader;
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
pub(crate) use hosting::service_run_loop;
pub(crate) use identity::{PluginFormat, PluginIdentity};
pub(crate) use loader::{
    PLUGIN_LOAD_CANCELLED, load as load_plugin, load_stack_with_fingerprints_progress,
    validate_plugin_stack,
};
pub(crate) use types::{
    PluginInfo, PluginParameter, PluginParameterValue, RackPlugin, RuntimePlugin,
};
pub(crate) use vst3_probe::run_probe_helper as run_vst3_probe_helper;
pub(crate) use windows::EditorWindow;
#[cfg(windows)]
pub(crate) use windows_sta::WindowsStaApartment;
