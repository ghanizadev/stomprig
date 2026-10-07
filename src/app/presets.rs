use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use super::plugin_runtime::PluginSnapshot;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Preset {
    pub(crate) plugins: Vec<PresetPlugin>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct PresetPlugin {
    pub(crate) path: String,
    pub(crate) parameters: Vec<PresetParameter>,
    #[serde(default)]
    pub(crate) bypassed: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct PresetParameter {
    pub(crate) id: u32,
    pub(crate) value: f64,
}

#[derive(Clone, Debug)]
pub(crate) struct PresetEntry {
    pub(crate) segments: Vec<String>,
    pub(crate) preset: Preset,
}

pub(crate) struct PresetStore {
    directory: PathBuf,
    entries: Vec<PresetEntry>,
}

impl PresetStore {
    pub(crate) fn load() -> Self {
        let directory = presets_directory();
        let mut store = Self {
            directory,
            entries: Vec::new(),
        };
        store.refresh();
        store
    }

    pub(crate) fn entries(&self) -> &[PresetEntry] {
        &self.entries
    }

    pub(crate) fn contains_name(&self, input_name: &str) -> Result<bool, String> {
        let segments = parse_preset_name(input_name)?;
        Ok(self.entries.iter().any(|entry| entry.segments == segments))
    }

    pub(crate) fn save(&mut self, input_name: &str, preset: &Preset) -> Result<(), String> {
        let segments = parse_preset_name(input_name)?;
        let file_name = format!("{}.toml", segments.last().expect("validated preset name"));
        let directory = segments[..segments.len() - 1]
            .iter()
            .fold(self.directory.clone(), |directory, segment| {
                directory.join(segment)
            });
        fs::create_dir_all(&directory)
            .map_err(|error| format!("Could not create preset directory: {error}"))?;
        let path = directory.join(file_name);
        let contents = toml::to_string_pretty(preset)
            .map_err(|error| format!("Could not serialize preset: {error}"))?;
        fs::write(&path, contents).map_err(|error| format!("Could not save preset: {error}"))?;
        self.refresh();
        Ok(())
    }

    fn refresh(&mut self) {
        self.entries.clear();
        collect_entries(&self.directory, &mut Vec::new(), &mut self.entries);
        self.entries.sort_by_cached_key(|entry| {
            entry
                .segments
                .iter()
                .map(|segment| segment.to_ascii_lowercase())
                .collect::<Vec<_>>()
        });
    }
}

pub(crate) fn preset_from_snapshots(
    plugin_paths: &[String],
    snapshots: &[PluginSnapshot],
    bypassed: &[bool],
) -> Result<Preset, String> {
    if plugin_paths.len() != snapshots.len() || snapshots.len() != bypassed.len() {
        return Err("Plugin stack changed while saving the preset.".to_owned());
    }

    let plugins = plugin_paths
        .iter()
        .zip(snapshots)
        .zip(bypassed)
        .map(|((path, snapshot), bypassed)| PresetPlugin {
            path: path.clone(),
            parameters: snapshot
                .parameters
                .iter()
                .map(|parameter| PresetParameter {
                    id: parameter.id,
                    value: parameter.value,
                })
                .collect(),
            bypassed: *bypassed,
        })
        .collect();

    Ok(Preset { plugins })
}

fn collect_entries(directory: &Path, categories: &mut Vec<String>, entries: &mut Vec<PresetEntry>) {
    let Ok(read_dir) = fs::read_dir(directory) else {
        return;
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let Some(name) = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
            else {
                continue;
            };
            categories.push(name);
            collect_entries(&path, categories, entries);
            categories.pop();
        } else if path
            .extension()
            .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("toml"))
        {
            let Ok(contents) = fs::read_to_string(&path) else {
                log::warn!("could not read preset {}", path.display());
                continue;
            };
            let Ok(preset) = toml::from_str::<Preset>(&contents) else {
                log::warn!("could not parse preset {}", path.display());
                continue;
            };
            let Some(name) = path
                .file_stem()
                .map(|name| name.to_string_lossy().into_owned())
            else {
                continue;
            };
            let mut segments = categories.clone();
            segments.push(name);
            entries.push(PresetEntry { segments, preset });
        }
    }
}

fn parse_preset_name(input: &str) -> Result<Vec<String>, String> {
    let segments = input
        .split('/')
        .map(str::trim)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if segments.is_empty() || segments.iter().any(String::is_empty) {
        return Err("Enter a preset name, optionally using / for categories.".to_owned());
    }
    if segments.iter().any(|segment| {
        segment == "."
            || segment == ".."
            || segment.contains('\\')
            || segment.chars().any(char::is_control)
            || segment
                .chars()
                .any(|character| matches!(character, ':' | '*' | '?' | '"' | '<' | '>' | '|'))
    }) {
        return Err("Preset names contain an invalid file or folder character.".to_owned());
    }
    Ok(segments)
}

fn presets_directory() -> PathBuf {
    #[cfg(windows)]
    let base = env::var_os("LOCALAPPDATA")
        .or_else(|| env::var_os("APPDATA"))
        .map(PathBuf::from)
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    #[cfg(target_os = "macos")]
    let base = env::var_os("HOME")
        .map(PathBuf::from)
        .map(|path| path.join("Library").join("Application Support"))
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    #[cfg(all(unix, not(target_os = "macos")))]
    let base = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|path| PathBuf::from(path).join(".local/share")))
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    base.join("rack").join("presets")
}

#[cfg(test)]
mod tests {
    use super::parse_preset_name;

    #[test]
    fn splits_categories_from_preset_name() {
        assert_eq!(
            parse_preset_name("Metal / Iron Man").unwrap(),
            ["Metal", "Iron Man"]
        );
    }

    #[test]
    fn rejects_empty_or_unsafe_segments() {
        assert!(parse_preset_name("").is_err());
        assert!(parse_preset_name("Metal/").is_err());
        assert!(parse_preset_name("../Iron Man").is_err());
        assert!(parse_preset_name("Metal\\Iron Man").is_err());
    }
}
