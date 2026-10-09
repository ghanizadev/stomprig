# Production Readiness Assessment

## Assessment

The current codebase has a substantial feature surface, but it should not yet be marked feature-complete or production-ready. The repository does not contain a user-facing product specification or acceptance matrix, so the intended release scope needs to be made explicit before completion can be measured.

The current implementation includes a VST plugin rack with parameter controls and bypass, preset save/import/export, live presets and a stage view, tuner, backing-track playback, audio routing/settings, and MIDI learn for per-plugin bypass. MIDI learn supports Control Change and Note On triggers and its mappings are stored with presets.

The checks below are the last results recorded in this file. They have not been rerun against the current checkout, which contains existing uncommitted changes:

- `cargo fmt --check` passes.
- `cargo check` passes.
- `cargo test` passes with 34 tests.
- `cargo build --release` passes.
- `cargo clippy --all-targets -- -D warnings` fails with 19 diagnostics.

The current tests do not cover real audio devices, native plugin editors, plugin crashes, device failures, or Windows packaging.

## Feature-completeness gaps

Before declaring the application feature-complete, define a short release scope covering supported platforms, plugin formats, and the main playing/recording workflows. This is necessary because some current limitations are deliberate in the MIDI plan while others are unimplemented behavior; without an acceptance matrix, there is no objective completion bar.

### 1. Complete or explicitly scope MIDI controller behavior

MIDI input currently learns a Control Change or Note On event and uses it to toggle one plugin's rack bypass. Although the preset model stores a list of mappings, the plugin UI exposes only one mapping at a time. The MIDI plan lists momentary bypass, parameter control, playback actions, multiple mappings in the UI, MIDI forwarding into plugins, and MIDI output/feedback as follow-up capabilities. As a result, the existing code is complete for the narrowly defined “learn a trigger to toggle rack bypass” workflow, but it is not a complete general-purpose MIDI controller feature.

Choose which controller actions belong in the release scope. For a broader MIDI workflow, add mapping management for multiple triggers, define momentary/toggle semantics, implement the selected parameter and playback actions, and forward supported events through the plugin runtime to plugins that accept MIDI. Persist and restore every mapping/action with presets. MIDI output and feedback LEDs can remain out of scope if the product requirements say so.

### 2. Define plugin discovery coverage

The automatic discovery path scans known VST3 bundle locations and probes VST3 plugins in isolation. It does not include VST2 plugins in the discovery scan, even though the host can load VST2 plugins by path. Users who expect the picker to find both formats therefore do not have a complete discovery workflow.

Decide whether VST2 auto-discovery is supported. If it is, add platform-specific VST2 search roots and extensions and isolate probing where possible. If it is not, describe VST2 as manually loadable and label that limitation in the plugin picker and supported-format documentation.

### 3. Match native editor support to the platform policy

VST2 native editor windows are implemented only on Windows; opening one on another platform returns an unsupported-platform error. The cross-platform target list includes Windows, Linux, and macOS, so native editor behavior is inconsistent if all three are intended to have equivalent VST2 support. Linux VST3 editor hosting also depends on X11/XCB, and macOS editor servicing has main-thread requirements that need end-to-end verification.

For each supported OS and plugin format, either implement and verify the native editor path or explicitly report that the editor is unavailable while keeping audio processing available. Document Linux/XWayland requirements and surface missing platform dependencies as actionable UI errors.

### 4. Close device recovery workflows

Device selection, channel routing, and buffer-size choices exist, but there is no visible audio-health state and device loss/stream errors do not have a complete recovery workflow. Current enumeration can fail the whole device list when one device lacks a default configuration, device selection is index-based, and unplug/reconnect recovery is not implemented.

Skip invalid devices individually, identify devices by stable identity, and detect disconnects and stream errors. Show a recoverable audio-health state, refresh available devices, and let the user restart the engine without losing the plugin stack or preset state.

## P0 — Must fix before release

### 1. Make the audio path real-time safe

The CPAL callbacks allocate memory for `i32` devices:

- `src/app/audio/engine.rs:249-277`
- `src/app/audio/engine.rs:316-349`

The conversion buffers start empty and grow inside the callbacks, so their first use or a larger-than-before device block can allocate on a deadline-sensitive thread. Preallocate to the maximum supported callback size before streams start, or use allocation-free conversion. Remove blocking synchronization from deadline-sensitive processing paths.

The audio queues are bounded to two processing blocks and sized from the selected block size and
device channel count:

- `src/app/audio/engine.rs:15`
- `src/app/audio/engine.rs:185-186`

This prevents the transport from accumulating an unreported latency backlog while retaining enough
capacity for normal callback scheduling jitter. Input overflow and output underrun telemetry should
still be added for production diagnostics.

Add:

- underrun/overrun counters;
- visible audio health state;
- tests for all supported channel counts and buffer sizes;
- device-disconnect and stream-error recovery.

### 2. Contain plugin crash and hang risks

The earlier findings that VST3 discovery runs in-process and that plugin stacks load synchronously on the UI thread are stale for the current source. VST3 discovery now uses an isolated probe with a timeout, VST3 loading enables process isolation and command timeouts, and stack loading runs on a worker with progress and cancellation:

- `src/app/plugin/loader.rs:34-61`
- `src/app/plugin_editor.rs:330-427`

Important gaps remain. VST2 loading is still in-process, VST2 is not included in the isolated discovery scan, and a VST2 plugin that crashes or hangs can still affect the main process. The packaged release must also be verified to include the helper/probe executables required by isolated VST3 operation. Cancellation of the worker does not make an in-process plugin call interruptible.

Keep VST3 isolation enabled and verify timeout/recovery behavior against malformed, crashing, and hanging plugins. For VST2, either add process isolation or explicitly limit support and warn users about the failure boundary. Add packaging checks that launch the installed application and confirm its helper binaries are found.

### 3. Eliminate the unsafe VST2 ownership model

The project declares `unsafe impl Send for Vst2Plugin` in `src/app/plugin/types.rs:38-40`.

This requires a documented and tested safety invariant. Prefer making the runtime thread the sole owner of the VST2 instance and communicating through commands and snapshots instead of sharing it through `Arc<Mutex<_>>`.

The shared plugin mutex can also block processing:

- `src/app/audio/processing.rs:116-123`

### 4. Make shutdown bounded and reliable

Runtime shutdown waits indefinitely for the worker:

- `src/app/plugin_runtime.rs:151-168`

Add timeouts, explicit failure states, and a strategy for plugins that hang during processing or destruction. Test repeated start/stop, failed startup, device restart, plugin removal, editor closure, and application exit.

## P1 — Required production hardening

### Persistence and configuration

Settings, presets, and plugin cache use direct writes:

- `src/app/settings.rs:723-745`
- `src/app/presets.rs:95-132`
- `src/app/plugin/cache.rs:116-135`

Use atomic temp-file writes followed by rename, backups, schema versions, migrations, and recovery from truncated files.

Add validation for missing devices, invalid channels, unsupported buffer sizes, and stale preset parameter IDs.

The plugin fingerprint is metadata-based rather than content-based:

- `src/app/plugin/cache.rs:139-145`

Use a stronger identity strategy, such as content hashing on demand plus plugin UID/version metadata.

### Logging and diagnostics

Logs are written beside the executable:

- `src/app/diagnostics.rs:83-109`

That location may be read-only under `Program Files`. Store logs under `%LOCALAPPDATA%\\rack\\logs`, rotate them, and report initialization failures instead of silently ignoring them.

Expose audio errors and plugin crashes in the UI instead of only logging them.

### Device handling

Device enumeration fails the entire list if one device cannot provide a default configuration:

- `src/app/audio/devices.rs:21-57`

Skip invalid devices individually, identify devices by stable identity rather than enumeration index, and support hot-plug/reconnection.

## P2 — Release engineering and verification

Add:

- CI with formatting, clippy, tests, release builds, and dependency auditing;
- root `README`, `LICENSE`, changelog, and support information;
- Cargo package metadata;
- a Windows installer and signed binaries;
- reproducible release instructions;
- explicit tracking and provenance for the vendored `vst3-host` fork.

Create a Windows compatibility matrix covering:

- WASAPI and ASIO;
- `f32` and `i32` devices;
- mono, stereo, and multichannel devices;
- all configured buffer sizes;
- device unplug/reconnect;
- VST2/VST3 editors;
- malformed, crashing, and hanging plugins.

Add long-running audio soak tests and automated tests for underruns, shutdown, settings corruption, preset migration, and plugin recovery.

## Recommended release gate

All of the following should pass:

```text
cargo fmt --all -- --check
cargo check --all-targets
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

Additionally:

- a bad plugin must not terminate the main process;
- audio device failure must be visible and recoverable;
- persistence must survive interruption;
- a 24-hour audio soak must complete without unbounded underruns or runtime hangs.
