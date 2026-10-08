# AI Agent Instructions for VST Plugin Rack

## 1. Project Context
`rack` is a high-performance Rust 2024 VST3 audio rack designed for live playing and recording, built with `eframe`/`egui` for the GUI and `cpal` for audio I/O.
- **Core Focus:** Extreme reliability, minimal pipeline latency for real-time DSP effects, and strict lock-free audio processing.
- **Role:** You are an expert Systems and Audio Software Engineer, strictly adhering to Rust idioms and real-time audio programming best practices.

## 2. Core Architectural Boundaries
This project enforces a hard separation of concerns. You must place code in the appropriate directories and respect the boundaries between them:

* **`src/ui/` (UI Competency):** Handles `egui` rendering, layout, and user interactions. Never process audio or block the main thread here. Native VST3 editor windows must be serviced here during the UI frame before `eframe` destroys the root window.
* **`src/audio/` (DSP & Audio Competency):** Handles the real-time callback loop via `cpal` and VST3 plugin hosting. Code running in the audio callback must be strictly lock-free and allocation-free. You are explicitly forbidden from using `Mutex` or `RwLock` in the audio processing path.
* **`src/core/` (Shared State & Infrastructure):** Handles application configuration, diagnostic logging, and lock-free channels.
* **`src/bin/main.rs`:** The application entry point that initializes diagnostics, configures the native viewport, and starts the UI app.

## 3. Real-Time Audio Constraints
* **Communication:** Always use `rtrb` ring buffers and shared atomics for cross-thread communication (e.g., metering, error reporting, parameter changes) between the UI and the audio engine.
* **Lifecycle Management:** Coordinate plugin lifecycle changes through engine shutdowns and restarts rather than dropping plugins dynamically from within the audio callback itself.
* **Device Configuration:** The `cpal` audio backend is configured with the `asio` feature. Ensure ASIO input and output use the same driver, and validate that selected devices expose matching sample rates and supported `f32`/`i32` formats.

## 4. Strict "Do Not Do" Constraints
When generating, modifying, or refactoring code, you must strictly obey the following rules:

* **DO NOT mix competencies:** Never put UI rendering code inside DSP logic, and never put signal processing logic inside `egui` update loops.
* **DO NOT use blocking synchronization in audio:** Never use standard library `Mutex`, `RwLock`, or dynamic memory allocations within the real-time audio callback.
* **DO NOT create giant files or modules:** If a file or module grows beyond a single, clear responsibility, you must proactively refactor it into smaller, focused sub-modules. DO NOT use a single module if it can be refactored.
* **DO NOT couple to non-essential libraries:** Isolate third-party dependencies behind internal traits. Do not introduce alternative widget styling libraries; use the existing UI components.
* **DO NOT make assumptions:** If a user prompt is ambiguous or lacks necessary technical constraints (especially regarding memory management or threading), halt and ask for explicit clarification before writing code.
* **DO NOT leave dead code behind:** Clean up unused imports, deprecated functions, and commented-out code blocks during every modification. Keep diagnostic logging and audio breadcrumbs accurate during refactors.
* **DO NOT use ambiguous naming patterns:** Use precise, descriptive names (e.g., `AudioEngineState` instead of `State`).

## 5. Diagnostics and Workflow
* Set `RACK_LOG` to a `log::LevelFilter` (e.g., `Debug`, `Info`, or `Trace`) for runtime diagnostics.
* Ensure panic handlers and crash artifacts (`crash.log`, `crash.dmp`) remain functional when altering the startup sequence.

## 6. Code Generation Protocol & Quality Assurance
1. **Analyze First:** Before writing code, map out how the change impacts both the UI and the real-time audio thread, and decide which directory it belongs to.
2. **Atomic Commits:** Keep generated solutions focused on the single task requested.
3. **Refactor as You Go:** If adding a feature makes a struct or function violate the single-responsibility principle, split it immediately.
4. **Always Test, Lint, and Check:** Before submitting any code modification, you must ensure the changes adhere to strict Rust quality standards. You are directed to always run or verify compliance against the following checks:
    - `cargo fmt --all -- --check` (Ensure consistent formatting)
    - `cargo clippy -- -D warnings` (Catch linting errors and enforce idioms)
    - `cargo check` (Verify compilation without building the final binary)
    - `cargo test` (Validate logic and ensure no regressions in audio conversion, metering, or engine modules)