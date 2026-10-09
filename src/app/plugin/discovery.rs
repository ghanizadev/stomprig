use super::cache::{PluginCache, PluginCacheEntry, fingerprint as plugin_fingerprint};
use super::loader::discover;
use crate::app::settings::PluginDiscoverySettings;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

const MAX_DISCOVERY_THREADS: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum PluginFormat {
    Vst3,
}

impl PluginFormat {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Vst3 => "VST3",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DiscoveredPlugin {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) category: String,
    pub(crate) format: PluginFormat,
}

pub(crate) fn discover_plugins_with_progress(
    settings: &PluginDiscoverySettings,
    mut report_progress: impl FnMut(f32, usize, usize),
) -> Vec<DiscoveredPlugin> {
    let mut plugin_paths = Vec::new();
    let mut visited_vst3_directories = HashSet::new();
    let mut visited_plugins = HashSet::new();
    if settings.vst3.enabled {
        for root in &settings.vst3.paths {
            collect_plugin_paths(
                Path::new(root),
                PluginFormat::Vst3,
                &mut plugin_paths,
                &mut visited_vst3_directories,
                &mut visited_plugins,
            );
        }
    }
    let plugin_count = plugin_paths.len();
    report_progress(20.0, 0, plugin_count);
    let plugin_paths = Arc::new(plugin_paths);
    let cache = Arc::new(Mutex::new(PluginCache::load()));
    let mut plugins = HashMap::new();
    let (sender, receiver) = mpsc::channel();
    let next_plugin = Arc::new(AtomicUsize::new(0));
    let worker_count = thread::available_parallelism()
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
                let (path, format) = path;
                let discovered = discover_plugin(path, *format, &cache);
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

fn collect_plugin_paths(
    directory: &Path,
    format: PluginFormat,
    plugin_paths: &mut Vec<(PathBuf, PluginFormat)>,
    visited_directories: &mut HashSet<PathBuf>,
    visited_plugins: &mut HashSet<PathBuf>,
) {
    let Ok(canonical_directory) = directory.canonicalize() else {
        return;
    };
    if is_plugin_path(&canonical_directory, format) {
        if visited_plugins.insert(canonical_directory.clone()) {
            plugin_paths.push((canonical_directory, format));
        }
        return;
    }
    if !visited_directories.insert(canonical_directory) {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if is_plugin_path(&path, format) {
            if let Ok(canonical_path) = path.canonicalize()
                && visited_plugins.insert(canonical_path.clone())
            {
                plugin_paths.push((canonical_path, format));
            }
        } else if path.is_dir() {
            collect_plugin_paths(
                &path,
                format,
                plugin_paths,
                visited_directories,
                visited_plugins,
            );
        }
    }
}

fn is_plugin_path(path: &Path, format: PluginFormat) -> bool {
    let Some(extension) = path.extension() else {
        return false;
    };
    match format {
        PluginFormat::Vst3 => extension.eq_ignore_ascii_case("vst3"),
    }
}

fn discover_plugin(
    path: &Path,
    format: PluginFormat,
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
            format,
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
            format: PluginFormat::Vst3,
        },
    )
}
