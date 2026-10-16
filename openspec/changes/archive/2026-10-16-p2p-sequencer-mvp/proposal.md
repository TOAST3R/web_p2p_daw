## Why

We want a browser DAW where friends on different laptops can make music together without hearing lag. The way to get that is to sync the project state (tiny edit ops) directly between browsers and have each laptop render its own audio. Before building anything bigger we need to prove two things: that a Rust/wasm audio engine works inside an AudioWorklet, and that two browsers can stay in sync peer-to-peer with no server in the data path.

## What Changes

- **Spike 0**: a Rust DSP engine (simple synth voices) compiled to wasm that runs inside a browser AudioWorklet, started and controlled from an egui UI (eframe on wasm32). Single laptop.
- **MVP 1**: a shared step sequencer (fixed grid of tracks × 16 steps, plus BPM).
  - Pattern state lives in a Yrs CRDT document.
  - Two browsers sync that document directly over a WebRTC data channel.
  - Each laptop plays the pattern locally through its own audio engine, so playback has zero network latency.
- A tiny stateless Rust (Axum) signaling server that only relays SDP/ICE messages so two peers can find each other. It never sees project data.
- Public STUN for NAT traversal. No TURN.
- Hand-written JavaScript is limited to the AudioWorklet loader shim. Everything else is Rust (wasm-bindgen glue is generated).
- Chrome is the target browser.

Out of scope: shared transport / clock sync, live audio streaming (jam mode), samples/blobs, TURN, persistence, more than 2 peers (the design must not prevent it), piano roll, other browsers.

## Capabilities

### New Capabilities
- `audio-engine`: Rust/wasm synth and sequencer clock running in an AudioWorklet; local transport (play/stop); takes pattern updates from the UI.
- `step-sequencer`: the egui grid UI and the pattern data model (tracks × steps, BPM) stored in a Yrs document.
- `peer-sync`: direct browser-to-browser sync of the Yrs document over a WebRTC data channel, including the initial state exchange and incremental updates.
- `signaling`: a stateless Axum WebSocket service that pairs peers by room id and relays offer/answer/ICE messages.

### Modified Capabilities
<!-- none: greenfield project -->

## Impact

- New Cargo workspace (greenfield): DSP crate, wasm app crate, signaling server crate, shared protocol types.
- New dependencies: `eframe`/`egui`, `yrs`, `wasm-bindgen`, `web-sys` (WebRTC, Web Audio, WebSocket features), `axum`, `tokio`, `serde`.
- Build tooling: `trunk` for the wasm app and the `wasm32-unknown-unknown` target.
- External services: a public STUN server (Google's).
