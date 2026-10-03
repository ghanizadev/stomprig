use std::sync::{Arc, Mutex};
use vst3_host::Plugin;

pub fn service_run_loop(plugin: &Arc<Mutex<Plugin>>) {
    if let Ok(mut plugin) = plugin.try_lock() {
        plugin.service_run_loop();
    }
}
