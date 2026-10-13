use clack_host::entry::PluginEntry;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
const PROBE_POLL_INTERVAL: Duration = Duration::from_millis(10);
const PROBE_ARGUMENT: &str = "--stomprig-clap-probe";
static NEXT_PROBE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ClapPluginMetadata {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) category: String,
}

pub(crate) fn probe_bundle_isolated(path: &Path) -> Result<Vec<ClapPluginMetadata>, String> {
    let result_path = std::env::temp_dir().join(format!(
        "stomprig-clap-probe-{}-{}.json",
        std::process::id(),
        NEXT_PROBE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&result_path, b"")
        .map_err(|error| format!("could not create temporary CLAP probe result file: {error}"))?;

    let result = run_probe_child(path, &result_path);
    let _ = fs::remove_file(&result_path);
    result
}

fn run_probe_child(path: &Path, result_path: &Path) -> Result<Vec<ClapPluginMetadata>, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("could not locate StompRig for CLAP probing: {error}"))?;
    let mut child = Command::new(executable)
        .arg(PROBE_ARGUMENT)
        .arg(path)
        .arg(result_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not start isolated CLAP probe: {error}"))?;

    let started_at = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("could not wait for isolated CLAP probe: {error}"))?
        {
            if !status.success() {
                return Err(format!("isolated CLAP probe exited with {status}"));
            }
            let contents = fs::read(result_path)
                .map_err(|error| format!("could not read isolated CLAP probe result: {error}"))?;
            let result =
                serde_json::from_slice::<Result<Vec<ClapPluginMetadata>, String>>(&contents)
                    .map_err(|error| {
                        format!("isolated CLAP probe returned invalid metadata: {error}")
                    })?;
            return result;
        }

        if started_at.elapsed() >= PROBE_TIMEOUT {
            terminate_child(&mut child);
            return Err(format!(
                "isolated CLAP probe exceeded the {} second timeout",
                PROBE_TIMEOUT.as_secs()
            ));
        }
        thread::sleep(PROBE_POLL_INTERVAL);
    }
}

fn terminate_child(child: &mut Child) {
    if let Err(error) = child.kill() {
        log::debug!("could not terminate timed-out CLAP probe: {error}");
    }
    if let Err(error) = child.wait() {
        log::debug!("could not reap timed-out CLAP probe: {error}");
    }
}

pub(crate) fn run_probe_helper(bundle_path: &OsStr, result_path: &OsStr) -> Result<(), String> {
    let result = probe_bundle(Path::new(bundle_path));
    let contents = serde_json::to_vec(&result)
        .map_err(|error| format!("could not serialize CLAP probe metadata: {error}"))?;
    fs::write(result_path, contents)
        .map_err(|error| format!("could not write CLAP probe metadata: {error}"))
}

fn probe_bundle(bundle_path: &Path) -> Result<Vec<ClapPluginMetadata>, String> {
    let entry = unsafe { PluginEntry::load(bundle_path) }
        .map_err(|error| format!("could not load CLAP bundle: {error}"))?;
    let factory = entry
        .get_plugin_factory()
        .ok_or_else(|| "CLAP bundle does not expose clap.plugin-factory".to_owned())?;

    let plugins = factory
        .plugin_descriptors()
        .filter_map(|descriptor| {
            let id = descriptor.id()?.to_string_lossy().into_owned();
            let name = descriptor.name()?.to_string_lossy().into_owned();
            let features = descriptor
                .features()
                .map(|feature| feature.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            if !is_audio_effect(&features) {
                return None;
            }
            Some(ClapPluginMetadata {
                id,
                name,
                category: effect_category(&features),
            })
        })
        .collect();

    Ok(plugins)
}

fn is_audio_effect(features: &[String]) -> bool {
    !features
        .iter()
        .any(|feature| matches!(feature.as_str(), "instrument" | "note-effect" | "analyzer"))
        && features.iter().any(|feature| is_effect_category(feature))
}

fn effect_category(features: &[String]) -> String {
    features
        .iter()
        .filter(|feature| feature.as_str() != "audio-effect")
        .find(|feature| is_effect_category(feature))
        .or_else(|| features.iter().find(|feature| is_effect_category(feature)))
        .map(|feature| {
            feature
                .split('-')
                .map(|word| {
                    let mut chars = word.chars();
                    chars.next().map_or_else(String::new, |first| {
                        first.to_uppercase().chain(chars).collect::<String>()
                    })
                })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_else(|| "Effect".to_owned())
}

fn is_effect_category(feature: &str) -> bool {
    matches!(
        feature,
        "audio-effect"
            | "distortion"
            | "filter"
            | "equalizer"
            | "frequency-shifter"
            | "pitch-shifter"
            | "granular"
            | "dynamics"
            | "compressor"
            | "limiter"
            | "gate"
            | "expander"
            | "reverb"
            | "delay"
            | "chorus"
            | "flanger"
            | "phaser"
            | "utility"
    )
}
