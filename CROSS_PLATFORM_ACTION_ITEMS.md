# Cross-Platform Implementation Backlog

Target Windows, Linux, and macOS first.

## P0 — Make the project compile outside Windows

### 1. Replace the Windows-only audio backend model

Update:

- `src/app/audio/types.rs`
- `src/app/audio/devices.rs`
- `src/app/audio/engine.rs`
- `src/app/settings.rs`

Implement platform-aware backends:

- Windows: WASAPI, ASIO
- Linux: ALSA, JACK/PipeWire
- macOS: CoreAudio

Prefer deriving the list from `cpal::available_hosts()` instead of hard-coding `HostId::Wasapi` and `HostId::Asio`.

Acceptance criteria:

- `cargo check` succeeds on Linux and macOS.
- The default backend is selected dynamically per OS.
- ASIO-specific validation only runs for ASIO.

### 2. Make Windows dependencies target-specific

Move `windows-sys` in `Cargo.toml` into a Windows-only dependency section:

```toml
[target.'cfg(windows)'.dependencies.windows-sys]
```

### 3. Add settings migration for backend changes

Windows settings may contain `Wasapi` or `Asio`, which may not exist on another OS. If the saved backend is unavailable, automatically select the platform default and preserve device names only when they still exist.

## P0 — Fix plugin discovery

Update `src/app/plugin_editor.rs`.

Add platform-specific VST paths:

- Windows: existing Program Files/Common Files paths
- macOS:
  - `/Library/Audio/Plug-Ins/VST3`
  - `~/Library/Audio/Plug-Ins/VST3`
  - `/Library/Audio/Plug-Ins/VST`
  - `~/Library/Audio/Plug-Ins/VST`
- Linux:
  - `/usr/lib/vst3`
  - `/usr/local/lib/vst3`
  - `~/.vst3`
  - `/usr/lib/vst`
  - `/usr/local/lib/vst`
  - `~/.vst`

Recognize platform extensions:

- Windows: `.dll`
- Linux: `.so`
- macOS: `.dylib`, `.vst`
- All platforms: `.vst3`

Replace Windows-style path strings such as `Common Files\\VST3` with platform-independent `PathBuf::join` components.

## P1 — Implement or disable non-Windows VST2 editors

`src/app/plugin/windows.rs` currently rejects all non-Windows VST2 editors.

Choose one:

- Implement Linux X11/XCB VST2 editor hosting.
- Implement macOS AppKit/NSView VST2 editor hosting.
- Add a feature flag and clearly disable VST2 native editors on unsupported platforms.

VST2 audio processing can remain available even if its native editor is unavailable.

## P1 — Complete native editor platform support

The vendored VST3 host already contains Windows, Linux, and macOS implementations. Verify them end-to-end:

- Linux VST3 editors currently depend on X11/XCB; document that Wayland users need XWayland, or add native Wayland support.
- macOS VST3 editors must be opened and serviced on the AppKit main thread.
- Add runtime error messages for missing X11/AppKit requirements.

## P1 — Replace Windows-only diagnostics

Update:

- `src/app/cpu.rs`
- `src/app/diagnostics.rs`

Tasks:

- Implement Unix CPU-time measurement using a portable crate or `libc::getrusage`.
- Keep Windows minidumps under `cfg(windows)`.
- Add Unix crash handling only if required; otherwise retain portable panic and file logging.
- Avoid silently returning `None` for CPU usage on non-Windows.

## P1 — Fix filesystem portability issues

1. In `src/app/plugin/cache.rs`, only lowercase cache keys on Windows. Linux and macOS filesystems can be case-sensitive.

2. Make preset-name validation platform-aware. The current validation rejects Windows-specific characters on every OS.

3. Verify settings, presets, and cache directories using platform-specific tests. The existing settings and preset paths are already mostly prepared for Windows, macOS, and Unix.

## P2 — Add platform CI

Add CI jobs for:

- Windows MSVC
- Linux x86_64
- macOS x86_64 or ARM64

Each job should run:

```text
cargo fmt --all -- --check
cargo check
cargo test
cargo build
```

Add tests for:

- Backend discovery
- Default backend selection
- Plugin search paths
- Plugin extensions
- Settings migration
- Case-sensitive cache keys

## P2 — Define the supported-platform policy

Initially support:

- Windows
- Linux/X11
- macOS

For unsupported targets, add a clear compile-time error or platform fallback. Supporting BSD, Android, or direct Wayland should be separate follow-up work because the current CPAL and VST3-host implementations do not fully cover those targets.
