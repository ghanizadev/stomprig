- Create a mono ↔ stereo compatibility layer for audio processing and accept both I/O.
- If I change the parameters via a native editor, it does not save the changed parameters into the preset file.

## Sidechain routing for CLAP and VST3

### Current behavior and gaps

- Rack currently processes one stereo main signal through a serial plugin chain. Each plugin receives the current main input and returns the main output; that output becomes the next plugin's input. There is no independent per-plugin sidechain source or send graph.
- CLAP layout detection recognizes a stereo main input plus one stereo secondary input, in either port order, when there is one stereo output. The processor allocates sidechain scratch buffers and passes them to the plugin, but clears those buffers to silence on every block. This allows some plugins with this layout to load; it does not provide usable sidechain audio. Other sidechain channel layouts are not accepted by this path.
- The CLAP metadata log describes ports and identifies the main input, but the app does not yet expose or persist a sidechain source selection. The [CLAP audio-ports spec](https://github.com/free-audio/clap/blob/main/include/clap/ext/audio-ports.h) provides an `IS_MAIN` flag and port metadata, but no standard auxiliary/sidechain flag. Rack must use port names or configuration as hints and provide explicit mapping for ambiguous secondary inputs; it must not assume every non-main port is a sidechain.
- The vendored VST3 host already has bus-aware APIs: it can query audio bus channel counts and activation, activate a bus while processing is stopped, allocate `BusAudioBuffers`, and process while preserving bus boundaries. VST3 identifies auxiliary buses with `kAux` ([bus API](https://steinbergmedia.github.io/vst3_doc/vstinterfaces/group__vstBus.html)). Rack currently calls the legacy `process_audio(AudioBuffers)` path, so it neither activates an auxiliary input nor supplies audio to one. The app also needs enough VST3 metadata to identify auxiliary (`kAux`) input buses by their bus index and channel count.
- `PluginSlot` and the saved `PresetPlugin` model have no sidechain routing field. The audio callback consumes all device input channels but maps only the selected main input pair into the Rack signal. Audio-clip playback is mixed after the plugin chain today, so it cannot currently be selected as a plugin sidechain source without adding a pre-chain tap.

### Recommended first version

Add one optional sidechain source per plugin slot, separate from that plugin's main-chain input. Start with `Off` and selectable Rack/device input sources. For multichannel input devices, allow choosing a channel or channel pair independently of the main input pair; this provides a genuinely independent key signal when the hardware exposes one. A Rack pre-effect input tap can also be offered, with its signal captured before the serial chain. Keep arbitrary sends from other plugin slots, parallel chains, and playback clips as a later graph-routing feature; Rack does not have that graph today.

Persist the choice with the corresponding preset plugin entry and give old presets a backward-compatible default of `Off`. `Off` must keep the current silent-sidechain behavior. If a selected source is unavailable after changing devices, fall back to silence and report the mismatch rather than indexing an unavailable channel or silently changing the user's route.

Keep Rack's canonical main path stereo. Adapt sidechain data to the plugin port/bus channel count at the host boundary: copy stereo sources as-is; mirror a mono source into both channels for a stereo sidechain; and use a documented, level-safe downmix when a stereo source feeds a mono sidechain. Reuse the existing mono/stereo compatibility rules where applicable.

### Implementation plan

1. **Model and source capture**
   - Add a serializable per-slot `SidechainSource` with an explicit `Off` default and source variants for Rack input and selected device input channels. Include the field in `PresetPlugin` with a serde default so existing preset files continue to load.
   - Add a per-slot source selector when VST3 metadata or CLAP port naming/configuration identifies a supported sidechain input, with explicit mapping for ambiguous CLAP secondary ports. Display the plugin's port/bus name, index, and channel count so the selected route is understandable. Do not add UI for unimplemented source types.
   - Extend audio routing to capture the chosen source while consuming device frames. Compute channel selections and channel conversions before or at engine configuration time. Make the sidechain block available to the matching plugin alongside its main buffers.

2. **Shared real-time path**
   - Extend the runtime plugin processing interface to receive the sidechain block separately from the main `AudioBuffers`; do not merge it into the main chain or change what is passed to the next plugin.
   - Allocate sidechain scratch storage and format-specific bus buffers before audio processing starts, size them for the configured maximum block, and reuse them in every callback. No heap allocation, port enumeration, blocking lock, or plugin reconfiguration may be added to the real-time callback.
   - Use the existing engine shutdown/restart lifecycle for changes that require plugin activation or buffer-layout changes. Route selection updates must reach the callback through the existing lock-free/runtime configuration mechanisms.

3. **CLAP adapter**
   - Use `IS_MAIN` to identify the main input and preserve each secondary port index, name, channel count, and type. CLAP has no standard sidechain/aux flag, so do not infer sidechain status from `IS_MAIN == false` alone. Treat a clear name such as `Sidechain` or plugin-specific port configuration as metadata; expose ambiguous secondary ports for explicit mapping or leave them unsupported with a diagnostic.
   - Replace `silent_sidechain_input` as the only source with preallocated buffers populated from the selected Rack source. Continue to clear the buffers when the route is `Off` so stale samples can never leak into later blocks.
   - If the plugin exposes [CLAP audio-port activation](https://github.com/free-audio/clap/blob/main/include/clap/ext/audio-ports-activation.h), update the sidechain port state while the plugin is deactivated (unless it explicitly supports activation during processing), then restore it after reconfiguration. Always provide correctly shaped zero-filled buffers while the sidechain is unrouted or inactive.
   - Initially support one stereo main input plus one secondary input explicitly identified or mapped as the sidechain, with one or two channels and one supported main output. Preserve the actual port indices when constructing `AudioPorts`. Log all declared layouts and explain which unsupported condition prevented loading or routing.
   - Keep normal mono, stereo, and dual-mono processing independent from sidechain detection. Validate mono and stereo sidechain conversion separately from main-input/output conversion.

4. **VST3 adapter**
   - Query and expose audio input bus metadata needed to distinguish the main bus from auxiliary (`kAux`) buses, including stable bus index, name, channel count, and activation state. Do not infer that every additional input bus is a sidechain.
   - During plugin configuration, stop processing, activate the selected auxiliary input bus with the VST3 host API, re-query the layout, then create/recreate the plugin's `BusAudioBuffers` outside the callback. Starting or changing this route must follow the host's inactive-component requirement.
   - In the callback, copy the current main-chain signal to the VST3 main input bus and the selected source to the auxiliary bus at its reported bus index. Call `process_bus_audio`, then copy the VST3 main output bus back into Rack's main output buffers for the next slot. Keep all other buses correctly shaped and silent unless Rack explicitly routes them.
   - Initially support one auxiliary audio input bus with one or two channels. Log bus layouts and reject or leave unsupported extra auxiliary buses unconnected rather than flattening buses or guessing their meaning.

5. **Verification and diagnostics**
   - Log the selected source and the plugin's resolved main/auxiliary port or bus mapping when the engine configures the slot. Include declared and active bus/port layouts in errors.
   - Verify `Off` produces silence, a mono source is mirrored into a stereo sidechain, stereo reaches stereo unchanged, stereo-to-mono uses the documented downmix, and a live source changes a known sidechain-capable plugin's response.
   - Verify CLAP metadata for a main input and a clearly named/mapped secondary sidechain, plus an ambiguous secondary port that must not be silently treated as a sidechain. Verify the VST3 layout with an initially inactive auxiliary bus that is activated before processing. Confirm unsupported or multiple auxiliary layouts fail with actionable diagnostics.
   - Verify presets written before this field existed still load with sidechain `Off`, and changing a route takes effect after the normal engine/plugin restart without callback allocations or blocking synchronization.

### Scope estimate

The first version is a medium-sized host feature: CLAP already has a narrow sidechain port path and preallocated scratch buffers; the vendored VST3 host already supports bus-aware processing and activation. Most work is in Rack's shared source model, per-slot UI/preset state, audio-callback routing, and integrating VST3 bus buffers into runtime configuration. Supporting arbitrary plugin-to-plugin sends, parallel chains, or clip playback as a key signal is a larger audio-graph project and should be planned separately.
