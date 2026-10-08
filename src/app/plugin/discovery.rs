use super::cache::{PluginCache, PluginCacheEntry, fingerprint as plugin_fingerprint};
use super::loader::discover;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

const MAX_DISCOVERY_THREADS: usize = 6;

#[derive(Clone, Debug)]
pub(crate) struct DiscoveredPlugin {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) category: String,
}

pub(crate) fn discover_plugins_with_progress(
    mut report_progress: impl FnMut(f32, usize, usize),
) -> Vec<DiscoveredPlugin> {
    let mut roots = Vec::new();
    #[cfg(windows)]
    {
        if let Some(program_files) = std::env::var_os("ProgramFiles") {
            let program_files = PathBuf::from(program_files);
            roots.push(program_files.join("Common Files").join("VST3"));
            roots.push(program_files.join("Steinberg").join("VSTPlugins"));
        }
        if let Some(program_files_x86) = std::env::var_os("ProgramFiles(x86)") {
            roots.push(
                PathBuf::from(program_files_x86)
                    .join("Common Files")
                    .join("VST3"),
            );
        }
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            let local_app_data = PathBuf::from(local_app_data);
            roots.push(
                local_app_data
                    .join("Programs")
                    .join("Common Files")
                    .join("VST3"),
            );
            roots.push(local_app_data.join("VST3"));
        }
        if let Some(app_data) = std::env::var_os("APPDATA") {
            roots.push(PathBuf::from(app_data).join("VST3"));
        }
    }

    #[cfg(target_os = "macos")]
    {
        roots.push(
            PathBuf::from("/Library")
                .join("Audio")
                .join("Plug-Ins")
                .join("VST3"),
        );
        roots.push(
            PathBuf::from("/Library")
                .join("Audio")
                .join("Plug-Ins")
                .join("VST"),
        );
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home)
                .join("Library")
                .join("Audio")
                .join("Plug-Ins");
            roots.push(home.join("VST3"));
            roots.push(home.join("VST"));
        }
    }

    #[cfg(target_os = "linux")]
    {
        roots.push(PathBuf::from("/usr").join("lib").join("vst3"));
        roots.push(PathBuf::from("/usr/local").join("lib").join("vst3"));
        roots.push(PathBuf::from("/usr").join("lib").join("vst"));
        roots.push(PathBuf::from("/usr/local").join("lib").join("vst"));
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            roots.push(home.join(".vst3"));
            roots.push(home.join(".vst"));
        }
    }

    let mut plugin_paths = Vec::new();
    for root in &roots {
        collect_plugin_paths(root, &mut plugin_paths);
    }

    let plugin_count = plugin_paths.len();
    report_progress(20.0, 0, plugin_count);
    let plugin_paths = Arc::new(plugin_paths);
    let cache = Arc::new(Mutex::new(PluginCache::load()));
    let mut plugins = HashMap::new();
    let (sender, receiver) = mpsc::channel();
    let next_plugin = Arc::new(AtomicUsize::new(0));
    let worker_count = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(MAX_DISCOVERY_THREADS)
        .min(plugin_count);
    let mut workers = Vec::with_capacity(worker_count);

    for _ in 0..worker_count {
        let plugin_paths = Arc::clone(&plugin_paths);
        let cache = Arc::clone(&cache);
        let next_plugin = Arc::clone(&next_plugin);
        let sender = sender.clone();
        workers.push(thread::spawn(move || {
            loop {
                let index = next_plugin.fetch_add(1, Ordering::Relaxed);
                let Some(path) = plugin_paths.get(index) else {
                    break;
                };
                let discovered = discover_plugin(path, &cache);
                if sender.send(discovered).is_err() {
                    break;
                }
            }
        }));
    }
    drop(sender);

    for (index, discovered) in receiver.into_iter().enumerate() {
        if let Some((identity, plugin)) = discovered {
            plugins.entry(identity).or_insert(plugin);
        }
        let progress = if plugin_count == 0 {
            95.0
        } else {
            20.0 + 75.0 * (index + 1) as f32 / plugin_count as f32
        };
        report_progress(progress, plugins.len(), plugin_count);
    }
    for worker in workers {
        worker
            .join()
            .expect("plugin discovery worker thread panicked");
    }
    if let Ok(cache) = cache.lock() {
        cache.save();
    } else {
        log::warn!("could not save plugin cache because its lock was poisoned");
    }

    let mut plugins: Vec<_> = plugins.into_values().collect();
    plugins.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            })
            .then_with(|| left.path.cmp(&right.path))
    });
    report_progress(95.0, plugins.len(), plugin_count);
    plugins
}

fn collect_plugin_paths(directory: &Path, plugin_paths: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if is_mac_vst2_bundle(&path) {
            if let Some(binary) = mac_vst2_binary(&path) {
                if is_compatible_mac_vst2_binary(&binary) {
                    plugin_paths.push(binary);
                } else {
                    log::info!(
                        "skipping incompatible macOS VST2 plugin {} for {} host",
                        path.display(),
                        mac_host_arch()
                    );
                }
            }
        } else if is_plugin_path(&path) {
            if is_mac_vst2_binary(&path) && !is_compatible_mac_vst2_binary(&path) {
                log::info!(
                    "skipping incompatible macOS VST2 plugin {} for {} host",
                    path.display(),
                    mac_host_arch()
                );
            } else {
                plugin_paths.push(path);
            }
        } else if path.is_dir() {
            collect_plugin_paths(&path, plugin_paths);
        }
    }
}

#[cfg(target_os = "macos")]
fn is_mac_vst2_bundle(path: &Path) -> bool {
    path.is_dir()
        && path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("vst"))
}

#[cfg(not(target_os = "macos"))]
fn is_mac_vst2_bundle(_path: &Path) -> bool {
    false
}

#[cfg(target_os = "macos")]
fn mac_vst2_binary(bundle: &Path) -> Option<PathBuf> {
    let macos_directory = bundle.join("Contents").join("MacOS");
    if let Some(bundle_name) = bundle.file_stem() {
        let expected_binary = macos_directory.join(bundle_name);
        if expected_binary.is_file() {
            return Some(expected_binary);
        }
    }

    let mut binaries = fs::read_dir(macos_directory)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    binaries.sort_unstable();
    binaries.into_iter().next()
}

#[cfg(not(target_os = "macos"))]
fn mac_vst2_binary(_bundle: &Path) -> Option<PathBuf> {
    None
}

#[cfg(target_os = "macos")]
fn is_mac_vst2_binary(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("dylib"))
}

#[cfg(not(target_os = "macos"))]
fn is_mac_vst2_binary(_path: &Path) -> bool {
    false
}

#[cfg(target_os = "macos")]
fn is_compatible_mac_vst2_binary(path: &Path) -> bool {
    let Ok(data) = fs::read(path) else {
        return true;
    };
    let Some(architectures) = mac_macho_architectures(&data) else {
        return true;
    };
    architectures
        .iter()
        .any(|architecture| *architecture == mac_host_arch())
}

#[cfg(not(target_os = "macos"))]
fn is_compatible_mac_vst2_binary(_path: &Path) -> bool {
    true
}

#[cfg(target_os = "macos")]
fn mac_host_arch() -> &'static str {
    #[cfg(target_arch = "aarch64")]
    {
        "arm64"
    }
    #[cfg(target_arch = "x86_64")]
    {
        "x86_64"
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        "unknown"
    }
}

#[cfg(not(target_os = "macos"))]
fn mac_host_arch() -> &'static str {
    "unknown"
}

#[cfg(target_os = "macos")]
fn mac_macho_architectures(data: &[u8]) -> Option<Vec<&'static str>> {
    fn architecture_name(cpu_type: u32) -> Option<&'static str> {
        match cpu_type {
            0x0100_0007 => Some("x86_64"),
            0x0100_000c => Some("arm64"),
            _ => None,
        }
    }

    fn read_u32(data: &[u8], offset: usize, big_endian: bool) -> Option<u32> {
        let bytes = data.get(offset..offset.checked_add(4)?)?;
        let bytes = [bytes[0], bytes[1], bytes[2], bytes[3]];
        Some(if big_endian {
            u32::from_be_bytes(bytes)
        } else {
            u32::from_le_bytes(bytes)
        })
    }

    if data.len() < 8 {
        return None;
    }

    let magic = [data[0], data[1], data[2], data[3]];
    let mut architectures = Vec::new();
    match magic {
        [0xca, 0xfe, 0xba, 0xbe] | [0xca, 0xfe, 0xba, 0xbf] => {
            let entry_size = if magic[3] == 0xbe { 20 } else { 32 };
            let count = read_u32(data, 4, true)? as usize;
            for index in 0..count {
                let offset = 8usize.checked_add(index.checked_mul(entry_size)?)?;
                if let Some(architecture) = architecture_name(read_u32(data, offset, true)?)
                    && !architectures.contains(&architecture)
                {
                    architectures.push(architecture);
                }
            }
        }
        [0xbe, 0xba, 0xfe, 0xca] | [0xbf, 0xba, 0xfe, 0xca] => {
            let entry_size = if magic[0] == 0xbe { 20 } else { 32 };
            let count = read_u32(data, 4, false)? as usize;
            for index in 0..count {
                let offset = 8usize.checked_add(index.checked_mul(entry_size)?)?;
                if let Some(architecture) = architecture_name(read_u32(data, offset, false)?)
                    && !architectures.contains(&architecture)
                {
                    architectures.push(architecture);
                }
            }
        }
        [0xce, 0xfa, 0xed, 0xfe] | [0xcf, 0xfa, 0xed, 0xfe] => {
            if let Some(architecture) = architecture_name(read_u32(data, 4, false)?) {
                architectures.push(architecture);
            }
        }
        [0xfe, 0xed, 0xfa, 0xce] | [0xfe, 0xed, 0xfa, 0xcf] => {
            if let Some(architecture) = architecture_name(read_u32(data, 4, true)?) {
                architectures.push(architecture);
            }
        }
        _ => return None,
    }

    Some(architectures)
}

fn is_plugin_path(path: &Path) -> bool {
    let Some(extension) = path.extension() else {
        return false;
    };
    let extension = extension.to_string_lossy();
    if extension.eq_ignore_ascii_case("vst3") {
        return true;
    }

    #[cfg(windows)]
    {
        extension.eq_ignore_ascii_case("dll")
    }
    #[cfg(target_os = "macos")]
    {
        extension.eq_ignore_ascii_case("dylib") || extension.eq_ignore_ascii_case("vst")
    }
    #[cfg(target_os = "linux")]
    {
        extension.eq_ignore_ascii_case("so")
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        false
    }
}

fn discover_plugin(
    path: &Path,
    cache: &Arc<Mutex<PluginCache>>,
) -> Option<(String, DiscoveredPlugin)> {
    let fingerprint = plugin_fingerprint(path).ok();
    if let Some(fingerprint) = fingerprint
        && let Ok(cache) = cache.lock()
        && let Some(entry) = cache.entry(path, fingerprint)
    {
        log::debug!("plugin cache hit for {} ({})", path.display(), entry.name);
        return Some(discovered_plugin_from_cache(entry));
    }

    if fingerprint.is_some()
        && let Ok(cache) = cache.lock()
        && cache.has_path(path)
    {
        log::info!(
            "plugin cache fingerprint changed; refreshing {}",
            path.display()
        );
    } else {
        log::debug!("plugin cache miss for {}", path.display());
    }

    let metadata = match discover(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            log::warn!(
                "could not read plugin metadata for {}: {error}",
                path.display()
            );
            return None;
        }
    };

    let name = if metadata.name.trim().is_empty() {
        path.file_stem()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Unknown plugin".to_owned())
    } else {
        metadata.name
    };
    let category = if metadata.category.trim().is_empty() {
        "Uncategorized".to_owned()
    } else {
        metadata.category.trim().to_owned()
    };
    let path = path.to_string_lossy().into_owned();
    let identity = if metadata.uid.trim().is_empty() {
        path.to_ascii_lowercase()
    } else {
        metadata.uid.to_ascii_lowercase()
    };

    if let Some(fingerprint) = fingerprint
        && let Ok(mut cache) = cache.lock()
    {
        cache.update(
            Path::new(&path),
            fingerprint,
            &name,
            &category,
            &metadata.uid,
        );
    }

    Some((
        identity,
        DiscoveredPlugin {
            name,
            path,
            category,
        },
    ))
}

fn discovered_plugin_from_cache(entry: PluginCacheEntry) -> (String, DiscoveredPlugin) {
    let identity = if entry.uid.trim().is_empty() {
        entry.path.to_ascii_lowercase()
    } else {
        entry.uid.to_ascii_lowercase()
    };
    (
        identity,
        DiscoveredPlugin {
            name: entry.name,
            path: entry.path,
            category: entry.category,
        },
    )
}
