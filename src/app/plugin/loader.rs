use vst3_host::{Plugin, simple};

pub fn load(path: &str, sample_rate: Option<f64>) -> Result<Plugin, String> {
    match sample_rate {
        Some(rate) => {
            simple::load_plugin_with_settings(path, rate, 512).map_err(|error| error.to_string())
        }
        None => simple::load_plugin(path).map_err(|error| error.to_string()),
    }
}
