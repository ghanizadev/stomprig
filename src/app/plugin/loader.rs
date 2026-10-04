use vst3_host::{Plugin, simple};

pub fn load(path: &str, sample_rate: Option<f64>, block_size: usize) -> Result<Plugin, String> {
    log::info!("loading plugin {path} (sample_rate={sample_rate:?}, block_size={block_size})");
    match sample_rate {
        Some(rate) => simple::load_plugin_with_settings(path, rate, block_size)
            .map_err(|error| error.to_string()),
        None => simple::load_plugin(path).map_err(|error| error.to_string()),
    }
}

pub fn load_stack(
    plugin_paths: &[String],
    sample_rate: Option<f64>,
    block_size: usize,
) -> Result<Vec<Plugin>, String> {
    let configured_paths = plugin_paths
        .iter()
        .filter(|path| !path.trim().is_empty())
        .collect::<Vec<_>>();
    if configured_paths.is_empty() {
        return Err("The plugin stack is empty".to_string());
    }
    configured_paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            load(path, sample_rate, block_size)
                .map_err(|error| format!("Could not load plugin {} ({}): {error}", index, path))
        })
        .collect()
}
