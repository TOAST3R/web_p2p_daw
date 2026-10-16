//! Shared project (design D1): Yrs root maps `transport`, `tracks` and `clips`, keyed by ids so
//! concurrent inserts/deletes never shift anything. Values that must change together (a launch,
//! a note) are stored as one `Any` map, so last-writer-wins can never mix fields of two writes.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;

use dsp::state::{BEATS_PER_BAR, Clip, EngineState, Instrument, Note, SynthParams, TrackState, Transport};
use yrs::{Any, Doc, Map, MapPrelim, MapRef, Number, Out, ReadTxn, Transact, TransactionMut};

pub const SCENES: usize = 8;
/// Play starts this far in the future so the other peer receives it before beat 0.
pub const PLAY_LEAD_MS: f64 = 300.0;
const DEFAULT_VOLUME: f64 = 0.8;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Synth,
    Drums,
}

/// Which clip a track plays: `clip` from beat `at`, `prev` (launched at `prev_at`) before that.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Launch {
    pub clip: Option<String>,
    pub at: f64,
    pub prev: Option<String>,
    pub prev_at: f64,
}

impl Launch {
    /// The clip audible at beat `b` and the beat it was launched at.
    pub fn effective(&self, b: f64) -> Option<(&str, f64)> {
        if b >= self.at {
            self.clip.as_deref().map(|c| (c, self.at))
        } else {
            self.prev.as_deref().map(|c| (c, self.prev_at))
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrackView {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub order: f64,
    pub volume: f64,
    pub mute: bool,
    pub params: SynthParams,
    pub slots: [Option<String>; SCENES],
    pub launch: Launch,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClipView {
    pub length_beats: f64,
    pub notes: Vec<(String, Note)>, // sorted by start, then id
}

/// Plain snapshot of the document for the UI and the engine.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ProjectView {
    pub transport: Transport,
    pub tracks: Vec<TrackView>, // sorted by (order, id)
    pub clips: HashMap<String, ClipView>,
}

pub fn next_bar(beat: f64) -> f64 {
    (beat / BEATS_PER_BAR).floor() * BEATS_PER_BAR + BEATS_PER_BAR
}

impl ProjectView {
    pub fn beat(&self, shared_ms: f64) -> Option<f64> {
        self.transport.playing.then(|| self.transport.beat_at(shared_ms))
    }

    pub fn track(&self, id: &str) -> Option<&TrackView> {
        self.tracks.iter().find(|t| t.id == id)
    }

    fn engine_clip(&self, id: Option<&String>) -> Option<Clip> {
        let c = self.clips.get(id?)?;
        let notes = c.notes.iter().map(|(_, n)| *n).filter(|n| n.start < c.length_beats).collect();
        Some(Clip { length_beats: c.length_beats, notes })
    }

    pub fn engine_state(&self) -> EngineState {
        let tracks = self
            .tracks
            .iter()
            .map(|t| TrackState {
                id: fnv1a(&t.id),
                instrument: match t.kind {
                    Kind::Synth => Instrument::Synth(t.params),
                    Kind::Drums => Instrument::DrumKit,
                },
                volume: t.volume as f32,
                mute: t.mute,
                now: self.engine_clip(t.launch.clip.as_ref()),
                at_beat: t.launch.at,
                prev: self.engine_clip(t.launch.prev.as_ref()),
                prev_at: t.launch.prev_at,
            })
            .collect();
        EngineState { transport: self.transport, tracks }
    }
}

fn fnv1a(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
}

// ---- reading helpers ----

fn any_f64(a: &Any) -> Option<f64> {
    match a {
        Any::Number(Number::Float(x)) => Some(*x),
        Any::Number(Number::Int(i)) => Some(*i as f64),
        _ => None,
    }
}

fn any_str(a: &Any) -> Option<String> {
    match a {
        Any::String(s) if !s.is_empty() => Some(s.to_string()),
        _ => None,
    }
}

fn get_any<T: ReadTxn>(m: &MapRef, txn: &T, k: &str) -> Option<Any> {
    match m.get(txn, k)? {
        Out::Any(a) => Some(a),
        _ => None,
    }
}

fn get_f64<T: ReadTxn>(m: &MapRef, txn: &T, k: &str) -> Option<f64> {
    any_f64(&get_any(m, txn, k)?)
}

fn get_str<T: ReadTxn>(m: &MapRef, txn: &T, k: &str) -> Option<String> {
    any_str(&get_any(m, txn, k)?)
}

fn get_bool<T: ReadTxn>(m: &MapRef, txn: &T, k: &str) -> Option<bool> {
    match get_any(m, txn, k)? {
        Any::Bool(b) => Some(b),
        _ => None,
    }
}

fn get_map<T: ReadTxn>(m: &MapRef, txn: &T, k: &str) -> Option<MapRef> {
    match m.get(txn, k)? {
        Out::YMap(m) => Some(m),
        _ => None,
    }
}

fn field<'a>(m: &'a HashMap<String, Any>, k: &str) -> Option<&'a Any> {
    m.get(k)
}

fn obj(fields: &[(&str, Any)]) -> Any {
    Any::Map(Arc::new(fields.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()))
}

fn num(x: f64) -> Any {
    Any::Number(Number::Float(x))
}

fn text(s: Option<&str>) -> Any {
    Any::String(Arc::from(s.unwrap_or("")))
}

fn launch_to_any(l: &Launch) -> Any {
    obj(&[("clip", text(l.clip.as_deref())), ("at", num(l.at)), ("prev", text(l.prev.as_deref())), ("prev_at", num(l.prev_at))])
}

fn launch_from_any(a: Option<Any>) -> Launch {
    let Some(Any::Map(m)) = a else { return Launch::default() };
    let f = |k| field(&m, k).and_then(any_f64).unwrap_or(0.0);
    let s = |k| field(&m, k).and_then(any_str);
    Launch { clip: s("clip"), at: f("at"), prev: s("prev"), prev_at: f("prev_at") }
}

fn note_to_any(n: &Note) -> Any {
    obj(&[("p", num(n.pitch as f64)), ("s", num(n.start)), ("l", num(n.len)), ("v", num(n.vel as f64))])
}

fn note_from_any(a: &Any) -> Option<Note> {
    let Any::Map(m) = a else { return None };
    let f = |k| field(m, k).and_then(any_f64);
    Some(Note { pitch: f("p")?.clamp(0.0, 127.0) as u8, start: f("s")?, len: f("l")?, vel: f("v")? as f32 })
}

fn read_params<T: ReadTxn>(m: Option<MapRef>, txn: &T) -> SynthParams {
    let d = SynthParams::default();
    let Some(m) = m else { return d };
    let f = |k, def: f32| get_f64(&m, txn, k).map_or(def, |x| x as f32);
    SynthParams {
        square: get_str(&m, txn, "wave").as_deref() == Some("square"),
        cutoff: f("cutoff", d.cutoff),
        resonance: f("resonance", d.resonance),
        attack: f("attack", d.attack),
        decay: f("decay", d.decay),
        sustain: f("sustain", d.sustain),
        release: f("release", d.release),
    }
}

/// Shared project document plus the local id counter.
pub struct Project {
    pub doc: Doc,
    transport: MapRef,
    tracks: MapRef,
    clips: MapRef,
    counter: Cell<u64>,
}

impl Default for Project {
    fn default() -> Self {
        let doc = Doc::new();
        let transport = doc.get_or_insert_map("transport");
        let tracks = doc.get_or_insert_map("tracks");
        let clips = doc.get_or_insert_map("clips");
        Self { doc, transport, tracks, clips, counter: Cell::new(0) }
    }
}

impl Project {
    pub fn client_id(&self) -> u64 {
        self.doc.client_id().get()
    }

    /// `{client_id}-{counter}`: unique across peers without coordination.
    fn new_id(&self) -> String {
        let n = self.counter.get() + 1;
        self.counter.set(n);
        format!("{:x}-{n}", self.client_id())
    }

    fn read_transport<T: ReadTxn>(&self, txn: &T) -> Transport {
        let d = Transport::default();
        Transport {
            playing: get_bool(&self.transport, txn, "playing").unwrap_or(d.playing),
            anchor_ms: get_f64(&self.transport, txn, "anchor_ms").unwrap_or(d.anchor_ms),
            anchor_beat: get_f64(&self.transport, txn, "anchor_beat").unwrap_or(d.anchor_beat),
            bpm: get_f64(&self.transport, txn, "bpm").unwrap_or(d.bpm),
        }
    }

    fn read_track<T: ReadTxn>(&self, txn: &T, id: &str, m: &MapRef) -> TrackView {
        let mut slots: [Option<String>; SCENES] = Default::default();
        if let Some(s) = get_map(m, txn, "slots") {
            for (i, slot) in slots.iter_mut().enumerate() {
                *slot = get_str(&s, txn, &i.to_string());
            }
        }
        TrackView {
            id: id.to_string(),
            name: get_str(m, txn, "name").unwrap_or_default(),
            kind: if get_str(m, txn, "kind").as_deref() == Some("drums") { Kind::Drums } else { Kind::Synth },
            order: get_f64(m, txn, "order").unwrap_or(0.0),
            volume: get_f64(m, txn, "volume").unwrap_or(DEFAULT_VOLUME),
            mute: get_bool(m, txn, "mute").unwrap_or(false),
            params: read_params(get_map(m, txn, "params"), txn),
            slots,
            launch: launch_from_any(get_any(m, txn, "launch")),
        }
    }

    fn read_view<T: ReadTxn>(&self, txn: &T) -> ProjectView {
        let mut tracks: Vec<TrackView> = self
            .tracks
            .iter(txn)
            .filter_map(|(id, out)| match out {
                Out::YMap(m) => Some(self.read_track(txn, id, &m)),
                _ => None,
            })
            .collect();
        tracks.sort_by(|a, b| a.order.total_cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
        let clips = self
            .clips
            .iter(txn)
            .filter_map(|(id, out)| {
                let Out::YMap(m) = out else { return None };
                let mut notes: Vec<(String, Note)> = get_map(&m, txn, "notes")
                    .map(|n| {
                        n.iter(txn)
                            .filter_map(|(nid, v)| match v {
                                Out::Any(a) => note_from_any(&a).map(|note| (nid.to_string(), note)),
                                _ => None,
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                notes.sort_by(|a, b| a.1.start.total_cmp(&b.1.start).then_with(|| a.0.cmp(&b.0)));
                let length_beats = get_f64(&m, txn, "length_beats").unwrap_or(BEATS_PER_BAR);
                Some((id.to_string(), ClipView { length_beats, notes }))
            })
            .collect();
        ProjectView { transport: self.read_transport(txn), tracks, clips }
    }

    pub fn view(&self) -> ProjectView {
        self.read_view(&self.doc.transact())
    }

    // ---- tracks ----

    pub fn add_track(&self, kind: Kind) -> String {
        let id = self.new_id();
        let mut txn = self.doc.transact_mut();
        let view = self.read_view(&txn);
        let order = view.tracks.iter().map(|t| t.order).fold(0.0, f64::max) + 1.0;
        let label = match kind {
            Kind::Synth => "Synth",
            Kind::Drums => "Drums",
        };
        let t = self.tracks.insert(&mut txn, id.clone(), MapPrelim::default());
        t.insert(&mut txn, "name", format!("{label} {}", view.tracks.len() + 1));
        t.insert(&mut txn, "kind", if kind == Kind::Drums { "drums" } else { "synth" });
        t.insert(&mut txn, "order", order);
        t.insert(&mut txn, "volume", DEFAULT_VOLUME);
        t.insert(&mut txn, "mute", false);
        t.insert(&mut txn, "slots", MapPrelim::default());
        let p = t.insert(&mut txn, "params", MapPrelim::default());
        let d = SynthParams::default();
        p.insert(&mut txn, "wave", "saw");
        for (k, v) in [
            ("cutoff", d.cutoff),
            ("resonance", d.resonance),
            ("attack", d.attack),
            ("decay", d.decay),
            ("sustain", d.sustain),
            ("release", d.release),
        ] {
            p.insert(&mut txn, k, v as f64);
        }
        id
    }

    fn track_map<T: ReadTxn>(&self, txn: &T, id: &str) -> Option<MapRef> {
        get_map(&self.tracks, txn, id)
    }

    /// Removes the track and the clips in its slots in one transaction.
    pub fn delete_track(&self, id: &str) {
        let mut txn = self.doc.transact_mut();
        let Some(m) = self.track_map(&txn, id) else { return };
        let clips: Vec<String> = self.read_track(&txn, id, &m).slots.into_iter().flatten().collect();
        for c in clips {
            self.clips.remove(&mut txn, &c);
        }
        self.tracks.remove(&mut txn, id);
    }

    fn set_track_field(&self, id: &str, key: &str, value: Any) {
        let mut txn = self.doc.transact_mut();
        if let Some(m) = self.track_map(&txn, id) {
            m.insert(&mut txn, key, value);
        }
    }

    pub fn set_volume(&self, track: &str, volume: f64) {
        self.set_track_field(track, "volume", num(volume.clamp(0.0, 1.0)));
    }

    pub fn set_mute(&self, track: &str, mute: bool) {
        self.set_track_field(track, "mute", Any::Bool(mute));
    }

    pub fn set_param(&self, track: &str, key: &str, value: f64) {
        let mut txn = self.doc.transact_mut();
        if let Some(p) = self.track_map(&txn, track).and_then(|m| get_map(&m, &txn, "params")) {
            p.insert(&mut txn, key, value);
        }
    }

    pub fn set_wave(&self, track: &str, square: bool) {
        let mut txn = self.doc.transact_mut();
        if let Some(p) = self.track_map(&txn, track).and_then(|m| get_map(&m, &txn, "params")) {
            p.insert(&mut txn, "wave", if square { "square" } else { "saw" });
        }
    }

    // ---- clips and notes ----

    /// Creates an empty 1-bar clip in `slot` (replacing any clip there) and returns its id.
    pub fn create_clip(&self, track: &str, slot: usize) -> Option<String> {
        let id = self.new_id();
        let mut txn = self.doc.transact_mut();
        let slots = self.track_map(&txn, track).and_then(|m| get_map(&m, &txn, "slots"))?;
        if let Some(old) = get_str(&slots, &txn, &slot.to_string()) {
            self.clips.remove(&mut txn, &old);
        }
        let c = self.clips.insert(&mut txn, id.clone(), MapPrelim::default());
        c.insert(&mut txn, "length_beats", BEATS_PER_BAR);
        c.insert(&mut txn, "notes", MapPrelim::default());
        slots.insert(&mut txn, slot.to_string(), id.clone());
        Some(id)
    }

    pub fn set_clip_length(&self, clip: &str, beats: f64) {
        let mut txn = self.doc.transact_mut();
        if let Some(c) = get_map(&self.clips, &txn, clip) {
            c.insert(&mut txn, "length_beats", beats.max(BEATS_PER_BAR));
        }
    }

    fn notes_map<T: ReadTxn>(&self, txn: &T, clip: &str) -> Option<MapRef> {
        get_map(&self.clips, txn, clip).and_then(|c| get_map(&c, txn, "notes"))
    }

    pub fn add_note(&self, clip: &str, note: Note) -> Option<String> {
        let id = self.new_id();
        let mut txn = self.doc.transact_mut();
        self.notes_map(&txn, clip)?.insert(&mut txn, id.clone(), note_to_any(&note));
        Some(id)
    }

    pub fn set_note(&self, clip: &str, note_id: &str, note: Note) {
        let mut txn = self.doc.transact_mut();
        if let Some(n) = self.notes_map(&txn, clip) {
            n.insert(&mut txn, note_id, note_to_any(&note));
        }
    }

    pub fn delete_note(&self, clip: &str, note_id: &str) {
        let mut txn = self.doc.transact_mut();
        if let Some(n) = self.notes_map(&txn, clip) {
            n.remove(&mut txn, note_id);
        }
    }

    // ---- transport (design D3) ----

    fn play_in(&self, txn: &mut TransactionMut, now: f64) {
        self.transport.insert(txn, "playing", true);
        self.transport.insert(txn, "anchor_ms", now + PLAY_LEAD_MS);
        self.transport.insert(txn, "anchor_beat", 0.0);
    }

    pub fn play(&self, now: f64) {
        self.play_in(&mut self.doc.transact_mut(), now);
    }

    /// Stops the transport and clears every track's launch.
    pub fn stop(&self) {
        let mut txn = self.doc.transact_mut();
        self.transport.insert(&mut txn, "playing", false);
        let ids: Vec<String> = self.tracks.keys(&txn).map(str::to_string).collect();
        for id in ids {
            if let Some(m) = self.track_map(&txn, &id) {
                m.insert(&mut txn, "launch", launch_to_any(&Launch::default()));
            }
        }
    }

    /// Changes tempo; while playing, re-anchors at the current beat so the position never jumps.
    pub fn set_bpm(&self, bpm: f64, now: f64) {
        let mut txn = self.doc.transact_mut();
        let t = self.read_transport(&txn);
        if t.playing {
            self.transport.insert(&mut txn, "anchor_beat", t.beat_at(now));
            self.transport.insert(&mut txn, "anchor_ms", now);
        }
        self.transport.insert(&mut txn, "bpm", bpm.clamp(20.0, 400.0));
    }

    // ---- launching (design D4) ----

    /// Launches `clip` (or stops with None) on `track` at the next bar; starts the transport if stopped.
    fn launch_in(&self, txn: &mut TransactionMut, track: &str, clip: Option<&str>, now: f64) {
        let Some(m) = self.track_map(txn, track) else { return };
        let t = self.read_transport(txn);
        let (at, effective) = if t.playing {
            let beat = t.beat_at(now);
            let cur = launch_from_any(get_any(&m, txn, "launch"));
            (next_bar(beat), cur.effective(beat).map(|(c, a)| (c.to_string(), a)))
        } else if clip.is_some() {
            self.play_in(txn, now);
            (0.0, None)
        } else {
            return; // stopping a track while the transport is stopped: nothing to do
        };
        let launch = Launch {
            clip: clip.map(str::to_string),
            at,
            prev_at: effective.as_ref().map_or(0.0, |e| e.1),
            prev: effective.map(|e| e.0),
        };
        m.insert(txn, "launch", launch_to_any(&launch));
    }

    pub fn launch(&self, track: &str, clip: Option<&str>, now: f64) {
        self.launch_in(&mut self.doc.transact_mut(), track, clip, now);
    }

    /// Launches row `scene` on every track in one transaction; tracks with an empty slot stop.
    pub fn launch_scene(&self, scene: usize, now: f64) {
        let mut txn = self.doc.transact_mut();
        let view = self.read_view(&txn);
        let has_clip = view.tracks.iter().any(|t| t.slots[scene].is_some());
        if !has_clip && !view.transport.playing {
            return;
        }
        // launch tracks with a clip first so a stopped transport starts before the stops are written
        let mut tracks: Vec<&TrackView> = view.tracks.iter().collect();
        tracks.sort_by_key(|t| t.slots[scene].is_none());
        for t in tracks {
            self.launch_in(&mut txn, &t.id, t.slots[scene].as_deref(), now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync;

    fn sync_both(a: &Project, b: &Project) {
        let rb = sync::handle(&b.doc, &sync::sync_step1(&a.doc)).unwrap().unwrap();
        let ra = sync::handle(&a.doc, &sync::sync_step1(&b.doc)).unwrap().unwrap();
        sync::handle(&a.doc, &rb).unwrap();
        sync::handle(&b.doc, &ra).unwrap();
        assert_eq!(a.view(), b.view(), "converged");
    }

    fn note(start: f64) -> Note {
        Note { pitch: 60, start, len: 0.25, vel: 0.8 }
    }

    #[test]
    fn empty_project_defaults() {
        let v = Project::default().view();
        assert!(v.tracks.is_empty() && v.clips.is_empty());
        assert!(!v.transport.playing);
        assert_eq!(v.transport.bpm, 120.0);
    }

    #[test]
    fn add_track_defaults() {
        let p = Project::default();
        let id = p.add_track(Kind::Synth);
        let v = p.view();
        let t = v.track(&id).unwrap();
        assert_eq!((t.kind, t.volume, t.mute), (Kind::Synth, 0.8, false));
        assert_eq!(t.params, SynthParams::default());
        assert!(t.slots.iter().all(Option::is_none));
    }

    #[test]
    fn concurrent_track_adds_keep_both_in_the_same_order() {
        let (a, b) = (Project::default(), Project::default());
        a.add_track(Kind::Synth);
        b.add_track(Kind::Drums);
        sync_both(&a, &b);
        let v = a.view();
        assert_eq!(v.tracks.len(), 2);
        assert_eq!(v.tracks[0].order, v.tracks[1].order, "same order value, tie broken by id");
    }

    #[test]
    fn delete_track_removes_its_clips() {
        let p = Project::default();
        let t = p.add_track(Kind::Drums);
        p.create_clip(&t, 0).unwrap();
        p.create_clip(&t, 3).unwrap();
        assert_eq!(p.view().clips.len(), 2);
        p.delete_track(&t);
        let v = p.view();
        assert!(v.tracks.is_empty() && v.clips.is_empty());
    }

    #[test]
    fn transport_play_lead_bpm_reanchor_and_stop() {
        let p = Project::default();
        p.play(10_000.0);
        let t = p.view().transport;
        assert!(t.playing);
        assert_eq!((t.anchor_ms, t.anchor_beat), (10_300.0, 0.0));

        let now = 10_300.0 + 17.5 * 500.0; // beat 17.5 at 120 BPM
        p.set_bpm(90.0, now);
        let t = p.view().transport;
        assert_eq!(t.beat_at(now), 17.5, "position kept");
        assert_eq!(t.beat_at(now + 1000.0), 19.0, "advances at 90 BPM");

        let track = p.add_track(Kind::Synth);
        let clip = p.create_clip(&track, 0).unwrap();
        p.launch(&track, Some(&clip), now);
        p.stop();
        let v = p.view();
        assert!(!v.transport.playing);
        assert_eq!(v.track(&track).unwrap().launch, Launch::default());
    }

    #[test]
    fn launch_quantizes_to_next_bar_and_keeps_prev() {
        let p = Project::default();
        let t = p.add_track(Kind::Drums);
        let a = p.create_clip(&t, 0).unwrap();
        let b = p.create_clip(&t, 1).unwrap();

        p.launch(&t, Some(&a), 0.0); // stopped → starts transport, clip at beat 0
        let v = p.view();
        assert!(v.transport.playing);
        assert_eq!(v.track(&t).unwrap().launch.at, 0.0);

        let at_9_3 = PLAY_LEAD_MS + 9.3 * 500.0;
        p.launch(&t, Some(&b), at_9_3);
        let l = p.view().track(&t).unwrap().launch.clone();
        assert_eq!((l.clip.as_deref(), l.at), (Some(b.as_str()), 12.0));
        assert_eq!((l.prev.as_deref(), l.prev_at), (Some(a.as_str()), 0.0));
        assert_eq!(l.effective(11.9), Some((a.as_str(), 0.0)));
        assert_eq!(l.effective(12.0), Some((b.as_str(), 12.0)));

        p.launch(&t, None, PLAY_LEAD_MS + 13.0 * 500.0); // track stop at the next bar
        let l = p.view().track(&t).unwrap().launch.clone();
        assert_eq!((l.clip, l.at), (None, 16.0));
        assert_eq!(l.prev.as_deref(), Some(b.as_str()));
    }

    #[test]
    fn concurrent_launches_converge() {
        let (a, b) = (Project::default(), Project::default());
        let t = a.add_track(Kind::Drums);
        let c0 = a.create_clip(&t, 0).unwrap();
        let c1 = a.create_clip(&t, 1).unwrap();
        a.play(0.0);
        sync_both(&a, &b);
        a.launch(&t, Some(&c0), 2000.0);
        b.launch(&t, Some(&c1), 2000.0);
        sync_both(&a, &b);
        assert!(a.view().track(&t).unwrap().launch.clip.is_some());
    }

    #[test]
    fn scene_launch_stops_tracks_with_empty_slots() {
        let p = Project::default();
        let t1 = p.add_track(Kind::Drums);
        let t2 = p.add_track(Kind::Synth);
        let c1 = p.create_clip(&t1, 0).unwrap();
        let c2 = p.create_clip(&t2, 0).unwrap();
        let r1 = p.create_clip(&t1, 1).unwrap();
        p.launch_scene(0, 0.0);
        let v = p.view();
        assert_eq!(v.track(&t1).unwrap().launch.clip.as_deref(), Some(c1.as_str()));
        assert_eq!(v.track(&t2).unwrap().launch.clip.as_deref(), Some(c2.as_str()));

        p.launch_scene(1, PLAY_LEAD_MS + 5.0 * 500.0);
        let v = p.view();
        let (l1, l2) = (&v.track(&t1).unwrap().launch, &v.track(&t2).unwrap().launch);
        assert_eq!((l1.clip.as_deref(), l1.at), (Some(r1.as_str()), 8.0));
        assert_eq!((l2.clip.as_deref(), l2.at), (None, 8.0), "empty slot stops the track");
    }

    #[test]
    fn engine_state_drops_notes_beyond_clip_length() {
        let p = Project::default();
        let t = p.add_track(Kind::Synth);
        let c = p.create_clip(&t, 0).unwrap();
        p.set_clip_length(&c, 8.0);
        p.add_note(&c, note(1.0)).unwrap();
        p.add_note(&c, note(6.0)).unwrap();
        p.set_clip_length(&c, 4.0);
        p.launch(&t, Some(&c), 0.0);
        let v = p.view();
        assert_eq!(v.clips[&c].notes.len(), 2, "note kept in the clip");
        let s = v.engine_state();
        assert_eq!(s.tracks[0].now.as_ref().unwrap().notes, vec![note(1.0)], "but not played");
    }

    #[test]
    fn note_edits() {
        let p = Project::default();
        let t = p.add_track(Kind::Synth);
        let c = p.create_clip(&t, 0).unwrap();
        let n = p.add_note(&c, note(1.25)).unwrap();
        p.set_note(&c, &n, Note { start: 1.5, len: 0.5, ..note(0.0) });
        assert_eq!(p.view().clips[&c].notes[0].1, Note { start: 1.5, len: 0.5, ..note(0.0) });
        p.delete_note(&c, &n);
        assert!(p.view().clips[&c].notes.is_empty());
    }
}
