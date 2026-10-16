//! Browser side: AudioWorklet bootstrap, shared clock wiring, signaling WebSocket and the WebRTC data channel.
//! The egui UI lives in `ui.rs`.
//! shortcut: event-handler closures are `forget()`-leaked (a few per connection); fine for an MVP.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use eframe::egui;
use js_sys::{Array, ArrayBuffer, Function, Object, Reflect, Uint8Array};
use protocol::Signal;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    AudioContext, AudioWorkletNode, AudioWorkletNodeOptions, MessageEvent, Response, RtcConfiguration, RtcDataChannel,
    RtcDataChannelEvent, RtcDataChannelType, RtcIceCandidateInit, RtcIceConnectionState, RtcIceServer, RtcPeerConnection,
    RtcPeerConnectionIceEvent, RtcSdpType, RtcSessionDescriptionInit, WebSocket,
};

use crate::clock::{self, ClockMsg, ClockSync};
use crate::model::{Project, ProjectView};
use crate::sync;

const STUN: &str = "stun:stun.l.google.com:19302";
const TICK_MS: i32 = 100;
const TICKS_PER_SECOND: u32 = 10;

type Slot<T> = Rc<RefCell<Option<T>>>;

pub(crate) fn log(msg: &str) {
    web_sys::console::log_1(&msg.into());
}

fn on<E: wasm_bindgen::convert::FromWasmAbi + 'static>(f: impl FnMut(E) + 'static) -> JsValue {
    Closure::<dyn FnMut(E)>::new(f).into_js_value()
}

/// Local clock in ms, near-epoch so offsets between laptops stay small and readable.
pub(crate) fn local_now() -> f64 {
    let p = web_sys::window().unwrap().performance().unwrap();
    p.time_origin() + p.now()
}

pub(crate) struct Audio {
    ctx: AudioContext,
    node: AudioWorkletNode,
    last_state: Vec<u8>,
}

impl Audio {
    fn post(&self, msg: &Array) {
        if let Err(e) = self.node.port().and_then(|p| p.post_message(msg)) {
            log(&format!("worklet post failed: {e:?}"));
        }
    }

    /// Sends engine state bytes only when they changed.
    fn push_state(&mut self, bytes: Vec<u8>) {
        if bytes != self.last_state {
            self.post(&Array::of2(&"state".into(), &Uint8Array::from(&bytes[..])));
            self.last_state = bytes;
        }
    }

    /// Maps an audio frame to shared time at the speakers (design D6). Positive nudge = hear it later.
    fn post_anchor(&self, offset: f64, nudge_ms: f64) {
        let ts = Reflect::get(&self.ctx, &"getOutputTimestamp".into())
            .ok()
            .and_then(|f| f.dyn_into::<Function>().ok())
            .and_then(|f| f.call0(&self.ctx).ok());
        let Some(ts) = ts else { return log("getOutputTimestamp unavailable") };
        let get = |k: &str| Reflect::get(&ts, &k.into()).ok().and_then(|v| v.as_f64()).unwrap_or(0.0);
        let (context_time, performance_time) = (get("contextTime"), get("performanceTime"));
        if context_time <= 0.0 {
            return; // output not running yet
        }
        let origin = web_sys::window().unwrap().performance().unwrap().time_origin();
        let frame = context_time * self.ctx.sample_rate() as f64;
        let shared_ms = origin + performance_time + offset - nudge_ms;
        self.post(&Array::of3(&"anchor".into(), &frame.into(), &shared_ms.into()));
    }
}

/// Handles shared by the UI and the async/network callbacks.
#[derive(Clone)]
pub(crate) struct Ctx {
    pub project: Rc<Project>,
    pub view: Rc<RefCell<ProjectView>>,
    pub egui: egui::Context,
    ws: WebSocket,
    pc: Slot<RtcPeerConnection>,
    dc: Slot<RtcDataChannel>, // set only while open; local updates are broadcast through it
    pub audio: Slot<Audio>,
    pub clock: Rc<RefCell<ClockSync>>,
    pub status: Rc<RefCell<String>>,
    dirty: Rc<Cell<bool>>,
    fast_pings: Rc<Cell<u32>>,
    pub nudge_ms: Rc<Cell<f64>>,
}

impl Ctx {
    fn new(egui: egui::Context, ws: WebSocket) -> Self {
        let project = Rc::new(Project::default());
        Self {
            clock: Rc::new(RefCell::new(ClockSync::new(project.client_id()))),
            view: Rc::new(RefCell::new(project.view())),
            project,
            egui,
            ws,
            pc: Default::default(),
            dc: Default::default(),
            audio: Default::default(),
            status: Rc::new(RefCell::new("connecting to signaling…".into())),
            dirty: Rc::new(Cell::new(true)),
            fast_pings: Rc::new(Cell::new(0)),
            nudge_ms: Rc::new(Cell::new(0.0)),
        }
    }

    pub fn status(&self, s: &str) {
        log(&format!("status: {s}"));
        *self.status.borrow_mut() = s.to_owned();
        self.egui.request_repaint();
    }

    pub fn shared_now(&self) -> f64 {
        self.clock.borrow().shared(local_now())
    }

    fn send(&self, s: Signal) {
        if let Err(e) = self.ws.send_with_str(&s.to_json()) {
            log(&format!("signaling send failed: {e:?}"));
        }
    }

    /// Rebuilds the view and pushes engine state if the document changed since the last flush.
    /// Returns whether anything changed.
    pub fn flush(&self) -> bool {
        if !self.dirty.replace(false) {
            return false;
        }
        let view = self.project.view();
        if let Some(a) = self.audio.borrow_mut().as_mut() {
            a.push_state(dsp::state::encode(&view.engine_state()));
        }
        *self.view.borrow_mut() = view;
        true
    }

    /// Remote changes are flushed right away so a hidden tab (no repaints) still hears them.
    fn after_remote_change(&self) {
        self.flush();
        self.egui.request_repaint();
    }

    pub fn post_anchor(&self) {
        if let Some(a) = self.audio.borrow().as_ref() {
            a.post_anchor(self.clock.borrow().offset(), self.nudge_ms.get());
        }
    }

    fn tick(&self, n: u32, last_offset: &Cell<f64>) {
        let dc = self.dc.borrow().clone();
        if let Some(dc) = dc {
            let fast = self.fast_pings.get();
            if fast > 0 || n.is_multiple_of(TICKS_PER_SECOND) {
                self.fast_pings.set(fast.saturating_sub(1));
                let _ = dc.send_with_u8_array(&clock::ping(local_now(), self.project.client_id()));
            }
        }
        let offset = self.clock.borrow().offset();
        if n.is_multiple_of(TICKS_PER_SECOND) || (offset - last_offset.get()).abs() > 0.5 {
            if (offset - last_offset.get()).abs() > 0.5 {
                log(&format!("clock: offset {offset:.1} ms to reference"));
            }
            last_offset.set(offset);
            self.post_anchor();
        }
    }
}

pub fn start() {
    let room = room_from_url();
    spawn_local(async move {
        let canvas = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id("canvas"))
            .and_then(|c| c.dyn_into().ok())
            .expect("index.html has <canvas id=canvas>");
        let created = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(move |cc| Ok(Box::new(crate::ui::App::new(connect(cc.egui_ctx.clone(), &room), room)))),
            )
            .await;
        if let Err(e) = created {
            log(&format!("eframe failed to start: {e:?}"));
        }
    });
}

/// `#room=<id>` from the URL, or a fresh random id written back so the link can be shared.
fn room_from_url() -> String {
    let loc = web_sys::window().unwrap().location();
    let hash = loc.hash().unwrap_or_default();
    let room: String = hash
        .strip_prefix("#room=")
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect();
    if !room.is_empty() {
        return room;
    }
    let room = format!("{:08x}", (js_sys::Math::random() * u32::MAX as f64) as u32);
    let _ = loc.set_hash(&format!("room={room}"));
    room
}

/// Opens signaling, wires the doc observers and starts the 100 ms clock/anchor tick.
fn connect(egui: egui::Context, room: &str) -> Ctx {
    let loc = web_sys::window().unwrap().location();
    let scheme = if loc.protocol().as_deref() == Ok("https:") { "wss" } else { "ws" };
    let ws = WebSocket::new(&format!("{scheme}://{}/ws?room={room}", loc.host().unwrap())).expect("websocket");
    let c = Ctx::new(egui, ws);

    let (dc, dirty) = (c.dc.clone(), c.dirty.clone());
    c.project
        .doc
        .observe_update_v1("net", move |txn, e| {
            dirty.set(true); // local and remote: the engine state may have changed
            if sync::is_remote(txn) {
                return;
            }
            if let Some(dc) = dc.borrow().as_ref() {
                let _ = dc.send_with_u8_array(&sync::update(&e.update));
            }
        })
        .expect("observer registers on a fresh doc");

    let ctx = c.clone();
    c.ws.set_onmessage(Some(
        on(move |e: MessageEvent| match e.data().as_string().map(|t| serde_json::from_str(&t)) {
            Some(Ok(sig)) => on_signal(&ctx, sig),
            other => log(&format!("bad signaling message: {other:?}")),
        })
        .unchecked_ref(),
    ));
    let ctx = c.clone();
    c.ws.set_onclose(Some(on(move |_: web_sys::Event| ctx.status("signaling disconnected — reload")).unchecked_ref()));

    let (ctx, n, last_offset) = (c.clone(), Cell::new(0u32), Cell::new(0.0));
    let tick = Closure::<dyn FnMut()>::new(move || {
        n.set(n.get().wrapping_add(1));
        ctx.tick(n.get(), &last_offset);
    });
    web_sys::window()
        .unwrap()
        .set_interval_with_callback_and_timeout_and_arguments_0(tick.as_ref().unchecked_ref(), TICK_MS)
        .expect("setInterval");
    tick.forget();
    c
}

fn on_signal(ctx: &Ctx, sig: Signal) {
    match sig {
        Signal::Joined { peer_count: 1 } => ctx.status("waiting for a peer — share this page's URL"),
        Signal::Joined { .. } => {
            ctx.status("peer found, connecting…");
            let ctx = ctx.clone();
            spawn_local(async move {
                if let Err(e) = make_offer(&ctx).await {
                    log(&format!("offer failed: {e:?}"));
                }
            });
        }
        Signal::PeerJoined => ctx.status("peer joining…"),
        Signal::Offer { sdp } => {
            let pc = new_peer(ctx);
            // Called synchronously so ICE candidates arriving next are queued behind it by the browser.
            let set_remote = pc.set_remote_description(&desc(RtcSdpType::Offer, &sdp));
            let ctx = ctx.clone();
            spawn_local(async move {
                if let Err(e) = answer(&ctx, &pc, set_remote).await {
                    log(&format!("answer failed: {e:?}"));
                }
            });
        }
        Signal::Answer { sdp } => {
            if let Some(pc) = ctx.pc.borrow().as_ref() {
                spawn_local(log_err(pc.set_remote_description(&desc(RtcSdpType::Answer, &sdp))));
            }
        }
        Signal::Ice { candidate, sdp_mid, sdp_m_line_index } => {
            if let Some(pc) = ctx.pc.borrow().as_ref() {
                let init = RtcIceCandidateInit::new(&candidate);
                init.set_sdp_mid(sdp_mid.as_deref());
                init.set_sdp_m_line_index(sdp_m_line_index);
                spawn_local(log_err(pc.add_ice_candidate_with_opt_rtc_ice_candidate_init(Some(&init))));
            }
        }
        Signal::PeerLeft => {
            ctx.dc.replace(None);
            ctx.clock.borrow_mut().peer_left();
            if let Some(pc) = ctx.pc.replace(None) {
                pc.close();
            }
            ctx.status("peer left — keep jamming, changes merge when they rejoin");
        }
        Signal::RoomFull => ctx.status("room is full (max 2 peers)"),
    }
}

async fn log_err(p: js_sys::Promise) {
    if let Err(e) = JsFuture::from(p).await {
        log(&format!("webrtc: {e:?}"));
    }
}

fn desc(kind: RtcSdpType, sdp: &str) -> RtcSessionDescriptionInit {
    let d = RtcSessionDescriptionInit::new(kind);
    d.set_sdp(sdp);
    d
}

fn sdp_of(v: &JsValue) -> Result<String, JsValue> {
    Reflect::get(v, &"sdp".into())?.as_string().ok_or_else(|| "missing sdp".into())
}

/// The second peer to join makes the offer and creates the data channel.
async fn make_offer(ctx: &Ctx) -> Result<(), JsValue> {
    let pc = new_peer(ctx);
    setup_channel(ctx, pc.create_data_channel("yrs"));
    let sdp = sdp_of(&JsFuture::from(pc.create_offer()).await?)?;
    // Send before setLocalDescription: our ICE candidates start after it, so they can't overtake the offer.
    ctx.send(Signal::Offer { sdp: sdp.clone() });
    JsFuture::from(pc.set_local_description(&desc(RtcSdpType::Offer, &sdp))).await?;
    Ok(())
}

async fn answer(ctx: &Ctx, pc: &RtcPeerConnection, set_remote: js_sys::Promise) -> Result<(), JsValue> {
    JsFuture::from(set_remote).await?;
    let sdp = sdp_of(&JsFuture::from(pc.create_answer()).await?)?;
    ctx.send(Signal::Answer { sdp: sdp.clone() });
    JsFuture::from(pc.set_local_description(&desc(RtcSdpType::Answer, &sdp))).await?;
    Ok(())
}

fn new_peer(ctx: &Ctx) -> RtcPeerConnection {
    let server = RtcIceServer::new();
    server.set_urls_str(STUN);
    let config = RtcConfiguration::new();
    config.set_ice_servers(&Array::of1(&server));
    let pc = RtcPeerConnection::new_with_configuration(&config).expect("RTCPeerConnection");

    let c = ctx.clone();
    pc.set_onicecandidate(Some(
        on(move |e: RtcPeerConnectionIceEvent| {
            if let Some(cand) = e.candidate().filter(|c| !c.candidate().is_empty()) {
                c.send(Signal::Ice {
                    candidate: cand.candidate(),
                    sdp_mid: cand.sdp_mid(),
                    sdp_m_line_index: cand.sdp_m_line_index(),
                });
            }
        })
        .unchecked_ref(),
    ));
    let (c, p) = (ctx.clone(), pc.clone());
    pc.set_oniceconnectionstatechange(Some(
        on(move |_: web_sys::Event| {
            let state = p.ice_connection_state();
            log(&format!("ice: {state:?}"));
            if state == RtcIceConnectionState::Failed {
                c.status("direct connection failed — strict NAT? (TURN not supported yet)");
            }
        })
        .unchecked_ref(),
    ));
    let c = ctx.clone();
    pc.set_ondatachannel(Some(on(move |e: RtcDataChannelEvent| setup_channel(&c, e.channel())).unchecked_ref()));

    if let Some(old) = ctx.pc.replace(Some(pc.clone())) {
        old.close();
    }
    pc
}

fn setup_channel(ctx: &Ctx, dc: RtcDataChannel) {
    dc.set_binary_type(RtcDataChannelType::Arraybuffer);

    let (c, ch) = (ctx.clone(), dc.clone());
    dc.set_onopen(Some(
        on(move |_: web_sys::Event| {
            c.dc.replace(Some(ch.clone()));
            let _ = ch.send_with_u8_array(&sync::sync_step1(&c.project.doc));
            c.fast_pings.set(5); // quick clock convergence, then once a second
            c.status("connected peer-to-peer");
        })
        .unchecked_ref(),
    ));

    let (c, ch) = (ctx.clone(), dc.clone());
    dc.set_onmessage(Some(
        on(move |e: MessageEvent| {
            let Ok(buf) = e.data().dyn_into::<ArrayBuffer>() else {
                return log("ignoring non-binary data channel message");
            };
            let bytes = Uint8Array::new(&buf).to_vec();
            if clock::is_clock(&bytes) {
                match clock::decode(&bytes) {
                    Ok(ClockMsg::Ping { t0, client_id }) => {
                        c.clock.borrow_mut().on_ping(client_id);
                        let _ = ch.send_with_u8_array(&clock::pong(t0, c.shared_now()));
                    }
                    Ok(ClockMsg::Pong { t0, t1 }) => c.clock.borrow_mut().on_pong(t0, t1, local_now()),
                    Err(e) => log(&e),
                }
                return;
            }
            match sync::handle(&c.project.doc, &bytes) {
                Ok(Some(reply)) => {
                    let _ = ch.send_with_u8_array(&reply);
                }
                Ok(None) => c.after_remote_change(),
                Err(e) => log(&format!("bad sync message: {e}")),
            }
        })
        .unchecked_ref(),
    ));

    let c = ctx.clone();
    dc.set_onclose(Some(
        on(move |_: web_sys::Event| {
            c.dc.replace(None);
            c.clock.borrow_mut().peer_left();
            c.status("peer disconnected — waiting for them to rejoin");
        })
        .unchecked_ref(),
    ));
}

/// Must run after a user click: browsers only allow audio to start from a gesture.
pub(crate) async fn start_audio(ctx: Ctx) -> Result<(), JsValue> {
    let ac = AudioContext::new()?;
    let window = web_sys::window().unwrap();
    let res: Response = JsFuture::from(window.fetch_with_str("dsp.wasm")).await?.dyn_into()?;
    if !res.ok() {
        return Err(format!("dsp.wasm: HTTP {}", res.status()).into());
    }
    let bytes = JsFuture::from(res.array_buffer()?).await?;
    JsFuture::from(ac.audio_worklet()?.add_module("worklet.js")?).await?;

    let processor_options = Object::new();
    Reflect::set(&processor_options, &"bytes".into(), &bytes)?;
    let opts = AudioWorkletNodeOptions::new();
    opts.set_processor_options(Some(&processor_options));
    let node = AudioWorkletNode::new_with_options(&ac, "dsp", &opts)?;
    node.connect_with_audio_node(&ac.destination())?;
    JsFuture::from(ac.resume()?).await?;

    ctx.audio.replace(Some(Audio { ctx: ac, node, last_state: Vec::new() }));
    ctx.dirty.set(true); // push the current state to the fresh engine
    ctx.flush();
    ctx.post_anchor();
    ctx.egui.request_repaint();
    Ok(())
}
