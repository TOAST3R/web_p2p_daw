## 1. Workspace setup

- [x] 1.1 Create Cargo workspace with crates `dsp`, `app`, `protocol`, `signaling`; add `wasm32-unknown-unknown` target and `trunk` to README setup steps
- [x] 1.2 Add dependencies: `eframe`/`egui`, `yrs`, `wasm-bindgen`, `web-sys` (Web Audio, WebRTC, WebSocket features) in `app`; `axum`, `tokio`, `tower-http` in `signaling`; `serde` in `protocol`

## 2. Spike 0: DSP engine (native)

- [x] 2.1 Implement `dsp` step clock (samples-per-step from BPM + sample rate) with unit test: 120 BPM @ 48 kHz = 6000 samples/step
- [x] 2.2 Implement 4 synthesized voices (kick, snare, hat, bass blip) triggered per active cell
- [x] 2.3 Implement play/stop (stop resets step to 0) and pattern replacement that applies from next step; unit tests for both
- [x] 2.4 Add raw `extern "C"` exports (`alloc`, `init`, `set_pattern`, `set_playing`, `process`) behind `cdylib` for wasm32

## 3. Spike 0: AudioWorklet bootstrap

- [x] 3.1 Write `worklet.js` shim: instantiate `dsp.wasm` from `processorOptions` bytes, call `process` per render quantum, copy output, forward port messages to `set_pattern`/`set_playing`
- [x] 3.2 Configure trunk to build/copy `dsp.wasm` and `worklet.js` into `dist/`
- [x] 3.3 In `app`, on "Start audio" click: create AudioContext, fetch `dsp.wasm`, `addModule(worklet.js)`, create AudioWorkletNode, connect to destination
- [x] 3.4 Minimal egui UI with Start audio + play/stop driving a hard-coded pattern; verify sound in Chrome (GO/NO-GO gate)
- [ ] 3.5 Verify 60 s playback with continuous UI interaction has no dropouts

## 4. MVP 1: Pattern model and grid UI

- [x] 4.1 Implement Yrs `pattern` map helpers (default BPM 120, get/set cell, get/set BPM, derive `Pattern` snapshot) with native unit tests
- [x] 4.2 Build egui 4×16 grid, BPM control, play/stop, playhead indicator reading from the Yrs doc
- [x] 4.3 Send pattern snapshot to the worklet after every doc change (local or remote) via `port.postMessage`
- [x] 4.4 Report current step from worklet to UI for the playhead

## 5. MVP 1: Signaling server

- [x] 5.1 Define `protocol` signaling messages (`Joined`, `PeerJoined`, `PeerLeft`, `Offer`, `Answer`, `Ice`, `RoomFull`) as serde JSON enum
- [x] 5.2 Implement axum `/ws?room=` with in-memory rooms, max 2 peers, verbatim relay, `PeerLeft` on disconnect, room drop when empty
- [x] 5.3 Serve trunk `dist/` as static files from the same binary
- [x] 5.4 Integration test with two WebSocket clients: join, relay Offer, RoomFull for third, PeerLeft on close

## 6. MVP 1: Peer sync

- [x] 6.1 Implement sync message codec (`0x00` SyncStep1, `0x01` SyncStep2, `0x02` Update) and native test: two Yrs docs with divergent edits converge via the handshake
- [x] 6.2 Native test: concurrent writes to the same cell converge to identical values; remote-origin updates are not re-broadcast
- [x] 6.3 In `app`, connect to signaling (room id from URL fragment, generated if absent); second joiner creates offer + `yrs` data channel; trickle ICE with Google STUN
- [x] 6.4 Wire data channel: on open send SyncStep1; handle SyncStep2/Update; broadcast local updates from `observe_update_v1`, skipping remote-origin
- [x] 6.5 Show connection status in UI; on `PeerLeft` keep editing locally, on rejoin redo handshake

## 7. End-to-end verification

- [x] 7.1 Two Chrome windows on one machine: edits converge both ways, each plays locally
- [ ] 7.2 Two laptops on different networks: connect, late joiner catches up, edits while disconnected merge on reconnect
- [x] 7.3 Confirm (server logs) that no Yrs data passes through the signaling server
- [x] 7.4 Document run instructions and known limits (no TURN, Chrome only, peers not in time with each other) in README
