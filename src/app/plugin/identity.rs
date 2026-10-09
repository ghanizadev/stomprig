use serde::{Deserialize, Serialize};
use std::path::Path;

const CLAP_CONFIG_PREFIX: &str = "rack-clap:";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PluginFormat {
    Vst3,
    Clap,
}

impl PluginFormat {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Vst3 => "VST3",
            Self::Clap => "CLAP",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub(crate) struct PluginIdentity {
    pub(crate) format: PluginFormat,
    pub(crate) bundle_path: String,
    #[serde(default)]
    pub(crate) plugin_id: Option<String>,
}

impl PluginIdentity {
    pub(crate) fn vst3(bundle_path: impl Into<String>, plugin_id: Option<String>) -> Self {
        Self {
            format: PluginFormat::Vst3,
            bundle_path: bundle_path.into(),
            plugin_id,
        }
    }

    pub(crate) fn clap(bundle_path: impl Into<String>, plugin_id: impl Into<String>) -> Self {
        Self {
            format: PluginFormat::Clap,
            bundle_path: bundle_path.into(),
            plugin_id: Some(plugin_id.into()),
        }
    }

    /// VST3 path strings are the legacy rack representation and remain unchanged.
    /// CLAP needs its plugin ID as well as its bundle path, so it uses a prefixed JSON key.
    pub(crate) fn config_key(&self) -> String {
        match self.format {
            PluginFormat::Vst3 => self.bundle_path.clone(),
            PluginFormat::Clap => serde_json::to_string(self)
                .map(|identity| format!("{CLAP_CONFIG_PREFIX}{identity}"))
                .unwrap_or_default(),
        }
    }

    pub(crate) fn from_config_key(key: &str) -> Self {
        if let Some(serialized) = key.strip_prefix(CLAP_CONFIG_PREFIX)
            && let Ok(identity) = serde_json::from_str::<Self>(serialized)
            && identity.format == PluginFormat::Clap
            && identity.plugin_id.is_some()
        {
            return identity;
        }

        Self::vst3(key, None)
    }

    pub(crate) fn same_plugin(&self, other: &Self) -> bool {
        self.format == other.format
            && normalized_path(&self.bundle_path) == normalized_path(&other.bundle_path)
            && self.plugin_id == other.plugin_id
    }
}

fn normalized_path(path: &str) -> String {
    let normalized = Path::new(path).to_string_lossy().replace('\\', "/");
    #[cfg(windows)]
    {
        normalized.to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        normalized
    }
}

#[cfg(test)]
mod tests {
    use super::{PluginFormat, PluginIdentity};

    #[test]
    fn clap_config_key_preserves_bundle_and_plugin_id() {
        let identity = PluginIdentity::clap("C:/Audio Plugins/reverb.clap", "org.example.reverb");

        let restored = PluginIdentity::from_config_key(&identity.config_key());

        assert_eq!(restored, identity);
        assert_eq!(restored.format, PluginFormat::Clap);
    }

    #[test]
    fn vst3_config_key_remains_a_legacy_path() {
        let path = "C:/Audio Plugins/reverb.vst3";
        let identity = PluginIdentity::vst3(path, None);

        assert_eq!(identity.config_key(), path);
        assert_eq!(
            PluginIdentity::from_config_key(path).format,
            PluginFormat::Vst3
        );
    }
}
