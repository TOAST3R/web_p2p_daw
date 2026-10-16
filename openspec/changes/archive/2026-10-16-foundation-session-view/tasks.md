## 1. Engine state types and build wiring

- [x] 1.1 Add `serde` + `postcard` to `dsp`; define `dsp::state` (`EngineState`, `Transport`, `TrackState`, `Instrument`, `SynthParams`, `Clip`, `Note`) with a native round-trip test
- [x] 1.2 Move raw exports behind a `worklet` cargo feature; update the trunk pre_build hook to pass `--features worklet`; make `app` depend on `dsp`; verify `dsp.wasm` still has zero imports

## 2. Engine: scheduling and instruments (native tests first)

- [x] 2.1 Implement the frame→shared-ms anchor and beat computation per render block (`set_anchor`, transport math), with tests for play anchor, stopped transport and BPM re-anchor
- [x] 2.2 Implement note scheduling over `[b0, b1)` with loop wraps and `prev`/`now` clip switching at `at_beat`; test that loop-boundary notes land on exact frames (96000 for 1 bar at 120 BPM / 48 kHz) and that the switch happens on the bar
- [x] 2.3 Implement the Synth (8 voices, saw/square, SVF low-pass with resonance, ADSR, oldest-voice stealing); test voice stealing and note-off release
- [x] 2.4 Port the MVP drum voices into a Drum Kit instrument mapped to notes 36/38/42/45
- [x] 2.5 Mix: track volume, mute, clamp; keep voices across `load_state` matched by track id; test that a sounding note survives a state reload
- [x] 2.6 Exports: `alloc`, `load_state` (returns an error code and keeps the old state on decode failure; test it), `set_anchor`, `process(frames, current_frame)`, `out_ptr`

## 3. Project model v2

- [x] 3.1 Rewrite `model.rs` with the D1 schema: create, delete and reorder tracks; slots; clips; notes; synth params; volume/mute; id generator `{client_id}-{counter}`
- [x] 3.2 Implement transport ops (play with a 300 ms lead, stop clearing launches, BPM re-anchor) and launch ops (clip, track stop, scene launch with `prev` and next-bar `at_beat`, auto-start when stopped)
- [x] 3.3 Implement `derive_engine_state(doc) -> EngineState` (effective/prev clips resolved, notes beyond length dropped)
- [x] 3.4 Native tests: empty project defaults; concurrent track add gives 2 tracks in the same order on both docs; delete track removes its clips; concurrent launches converge; scene launch stops tracks with empty slots

## 4. Shared clock

- [x] 4.1 Implement `clock.rs` (`ClockSync`: samples, min-RTT selection over the last 8, offset, reference by lower client id); test the 4000 ms example and that the high-RTT sample is ignored
- [x] 4.2 Add a data-channel dispatcher: tags 0x00–0x02 go to `sync::handle`, 0x10/0x11 Ping/Pong go to the clock; native codec tests including malformed input
- [x] 4.3 Wire the ping schedule (5× at 100 ms, then every 1 s) and reference handover on disconnect
- [x] 4.4 Post `["anchor", frame, shared_ms + nudge]` from `getOutputTimestamp` every 1 s and on offset change; add the local latency nudge setting (±200 ms)

## 5. Worklet and app↔engine plumbing

- [x] 5.1 Update `worklet.js`: `["state", bytes]` → `alloc` + `load_state`; `["anchor", f, ms]` → `set_anchor`; call `process(n, currentFrame)`; remove step messages
- [x] 5.2 Set a dirty flag from `observe_update_v1` (local + remote); on dirty, derive the state, serialize it and post it when the bytes changed

## 6. Session View UI

- [x] 6.1 Top bar: Start audio, play/stop, BPM, latency nudge, status; remove the old grid
- [x] 6.2 Session grid: track columns ordered by `order`, 8 slot rows, + Synth / + Drums, delete track, per-track stop, volume, mute, scene launch column
- [x] 6.3 Slot interactions and states: double-click creates a 1-bar clip, click selects, ▶ launches; show stopped / queued (blinking) / playing (progress) from the shared beat

## 7. Piano roll

- [x] 7.1 Grid rendering with 1/16 snap, pitch rows for Synth and 4 named lanes for Drums, playhead
- [x] 7.2 Note interactions: click to add, drag to move, drag the right edge to resize, right-click to delete
- [x] 7.3 Velocity strip, clip length selector (1/2/4/8 bars), synth parameter controls for the selected track

## 8. Verification

- [x] 8.1 `cargo test --workspace` and clippy (native + wasm32) clean
- [x] 8.2 Two headless Chrome peers (scripted as in the MVP): both report the same beat within 10 ms; a clip launched on A at beat ~9 starts at beat 12 on both; edits sync both ways
- [ ] 8.3 Manual: listen on two windows/laptops for flams between peers; 60 s with 4 tracks while editing with no dropouts
- [x] 8.4 Update the README (Session View usage, latency nudge, wired headphones advice)
