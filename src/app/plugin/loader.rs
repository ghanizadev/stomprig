use vst3_host::{Plugin, simple};

// Replace these entries with the ordered VST3 plugins for the rack.
pub const PLUGIN_PATHS: &[&str] = &[
    r"C:\Program Files\Common Files\VST3\Kilohearts\kHs Reverb.vst3",
    r"C:\Program Files\Common Files\VST3\Kilohearts\kHs Delay.vst3",
];

pub fn load(path: &str, sample_rate: Option<f64>) -> Result<Plugin, String> {
    match sample_rate {
        Some(rate) => {
            simple::load_plugin_with_settings(path, rate, 512).map_err(|error| error.to_string())
        }
        None => simple::load_plugin(path).map_err(|error| error.to_string()),
    }
}

pub fn load_stack(sample_rate: Option<f64>) -> Result<Vec<Plugin>, String> {
    if PLUGIN_PATHS.is_empty() {
        return Err("The hard-coded plugin stack is empty".to_string());
    }
    PLUGIN_PATHS
        .iter()
        .enumerate()
        .map(|(index, path)| {
            load(path, sample_rate)
                .map_err(|error| format!("Could not load plugin {} ({}): {error}", index, path))
        })
        .collect()
}
