mod cache;
mod discovery;
mod hosting;
mod loader;
mod types;
mod windows;
#[cfg(windows)]
mod windows_sta;
#[cfg(windows)]
mod windows_vst3_host;

pub(crate) use cache::{PluginCache, fingerprint as plugin_fingerprint};
pub(crate) use discovery::{DiscoveredPlugin, PluginFormat, discover_plugins_with_progress};
pub(crate) use hosting::service_run_loop;
pub(crate) use loader::{
    PLUGIN_LOAD_CANCELLED, load as load_plugin, load_stack_with_fingerprints_progress,
    validate_plugin_stack,
};
pub(crate) use types::{PluginInfo, PluginParameter, PluginParameterValue, RackPlugin};
pub(crate) use windows::EditorWindow;
#[cfg(windows)]
pub(crate) use windows_sta::WindowsStaApartment;
