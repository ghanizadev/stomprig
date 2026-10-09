# MIDI Controller Follow-up Backlog

The initial rack-level workflow is implemented: MIDI input can learn a Control Change or Note On trigger to toggle one plugin's bypass, and mappings are saved with presets. This file tracks capabilities beyond that scope.

## Remaining work

### Mapping management

The preset and runtime models hold mapping lists, but the plugin UI exposes only one mapping per plugin. Add UI support to create, edit, and remove multiple mappings without replacing the existing mapping when learning a new one.

### Additional actions

Add mapping actions such as momentary bypass, plugin parameter control, and backing-track transport. Program Change selection within the active live preset is already supported; extend the action model only for additional requested behaviors.

### MIDI forwarding

Forward supported MIDI events to VST3 plugins that accept MIDI, with bounded event delivery and clear handling for plugins that do not expose MIDI input.

### MIDI output and feedback

Add MIDI output and controller feedback, including feedback LEDs where the device supports them.

## Acceptance criteria

- Multiple mappings can be managed independently for each plugin.
- Each new action has defined trigger and release behavior.
- Mapping changes remain associated with the correct plugin through reorder, removal, preset save, and preset load.
- MIDI forwarding and output remain independent of the audio callback and UI frame rate.
- Unsupported devices and plugins produce actionable status without interrupting audio.
