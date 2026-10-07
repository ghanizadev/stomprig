use super::RackPlugin;
use std::sync::{Arc, Mutex};

pub(crate) fn service_run_loop(plugin: &Arc<Mutex<RackPlugin>>) {
    let Ok(plugin) = plugin.try_lock() else {
        return;
    };
    if let RackPlugin::Vst3 { plugin } = &*plugin
        && let Ok(mut plugin) = plugin.try_lock()
    {
        plugin.service_run_loop();
    }
}
