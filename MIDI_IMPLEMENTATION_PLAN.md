# MIDI Controller Implementation Plan

## Goal

Allow a MIDI controller to trigger actions for an individual loaded plugin. The first action will be toggling that plugin's rack bypass state. MIDI mappings must be saved and restored as part of presets.

The first version will handle MIDI at the rack level. It will not depend on a plugin exposing MIDI parameters, and it will not forward MIDI events into VST2 or VST3 plugins yet.

## Existing pieces to reuse

- `plugin_bypass_states` already contains one `Arc<AtomicBool>` per loaded plugin.
- `src/app/audio/processing.rs` already checks the bypass atomics inside the runtime processing path.
- `PluginRuntime` owns the thread that processes plugin blocks and is the right place to evaluate MIDI mappings.
- `PresetPlugin` already stores per-plugin parameters and bypass state.
- Plugin paths and fingerprints already identify the stack and protect preset loading.
- The vendored `vst3-host` crate already contains MIDI event types and a `midi-input` feature built on `midir`.

## Recommended first-version behavior

### Supported triggers

Support these event types initially:

- Control Change, useful for footswitches and knobs
- Note On, useful for keyboard keys and pads

Each learned trigger stores its MIDI channel and controller-specific value:

- Control Change: controller number
- Note On: note number

MIDI channels should be shown as 1 through 16 in the UI and persisted as 1 through 16.

### Supported action

The first action is:

```text
Toggle plugin bypass
```

The mapping model should still use an action enum so future actions can be added without changing the preset structure.

### Control Change edge behavior

A Control Change should not toggle on every value change. Treat values greater than or equal to an activation threshold, initially `64`, as active and toggle only on the transition from inactive to active.

This makes common footswitch messages work correctly even when a device sends multiple values for one press.

Note On should trigger on velocity greater than zero. Note Off and Note On with velocity zero should release the trigger without toggling again.

## Runtime architecture

The MIDI input callback must remain independent of the audio callback and UI frame rate:

```text
MIDI device
    |
    | midir callback
    | nonblocking push
    v
MIDI ingress ring
    |
    v
PluginRuntime worker
    |
    | match mapping and update state
    v
plugin bypass AtomicBool
    |
    v
audio processing block
```

### MIDI input connection

1. Enable the `midi-input` feature for the vendored `vst3-host` dependency.
2. Add a MIDI input manager owned by `PluginEditor` or a small dedicated module.
3. Enumerate ports with `list_midi_input_ports`.
4. Keep the selected `MidiInputConnection` alive for as long as the input is enabled.
5. Reconnect when the selected port changes or when the device is refreshed.
6. Treat unavailable devices as a recoverable condition. Loaded mappings should remain active in memory and in presets.

The MIDI callback should push `MidiEvent` values into a bounded single-producer ring. It must not acquire plugin locks, update egui state, or send blocking commands.

### Mapping evaluation

`PluginRuntime` should own the active runtime mapping state. Add runtime commands for:

- Replacing the current mapping set
- Starting MIDI learn for a plugin index
- Cancelling MIDI learn
- Installing or replacing the MIDI event ingress consumer when a connection is recreated

Before processing each audio block, the runtime drains a bounded number of MIDI events and:

1. Handles MIDI learn if active.
2. Finds mappings matching the event.
3. Applies the action to the target plugin's bypass atomic.
4. Records the previous active state for Control Change edge detection.

The runtime should evaluate mappings even when audio is stopped so learn mode can still receive events. Events that arrive while no mapping is active may be discarded.

## MIDI learn flow

1. The user presses `Learn MIDI` for a plugin.
2. The UI sends a learn-target command containing the plugin index.
3. The runtime marks that plugin as the current learn target.
4. The next supported MIDI event is sent to a bounded feedback ring for the UI.
5. The event is not applied to an existing mapping while learning.
6. The UI creates or replaces the selected plugin's mapping.
7. The UI sends the new mapping set to the runtime and ends learn mode.

The first supported event should complete learning. Unsupported event types should be ignored and leave learn mode active.

The UI should show a clear listening state, such as `Listening for MIDI...`, and provide a cancel action.

## Data model

Use a serializable preset type separate from the runtime `vst3_host::MidiEvent` type. This keeps the preset format stable and makes validation explicit.

Conceptually:

```rust
struct PresetMidiMapping {
    event: MidiTrigger,
    action: MidiAction,
}

enum MidiTrigger {
    ControlChange {
        channel: u8,
        controller: u8,
        threshold: u8,
    },
    NoteOn {
        channel: u8,
        note: u8,
    },
}

enum MidiAction {
    ToggleBypass,
}
```

Store a list on each `PresetPlugin`:

```rust
#[serde(default)]
pub(crate) midi_mappings: Vec<PresetMidiMapping>,
```

The list allows the first UI to expose one mapping while keeping the file format ready for multiple mappings per plugin.

Example preset representation:

```toml
[[plugins]]
path = "/Library/Audio/Plug-Ins/VST3/Example.vst3"
parameters = []
bypassed = false

[[plugins.midi_mappings]]
action = "toggle_bypass"
event = "control_change"
channel = 1
controller = 20
threshold = 64
```

`#[serde(default)]` keeps existing presets valid and gives them an empty mapping list.

## Preset lifecycle integration

### Capture

Extend `preset_from_snapshots` and `PluginEditor::capture_preset` to validate and capture one mapping list per loaded plugin.

The capture validation should ensure that these arrays remain aligned:

- Configured plugin paths
- Plugin snapshots
- Bypass states
- MIDI mapping lists

The current bypass state remains independent from the mapping. Saving a preset after a MIDI performance therefore preserves both the current bypass state and the mapping configuration.

### Load

When a preset is loaded:

1. Validate plugin paths and fingerprints as today.
2. Load the plugin stack.
3. Install the plugin and bypass state arrays.
4. Apply parameters and saved bypass states.
5. Apply the saved per-plugin MIDI mappings to the runtime.
6. Start audio.

Mappings should be installed before audio starts so the stack is fully configured when the first MIDI event arrives.

### Stack changes

Mappings must follow the same vector operations as the rest of the plugin state:

- Add an empty mapping list for a new plugin.
- Remove the mapping list with the removed plugin.
- Move the mapping list when plugins are reordered.
- Clear mappings when the stack is cleared.

Because mappings are stored inside each `PresetPlugin`, preset loading naturally keeps them associated with the corresponding stack position after fingerprint validation.

## UI design

Add a MIDI area to each plugin container, initially in the expanded plugin body or header:

- `Learn MIDI`
- Current learned trigger, such as `CC 20 · Channel 1`
- Action label, `Toggle bypass`
- `Clear`

The first UI can show one active mapping per plugin. Internally and in the preset format, keep a list so `Add mapping` can be introduced later.

Add MIDI device controls to the settings UI:

- MIDI input device selector
- Refresh devices
- Enable or disable MIDI input
- Connection status and error text

Persist the preferred MIDI input device name in application settings, not in presets. A MIDI device is part of the host setup, while the mapping is part of the musical/plugin configuration.

## File and module changes

Likely changes:

- `Cargo.toml`: enable `vst3-host`'s `midi-input` feature.
- `src/app/midi.rs`: MIDI port discovery, connection lifecycle, event ingress, and learn feedback types.
- `src/app/plugin_runtime.rs`: MIDI ingress, mapping commands, learn handling, and runtime matching.
- `src/app/plugin_editor.rs`: mapping state, learn UI, stack synchronization, and preset integration.
- `src/app/presets.rs`: serializable mapping types and capture/load support.
- `src/app/settings.rs`: preferred MIDI device and MIDI enable state.
- `src/app/mod.rs`: settings persistence wiring if MIDI settings are surfaced there.

The exact module split can be adjusted after the first implementation pass, but MIDI matching should remain independently testable from the UI and plugin loading code.

## Delivery phases

### Phase 1: Types and matching

- Define runtime and preset mapping types.
- Validate MIDI channel, controller, note, and threshold ranges.
- Implement event matching and Control Change edge detection.
- Define conversion between `vst3_host::MidiEvent` and the serializable trigger type.

### Phase 2: MIDI input lifecycle

- Enable the dependency feature.
- Enumerate MIDI input ports.
- Connect and disconnect ports safely.
- Add preferred-device persistence and settings UI.
- Add the bounded ingress ring.

### Phase 3: Runtime action path

- Add mapping commands to `PluginRuntime`.
- Drain MIDI events in the runtime worker.
- Update plugin bypass atomics from matched events.
- Add learn-target and learn-feedback handling.

### Phase 4: Plugin UI

- Add per-plugin learn controls.
- Display learned events and mapping actions.
- Add clear and cancel behavior.
- Keep mappings synchronized through add, remove, reorder, and clear operations.

### Phase 5: Presets

- Extend `PresetPlugin` with defaulted mappings.
- Capture and restore mappings.
- Preserve compatibility with existing presets.
- Ensure imported and exported presets include mappings.

### Phase 6: Follow-up capabilities

Potential later actions include:

- Momentary bypass
- Parameter control
- Set plugin parameter value
- Enable or disable playback
- Multiple MIDI mappings per plugin in the UI
- MIDI forwarding into VST2/VST3 plugins
- MIDI output and feedback LEDs

## Acceptance criteria

- A selected MIDI input device can be connected without blocking the audio callback.
- Pressing `Learn MIDI` and sending a supported event creates a mapping for the selected plugin.
- The learned event toggles only the targeted plugin's bypass state.
- A Control Change does not toggle repeatedly while its value remains active.
- MIDI bypass changes take effect within approximately one audio block.
- Saving and reloading a preset preserves the mapping and the current bypass state.
- Existing presets without MIDI data continue to load.
- Removing or reordering plugins keeps mappings associated with the correct plugin.
- Disconnecting the MIDI device does not lose mappings or interrupt audio.
