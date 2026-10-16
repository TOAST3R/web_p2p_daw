# P2P DAW

A browser DAW with an Ableton-style Session View where two laptops jam together live. Project edits go
**directly between browsers** (WebRTC data channel, Yrs CRDT), each laptop renders its own audio in an
AudioWorklet, and a shared clock keeps both laptops on the same beat. Clip launches are quantized to the
next bar, which hides network lag. Rust everywhere; the only hand-written JS is `crates/app/worklet.js`.

```
crates/dsp        engine: note scheduling from the shared timeline, Synth + Drum Kit (raw wasm for the AudioWorklet)
crates/app        egui Session View + piano roll, Yrs project model, shared clock, sync, WebRTC + audio bootstrap
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
second laptop, and click **Start audio** on each.

- **+ Synth / + Drums** adds a track. **Double-click** an empty slot to create a clip, **click** a clip to edit it below.
- Piano roll: click to add a note, drag to move, drag the right edge to resize, right-click to delete, drag in
  the strip under the grid to set velocity. Clip length and synth sound are set above the grid.
- **▶** on a clip launches it at the next bar (and starts the transport if stopped); the scene buttons on the
  left launch a whole row; **■** under a track stops it at the next bar. Play/Stop and BPM are shared.
- **Latency nudge** (the `ms` box): if your audio is early or late against your friend's, shift it here.
  It only affects you. Wired headphones keep this near 0; Bluetooth usually needs +150–250 ms.

Tests: `cargo test --workspace` (engine scheduling and voices, project model and launches, clock sync,
sync convergence, signaling rooms).

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
- If two people edit the same note or launch on the same track at the same moment, the last write wins.
- No undo, no persistence: the project lives only in the open tabs.
- Timing between laptops depends on the browser's reported output latency; use the nudge if you hear flams.
- No persistence: the pattern lives only in the open tabs.
