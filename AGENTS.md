# AI Agent Guide

## Project Overview

`rack` is a Rust 2024 desktop VST3 audio rack for Windows, built as an `eframe`/`egui` GUI with `cpal` audio I/O. The crate exposes `src/lib.rs` and launches the `rack` binary from `src/bin/main.rs`.

## Repository Layout

- `src/bin/main.rs` — application entry point; initializes diagnostics, configures the native viewport, installs the Lucide icon font, and starts `RackApp`.
- `src/app/mod.rs` — `RackApp` lifecycle and top-level UI; owns `PluginEditor` and `SettingsStore`.
- `src/app/audio.rs` and `src/app/audio/` — audio backends, device enumeration, sample conversion, stream setup, metering, lock-free processing, and audio types. `AudioEngine` owns the CPAL streams and plugin processing callbacks.
- `src/app/plugin/` — VST3 loading and native editor servicing. The ordered stack is the `PLUGIN_PATHS` constant in `src/app/plugin/loader.rs`.
- `src/app/plugin_editor.rs` — connects settings, plugin parameters/editors, audio engine startup/restart, and the rack UI.
- `src/app/settings.rs` — egui settings UI plus TOML persistence. On Windows settings are stored at `%APPDATA%\rack\settings.toml`.
- `src/app/windows/` — audio configuration, audio monitor, and theme UI components.
- `src/shadcn/` — local egui wrappers/components; `shadcn-rs.toml` generates into this directory.
- `src/app/diagnostics.rs` — logging, panic handling, Windows native crash handling/minidumps, and lock-free audio breadcrumbs.

## Architecture Notes

- `RackApp::default` loads persisted settings and passes audio settings into `PluginEditor`; changes are saved from the UI update loop.
- Loading the stack first calls `load_stack`, installs `Arc<Mutex<Plugin>>` instances, then starts `AudioEngine` when a complete device/channel configuration is available.
- Audio callbacks use `rtrb` ring buffers and shared atomics for metering/error reporting. Keep callback work compatible with real-time audio constraints; coordinate lifecycle changes through `AudioEngine::shutdown`/restart rather than dropping plugins from a callback.
- Native VST3 editor windows must be serviced each UI frame and detached before eframe destroys the root window. Application exit intentionally uses `ShutdownMode::KeepPluginsLoaded` to avoid third-party native destructors during teardown.
- `cpal` is configured with the `asio` feature. ASIO input and output must use the same driver, and the selected devices must expose matching sample rates and supported `f32`/`i32` formats.

## Development Workflow

Run commands from the repository root:

```powershell
cargo fmt --all -- --check
cargo check
cargo test
cargo build
cargo build --release
cargo run
```

`cargo test` currently runs unit tests across the audio conversion, engine, metering, processing, types, and local shadcn button modules; there is no separate integration-test directory. `cargo build`/`cargo run` may require the Windows native toolchain and ASIO-related dependencies because of the `cpal` feature configuration.

For runtime diagnostics, set `RACK_LOG` to a `log::LevelFilter` value such as `Debug`, `Info`, or `Trace`. Logs and crash artifacts are written to a `logs` directory beside the executable (`rack.log`, `crash.log`, and on Windows `crash.dmp`).

## Code Conventions

- Keep module boundaries aligned with the existing `src/app/audio`, `src/app/plugin`, and `src/app/windows` submodules; re-export only the public types needed by their parent module.
- Use the existing egui/shadcn components and theme flow in `src/app/settings.rs` and `src/shadcn/` rather than introducing a second widget style.
- When changing audio configuration, preserve channel normalization and device-name restoration in `Settings`; when changing plugin loading, update the ordered `PLUGIN_PATHS` list and its Windows VST3 paths explicitly.
- Preserve diagnostic logging around plugin/audio lifecycle transitions and update audio breadcrumb stages when adding processing stages.
- Format Rust changes with rustfmt and keep dependency versions/features synchronized between `Cargo.toml` and `Cargo.lock`.


