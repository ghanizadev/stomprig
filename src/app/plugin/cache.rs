use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const CACHE_VERSION: u32 = 1;
const CACHE_FILE_NAME: &str = "plugin-cache.toml";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct PluginCacheEntry {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) category: String,
    #[serde(default)]
    pub(crate) uid: String,
    pub(crate) fingerprint: u64,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct PluginCacheFile {
    version: u32,
    plugins: Vec<PluginCacheEntry>,
}

#[derive(Debug)]
pub(crate) struct PluginCache {
    path: Option<PathBuf>,
    file: PluginCacheFile,
}

impl PluginCache {
    pub(crate) fn purge() -> Result<(), String> {
        let path = cache_path()
            .ok_or_else(|| "plugin cache location is unavailable on this system".to_owned())?;

        match fs::remove_file(&path) {
            Ok(()) => {
                log::info!("purged plugin cache {}", path.display());
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "could not purge plugin cache {}: {error}",
                path.display()
            )),
        }
    }

    pub(crate) fn load() -> Self {
        let path = cache_path();
        let Some(path) = path.clone() else {
            log::debug!("plugin cache location is unavailable");
            return Self {
                path,
                file: empty_cache_file(),
            };
        };

        let file = match fs::read_to_string(&path) {
            Ok(contents) => match toml::from_str::<PluginCacheFile>(&contents) {
                Ok(file) if file.version == CACHE_VERSION => file,
                Ok(_) => {
                    log::info!("ignoring plugin cache with an unsupported version");
                    empty_cache_file()
                }
                Err(error) => {
                    log::warn!("could not read plugin cache {}: {error}", path.display());
                    empty_cache_file()
                }
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => empty_cache_file(),
            Err(error) => {
                log::warn!("could not open plugin cache {}: {error}", path.display());
                empty_cache_file()
            }
        };

        Self {
            path: Some(path),
            file,
        }
    }

    pub(crate) fn entry(&self, path: &Path, fingerprint: u64) -> Option<PluginCacheEntry> {
        let key = cache_key(path);
        self.file
            .plugins
            .iter()
            .find(|entry| {
                cache_key(Path::new(&entry.path)) == key && entry.fingerprint == fingerprint
            })
            .cloned()
    }

    pub(crate) fn entries(&self, path: &Path, fingerprint: u64) -> Vec<PluginCacheEntry> {
        let key = cache_key(path);
        self.file
            .plugins
            .iter()
            .filter(|entry| {
                cache_key(Path::new(&entry.path)) == key && entry.fingerprint == fingerprint
            })
            .cloned()
            .collect()
    }

    pub(crate) fn has_path(&self, path: &Path) -> bool {
        let key = cache_key(path);
        self.file
            .plugins
            .iter()
            .any(|entry| cache_key(Path::new(&entry.path)) == key)
    }

    pub(crate) fn update(
        &mut self,
        path: &Path,
        fingerprint: u64,
        name: &str,
        category: &str,
        uid: &str,
    ) {
        let key = cache_key(path);
        let entry = PluginCacheEntry {
            path: path.to_string_lossy().into_owned(),
            name: name.to_owned(),
            category: category.to_owned(),
            uid: uid.to_owned(),
            fingerprint,
        };

        if let Some(existing) = self
            .file
            .plugins
            .iter_mut()
            .find(|entry| cache_key(Path::new(&entry.path)) == key && entry.uid == uid)
        {
            *existing = entry;
        } else {
            self.file.plugins.push(entry);
        }
    }

    pub(crate) fn retain_bundle_entries(&mut self, path: &Path, plugin_ids: &[&str]) {
        let key = cache_key(path);
        self.file.plugins.retain(|entry| {
            cache_key(Path::new(&entry.path)) != key || plugin_ids.contains(&entry.uid.as_str())
        });
    }

    pub(crate) fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };

        let result = (|| -> Result<(), String> {
            let parent = path
                .parent()
                .ok_or_else(|| "plugin cache has no parent directory".to_owned())?;
            fs::create_dir_all(parent)
                .map_err(|error| format!("could not create cache directory: {error}"))?;
            let contents = toml::to_string_pretty(&self.file)
                .map_err(|error| format!("could not serialize plugin cache: {error}"))?;
            fs::write(path, contents)
                .map_err(|error| format!("could not save plugin cache: {error}"))
        })();

        if let Err(error) = result {
            log::warn!("{error}");
        }
    }
}

pub(crate) fn fingerprint(path: &Path) -> io::Result<u64> {
    let mut records = Vec::new();
    collect_fingerprint_records(path, path, &mut records)?;
    records.sort_unstable();

    // This is deliberately a metadata fingerprint rather than a content digest. It avoids
    // reading potentially large plugin binaries during startup while still detecting normal
    // installs, upgrades, removals, and bundle layout changes.
    let mut hash = 0xcbf29ce484222325_u64;
    for record in records {
        for byte in record.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    Ok(hash)
}

fn collect_fingerprint_records(
    root: &Path,
    path: &Path,
    records: &mut Vec<String>,
) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    let relative = path.strip_prefix(root).unwrap_or(path);
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok());
    let modified = modified
        .map(|time| format!("{}.{:09}", time.as_secs(), time.subsec_nanos()))
        .unwrap_or_else(|| "unknown".to_owned());
    let kind = if metadata.file_type().is_dir() {
        "d"
    } else if metadata.file_type().is_symlink() {
        "l"
    } else {
        "f"
    };
    records.push(format!(
        "{kind}\0{}\0{}\0{modified}",
        relative.to_string_lossy(),
        metadata.len()
    ));

    if metadata.file_type().is_dir() {
        let mut children = fs::read_dir(path)?.collect::<Result<Vec<_>, _>>()?;
        children.sort_unstable_by_key(|entry| entry.file_name());
        for child in children {
            collect_fingerprint_records(root, &child.path(), records)?;
        }
    }
    Ok(())
}

fn cache_key(path: &Path) -> String {
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let path = path.to_string_lossy().into_owned();
    #[cfg(windows)]
    {
        path.to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        path
    }
}

fn cache_path() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("PROGRAMDATA")
            .map(PathBuf::from)
            .map(|base| base.join("rack").join(CACHE_FILE_NAME))
    }

    #[cfg(not(windows))]
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .map(|base| base.join("rack").join(CACHE_FILE_NAME))
}

fn empty_cache_file() -> PluginCacheFile {
    PluginCacheFile {
        version: CACHE_VERSION,
        plugins: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::fingerprint;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn fingerprint_changes_when_bundle_contents_change() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after the epoch")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("rack-plugin-cache-{suffix}"));
        fs::create_dir_all(&directory).expect("test directory should be created");
        let file = directory.join("Contents").join("plugin.dll");
        fs::create_dir_all(file.parent().expect("test file should have a parent"))
            .expect("test contents directory should be created");
        fs::write(&file, b"before").expect("test file should be written");
        let before = fingerprint(&directory).expect("initial fingerprint should work");
        fs::write(&file, b"after with a different size")
            .expect("changed test file should be written");
        let after = fingerprint(&directory).expect("changed fingerprint should work");
        fs::remove_dir_all(&directory).expect("test directory should be removed");

        assert_ne!(before, after);
    }
}
