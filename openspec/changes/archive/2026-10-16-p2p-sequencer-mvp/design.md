## Context

This is a greenfield repo. The long-term goal is a browser DAW for live collaboration among friends, possibly scaling later. The constraints are Rust everywhere, as little hand-written JS as possible, egui for the UI, and no server in the data path. Network latency can't be removed, so the core idea is to **sync state, not audio**: peers exchange small CRDT updates, and each peer renders audio locally.

```
 Browser A                                        Browser B
┌─────────────────────────────┐            ┌─────────────────────────────┐
│ egui app (wasm, main thread)│            │ egui app (wasm, main thread)│
│  ├─ Yrs Doc  ◀──────────────┼─WebRTC DC─▶┼──▶ Yrs Doc                  │
│  └─ pattern snapshot        │  (direct)  │      pattern snapshot ─┐    │
│        │ port.postMessage   │            │                        ▼    │
│ AudioWorklet (dsp.wasm)     │            │ AudioWorklet (dsp.wasm)     │
│  clock + synth → speakers   │            │  clock + synth → speakers   │
└──────────────┬──────────────┘            └──────────────┬──────────────┘
               └──── WebSocket (SDP/ICE only) ─┬──────────┘
                                     ┌─────────┴─────────┐
                                     │ Axum signaling    │  stateless, in-memory rooms
                                     └───────────────────┘
```

## Goals / Non-Goals

**Goals:**
- Prove that Rust DSP runs glitch-free in an AudioWorklet (Spike 0, the gate for everything else).
- Two Chrome browsers on different networks edit one step pattern and converge, with no server touching pattern data.
- Keep the data model and sync protocol peer-count-agnostic so mesh or relay can come later.

**Non-Goals:**
- Synced transport between peers. Each peer presses play on their own machine.
- Audio streaming, samples, persistence, TURN, auth, Safari/Firefox support.

## Decisions

### D1. Workspace layout
```
crates/
  dsp/        pure Rust, no deps; synth voices + step clock. Builds native (tests) and wasm32 cdylib (worklet)
  app/        eframe/egui wasm app: UI, Yrs doc, WebRTC, audio bootstrap (trunk)
  protocol/   serde types for signaling messages, shared by app + server
  signaling/  axum WebSocket server
```
**Why:** `dsp` must be testable natively and must load as a separate wasm instance inside the worklet, so it can't depend on wasm-bindgen or eframe.

### D2. The worklet runs a separate, raw-export wasm (no SharedArrayBuffer)
The worklet loads `dsp.wasm`: plain `extern "C"` exports (`init(sample_rate)`, `set_bpm(f32)`, `set_cell(index, on)`, `set_playing(on)`, `process(frames) -> current_step`, `out_ptr()`), with zero imports. Per-cell setters replaced the planned `set_pattern(ptr,len)` + `alloc`, so no shared-memory writes are needed. It is instantiated from bytes passed via `processorOptions`. The UI sends pattern snapshots to the worklet with `port.postMessage` (a few hundred bytes, only on change).
- **Why:** this avoids wasm threads, COOP/COEP headers and SharedArrayBuffer, which are the most fragile part of wasm audio. Pattern edits are rare events, so message passing is enough.
- **Alternative:** share one wasm module plus memory between the main thread and the worklet (threads). Rejected for the MVP because it needs a nightly/atomics build and cross-origin isolation.
- **JS footprint:** one `worklet.js` (~40 lines: instantiate, the `process()` loop copying from wasm memory, message handling) and trunk's generated loader. Nothing else.

### D3. Sequencer clock lives in the worklet
Step timing is computed per sample inside `dsp`, so it is sample-accurate and unaffected by main-thread jank from egui repaints or WebRTC.

### D4. CRDT: Yrs
- **Why Yrs:** it is the most mature Rust CRDT, with a small binary update format and built-in state-vector diffing (`encode_state_as_update_v1(sv)`). It is also wire-compatible with Yjs if a JS peer or tool is ever needed.
- **Alternatives:** Automerge (heavier wasm, richer history, which we don't need yet) and Loro (strong movable lists, which we don't need for a grid).
- **Data model:** one root `YMap` named `pattern`:
  - `bpm` → number
  - `cell:{track}:{step}` → bool (4 tracks × 16 steps)

  If two peers toggle the same cell at the same time, last-writer-wins converges, which is acceptable for a grid. Flat keys keep updates tiny.
- After every local or remote change, the app derives a plain `Pattern` struct from the map and sends it to the worklet.

### D5. Sync protocol over the data channel
The data channel is reliable and ordered, binary, and labeled `yrs`. Each message is a 1-byte tag plus a payload:
- `0x00 SyncStep1(state_vector)`: sent by both sides on open.
- `0x01 SyncStep2(update)`: the reply, the diff for the peer's state vector.
- `0x02 Update(update)`: an incremental local change, emitted from `doc.observe_update_v1`.

Updates applied from the network are tagged with a remote origin so they aren't echoed back. The protocol is symmetric and per-connection, so a later mesh just runs it once per peer.

### D6. WebRTC via web-sys
`RtcPeerConnection` uses `iceServers: [stun:stun.l.google.com:19302]` with trickle ICE. Role: the peer that joins second creates the offer (the server tells it `peer_joined`). The data channel is created by the offerer.

### D7. Signaling server
- Axum `GET /ws?room=<id>` WebSocket. Rooms are in-memory, `HashMap<RoomId, Vec<Sender>>`, max 2 peers. A third peer gets `room_full`.
- Messages (`protocol` crate, JSON): `Joined{peer_count}`, `PeerJoined`, `PeerLeft`, `Offer{sdp}`, `Answer{sdp}`, `Ice{candidate}`, `RoomFull`. The server forwards Offer/Answer/Ice verbatim to the other peer and never inspects or stores them.
- It also serves the trunk `dist/` as static files, so one binary runs the whole demo.

### D8. Audio start
The browser autoplay policy requires a user gesture, so the AudioContext and worklet are created on the first click of a "Start audio" button.

## Risks / Trade-offs

- [AudioWorklet + wasm bootstrap is awkward] → Spike 0 comes first and is a go/no-go gate. If it fails, fall back to `ScriptProcessorNode` temporarily.
- [AudioWorklet needs a secure origin (`https://` or `localhost`), so a second laptop on `http://<LAN-IP>` gets no audio] → document using an HTTPS tunnel (e.g. cloudflared); the app picks `wss://` automatically.
- [VPN interfaces can break host/srflx pairing (observed locally with a `utun` VPN)] → documented; TURN would cover it later.
- [Symmetric NAT without TURN, ~10–20% of networks] → document it. Same-LAN always works. TURN is a later change.
- [Main-thread egui load] → audio is unaffected (D3). The UI repaints only on input or remote change.
- [Last-writer-wins on the same cell can surprise people] → acceptable for a grid. Revisit for notes/clips.
- [Yrs wasm size, a few hundred KB] → acceptable for the MVP.
- [Peers not in time with each other] → expected (no shared transport). MVP 2 adds clock sync.

## Open Questions

- Should the room id live in the URL fragment (`#room=abc`) or be typed into the UI? Default: URL fragment, generated if absent.
- Sound: three simple drum voices + 1 bass voice, or 4 sine-pitch voices? Default: kick/snare/hat synthesized, plus a simple bass blip.
