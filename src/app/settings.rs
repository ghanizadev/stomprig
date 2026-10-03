use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;

use super::audio::{AudioBackend, AudioBufferSize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioSettings {
    pub backend: AudioBackend,
    #[serde(default)]
    pub buffer_size: AudioBufferSize,
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub input_left_channel: Option<usize>,
    pub input_right_channel: Option<usize>,
    pub output_left_channel: Option<usize>,
    pub output_right_channel: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppSettings {
    pub gain: f32,
    pub tone: f32,
    pub level: f32,
    pub audio: AudioSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            gain: 0.5,
            tone: 0.5,
            level: 0.5,
            audio: AudioSettings::default(),
        }
    }
}

pub struct SettingsStore {
    pub values: AppSettings,
    path: PathBuf,
    last_saved: AppSettings,
    pub error: Option<String>,
}

impl SettingsStore {
    pub fn load() -> Self {
        let path = settings_path();
        match fs::read_to_string(&path) {
            Ok(contents) => match toml::from_str::<AppSettings>(&contents) {
                Ok(values) => Self {
                    last_saved: values.clone(),
                    values,
                    path,
                    error: None,
                },
                Err(error) => Self {
                    values: AppSettings::default(),
                    last_saved: AppSettings::default(),
                    path,
                    error: Some(format!("Could not read settings: {error}")),
                },
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self {
                values: AppSettings::default(),
                last_saved: AppSettings::default(),
                path,
                error: None,
            },
            Err(error) => Self {
                values: AppSettings::default(),
                last_saved: AppSettings::default(),
                path,
                error: Some(format!("Could not open settings: {error}")),
            },
        }
    }

    pub fn save_if_changed(&mut self, values: &AppSettings) {
        if *values == self.last_saved {
            return;
        }

        let result = (|| {
            let contents = toml::to_string_pretty(values)
                .map_err(|error| format!("Could not serialize settings: {error}"))?;
            if let Some(parent) = self.path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("Could not create settings directory: {error}"))?;
            }
            fs::write(&self.path, contents)
                .map_err(|error| format!("Could not save settings: {error}"))
        })();

        match result {
            Ok(()) => {
                self.last_saved = values.clone();
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
}

fn settings_path() -> PathBuf {
    #[cfg(windows)]
    let base = env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    #[cfg(target_os = "macos")]
    let base = env::var_os("HOME")
        .map(PathBuf::from)
        .map(|path| path.join("Library").join("Application Support"))
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    #[cfg(all(unix, not(target_os = "macos")))]
    let base = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|path| PathBuf::from(path).join(".config")))
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    base.join("rack").join("settings.toml")
}
