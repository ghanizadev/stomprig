#![cfg(target_os = "macos")]

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use vst3_host::{ProbeResult, SafeDiscoverySkip, Vst3Host};

const PLUGIN_FIXTURE_SOURCE: &str = r#"
#include <fcntl.h>
#include <stdlib.h>
#include <unistd.h>

typedef const struct __CFBundle *CFBundleRef;

__attribute__((visibility("default"))) unsigned char bundleEntry(CFBundleRef bundle) {
    (void)bundle;
#if defined(RACK_FIXTURE_CRASH)
    abort();
#elif defined(RACK_FIXTURE_HANG)
    for (;;) pause();
#elif defined(RACK_FIXTURE_CRASH_ONCE)
    const char *marker = getenv("RACK_VST3_CRASH_ONCE_MARKER");
    if (marker != NULL) {
        int marker_file = open(marker, O_WRONLY | O_CREAT | O_EXCL, 0600);
        if (marker_file >= 0) {
            close(marker_file);
            abort();
        }
    }
#endif
    return 1;
}

__attribute__((visibility("default"))) unsigned char bundleExit(void) {
    return 1;
}

__attribute__((visibility("default"))) void *GetPluginFactory(void) {
    return NULL;
}
"#;

struct FixtureDirectory(PathBuf);

impl FixtureDirectory {
    fn new() -> Self {
        let unique_id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rack-vst3-isolation-test-{}-{unique_id}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create VST3 isolation test directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn malformed_bundle(&self) -> PathBuf {
        let path = self.0.join("Malformed.vst3");
        fs::create_dir(&path).expect("create malformed bundle");
        path
    }

    fn plugin_bundle(&self, name: &str, behavior: Option<&str>) -> PathBuf {
        let bundle = self.0.join(format!("{name}.vst3"));
        let contents = bundle.join("Contents");
        let macos = contents.join("MacOS");
        fs::create_dir_all(&macos).expect("create plugin bundle directories");
        fs::write(
            contents.join("Info.plist"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                 <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
                 <plist version=\"1.0\"><dict>\n\
                 <key>CFBundleIdentifier</key><string>org.example.rack.{name}</string>\n\
                 <key>CFBundleExecutable</key><string>RackFixture</string>\n\
                 <key>CFBundlePackageType</key><string>BNDL</string>\n\
                 <key>CFBundleVersion</key><string>1.0</string>\n\
                 </dict></plist>\n"
            ),
        )
        .expect("write plugin Info.plist");

        let source = self.0.join(format!("{name}.c"));
        fs::write(&source, PLUGIN_FIXTURE_SOURCE).expect("write plugin fixture source");
        let binary = macos.join("RackFixture");
        let mut compiler = Command::new("cc");
        compiler
            .arg("-dynamiclib")
            .arg("-arch")
            .arg(macos_target_architecture())
            .arg("-o")
            .arg(&binary);
        if let Some(behavior) = behavior {
            compiler.arg(format!("-D{behavior}"));
        }
        let output = compiler
            .arg(&source)
            .output()
            .expect("compile plugin fixture with the Apple C compiler");
        assert!(
            output.status.success(),
            "plugin fixture compilation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        bundle
    }
}

impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct EnvironmentRestore {
    name: &'static str,
    previous: Option<OsString>,
}

impl EnvironmentRestore {
    fn set(name: &'static str, value: impl Into<OsString>) -> Self {
        let previous = std::env::var_os(name);
        // This integration target contains one test; no concurrent test mutates or reads this
        // override while the child probe processes inherit it.
        unsafe { std::env::set_var(name, value.into()) };
        Self { name, previous }
    }
}

impl Drop for EnvironmentRestore {
    fn drop(&mut self) {
        // The test has finished all helper calls before restoring the inherited environment.
        unsafe {
            if let Some(previous) = self.previous.take() {
                std::env::set_var(self.name, previous);
            } else {
                std::env::remove_var(self.name);
            }
        }
    }
}

#[cfg(target_arch = "aarch64")]
const fn macos_target_architecture() -> &'static str {
    "arm64"
}

#[cfg(target_arch = "x86_64")]
const fn macos_target_architecture() -> &'static str {
    "x86_64"
}

#[test]
fn probes_and_isolated_loads_contain_malformed_crashing_and_hanging_plugins() {
    let fixtures = FixtureDirectory::new();
    let malformed = fixtures.malformed_bundle();
    let crashing = fixtures.plugin_bundle("Crashing", Some("RACK_FIXTURE_CRASH"));
    let hanging = fixtures.plugin_bundle("Hanging", Some("RACK_FIXTURE_HANG"));
    let crash_once = fixtures.plugin_bundle("CrashOnce", Some("RACK_FIXTURE_CRASH_ONCE"));

    let _probe_path = EnvironmentRestore::set(
        "VST3_HOST_PROBE_PATH",
        env!("CARGO_BIN_EXE_vst3-host-probe"),
    );
    let discovery_host = Vst3Host::builder()
        .add_scan_path(fixtures.path())
        .probe_timeout(Duration::from_secs(2))
        .build()
        .expect("configure isolated discovery");
    let discovery = discovery_host.discover_plugins_safe();
    assert!(
        discovery.scan_ran(),
        "discovery failed: {:?}",
        discovery.error
    );

    assert!(matches!(
        skip_for(&discovery.skipped, &malformed),
        SafeDiscoverySkip::Failed { detail, .. } if !detail.contains("failed to spawn probe")
    ));
    assert!(matches!(
        skip_for(&discovery.skipped, &crashing),
        SafeDiscoverySkip::Crashed { .. }
    ));
    assert!(matches!(
        skip_for(&discovery.skipped, &hanging),
        SafeDiscoverySkip::TimedOut { .. }
    ));

    let runtime_host = Vst3Host::builder()
        .with_process_isolation(true)
        .helper_path(env!("CARGO_BIN_EXE_vst3-host-helper"))
        .response_timeout(Duration::from_secs(5))
        .slow_command_timeout(Duration::from_secs(2))
        .build()
        .expect("configure isolated VST3 helper");

    assert!(matches!(
        runtime_host.probe_plugin(&malformed),
        ProbeResult::Failed(message) if !message.contains("helper unavailable")
    ));
    assert_eq!(runtime_host.probe_plugin(&crashing), ProbeResult::Crashed);
    assert_eq!(runtime_host.probe_plugin(&hanging), ProbeResult::TimedOut);

    let crash_once_marker = fixtures.path().join("crash-once.marker");
    let _crash_once_marker =
        EnvironmentRestore::set("RACK_VST3_CRASH_ONCE_MARKER", crash_once_marker.as_os_str());
    assert!(matches!(
        runtime_host.probe_plugin(&crash_once),
        ProbeResult::Failed(message) if !message.contains("helper unavailable")
    ));
    assert!(
        crash_once_marker.exists(),
        "the load should have retried in a fresh helper after the first crash"
    );
}

fn skip_for<'a>(skips: &'a [SafeDiscoverySkip], path: &Path) -> &'a SafeDiscoverySkip {
    skips
        .iter()
        .find(|skip| skip.path() == path)
        .unwrap_or_else(|| panic!("no discovery result was recorded for {}", path.display()))
}
