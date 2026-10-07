use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use super::midi::{PresetMidiMapping, validate_midi_mapping};
use super::plugin_runtime::PluginSnapshot;

mod fingerprint_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Value {
        Text(String),
        Signed(i64),
        Unsigned(u64),
    }

    pub(super) fn serialize<S>(value: &Option<u64>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        value.map(|value| value.to_string()).serialize(serializer)
    }

    pub(super) fn deserialize<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Option::<Value>::deserialize(deserializer)?;
        match value {
            None => Ok(None),
            Some(Value::Text(value)) => value.parse().map(Some).map_err(serde::de::Error::custom),
            Some(Value::Signed(value)) if value >= 0 => Ok(Some(value as u64)),
            Some(Value::Signed(_)) => Err(serde::de::Error::custom(
                "plugin fingerprint cannot be negative",
            )),
            Some(Value::Unsigned(value)) => Ok(Some(value)),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Preset {
    pub(crate) plugins: Vec<PresetPlugin>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct PresetPlugin {
    pub(crate) path: String,
    #[serde(default, with = "fingerprint_serde")]
    pub(crate) fingerprint: Option<u64>,
    pub(crate) parameters: Vec<PresetParameter>,
    #[serde(default)]
    pub(crate) bypassed: bool,
    #[serde(default)]
    pub(crate) midi_mappings: Vec<PresetMidiMapping>,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct LivePreset {
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) presets: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct LivePresetEntry {
    pub(crate) segments: Vec<String>,
    pub(crate) preset: LivePreset,
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

    pub(crate) fn get(&self, input_name: &str) -> Option<&Preset> {
        let segments = parse_preset_name(input_name).ok()?;
        self.entries
            .iter()
            .find(|entry| entry.segments == segments)
            .map(|entry| &entry.preset)
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

    pub(crate) fn delete(&mut self, input_name: &str) -> Result<(), String> {
        let segments = parse_preset_name(input_name)?;
        let file_name = format!("{}.toml", segments.last().expect("validated preset name"));
        let directory = segments[..segments.len() - 1]
            .iter()
            .fold(self.directory.clone(), |directory, segment| {
                directory.join(segment)
            });
        let path = directory.join(file_name);
        fs::remove_file(&path)
            .map_err(|error| format!("Could not delete preset \"{input_name}\": {error}"))?;
        self.refresh();
        Ok(())
    }

    pub(crate) fn export(path: &Path, preset: &Preset) -> Result<(), String> {
        let preset = preset_with_fingerprints(preset)?;
        let contents = toml::to_string_pretty(&preset)
            .map_err(|error| format!("Could not serialize preset: {error}"))?;
        fs::write(path, contents).map_err(|error| format!("Could not export preset: {error}"))
    }

    pub(crate) fn import(path: &Path) -> Result<Preset, String> {
        let contents = fs::read_to_string(path)
            .map_err(|error| format!("Could not read preset {}: {error}", path.display()))?;
        let preset = toml::from_str::<Preset>(&contents)
            .map_err(|error| format!("Could not parse preset {}: {error}", path.display()))?;
        validate_plugin_installations(&preset)?;
        Ok(preset)
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

pub(crate) struct LivePresetStore {
    directory: PathBuf,
    entries: Vec<LivePresetEntry>,
}

impl LivePresetStore {
    pub(crate) fn load() -> Self {
        let directory = live_presets_directory();
        let mut store = Self {
            directory,
            entries: Vec::new(),
        };
        store.refresh();
        store
    }

    pub(crate) fn entries(&self) -> &[LivePresetEntry] {
        &self.entries
    }

    pub(crate) fn get(&self, input_name: &str) -> Option<&LivePreset> {
        let segments = parse_preset_name(input_name).ok()?;
        self.entries
            .iter()
            .find(|entry| entry.segments == segments)
            .map(|entry| &entry.preset)
    }

    pub(crate) fn save(
        &mut self,
        input_name: &str,
        live_preset: &LivePreset,
    ) -> Result<(), String> {
        let segments = parse_preset_name(input_name)?;
        let file_name = format!(
            "{}.toml",
            segments.last().expect("validated live preset name")
        );
        let directory = segments[..segments.len() - 1]
            .iter()
            .fold(self.directory.clone(), |directory, segment| {
                directory.join(segment)
            });
        fs::create_dir_all(&directory)
            .map_err(|error| format!("Could not create live preset directory: {error}"))?;
        let path = directory.join(file_name);
        let mut live_preset = live_preset.clone();
        live_preset.name = segments.join("/");
        let contents = toml::to_string_pretty(&live_preset)
            .map_err(|error| format!("Could not serialize live preset: {error}"))?;
        fs::write(&path, contents)
            .map_err(|error| format!("Could not save live preset: {error}"))?;
        self.refresh();
        Ok(())
    }

    pub(crate) fn delete(&mut self, input_name: &str) -> Result<(), String> {
        let segments = parse_preset_name(input_name)?;
        let file_name = format!(
            "{}.toml",
            segments.last().expect("validated live preset name")
        );
        let directory = segments[..segments.len() - 1]
            .iter()
            .fold(self.directory.clone(), |directory, segment| {
                directory.join(segment)
            });
        fs::remove_file(directory.join(file_name))
            .map_err(|error| format!("Could not delete live preset \"{input_name}\": {error}"))?;
        self.refresh();
        Ok(())
    }

    pub(crate) fn export(path: &Path, live_preset: &LivePreset) -> Result<(), String> {
        let contents = toml::to_string_pretty(live_preset)
            .map_err(|error| format!("Could not serialize live preset: {error}"))?;
        fs::write(path, contents).map_err(|error| format!("Could not export live preset: {error}"))
    }

    pub(crate) fn import(path: &Path) -> Result<LivePreset, String> {
        let contents = fs::read_to_string(path)
            .map_err(|error| format!("Could not read live preset {}: {error}", path.display()))?;
        let live_preset = toml::from_str::<LivePreset>(&contents)
            .map_err(|error| format!("Could not parse live preset {}: {error}", path.display()))?;
        for name in &live_preset.presets {
            parse_preset_name(name)?;
        }
        Ok(live_preset)
    }

    fn refresh(&mut self) {
        self.entries.clear();
        collect_live_entries(&self.directory, &mut Vec::new(), &mut self.entries);
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
    midi_mappings: &[Vec<PresetMidiMapping>],
) -> Result<Preset, String> {
    if plugin_paths.len() != snapshots.len()
        || snapshots.len() != bypassed.len()
        || snapshots.len() != midi_mappings.len()
    {
        return Err("Plugin stack changed while saving the preset.".to_owned());
    }

    let plugins =
        plugin_paths
            .iter()
            .zip(snapshots)
            .zip(bypassed)
            .zip(midi_mappings)
            .map(|(((path, snapshot), bypassed), midi_mappings)| {
                for mapping in midi_mappings {
                    validate_midi_mapping(mapping)?;
                }
                Ok(PresetPlugin {
                    path: path.clone(),
                    fingerprint: Some(super::plugin::plugin_fingerprint(Path::new(path)).map_err(
                        |error| format!("Could not fingerprint plugin {}: {error}", path),
                    )?),
                    parameters: snapshot
                        .parameters
                        .iter()
                        .map(|parameter| PresetParameter {
                            id: parameter.id,
                            value: parameter.value,
                        })
                        .collect(),
                    bypassed: *bypassed,
                    midi_mappings: midi_mappings.clone(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;

    Ok(Preset { plugins })
}

pub(crate) fn validate_plugin_installations(preset: &Preset) -> Result<(), String> {
    for (plugin_index, plugin) in preset.plugins.iter().enumerate() {
        for mapping in &plugin.midi_mappings {
            validate_midi_mapping(mapping).map_err(|error| {
                format!("invalid MIDI mapping for plugin {plugin_index}: {error}")
            })?;
        }
    }
    let paths = preset
        .plugins
        .iter()
        .map(|plugin| plugin.path.clone())
        .collect::<Vec<_>>();
    let fingerprints = preset
        .plugins
        .iter()
        .map(|plugin| plugin.fingerprint)
        .collect::<Option<Vec<_>>>();
    super::plugin::validate_plugin_stack(&paths, fingerprints.as_deref()).map(|_| ())
}

fn preset_with_fingerprints(preset: &Preset) -> Result<Preset, String> {
    let mut preset = preset.clone();
    for plugin in &mut preset.plugins {
        if plugin.fingerprint.is_none() {
            plugin.fingerprint = Some(
                super::plugin::plugin_fingerprint(Path::new(&plugin.path)).map_err(|error| {
                    format!("Could not fingerprint plugin {}: {error}", plugin.path)
                })?,
            );
        }
    }
    Ok(preset)
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

fn collect_live_entries(
    directory: &Path,
    categories: &mut Vec<String>,
    entries: &mut Vec<LivePresetEntry>,
) {
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
            collect_live_entries(&path, categories, entries);
            categories.pop();
        } else if path
            .extension()
            .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("toml"))
        {
            let Ok(contents) = fs::read_to_string(&path) else {
                log::warn!("could not read live preset {}", path.display());
                continue;
            };
            let Ok(mut preset) = toml::from_str::<LivePreset>(&contents) else {
                log::warn!("could not parse live preset {}", path.display());
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
            if preset.name.trim().is_empty() {
                preset.name = segments.join("/");
            }
            entries.push(LivePresetEntry { segments, preset });
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

fn live_presets_directory() -> PathBuf {
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

    base.join("rack").join("live-presets")
}

#[cfg(test)]
mod tests {
    use super::{LivePreset, Preset, PresetParameter, PresetPlugin, parse_preset_name};

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

    #[test]
    fn serializes_full_width_fingerprints_as_strings() {
        let preset = Preset {
            plugins: vec![PresetPlugin {
                path: "plugin.vst3".to_owned(),
                fingerprint: Some(u64::MAX),
                parameters: vec![PresetParameter { id: 1, value: 0.5 }],
                bypassed: false,
                midi_mappings: Vec::new(),
            }],
        };

        let contents = toml::to_string_pretty(&preset).unwrap();
        assert!(contents.contains("fingerprint = \"18446744073709551615\""));
        let restored = toml::from_str::<Preset>(&contents).unwrap();
        assert_eq!(restored.plugins[0].fingerprint, Some(u64::MAX));
    }

    #[test]
    fn serializes_live_preset_name_and_ordered_board_presets() {
        let live_preset = LivePreset {
            name: "Friday/Set".to_owned(),
            presets: vec!["Clean", "Drive/Lead"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        };

        let contents = toml::to_string_pretty(&live_preset).unwrap();
        let restored = toml::from_str::<LivePreset>(&contents).unwrap();
        assert_eq!(restored.name, "Friday/Set");
        assert_eq!(restored.presets, ["Clean", "Drive/Lead"]);
    }
}
