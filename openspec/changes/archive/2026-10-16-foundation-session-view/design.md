## Context

The MVP (archived `2026-10-16-p2p-sequencer-mvp`) syncs a flat Yrs map of 64 booleans between two browsers. Each browser renders 4 hard-coded drum voices in an AudioWorklet (`dsp.wasm`, raw exports, zero imports), and each laptop runs its own local play/stop. Signaling, WebRTC and the Yrs sync handshake work well and stay as they are.

This change replaces the model, the engine and the UI, and adds a shared musical timeline. The goal is that two laptops can jam in Session View and hear clips start **on the same bar**.

```
 Browser A                                            Browser B
┌──────────────────────────────────┐            ┌──────────────────────────────────┐
│ egui: Session grid + piano roll  │            │ egui: Session grid + piano roll  │
│   │ edits            ▲ redraw    │            │                                  │
│ Yrs project doc ◀────┼───────────┼─ yrs 0x00-02 ─▶ Yrs project doc               │
│   │ derive (on change)           │            │                                  │
│ EngineState ─postcard─┐          │            │ ClockSync: offset to reference ◀─┼─ ping/pong 0x10/11
│ ClockSync ─ anchor ───┤          │            │                                  │
│ AudioWorklet: dsp.wasm▼          │            │ AudioWorklet: dsp.wasm           │
│   shared-time → beat → notes     │            │   same beat at the same instant  │
└──────────────────────────────────┘            └──────────────────────────────────┘
```

## Goals / Non-Goals

**Goals:**
- A song structure (tracks → slots → clips → notes, plus instruments) that later changes can extend: devices, samples, arrangement, automation.
- Both peers agree on beat position within a few milliseconds, including at play, stop and BPM change.
- Session View with quantized launching, and a usable piano roll.

**Non-Goals:**
- Effects, racks, macros, samples, arrangement, automation, undo, MIDI input, more than 2 peers.
- Bit-exact audio across peers. The goal is timing agreement, not identical waveforms.

## Decisions

### D1. Project schema in Yrs (three root maps)
```
transport : YMap { playing: bool, anchor_ms: f64, anchor_beat: f64, bpm: f64 }
tracks    : YMap  id → YMap {
              name, order: f64, kind: "synth"|"drums", volume: f64, mute: bool,
              params: YMap { wave, cutoff, resonance, attack, decay, sustain, release },
              slots:  YMap "0".."7" → clip id,
              launch: Any { clip: id|"", at: f64, prev: id|"", prev_at: f64 } }
clips     : YMap  id → YMap { name, length_beats: f64, notes: YMap note_id → Any { p, s, l, v } }
```
- **Why maps keyed by id, not arrays:** concurrent inserts and deletes of tracks or notes never shift indexes. Track order is a fractional `order` (new = max + 1, ties broken by id). This avoids the classic "move in a CRDT list" problem.
- **Notes as immutable `Any` values:** editing a note replaces its value. Concurrent edits of the same note are last-writer-wins per note, which is acceptable for a piano roll.
- **Ids:** `{yrs_client_id:x}-{local_counter}`. Collision-free without coordination.
- **Empty doc = empty project.** There is no seeded default content: two fresh peers creating "default tracks" at the same time would get duplicates. Users add tracks with "+ Synth" / "+ Drums".
- **Alternative considered:** one big serialized blob per clip. Rejected because every note edit would then conflict with every other edit in that clip.

### D2. Shared clock: NTP-style offset to a reference peer
- New data-channel messages next to the Yrs tags: `0x10 Ping { t0, client_id }` and `0x11 Pong { t0, t1 }`. Local time is `performance.timeOrigin + performance.now()` (ms).
- **Reference** = the peer with the lower Yrs client id. Pongs carry the responder's *shared* time, so the reference keeps whatever offset it has (0 unless it inherited the role). The other peer keeps the last 8 samples and uses the offset from the sample with the **lowest round-trip time**: `offset = t1 − (t0 + t3) / 2`. It pings 5× at 100 ms on connect, then every 1 s.
- `shared_now = local_now + offset`. This generalizes to a mesh later: everyone syncs to the lowest id.
- Pure logic lives in `app/src/clock.rs`, testable natively. **Alternative:** a server time source. Rejected because it puts a server back in the timing path.

### D3. Shared transport timeline
- `beat(T) = anchor_beat + (T − anchor_ms) · bpm / 60000` while `playing`.
- **Play:** `anchor_beat = 0` and `anchor_ms = shared_now + 300 ms`. The lead gives the remote peer time to receive the update before beat 0, so both start cleanly. A peer that receives it late still lands on the correct beat; it only misses the first milliseconds.
- **BPM change while playing:** re-anchor in one transaction (`anchor_beat = beat(now)`, `anchor_ms = now`, new `bpm`), so the position never jumps.
- **Stop:** `playing = false` and every track's launch is cleared.
- **Launch quantum:** 1 bar of 4/4 = 4 beats, fixed for now.

### D4. Quantized clip launch as shared state
- Launching clip C on track K writes `launch = { clip: C, at: next_bar(beat(now)), prev: <clip effective now>, prev_at: <its launch beat> }`. Stopping writes `clip: ""`. A scene launch writes every track's launch in one transaction.
- If the transport is stopped, launching also starts it (D3 Play) with `at_beat = 0`.
- Playback rule (engine and UI): at beat `b` the track plays `clip` if `b ≥ at_beat`, otherwise `prev`. Clip-local position is `(b − at) mod length_beats`; `prev` keeps its own phase, `(b − prev_at) mod length`.
- **Why it hides latency:** the op only has to reach the other peer before the next bar (≥ 0 s, typically ~1 s at 120 BPM). Network delay is ~50 ms.
- Concurrent launches on one track are last-writer-wins. Both peers converge on the same clip.

### D5. dsp becomes the engine, and `app` shares its types
- `dsp::state::EngineState { transport, tracks: Vec<TrackState> }` with `TrackState { id, instrument: Synth(SynthParams) | DrumKit, volume, mute, now: Option<Clip>, at_beat, prev: Option<Clip> }` and `Clip { length_beats, notes: Vec<Note { pitch, start, len, vel }> }`, serialized with **serde + postcard** (no_std-friendly, compact, no wasm imports).
- The raw exports move behind a cargo feature `worklet`. The trunk pre_build hook builds `dsp` with `--features worklet`. `app` depends on `dsp` without it, which fixes the MVP's duplicate-symbol concern.
- The app derives `EngineState` from the doc **only when the doc changes** (a dirty flag set by `observe_update_v1` for both local and remote txns). It sends the bytes when they differ from the last send.
- **Alternative:** keep incremental per-field messages. Rejected because it grows a second protocol that duplicates the model.

### D6. Worklet timing: map audio frames to shared time
- Every 1 s, and on any offset change, the main thread reads `ctx.getOutputTimestamp()` → `{contextTime, performanceTime}` and posts `["anchor", contextTime·sr, timeOrigin + performanceTime + offset + user_latency_ms]`.
- The worklet then computes `shared_ms(frame) = ref_ms + (frame − ref_frame) / sr · 1000`, with `frame = currentFrame`.
- Because `getOutputTimestamp` describes the sample **leaving the speakers**, output latency is compensated automatically. A local "latency nudge" (±200 ms, per user, not synced) covers devices that report latency wrongly, such as Bluetooth headphones.
- Worklet messages in: `["state", Uint8Array]` → `alloc` + `load_state`, and `["anchor", f64, f64]` → `set_anchor`. Messages out: none. The UI computes the playhead itself from the shared clock.
- New exports: `alloc(len) -> ptr` (one reusable buffer), `load_state(len) -> i32` (reads the `alloc` buffer), `set_anchor(frame, ms)`, `process(frames, current_frame) -> ()`, `out_ptr()`.

### D7. Engine rendering
- Per render block: compute the beat range `[b0, b1)` for the block. For each unmuted track, find note-ons whose loop-adjusted start falls in the range (iterating loop wraps), then schedule them with exact frame offsets. Note-offs come at `start + len`.
- Each track keeps its voices across `load_state` (matched by track id), so editing while playing doesn't cut sounding notes.
- **Synth:** 8-voice polyphony, saw/square oscillator → resonant low-pass (state-variable filter) → ADSR. Oldest-voice stealing.
- **Drum Kit:** the MVP's 4 synthesized voices mapped to MIDI notes 36 (kick), 38 (snare), 42 (hat), 45 (bass/tom).
- Output: mono sum × track volume → master, clamped to [−1, 1].

### D8. UI layout (egui)
```
┌ top bar: [Start audio] [▶/■] BPM [120] · latency nudge · status ──────────────┐
├ Session grid ──────────────────────────────────────────────────────────────────┤
│        Track A     Track B     Track C    [+ Synth] [+ Drums]   │ scenes      │
│  1     [▶ clip ]   [  empty ]  [▶ clip ]                         │  [▶ 1]      │
│  …     8 rows                                                    │  [▶ …]      │
│  stop  [■]         [■]         [■]                               │             │
│  mix   vol ▮ mute  vol ▮ mute  vol ▮ mute                        │             │
├ Detail panel (selected clip) ──────────────────────────────────────────────────┤
│  piano roll: 1/16 grid, pitch rows (drums: 4 named lanes), velocity strip      │
│  clip length [1|2|4|8 bars] · synth params of the selected track               │
└─────────────────────────────────────────────────────────────────────────────────┘
```
- Slot interactions: double-click an empty slot to create a 1-bar clip; click a clip to select it; click its ▶ to launch. A queued clip blinks until `at_beat`, and a playing clip shows its loop progress.
- Piano roll: click empty space to add a note (1/16 or the last-used length); drag the body to move, the right edge to resize, right-click to delete. Velocity is set by dragging bars in the strip under the grid.

## Risks / Trade-offs

- [Clock error from asymmetric Wi-Fi routes, typically ±2–5 ms] → the min-RTT filter, plus the latency nudge for anything systematic. Below the ~10 ms where a flam becomes obvious.
- [`outputLatency`/`getOutputTimestamp` inaccurate on some devices (Bluetooth)] → the per-user latency nudge. Document wired headphones for tight sync.
- [Audio clock drifts against the performance clock] → re-anchor every second. Corrections are sub-millisecond.
- [Both peers create a clip in the same empty slot] → slot is last-writer-wins, and the losing clip is orphaned in `clips` (invisible garbage). Acceptable until there is persistence and GC.
- [Engine state grows with project size] → derived only on change. postcard keeps typical projects in the tens of KB, fine for `postMessage`.
- [Piano roll interaction complexity in egui] → stick to the interactions listed in D8, no box-select or copy/paste in this change.
- [Breaking model change] → no persistence exists, so peers must simply run the same build. The sync handshake would accept the old model but nothing would render it.

## Migration Plan

Deploy is "rebuild and reload both browsers". Peers on different builds will sync unrecognized keys harmlessly but won't hear each other's content.

## Open Questions

- Should the launch quantum be a setting (none / 1/4 / 1 bar / 2 bars)? Fixed at 1 bar for now.
- Resolved: when the reference leaves, the remaining peer becomes the reference and keeps its current offset, so the timeline never jumps.
