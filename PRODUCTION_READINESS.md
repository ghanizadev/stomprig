# Production Readiness Assessment

## Assessment

The current codebase has a substantial feature surface, but it should not yet be marked feature-complete or production-ready. The repository does not contain a user-facing product specification or acceptance matrix, so the intended release scope needs to be made explicit before completion can be measured.

The current implementation includes a VST3 plugin rack with parameter controls and bypass, preset save/import/export, live presets and a stage view, tuner, backing-track playback, audio routing/settings, and MIDI learn for per-plugin bypass and per-parameter controls. Bypass triggers support Control Change and Note On; parameter mappings support Control Change, and mappings are stored with presets.

The checks below are the last results recorded in this file. They have not been rerun against the current checkout:

- `cargo fmt --check` passes.
- `cargo check` passes.
- `cargo test` passes with 34 tests.
- `cargo build --release` passes.
- `cargo clippy --all-targets -- -D warnings` fails with 19 diagnostics.

The current tests do not cover real audio devices, native plugin editors, plugin crashes, device failures, or Windows packaging.

## Feature-completeness gaps

Before declaring the application feature-complete, define a short release scope covering supported platforms, VST3 support, and the main playing/recording workflows. This is necessary because some current limitations are deliberate in the MIDI plan while others are unimplemented behavior; without an acceptance matrix, there is no objective completion bar.

The initial supported-platform scope is Windows, Linux/X11, and macOS. Document the XWayland requirement where Linux VST3 editors depend on X11/XCB, and set supported minimum OS versions and architectures. For other targets, provide a clear compile-time error or explicit fallback; BSD, Android, and native Wayland support require separate scope.

### 1. Complete or explicitly scope MIDI controller behavior

MIDI input currently learns a Control Change or Note On event and uses it to toggle one plugin's rack bypass. Although the preset model stores a list of mappings, the plugin UI exposes only one bypass mapping at a time; parameters have per-parameter CC learn controls, so multiple parameter mappings can coexist. Remaining MIDI work includes momentary bypass, ~~parameter control~~, backing-track transport, multiple bypass triggers in the UI, MIDI forwarding into plugins, and MIDI output/feedback. Program Change selection within the active live preset is already supported. The current code supports rack bypass triggers and per-parameter CC mappings, but it is not a complete general-purpose MIDI controller feature.

Choose which controller actions belong in the release scope. For a broader MIDI workflow, add management for multiple bypass triggers, define momentary/toggle semantics, implement selected transport actions, and forward supported events through the plugin runtime to plugins that accept MIDI. Persist and restore every mapping/action with presets. MIDI output and feedback LEDs can remain out of scope if the product requirements say so. Each action needs defined trigger and release behavior. Keep mappings attached to the correct plugin through reorder, removal, save, and load. Use bounded, non-blocking event delivery, and report unsupported device or plugin capabilities without interrupting audio.

### 2. Define plugin discovery coverage

The automatic discovery path scans configured VST3 search paths and probes candidates in isolation. Verify platform-specific default and configured search roots, bundle/file extensions, and candidate filtering on Windows, Linux, and macOS.

### 3. Match native editor support to the platform policy

Linux VST3 editor hosting depends on X11/XCB, and macOS editor servicing has main-thread requirements that need end-to-end verification.

For each supported OS, verify the VST3 native editor path. Document Linux/XWayland requirements, verify macOS editor creation and servicing on the AppKit main thread, and surface missing platform dependencies as actionable UI errors.

### 4. Close device recovery workflows

Device selection, channel routing, and buffer-size choices exist, but there is no visible audio-health state and device loss/stream errors do not have a complete recovery workflow. Current enumeration ~~can fail the whole device list when one device lacks a default configuration~~, device selection ~~is index-based~~, and unplug/reconnect recovery is not implemented.

~~Skip invalid devices individually and identify devices by stable identity.~~ Detect disconnects and ~~stream errors~~, show a recoverable audio-health state, refresh available devices, and let the user restart the engine without losing the plugin stack or preset state.

## P0 — Must fix before release

### ~~1. Make the audio path real-time safe~~

~~The CPAL callbacks allocate memory for `i32` devices:~~

~~- `src/app/audio/engine.rs:249-277`~~
~~- `src/app/audio/engine.rs:316-349`~~

~~The conversion buffers start empty and grow inside the callbacks, so their first use or a larger-than-before device block can allocate on a deadline-sensitive thread. Preallocate to the maximum supported callback size before streams start, or use allocation-free conversion. Remove blocking synchronization from deadline-sensitive processing paths.~~

~~Add:~~

- ~~underrun/overrun counters;~~
- ~~visible audio health state;~~
- ~~tests for all supported channel counts and buffer sizes;~~
- device-disconnect and stream-error recovery.

### 2. ~~Contain plugin crash and hang risks~~

~~The earlier findings that VST3 discovery runs in-process and that plugin stacks load synchronously on the UI thread are stale for the current source. VST3 discovery now uses an isolated probe with a timeout, VST3 loading enables process isolation and command timeouts, and stack loading runs on a worker with progress and cancellation:~~

- ~~`src/app/plugin/loader.rs:34-61`~~
- ~~`src/app/plugin_editor.rs:330-427`~~

~~Important gaps remain. The packaged release must be verified to include the helper/probe executables required by isolated VST3 operation. Cancellation of the worker does not make an in-process plugin call interruptible.~~

~~Verify timeout and recovery behavior against malformed, crashing, and hanging VST3 plugins. Add packaging checks that launch the installed application and confirm its helper binaries are found.~~

~~The shared plugin mutex can also block processing:~~

- ~~`src/app/audio/processing.rs:116-123`~~

### ~~3. Make shutdown bounded and reliable~~

~~Runtime shutdown waits indefinitely for the worker:~~

~~- `src/app/plugin_runtime.rs:151-168`~~

~~Add timeouts, explicit failure states, and a strategy for plugins that hang during processing or destruction. Test repeated start/stop, failed startup, device restart, plugin removal, editor closure, and application exit.~~

## P1 — Required production hardening

### ~~Persistence and configuration~~

~~Settings, presets, and plugin cache use direct writes:~~

- ~~`src/app/settings.rs:723-745`~~
- ~~`src/app/presets.rs:95-132`~~
- ~~`src/app/plugin/cache.rs:116-135`~~

~~Use atomic temp-file writes followed by rename, backups, schema versions, migrations, and recovery from truncated files.~~

~~Add validation for missing devices, invalid channels, unsupported buffer sizes, and stale preset parameter IDs.~~

~~The plugin fingerprint is metadata-based rather than content-based:~~

- ~~`src/app/plugin/cache.rs:139-145`~~

~~Use a stronger identity strategy, such as content hashing on demand plus plugin UID/version metadata.~~

### ~~Logging and diagnostics~~

~~Logs are written beside the executable:~~

- ~~`src/app/diagnostics.rs:83-109`~~

~~That location may be read-only under `Program Files`. Store logs under `%LOCALAPPDATA%\\rack\\logs`, rotate them, and report initialization failures instead of silently ignoring them.~~

~~Expose audio errors and plugin crashes in the UI instead of only logging them.~~

### Device handling

~~Device enumeration fails the entire list if one device cannot provide a default configuration:~~

- ~~`src/app/audio/devices.rs:21-57`~~

~~Skip invalid devices individually and identify devices by stable identity rather than enumeration index;~~ support hot-plug/reconnection.

### Cross-platform behavior

Make preset-name validation platform-aware. Continue rejecting path traversal, separators, and control characters on every platform, but apply Windows-only filename restrictions only on Windows; the current validator rejects those characters on Linux and macOS too.

Settings, preset, and cache locations already vary by platform. Verify the resolved directories and settings fallback behavior on every supported OS.

## P2 — Release engineering and verification

Add:

- CI on Windows, Linux x86_64, and macOS x86_64 or ARM64, running formatting, checks, tests, and builds; include clippy, release builds, and dependency auditing;
- tests for audio backend discovery/default selection, plugin search roots/extensions, settings fallback, platform-specific settings/preset/cache directories, and case-sensitive cache keys;
- root `README`, `LICENSE`, changelog, and support information;
- Cargo package metadata;
- a Windows installer and signed binaries;
- reproducible release instructions;
- explicit tracking and provenance for the vendored `vst3-host` fork.

Create a supported-platform compatibility matrix covering:

- Windows WASAPI/ASIO, Linux ALSA/JACK/PipeWire, and macOS CoreAudio;
- `f32` and `i32` devices;
- mono, stereo, and multichannel devices;
- all configured buffer sizes;
- device unplug/reconnect;
- VST3 editors;
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
