# Cross-Platform Implementation Backlog

Target Windows, Linux, and macOS first. Platform-aware audio backends, target-specific Windows dependencies, unavailable-backend fallback, and VST3 search roots for these platforms are already implemented.

## P0 — Remaining platform support

### 1. Add VST2 plugin discovery

The discovery scanner currently searches platform-specific VST3 locations only. Add VST2 search roots and extensions for Windows, Linux, and macOS, or document VST2 as manually loadable. Keep probing isolated where possible because VST2 loading currently runs in-process.

### 2. Define VST2 native editor support

VST2 native editors are implemented only on Windows. Either add Linux and macOS editor hosting or clearly report that native editors are unavailable on those platforms while keeping audio processing available.

## P1 — Complete native editor and diagnostics support

### Native VST3 editors

The vendored VST3 host contains platform implementations, but verify them end-to-end:

- Linux VST3 editors depend on X11/XCB; document the XWayland requirement or add native Wayland support.
- macOS VST3 editors must be opened and serviced on the AppKit main thread.
- Surface missing platform requirements as actionable UI errors.

### Portable diagnostics

Update `src/app/cpu.rs` to measure process CPU time on Unix. It currently reports no CPU usage outside Windows. Keep minidumps Windows-only; the portable panic hook and file logging are already in place.

### Filesystem behavior

Update `src/app/plugin/cache.rs` to lowercase cache keys only on Windows; Linux and macOS filesystems can be case-sensitive.

Make preset-name validation platform-aware. It currently rejects Windows-specific filename characters on every OS.

Add platform-specific coverage for settings, preset, and cache directories. These paths already select platform-specific locations.

## P2 — Build and release verification

Add CI jobs for:

- Windows MSVC
- Linux x86_64
- macOS x86_64 or ARM64

Each job should run formatting, checking, tests, and a build. Add tests for backend discovery, default backend selection, plugin search paths and extensions, settings fallback, and case-sensitive cache keys.

## Supported-platform policy

Initially support Windows, Linux/X11, and macOS. For unsupported targets, add a clear compile-time error or platform fallback. Supporting BSD, Android, or direct Wayland should be separate follow-up work because the current CPAL and VST3-host implementations do not fully cover those targets.
