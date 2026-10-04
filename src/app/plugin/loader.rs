use super::{PluginCache, plugin_fingerprint};
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
    let mut cache = PluginCache::load();
    let mut plugins = Vec::with_capacity(configured_paths.len());

    for (index, path) in configured_paths.iter().enumerate() {
        let path = std::path::Path::new(path.as_str());
        let fingerprint = plugin_fingerprint(path).ok();
        let cached_entry = fingerprint.and_then(|fingerprint| cache.entry(path, fingerprint));
        if let Some(entry) = &cached_entry {
            log::debug!(
                "plugin cache hit at load stage for {} ({})",
                path.display(),
                entry.name
            );
        } else if cache.has_path(path) {
            log::info!(
                "plugin cache fingerprint changed; reloading {}",
                path.display()
            );
        } else {
            log::debug!("plugin cache miss at load stage for {}", path.display());
        }

        let plugin = match load(path.to_string_lossy().as_ref(), sample_rate, block_size) {
            Ok(plugin) => plugin,
            Err(error) => {
                cache.save();
                return Err(format!(
                    "Could not load plugin {} ({}): {error}",
                    index,
                    path.display()
                ));
            }
        };
        let info = plugin.info();
        if let Some(entry) = &cached_entry
            && !entry.name.eq(&info.name)
        {
            log::info!(
                "plugin cache name changed for {} ({} -> {})",
                path.display(),
                entry.name,
                info.name
            );
        }
        if let Some(fingerprint) = fingerprint {
            cache.update(path, fingerprint, &info.name, &info.category, &info.uid);
        }
        plugins.push(plugin);
    }
    cache.save();
    Ok(plugins)
}
