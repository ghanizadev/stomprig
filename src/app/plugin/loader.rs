use super::{PluginCache, PluginInfo, RackPlugin, plugin_fingerprint};
use std::path::Path;
use vst3_host::{get_detailed_plugin_info, simple};

pub fn load(path: &str, sample_rate: Option<f64>, block_size: usize) -> Result<RackPlugin, String> {
    let path = Path::new(path);
    let _ = plugin_fingerprint(path).map_err(|error| {
        format!(
            "plugin is not installed or cannot be fingerprinted ({}): {error}",
            path.display()
        )
    })?;
    log::info!(
        "loading plugin {} (sample_rate={sample_rate:?}, block_size={block_size})",
        path.display()
    );
    if is_vst3_path(path) {
        let plugin = match sample_rate {
            Some(rate) => {
                simple::load_plugin_with_settings(path.to_string_lossy().as_ref(), rate, block_size)
            }
            None => simple::load_plugin(path.to_string_lossy().as_ref()),
        }
        .map_err(|error| error.to_string())?;
        Ok(RackPlugin::from_vst3(plugin))
    } else {
        let _ = (sample_rate, block_size);
        RackPlugin::from_vst2(path)
    }
}

pub fn discover(path: &Path) -> Result<PluginInfo, String> {
    if is_vst3_path(path) {
        let metadata = get_detailed_plugin_info(path)
            .map_err(|error| format!("could not read VST3 metadata: {error}"))?
            .info;
        Ok(PluginInfo {
            name: metadata.name,
            category: metadata.category,
            uid: metadata.uid,
        })
    } else {
        RackPlugin::from_vst2(path).map(|plugin| plugin.info())
    }
}

fn is_vst3_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("vst3"))
}

pub fn load_stack(
    plugin_paths: &[String],
    sample_rate: Option<f64>,
    block_size: usize,
) -> Result<Vec<RackPlugin>, String> {
    load_stack_with_fingerprints(plugin_paths, None, sample_rate, block_size)
}

pub fn load_stack_with_fingerprints(
    plugin_paths: &[String],
    expected_fingerprints: Option<&[u64]>,
    sample_rate: Option<f64>,
    block_size: usize,
) -> Result<Vec<RackPlugin>, String> {
    let fingerprints = validate_plugin_stack(plugin_paths, expected_fingerprints)?;
    let configured_paths = plugin_paths
        .iter()
        .filter(|path| !path.trim().is_empty())
        .collect::<Vec<_>>();

    let mut cache = PluginCache::load();
    let mut plugins = Vec::with_capacity(configured_paths.len());

    for (index, path) in configured_paths.iter().enumerate() {
        let path = std::path::Path::new(path.as_str());
        let fingerprint = Some(fingerprints[index]);
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
        cache.update(
            path,
            fingerprints[index],
            &info.name,
            &info.category,
            &info.uid,
        );
        plugins.push(plugin);
    }
    cache.save();
    Ok(plugins)
}

pub fn validate_plugin_stack(
    plugin_paths: &[String],
    expected_fingerprints: Option<&[u64]>,
) -> Result<Vec<u64>, String> {
    let configured_paths = plugin_paths
        .iter()
        .filter(|path| !path.trim().is_empty())
        .collect::<Vec<_>>();
    let expected_fingerprints = expected_fingerprints.map(<[u64]>::to_vec);
    if let Some(expected_fingerprints) = &expected_fingerprints
        && expected_fingerprints.len() != configured_paths.len()
    {
        return Err("plugin paths and fingerprints do not have the same length".to_owned());
    }

    // Do this as a separate pass so a missing or changed plugin cannot leave the stack
    // partially instantiated.
    let fingerprints = configured_paths
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let path = Path::new(path.as_str());
            let fingerprint = plugin_fingerprint(path).map_err(|error| {
                format!(
                    "Could not find or fingerprint plugin {index} ({}): {error}",
                    path.display()
                )
            })?;
            if let Some(expected) = expected_fingerprints
                .as_ref()
                .and_then(|fingerprints| fingerprints.get(index))
                && fingerprint != *expected
            {
                return Err(format!(
                    "Plugin {index} has changed since the preset was saved ({}).",
                    path.display()
                ));
            }
            Ok(fingerprint)
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(fingerprints)
}
