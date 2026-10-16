## Why

The MVP proved the stack, but its model is a fixed 4×16 grid of hard-coded drum voices, and each laptop plays on its own clock. Every Ableton feature we want (racks, samples, arrangement, automation) needs a real song structure underneath, and collaboration needs peers to play **in time**. Session View is the first feature on top of that foundation because quantized clip launching hides network latency, which makes live jamming between laptops possible without streaming audio.

## What Changes

- **BREAKING** Replace the flat `cell:{t}:{s}` pattern with **project model v2** in Yrs: tracks, clip slots, MIDI clips with notes, instruments with parameters, and shared launch state. There is no persistence, so nothing needs migrating.
- **Shared clock:** peers estimate their clock offset over the data channel and agree on one musical timeline (transport anchor + BPM), so play/stop and beat positions match across laptops, including output-latency compensation.
- **Audio graph v2:** the engine renders N tracks, each with an instrument (polyphonic **Synth** or **Drum Kit**) plus track volume and mute, summed to master. It receives a serialized engine state instead of per-cell messages.
- **Session View:** a grid of tracks × 8 scenes. Clips are launched and stopped quantized to the next bar, scenes launch whole rows, and launches are shared so both laptops start the clip on the same bar.
- **Piano roll:** edit the selected MIDI clip's notes (add, move, resize, delete, velocity), set the clip length, and snap to a 1/16 grid.
- **BREAKING** The step-sequencer grid UI and its local play/stop are removed (Session View replaces them).

Out of scope (later changes): device chains / effects / racks / macros, samples and blob sync, arrangement view, automation, undo/redo, MIDI input/recording, more than 2 peers, Ableton Link interop.

## Capabilities

### New Capabilities
- `project-model`: the Yrs document schema for tracks, clips, notes, instruments and launch state, plus collision-free ids and conflict rules.
- `shared-clock`: clock-offset estimation between peers and the shared transport timeline (anchor, BPM, play/stop), with output-latency compensation.
- `session-view`: the clip-launch grid, quantized clip and scene launching, stop buttons, and track management.
- `clip-editor`: the piano roll for MIDI clip notes and clip length.

### Modified Capabilities
- `audio-engine`: renders multiple tracks with Synth/Drum Kit instruments from a serialized engine state, and schedules from the shared timeline instead of a local step clock and local transport.
- `step-sequencer`: all requirements removed. Session View and the piano roll replace it.

## Impact

- `crates/app`: `model.rs` is rewritten (project model v2). The UI is rewritten (session grid + piano roll). New clock-sync messages go on the data channel. Signaling and WebRTC code are unchanged.
- `crates/dsp`: becomes a multi-track engine with voices/instruments and a state decoder. Its exports move behind a `worklet` cargo feature so `app` can depend on `dsp` for shared state types.
- `crates/app/worklet.js`: the message protocol changes to "load state bytes" plus a timing anchor.
- New dependencies: `serde` + `postcard` in `dsp` (compact binary engine state, no wasm imports).
- `signaling`, `protocol`: unchanged.
