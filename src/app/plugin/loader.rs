use super::{PluginCache, PluginInfo, RackPlugin, plugin_fingerprint};
use std::path::Path;
use std::time::Duration;
use vst3_host::{Vst3Host, discovery::DEFAULT_PROBE_TIMEOUT};

pub(crate) const PLUGIN_LOAD_CANCELLED: &str = "plugin load cancelled";
const PLUGIN_RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
const PLUGIN_LOAD_TIMEOUT: Duration = Duration::from_secs(20);
const PLUGIN_LOAD_RECOVERY_ATTEMPTS: u32 = 1;

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
        let rate = sample_rate.unwrap_or(44_100.0);
        #[cfg(windows)]
        {
            let path = path.to_path_buf();
            let (plugin, editor_host) =
                super::windows_vst3_host::WindowsVst3EditorHost::spawn_for_plugin(move || {
                    let mut host = configured_vst3_host(rate, block_size)?;
                    host.load_plugin(path).map_err(|error| error.to_string())
                })?;
            Ok(RackPlugin::from_vst3(plugin, editor_host))
        }
        #[cfg(not(windows))]
        {
            let mut host = configured_vst3_host(rate, block_size)?;
            let plugin = host.load_plugin(path).map_err(|error| error.to_string())?;
            Ok(RackPlugin::from_vst3(plugin))
        }
    } else {
        Err(format!(
            "unsupported plugin format: {} (Rack supports VST3 plugins)",
            path.display()
        ))
    }
}

pub fn discover(path: &Path) -> Result<PluginInfo, String> {
    if is_vst3_path(path) {
        let metadata = vst3_host::probe_plugin_info_isolated(path, DEFAULT_PROBE_TIMEOUT)
            .map_err(|error| format!("could not read VST3 metadata in isolation: {error}"))?
            .info;
        Ok(PluginInfo {
            name: metadata.name,
            category: metadata.category,
            uid: metadata.uid,
        })
    } else {
        Err(format!(
            "unsupported plugin format: {} (Rack supports VST3 plugins)",
            path.display()
        ))
    }
}

fn configured_vst3_host(sample_rate: f64, block_size: usize) -> Result<Vst3Host, String> {
    Vst3Host::builder()
        .sample_rate(sample_rate)
        .block_size(block_size.max(1))
        .input_channels(2)
        .output_channels(2)
        // The helper owns a native editor event loop only on macOS. Windows and Linux must
        // keep VST3 plugins in-process so PluginWindow can attach their editor to the native
        // HWND/X11 window created by this process.
        .with_process_isolation(use_isolated_plugin_host())
        .response_timeout(PLUGIN_RESPONSE_TIMEOUT)
        .slow_command_timeout(PLUGIN_LOAD_TIMEOUT)
        .auto_recover_plugins(true)
        .auto_recover_max_retries(PLUGIN_LOAD_RECOVERY_ATTEMPTS)
        .build()
        .map_err(|error| format!("could not configure VST3 host: {error}"))
}

const fn use_isolated_plugin_host() -> bool {
    cfg!(target_os = "macos")
}

fn is_vst3_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("vst3"))
}

pub fn load_stack_with_fingerprints_progress(
    plugin_paths: &[String],
    expected_fingerprints: Option<&[u64]>,
    sample_rate: Option<f64>,
    block_size: usize,
    should_cancel: impl Fn() -> bool,
    mut report_progress: impl FnMut(usize, usize),
) -> Result<Vec<RackPlugin>, String> {
    let fingerprints = validate_plugin_stack(plugin_paths, expected_fingerprints)?;
    let configured_paths = plugin_paths
        .iter()
        .filter(|path| !path.trim().is_empty())
        .collect::<Vec<_>>();

    let mut cache = PluginCache::load();
    let mut plugins = Vec::with_capacity(configured_paths.len());

    for (index, path) in configured_paths.iter().enumerate() {
        if should_cancel() {
            return Err(PLUGIN_LOAD_CANCELLED.to_owned());
        }
        report_progress(index + 1, configured_paths.len());
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
    if should_cancel() {
        return Err(PLUGIN_LOAD_CANCELLED.to_owned());
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

#[cfg(test)]
mod tests {
    use super::use_isolated_plugin_host;

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn platforms_without_a_helper_editor_loop_keep_vst3_plugins_in_process() {
        assert!(!use_isolated_plugin_host());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_uses_the_helper_editor_loop_for_vst3_plugins() {
        assert!(use_isolated_plugin_host());
    }
}
