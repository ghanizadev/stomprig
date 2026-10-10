use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const PROBE_TIMEOUT: Duration = vst3_host::discovery::DEFAULT_PROBE_TIMEOUT;
const PROBE_POLL_INTERVAL: Duration = Duration::from_millis(10);
const PROBE_ARGUMENT: &str = "--rack-vst3-probe";
static NEXT_PROBE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Deserialize, Serialize)]
pub(crate) struct Vst3PluginMetadata {
    pub(crate) name: String,
    pub(crate) category: String,
    pub(crate) uid: String,
}

pub(crate) fn probe_bundle_isolated(path: &Path) -> Result<Vst3PluginMetadata, String> {
    let result_path = std::env::temp_dir().join(format!(
        "rack-vst3-probe-{}-{}.json",
        std::process::id(),
        NEXT_PROBE_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&result_path, b"")
        .map_err(|error| format!("could not create temporary VST3 probe result file: {error}"))?;

    let result = run_probe_child(path, &result_path);
    let _ = fs::remove_file(&result_path);
    result
}

fn run_probe_child(path: &Path, result_path: &Path) -> Result<Vst3PluginMetadata, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("could not locate Rack for VST3 probing: {error}"))?;
    let mut child = Command::new(executable)
        .arg(PROBE_ARGUMENT)
        .arg(path)
        .arg(result_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not start isolated VST3 probe: {error}"))?;

    let started_at = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("could not wait for isolated VST3 probe: {error}"))?
        {
            if !status.success() {
                return Err(format!("isolated VST3 probe exited with {status}"));
            }
            let contents = fs::read(result_path)
                .map_err(|error| format!("could not read isolated VST3 probe result: {error}"))?;
            let result = serde_json::from_slice::<Result<Vst3PluginMetadata, String>>(&contents)
                .map_err(|error| {
                    format!("isolated VST3 probe returned invalid metadata: {error}")
                })?;
            return result;
        }

        if started_at.elapsed() >= PROBE_TIMEOUT {
            terminate_child(&mut child);
            return Err(format!(
                "isolated VST3 probe exceeded the {} second timeout",
                PROBE_TIMEOUT.as_secs()
            ));
        }
        thread::sleep(PROBE_POLL_INTERVAL);
    }
}

fn terminate_child(child: &mut Child) {
    if let Err(error) = child.kill() {
        log::debug!("could not terminate timed-out VST3 probe: {error}");
    }
    if let Err(error) = child.wait() {
        log::debug!("could not reap timed-out VST3 probe: {error}");
    }
}

pub(crate) fn run_probe_helper(bundle_path: &OsStr, result_path: &OsStr) -> Result<(), String> {
    let result = probe_bundle(Path::new(bundle_path));
    let contents = serde_json::to_vec(&result)
        .map_err(|error| format!("could not serialize VST3 probe metadata: {error}"))?;
    fs::write(result_path, contents)
        .map_err(|error| format!("could not write VST3 probe metadata: {error}"))
}

fn probe_bundle(bundle_path: &Path) -> Result<Vst3PluginMetadata, String> {
    let details = vst3_host::get_detailed_plugin_info(bundle_path)
        .map_err(|error| format!("could not read VST3 plugin metadata: {error}"))?;
    Ok(Vst3PluginMetadata {
        name: details.info.name,
        category: details.info.category,
        uid: details.info.uid,
    })
}
