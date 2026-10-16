# P2P Sequencer

A browser step sequencer where two laptops edit the same pattern live. Pattern edits go **directly
between browsers** (WebRTC data channel, Yrs CRDT), and each laptop renders its own audio in an AudioWorklet,
so playback has no network lag. Rust everywhere; the only hand-written JS is `crates/app/worklet.js`.

```
crates/dsp        synth + step clock (pure Rust; raw wasm for the AudioWorklet)
crates/app        egui UI, Yrs pattern doc, sync protocol, WebRTC + audio bootstrap
crates/protocol   signaling message types
crates/signaling  axum: pairs peers by room, relays SDP/ICE, serves the app
```

## Setup

```sh
rustup target add wasm32-unknown-unknown
brew install trunk            # or: cargo install trunk --locked
```

## Run

```sh
(cd crates/app && trunk build --release)   # also builds dsp.wasm via a pre_build hook
cargo run --release -p signaling           # http://localhost:8080
```

Open `http://localhost:8080`. The URL gets a `#room=<id>`; open the same URL in a second window or on a
second laptop. Click **Start audio**, then **Play**. Each person presses play on their own machine.

Tests: `cargo test --workspace` (DSP timing, CRDT model, sync convergence, signaling rooms).

## Using a second laptop

Browsers only allow AudioWorklet on **secure origins** (`https://` or `localhost`), so
`http://192.168.x.x:8080` loads but audio won't start. Use an HTTPS tunnel instead, e.g.:

```sh
cloudflared tunnel --url http://localhost:8080   # share the https URL + #room=...
```

The tunnel only carries the page and signaling (the app switches to `wss://` automatically); pattern
data still goes peer-to-peer.

## Known limits

- Chrome only (tested); 2 peers per room.
- No TURN: on some strict NATs the direct link fails; the status line then says so. Same Wi-Fi always works.
- VPNs can block the direct link (seen locally with a `utun` VPN interface): disconnect the VPN if stuck on "peer found, connecting…".
- Peers aren't in time with each other (no shared transport yet).
- If two people toggle the same cell at the same moment, the last write wins.
- No persistence: the pattern lives only in the open tabs.
