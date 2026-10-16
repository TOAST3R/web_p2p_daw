//! Browser side: egui UI, AudioWorklet bootstrap, signaling WebSocket and the WebRTC data channel.
//! shortcut: event-handler closures are `forget()`-leaked (a few per connection); fine for an MVP.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use eframe::egui;
use js_sys::{Array, ArrayBuffer, Object, Reflect, Uint8Array};
use protocol::Signal;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    AudioContext, AudioWorkletNode, AudioWorkletNodeOptions, MessageEvent, Response, RtcConfiguration,
    RtcDataChannel, RtcDataChannelEvent, RtcDataChannelType, RtcIceCandidateInit, RtcIceConnectionState, RtcIceServer, RtcPeerConnection,
    RtcPeerConnectionIceEvent, RtcSdpType, RtcSessionDescriptionInit, WebSocket,
};

use crate::model::{Pattern, PatternDoc, STEPS, TRACKS};
use crate::sync;

const STUN: &str = "stun:stun.l.google.com:19302";
const TRACK_NAMES: [&str; TRACKS] = ["Kick", "Snare", "Hat", "Bass"];

type Slot<T> = Rc<RefCell<Option<T>>>;

fn log(msg: &str) {
    web_sys::console::log_1(&msg.into());
}

fn on<E: wasm_bindgen::convert::FromWasmAbi + 'static>(f: impl FnMut(E) + 'static) -> JsValue {
    Closure::<dyn FnMut(E)>::new(f).into_js_value()
}

struct Audio {
    node: AudioWorkletNode,
    last_sent: Option<Pattern>,
}

impl Audio {
    fn post(&self, msg: &Array) {
        if let Err(e) = self.node.port().and_then(|p| p.post_message(msg)) {
            log(&format!("worklet post failed: {e:?}"));
        }
    }

    /// Sends the pattern to the worklet only when it changed.
    fn push(&mut self, p: Pattern) {
        if self.last_sent == Some(p) {
            return;
        }
        let cells: Vec<u8> = p.cells.iter().flatten().map(|&c| c as u8).collect();
        self.post(&Array::of3(&"pattern".into(), &p.bpm.into(), &Uint8Array::from(&cells[..])));
        self.last_sent = Some(p);
    }

    fn set_playing(&self, playing: bool) {
        self.post(&Array::of2(&"play".into(), &(playing as u8).into()));
    }
}

/// Handles shared by the UI and the async/network callbacks.
#[derive(Clone)]
struct Ctx {
    doc: PatternDoc,
    egui: egui::Context,
    ws: WebSocket,
    pc: Slot<RtcPeerConnection>,
    dc: Slot<RtcDataChannel>, // set only while open; local updates are broadcast through it
    audio: Slot<Audio>,
    step: Rc<Cell<i32>>,
    status: Rc<RefCell<String>>,
}

impl Ctx {
    fn status(&self, s: &str) {
        log(&format!("status: {s}"));
        *self.status.borrow_mut() = s.to_owned();
        self.egui.request_repaint();
    }

    fn send(&self, s: Signal) {
        if let Err(e) = self.ws.send_with_str(&s.to_json()) {
            log(&format!("signaling send failed: {e:?}"));
        }
    }

    /// Remote changes also go straight to the engine, so a hidden tab (no repaints) still hears them.
    fn after_remote_change(&self) {
        if let Some(a) = self.audio.borrow_mut().as_mut() {
            a.push(self.doc.snapshot());
        }
        self.egui.request_repaint();
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
            .start(canvas, eframe::WebOptions::default(), Box::new(move |cc| Ok(Box::new(App::new(cc, room)))))
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

struct App {
    c: Ctx,
    room: String,
    playing: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, room: String) -> Self {
        let loc = web_sys::window().unwrap().location();
        let scheme = if loc.protocol().as_deref() == Ok("https:") { "wss" } else { "ws" };
        let ws = WebSocket::new(&format!("{scheme}://{}/ws?room={room}", loc.host().unwrap())).expect("websocket");
        let c = Ctx {
            doc: PatternDoc::default(),
            egui: cc.egui_ctx.clone(),
            ws,
            pc: Default::default(),
            dc: Default::default(),
            audio: Default::default(),
            step: Rc::new(Cell::new(-1)),
            status: Rc::new(RefCell::new("connecting to signaling…".into())),
        };

        let dc = c.dc.clone();
        c.doc
            .doc
            .observe_update_v1("net", move |txn, e| {
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

        Self { c, room, playing: false }
    }
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
            if let Some(pc) = ctx.pc.replace(None) {
                pc.close();
            }
            ctx.status("peer left — keep editing, changes merge when they rejoin");
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
            let _ = ch.send_with_u8_array(&sync::sync_step1(&c.doc.doc));
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
            match sync::handle(&c.doc.doc, &Uint8Array::new(&buf).to_vec()) {
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
            c.status("peer disconnected — waiting for them to rejoin");
        })
        .unchecked_ref(),
    ));
}

/// Must run after a user click: browsers only allow audio to start from a gesture.
async fn start_audio(ctx: Ctx) -> Result<(), JsValue> {
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

    let c = ctx.clone();
    node.port()?.set_onmessage(Some(
        on(move |e: MessageEvent| {
            c.step.set(e.data().as_f64().unwrap_or(-1.0) as i32);
            c.egui.request_repaint();
        })
        .unchecked_ref(),
    ));
    JsFuture::from(ac.resume()?).await?;

    let mut audio = Audio { node, last_sent: None };
    audio.push(ctx.doc.snapshot());
    ctx.audio.replace(Some(audio));
    std::mem::forget(ac); // lives for the page's lifetime
    ctx.egui.request_repaint();
    Ok(())
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let c = &self.c;
        let audio_ready = c.audio.borrow().is_some();
        egui::Frame::central_panel(ui.style()).show(ui, |ui| {
            ui.heading("P2P Sequencer");
            ui.label(format!("Room {} · {}", self.room, c.status.borrow()));
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                if !audio_ready {
                    if ui.button("▶ Start audio").clicked() {
                        let c = c.clone();
                        spawn_local(async move {
                            if let Err(e) = start_audio(c.clone()).await {
                                log(&format!("audio failed: {e:?}"));
                                c.status("audio failed to start (see console)");
                            }
                        });
                    }
                } else if ui.button(if self.playing { "■ Stop" } else { "▶ Play" }).clicked() {
                    self.playing = !self.playing;
                    if let Some(a) = c.audio.borrow().as_ref() {
                        a.set_playing(self.playing);
                    }
                }
                let mut bpm = c.doc.snapshot().bpm;
                if ui.add(egui::DragValue::new(&mut bpm).range(40.0..=240.0).suffix(" BPM")).changed() {
                    c.doc.set_bpm(bpm.round());
                }
            });
            ui.add_space(8.0);

            let p = c.doc.snapshot();
            let playhead = if self.playing { c.step.get() } else { -1 };
            egui::Grid::new("grid").spacing([4.0, 4.0]).show(ui, |ui| {
                for (t, name) in TRACK_NAMES.iter().enumerate() {
                    ui.label(*name);
                    for s in 0..STEPS {
                        let on = p.cells[t][s];
                        let (rect, resp) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::click());
                        let fill = match (on, s % 4 == 0) {
                            (true, _) => egui::Color32::from_rgb(255, 140, 40),
                            (false, true) => egui::Color32::from_gray(70),
                            (false, false) => egui::Color32::from_gray(50),
                        };
                        ui.painter().rect_filled(rect, 4.0, fill);
                        if s as i32 == playhead {
                            let stroke = egui::Stroke::new(2.0, egui::Color32::WHITE);
                            ui.painter().rect_stroke(rect, 4.0, stroke, egui::StrokeKind::Inside);
                        }
                        if resp.clicked() {
                            c.doc.set_cell(t, s, !on);
                        }
                    }
                    ui.end_row();
                }
            });
        });
        if let Some(a) = c.audio.borrow_mut().as_mut() {
            a.push(c.doc.snapshot());
        }
    }
}
