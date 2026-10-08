# Production Readiness Assessment

## Assessment

The project builds successfully, but it is not production-ready yet.

Current checks:

- `cargo fmt --check` passes.
- `cargo check` passes.
- `cargo test` passes with 34 tests.
- `cargo build --release` passes.
- `cargo clippy --all-targets -- -D warnings` fails with 19 diagnostics.

The current tests do not cover real audio devices, native plugin editors, plugin crashes, device failures, or Windows packaging.

## P0 — Must fix before release

### 1. Make the audio path real-time safe

The CPAL callbacks allocate memory for `i32` devices:

- `src/app/audio/engine.rs:224-242`
- `src/app/audio/engine.rs:280-301`

Preallocate conversion buffers or use allocation-free conversion. Remove blocking synchronization from deadline-sensitive paths.

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
- bounded queues;
- tests for all supported channel counts and buffer sizes;
- device-disconnect and stream-error recovery.

### 2. Remove plugin crash and hang risks

Plugin discovery currently loads native code in-process and scans every `.dll` beneath VST3 directories:

- `src/app/plugin_editor.rs:141-155`
- `src/app/plugin/loader.rs:32-44`

This can crash or hang the application. The vendored `vst3-host` already provides isolated probing and process isolation. Use isolated discovery by default, restrict scanning to known plugin extensions/directories, and add timeouts.

Plugin stack loading also occurs synchronously on the UI thread:

- `src/app/plugin_editor.rs:874-910`

Provide progress, cancellation, timeout handling, and recovery for plugins that do not return. Package the required helper/probe executables with the application.

### 3. Eliminate the unsafe VST2 ownership model

The project declares `unsafe impl Send for Vst2Plugin` in `src/app/plugin/types.rs:38-40`.

This requires a documented and tested safety invariant. Prefer making the runtime thread the sole owner of the VST2 instance and communicating through commands and snapshots instead of sharing it through `Arc<Mutex<_>>`.

The shared plugin mutex can also block processing:

- `src/app/audio/processing.rs:101-116`

### 4. Make shutdown bounded and reliable

Runtime shutdown waits indefinitely for the worker:

- `src/app/plugin_runtime.rs:122-139`

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
