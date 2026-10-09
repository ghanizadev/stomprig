use super::cache::{PluginCache, PluginCacheEntry, fingerprint as plugin_fingerprint};
use super::clap::probe_bundle_isolated;
use super::identity::{PluginFormat, PluginIdentity};
use super::loader::discover;
use crate::app::settings::PluginDiscoverySettings;
use std::collections::HashSet;
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
    pub(crate) format: PluginFormat,
    pub(crate) identity: PluginIdentity,
}

impl DiscoveredPlugin {
    pub(crate) fn config_key(&self) -> String {
        self.identity.config_key()
    }
}

pub(crate) fn discover_plugins_with_progress(
    settings: &PluginDiscoverySettings,
    mut report_progress: impl FnMut(f32, usize, usize),
) -> Vec<DiscoveredPlugin> {
    let mut plugin_paths = Vec::new();
    collect_enabled_paths(
        &settings.vst3,
        PluginFormat::Vst3,
        &mut plugin_paths,
        &mut HashSet::new(),
        &mut HashSet::new(),
    );
    collect_enabled_paths(
        &settings.clap,
        PluginFormat::Clap,
        &mut plugin_paths,
        &mut HashSet::new(),
        &mut HashSet::new(),
    );

    let bundle_count = plugin_paths.len();
    report_progress(20.0, 0, bundle_count);
    let plugin_paths = Arc::new(plugin_paths);
    let cache = Arc::new(Mutex::new(PluginCache::load()));
    let mut plugins = Vec::new();
    let mut identities = Vec::<PluginIdentity>::new();
    let (sender, receiver) = mpsc::channel();
    let next_plugin = Arc::new(AtomicUsize::new(0));
    let worker_count = thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(MAX_DISCOVERY_THREADS)
        .min(bundle_count);
    let mut workers = Vec::with_capacity(worker_count);

    for _ in 0..worker_count {
        let plugin_paths = Arc::clone(&plugin_paths);
        let cache = Arc::clone(&cache);
        let next_plugin = Arc::clone(&next_plugin);
        let sender = sender.clone();
        workers.push(thread::spawn(move || {
            loop {
                let index = next_plugin.fetch_add(1, Ordering::Relaxed);
                let Some((path, format)) = plugin_paths.get(index) else {
                    break;
                };
                let discovered = discover_bundle(path, *format, &cache);
                if sender.send(discovered).is_err() {
                    break;
                }
            }
        }));
    }
    drop(sender);

    for (index, discovered) in receiver.into_iter().enumerate() {
        for plugin in discovered {
            if !identities
                .iter()
                .any(|identity| identity.same_plugin(&plugin.identity))
            {
                identities.push(plugin.identity.clone());
                plugins.push(plugin);
            }
        }
        let progress = if bundle_count == 0 {
            95.0
        } else {
            20.0 + 75.0 * (index + 1) as f32 / bundle_count as f32
        };
        report_progress(progress, plugins.len(), bundle_count);
    }
    for worker in workers {
        if worker.join().is_err() {
            log::error!("plugin discovery worker thread panicked");
        }
    }
    if let Ok(cache) = cache.lock() {
        cache.save();
    } else {
        log::warn!("could not save plugin cache because its lock was poisoned");
    }

    plugins.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            })
            .then_with(|| left.format.cmp(&right.format))
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.identity.plugin_id.cmp(&right.identity.plugin_id))
    });
    report_progress(95.0, plugins.len(), bundle_count);
    plugins
}

fn collect_enabled_paths(
    settings: &super::super::settings::PluginFormatSettings,
    format: PluginFormat,
    plugin_paths: &mut Vec<(PathBuf, PluginFormat)>,
    visited_directories: &mut HashSet<PathBuf>,
    visited_plugins: &mut HashSet<PathBuf>,
) {
    if !settings.enabled {
        return;
    }
    for root in &settings.paths {
        collect_plugin_paths(
            Path::new(root),
            format,
            plugin_paths,
            visited_directories,
            visited_plugins,
        );
    }
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
    let expected = match format {
        PluginFormat::Vst3 => "vst3",
        PluginFormat::Clap => "clap",
    };
    extension.eq_ignore_ascii_case(expected)
}

fn discover_bundle(
    path: &Path,
    format: PluginFormat,
    cache: &Arc<Mutex<PluginCache>>,
) -> Vec<DiscoveredPlugin> {
    let fingerprint = plugin_fingerprint(path).ok();
    if let Some(fingerprint) = fingerprint
        && let Ok(cache) = cache.lock()
    {
        let entries = match format {
            PluginFormat::Vst3 => cache.entry(path, fingerprint).into_iter().collect(),
            PluginFormat::Clap => cache.entries(path, fingerprint),
        };
        if !entries.is_empty() {
            log::debug!(
                "plugin cache hit for {} ({} entries)",
                path.display(),
                entries.len()
            );
            return entries
                .into_iter()
                .map(|entry| discovered_plugin_from_cache(entry, format))
                .collect();
        }
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

    let metadata = match format {
        PluginFormat::Vst3 => match discover(path) {
            Ok(metadata) => vec![super::clap::ClapPluginMetadata {
                id: metadata.uid,
                name: metadata.name,
                category: metadata.category,
            }],
            Err(error) => {
                log::warn!(
                    "could not read plugin metadata for {}: {error}",
                    path.display()
                );
                return Vec::new();
            }
        },
        PluginFormat::Clap => match probe_bundle_isolated(path) {
            Ok(metadata) => metadata,
            Err(error) => {
                log::warn!("could not probe CLAP bundle {}: {error}", path.display());
                return Vec::new();
            }
        },
    };

    if fingerprint.is_some()
        && let Ok(mut cache) = cache.lock()
    {
        let plugin_ids = metadata
            .iter()
            .map(|metadata| metadata.id.as_str())
            .collect::<Vec<_>>();
        cache.retain_bundle_entries(path, &plugin_ids);
    }

    let path_string = path.to_string_lossy().into_owned();
    let mut discovered = Vec::with_capacity(metadata.len());
    for metadata in metadata {
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
        let plugin_id = (!metadata.id.trim().is_empty()).then_some(metadata.id);
        if let Some(fingerprint) = fingerprint
            && let Ok(mut cache) = cache.lock()
        {
            cache.update(
                path,
                fingerprint,
                &name,
                &category,
                plugin_id.as_deref().unwrap_or_default(),
            );
        }
        let identity = match format {
            PluginFormat::Clap => {
                PluginIdentity::clap(path_string.clone(), plugin_id.unwrap_or_default())
            }
            PluginFormat::Vst3 => PluginIdentity::vst3(path_string.clone(), plugin_id),
        };
        discovered.push(DiscoveredPlugin {
            name,
            path: path_string.clone(),
            category,
            format,
            identity,
        });
    }
    discovered
}

fn discovered_plugin_from_cache(entry: PluginCacheEntry, format: PluginFormat) -> DiscoveredPlugin {
    let plugin_id = (!entry.uid.trim().is_empty()).then_some(entry.uid);
    let identity = match format {
        PluginFormat::Clap => {
            PluginIdentity::clap(entry.path.clone(), plugin_id.unwrap_or_default())
        }
        PluginFormat::Vst3 => PluginIdentity::vst3(entry.path.clone(), plugin_id),
    };
    DiscoveredPlugin {
        name: entry.name,
        path: entry.path,
        category: entry.category,
        format,
        identity,
    }
}
