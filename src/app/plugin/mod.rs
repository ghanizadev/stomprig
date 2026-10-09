mod cache;
mod discovery;
mod hosting;
mod loader;
mod types;
mod windows;

pub(crate) use cache::{PluginCache, fingerprint as plugin_fingerprint};
pub(crate) use discovery::{DiscoveredPlugin, discover_plugins_with_progress};
pub(crate) use hosting::service_run_loop;
pub(crate) use loader::{
    PLUGIN_LOAD_CANCELLED, load as load_plugin, load_stack_with_fingerprints_progress,
    validate_plugin_stack,
};
pub(crate) use types::{PluginInfo, PluginParameter, RackPlugin};
pub(crate) use windows::EditorWindow;
