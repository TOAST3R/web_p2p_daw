//! Multi-track engine: schedules clip notes from the shared timeline and renders Synth / Drum Kit
//! instruments. Runs natively (tests) and as a raw wasm module inside the AudioWorklet.

pub mod state;

use state::{Clip, EngineState, Instrument, SynthParams};

const MAX_FRAMES: usize = 1024;
const SYNTH_VOICES: usize = 8;
/// Drum Kit note map: kick, snare, hat, bass.
pub const DRUM_NOTES: [u8; 4] = [36, 38, 42, 45];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Stage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Clone, Copy)]
struct SynthVoice {
    stage: Stage,
    level: f32,
    #[cfg_attr(not(test), allow(dead_code))] // inspected by tests
    pitch: u8,
    freq: f32,
    phase: f32,
    vel: f32,
    off_at: f64, // beat at which the note releases
    started: u64,
    ic1: f32, // state-variable filter state
    ic2: f32,
}

const IDLE_SYNTH: SynthVoice = SynthVoice {
    stage: Stage::Idle,
    level: 0.0,
    pitch: 0,
    freq: 0.0,
    phase: 0.0,
    vel: 0.0,
    off_at: 0.0,
    started: 0,
    ic1: 0.0,
    ic2: 0.0,
};

#[derive(Clone, Copy)]
struct DrumVoice {
    t: f32, // seconds since trigger; < 0 means idle
    phase: f32,
    vel: f32,
}

struct TrackVoices {
    id: u64,
    synth: [SynthVoice; SYNTH_VOICES],
    drums: [DrumVoice; 4],
    noise: u32,
    svf: (f32, f32, f32), // a1, a2, a3 for this block's cutoff/resonance
}

impl TrackVoices {
    fn new(id: u64) -> Self {
        Self {
            id,
            synth: [IDLE_SYNTH; SYNTH_VOICES],
            drums: [DrumVoice { t: -1.0, phase: 0.0, vel: 0.0 }; 4],
            noise: 0x1234_5678 ^ id as u32,
            svf: (0.0, 0.0, 0.0),
        }
    }

    fn note_on(&mut self, instrument: &Instrument, pitch: u8, vel: f32, off_at: f64, counter: u64) {
        match instrument {
            Instrument::DrumKit => {
                if let Some(i) = DRUM_NOTES.iter().position(|&n| n == pitch) {
                    self.drums[i] = DrumVoice { t: 0.0, phase: 0.0, vel };
                }
            }
            Instrument::Synth(_) => {
                let slot = self.synth.iter().position(|v| v.stage == Stage::Idle).unwrap_or_else(|| {
                    // all busy: steal the oldest
                    (0..SYNTH_VOICES).min_by_key(|&i| self.synth[i].started).unwrap()
                });
                let v = &mut self.synth[slot];
                *v = SynthVoice {
                    stage: Stage::Attack,
                    level: 0.0,
                    pitch,
                    freq: 440.0 * 2f32.powf((pitch as f32 - 69.0) / 12.0),
                    phase: 0.0,
                    vel,
                    off_at,
                    started: counter,
                    ..*v
                };
            }
        }
    }

    fn white(&mut self) -> f32 {
        // xorshift32
        self.noise ^= self.noise << 13;
        self.noise ^= self.noise >> 17;
        self.noise ^= self.noise << 5;
        self.noise as f32 / u32::MAX as f32 * 2.0 - 1.0
    }

    fn set_filter(&mut self, p: &SynthParams, sr: f32) {
        // Simper/Cytomic SVF coefficients
        let g = (std::f32::consts::PI * p.cutoff.clamp(20.0, sr * 0.45) / sr).tan();
        let k = 2.0 - 2.0 * p.resonance.clamp(0.0, 0.95);
        let a1 = 1.0 / (1.0 + g * (g + k));
        self.svf = (a1, g * a1, g * g * a1);
    }

    /// `beat` is None when the transport is stopped (everything releases).
    fn synth_sample(&mut self, p: &SynthParams, beat: Option<f64>, dt: f32) -> f32 {
        let (a1, a2, a3) = self.svf;
        let mut sum = 0.0;
        for v in self.synth.iter_mut() {
            if v.stage == Stage::Idle {
                continue;
            }
            if v.stage != Stage::Release && beat.is_none_or(|b| b >= v.off_at) {
                v.stage = Stage::Release;
            }
            match v.stage {
                Stage::Attack => {
                    v.level += dt / p.attack.max(0.001);
                    if v.level >= 1.0 {
                        v.level = 1.0;
                        v.stage = Stage::Decay;
                    }
                }
                Stage::Decay => {
                    v.level -= dt / p.decay.max(0.001) * (1.0 - p.sustain);
                    if v.level <= p.sustain {
                        v.level = p.sustain;
                        v.stage = Stage::Sustain;
                    }
                }
                Stage::Sustain => v.level = p.sustain,
                Stage::Release => {
                    v.level -= dt / p.release.max(0.001);
                    if v.level <= 0.0 {
                        v.level = 0.0;
                        v.stage = Stage::Idle;
                        continue;
                    }
                }
                Stage::Idle => {}
            }
            // shortcut: naive (aliasing) oscillators; switch to PolyBLEP if the highs sound harsh
            let raw = if p.square { if v.phase < 0.5 { 1.0 } else { -1.0 } } else { 2.0 * v.phase - 1.0 };
            v.phase = (v.phase + v.freq * dt).fract();
            let v3 = raw - v.ic2;
            let v1 = a1 * v.ic1 + a2 * v3;
            let v2 = v.ic2 + a2 * v.ic1 + a3 * v3;
            v.ic1 = 2.0 * v1 - v.ic1;
            v.ic2 = 2.0 * v2 - v.ic2;
            sum += v2 * v.level * v.vel * 0.25;
        }
        sum
    }

    fn drum_sample(&mut self, dt: f32) -> f32 {
        let mut mix = 0.0;
        for i in 0..4 {
            let DrumVoice { t, phase, vel } = self.drums[i];
            if t < 0.0 {
                continue;
            }
            let (s, len) = match i {
                // kick: sine sweeping 150 -> 45 Hz
                0 => {
                    let f = 45.0 + 105.0 * (-t * 30.0).exp();
                    self.drums[0].phase = phase + f * dt;
                    ((phase * std::f32::consts::TAU).sin() * (-t * 8.0).exp(), 0.5)
                }
                // snare: noise + 180 Hz body
                1 => {
                    let n = self.white();
                    self.drums[1].phase = phase + 180.0 * dt;
                    let body = (phase * std::f32::consts::TAU).sin() * 0.4;
                    ((n * 0.6 + body) * (-t * 20.0).exp(), 0.25)
                }
                // hat: short noise burst
                2 => (self.white() * 0.4 * (-t * 60.0).exp(), 0.1),
                // bass: 55 Hz square-ish blip
                _ => {
                    self.drums[3].phase = (phase + 55.0 * dt).fract();
                    let sq = if phase < 0.5 { 1.0 } else { -1.0 };
                    (sq * 0.3 * (-t * 6.0).exp(), 0.4)
                }
            };
            mix += s * vel * 0.5;
            let nt = t + dt;
            self.drums[i].t = if nt > len { -1.0 } else { nt };
        }
        mix
    }
}

/// Beat span of the block being rendered: start beat, beats per frame, frame count.
#[derive(Clone, Copy)]
struct Block {
    b0: f64,
    bpf: f64,
    n: usize,
}

struct Event {
    frame: usize,
    track: usize,
    #[cfg_attr(not(test), allow(dead_code))] // inspected by tests
    pitch: u8,
    vel: f32,
    off_at: f64,
}

pub struct Engine {
    sr: f64,
    state: EngineState,
    anchor: Option<(f64, f64)>, // (audio frame, shared ms) of the same instant at the speakers
    last_hi: Option<f64>,       // end beat of the previous block, so blocks tile without gaps
    voices: Vec<TrackVoices>,   // parallel to state.tracks
    counter: u64,
    events: Vec<Event>,
    out: [f32; MAX_FRAMES],
    #[cfg(test)]
    log: Vec<(u64, u8)>,
}

impl Engine {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sr: sample_rate as f64,
            state: EngineState::default(),
            anchor: None,
            last_hi: None,
            voices: Vec::new(),
            counter: 0,
            events: Vec::new(),
            out: [0.0; MAX_FRAMES],
            #[cfg(test)]
            log: Vec::new(),
        }
    }

    /// Replaces the state; voices already sounding on a track survive (matched by track id).
    pub fn load(&mut self, state: EngineState) {
        let mut old = std::mem::take(&mut self.voices);
        self.voices = state
            .tracks
            .iter()
            .map(|t| match old.iter().position(|v| v.id == t.id) {
                Some(i) => old.swap_remove(i),
                None => TrackVoices::new(t.id),
            })
            .collect();
        self.state = state;
    }

    /// Decodes and loads; on malformed bytes keeps the current state and returns false.
    pub fn load_bytes(&mut self, bytes: &[u8]) -> bool {
        match state::decode(bytes) {
            Ok(s) => {
                self.load(s);
                true
            }
            Err(_) => false,
        }
    }

    pub fn set_anchor(&mut self, frame: f64, shared_ms: f64) {
        self.anchor = Some((frame, shared_ms));
    }

    fn beat(&self, frame: f64) -> Option<f64> {
        let t = &self.state.transport;
        let (rf, rms) = self.anchor?;
        // (rms − anchor_ms) first: both are ~1e12 epoch ms, their difference is small and precise
        t.playing.then(|| t.anchor_beat + ((rms - t.anchor_ms) + (frame - rf) / self.sr * 1000.0) * t.bpm / 60_000.0)
    }

    fn collect(&mut self, track: usize, clip: &Clip, clip_at: f64, lo: f64, hi: f64, blk: Block) {
        let len = clip.length_beats;
        if lo >= hi || len < 1e-3 {
            return;
        }
        for note in clip.notes.iter().filter(|n| n.start < len) {
            let mut k = ((lo - clip_at - note.start) / len).ceil().max(0.0);
            loop {
                let t = clip_at + k * len + note.start;
                if t >= hi {
                    break;
                }
                if t >= lo {
                    let frame = ((t - blk.b0) / blk.bpf - 1e-9).ceil().clamp(0.0, (blk.n - 1) as f64) as usize;
                    self.events.push(Event { frame, track, pitch: note.pitch, vel: note.vel, off_at: t + note.len });
                }
                k += 1.0;
            }
        }
    }

    /// Renders `frames` mono samples starting at audio frame `current_frame`.
    pub fn process(&mut self, frames: usize, current_frame: f64) -> &[f32] {
        let n = frames.min(MAX_FRAMES);
        let span = match (self.beat(current_frame), self.beat(current_frame + n as f64)) {
            (Some(b0), Some(hi)) if hi > b0 => Some((b0, hi)),
            _ => None,
        };
        self.events.clear();
        let (b0, bpf) = match span {
            Some((b0, hi)) => {
                let lo = match self.last_hi {
                    Some(h) if (b0 - h).abs() < 0.1 => h,
                    _ => b0, // first block or a timeline jump (play, BPM change, re-anchor)
                };
                self.last_hi = Some(hi);
                let bpf = (hi - b0) / n as f64;
                let blk = Block { b0, bpf, n };
                let tracks = std::mem::take(&mut self.state.tracks);
                for (i, tr) in tracks.iter().enumerate().filter(|(_, t)| !t.mute) {
                    if let Some(c) = &tr.prev {
                        self.collect(i, c, tr.prev_at, lo.max(tr.prev_at), hi.min(tr.at_beat), blk);
                    }
                    if let Some(c) = &tr.now {
                        self.collect(i, c, tr.at_beat, lo.max(tr.at_beat), hi, blk);
                    }
                }
                self.state.tracks = tracks;
                self.events.sort_by_key(|e| e.frame);
                (b0, bpf)
            }
            None => {
                self.last_hi = None;
                (0.0, 0.0)
            }
        };

        let sr = self.sr as f32;
        for (v, tr) in self.voices.iter_mut().zip(&self.state.tracks) {
            if let Instrument::Synth(p) = &tr.instrument {
                v.set_filter(p, sr);
            }
        }
        let dt = 1.0 / sr;
        let mut next = 0;
        for i in 0..n {
            while let Some(e) = self.events.get(next).filter(|e| e.frame == i) {
                self.counter += 1;
                let tr = &self.state.tracks[e.track];
                self.voices[e.track].note_on(&tr.instrument, e.pitch, e.vel, e.off_at, self.counter);
                #[cfg(test)]
                self.log.push((current_frame as u64 + i as u64, e.pitch));
                next += 1;
            }
            let beat = span.map(|_| b0 + i as f64 * bpf);
            let mut mix = 0.0;
            for (v, tr) in self.voices.iter_mut().zip(&self.state.tracks) {
                let s = match &tr.instrument {
                    Instrument::Synth(p) => v.synth_sample(p, beat, dt),
                    Instrument::DrumKit => v.drum_sample(dt),
                };
                if !tr.mute {
                    mix += s * tr.volume;
                }
            }
            self.out[i] = mix.clamp(-1.0, 1.0);
        }
        &self.out[..n]
    }
}

/// Raw exports for the AudioWorklet (no wasm-bindgen; see crates/app/worklet.js).
#[cfg(all(target_arch = "wasm32", feature = "worklet"))]
mod exports {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        static ENGINE: RefCell<Engine> = RefCell::new(Engine::new(48_000.0));
        static INBOX: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn init(sample_rate: f32) {
        ENGINE.with(|e| *e.borrow_mut() = Engine::new(sample_rate));
    }

    /// Returns a buffer of `len` bytes for the next `load_state` call.
    #[unsafe(no_mangle)]
    pub extern "C" fn alloc(len: usize) -> *mut u8 {
        INBOX.with(|b| {
            let mut b = b.borrow_mut();
            b.resize(len, 0);
            b.as_mut_ptr()
        })
    }

    /// Loads the state written into the `alloc` buffer. Returns 0 on success, -1 on malformed bytes.
    #[unsafe(no_mangle)]
    pub extern "C" fn load_state(len: usize) -> i32 {
        INBOX.with(|b| {
            let b = b.borrow();
            let ok = ENGINE.with(|e| e.borrow_mut().load_bytes(&b[..len.min(b.len())]));
            if ok { 0 } else { -1 }
        })
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn set_anchor(frame: f64, shared_ms: f64) {
        ENGINE.with(|e| e.borrow_mut().set_anchor(frame, shared_ms));
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn process(frames: usize, current_frame: f64) {
        ENGINE.with(|e| {
            e.borrow_mut().process(frames, current_frame);
        });
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn out_ptr() -> *const f32 {
        ENGINE.with(|e| e.borrow().out.as_ptr())
    }
}

#[cfg(test)]
mod tests {
    use super::state::*;
    use super::*;

    const SR: f32 = 48_000.0;

    fn playing(anchor_ms: f64) -> Transport {
        Transport { playing: true, anchor_ms, anchor_beat: 0.0, bpm: 120.0 }
    }

    fn clip(notes: &[(u8, f64, f64)]) -> Clip {
        Clip { length_beats: 4.0, notes: notes.iter().map(|&(pitch, start, len)| Note { pitch, start, len, vel: 0.8 }).collect() }
    }

    fn track(instrument: Instrument, now: Option<Clip>, at_beat: f64, prev: Option<Clip>) -> TrackState {
        TrackState { id: 1, instrument, volume: 0.8, mute: false, now, at_beat, prev, prev_at: 0.0 }
    }

    fn engine(transport: Transport, tracks: Vec<TrackState>) -> Engine {
        let mut e = Engine::new(SR);
        e.load(EngineState { transport, tracks });
        e.set_anchor(0.0, 0.0);
        e
    }

    fn run(e: &mut Engine, until_frame: u64) {
        let mut f = 0;
        while f < until_frame {
            e.process(128, f as f64);
            f += 128;
        }
    }

    #[test]
    fn loop_notes_land_on_exact_frames() {
        let mut e = engine(playing(0.0), vec![track(Instrument::DrumKit, Some(clip(&[(36, 0.0, 0.25)])), 0.0, None)]);
        run(&mut e, 192_128);
        assert_eq!(e.log, vec![(0, 36), (96_000, 36), (192_000, 36)]);
    }

    #[test]
    fn launch_switches_clip_on_the_bar() {
        let a = clip(&[(36, 0.0, 0.25)]);
        let b = clip(&[(38, 0.0, 0.25)]);
        let mut e = engine(playing(0.0), vec![track(Instrument::DrumKit, Some(b), 8.0, Some(a))]);
        run(&mut e, 288_128);
        assert_eq!(e.log, vec![(0, 36), (96_000, 36), (192_000, 38), (288_000, 38)]);
    }

    #[test]
    fn play_anchor_in_future_and_stopped_transport() {
        let c = || Some(clip(&[(36, 0.0, 0.25)]));
        let mut e = engine(playing(1000.0), vec![track(Instrument::DrumKit, c(), 0.0, None)]);
        run(&mut e, 48_128);
        assert_eq!(e.log, vec![(48_000, 36)], "beat 0 is 1000 ms after the anchor");

        let stopped = Transport { playing: false, ..playing(0.0) };
        let mut e = engine(stopped, vec![track(Instrument::DrumKit, c(), 0.0, None)]);
        run(&mut e, 96_128);
        assert!(e.log.is_empty());
    }

    #[test]
    fn nine_notes_steal_the_oldest_voice() {
        let notes: Vec<_> = (60..69).map(|p| (p, 0.0, 4.0)).collect();
        let mut e = engine(playing(0.0), vec![track(Instrument::Synth(SynthParams::default()), Some(clip(&notes)), 0.0, None)]);
        e.process(128, 0.0);
        let pitches: Vec<u8> = e.voices[0].synth.iter().filter(|v| v.stage != Stage::Idle).map(|v| v.pitch).collect();
        assert_eq!(pitches.len(), 8);
        assert!(!pitches.contains(&60), "first (oldest) note was stolen");
        assert!(pitches.contains(&68));
    }

    #[test]
    fn note_off_releases_then_goes_idle() {
        let p = SynthParams { release: 0.1, ..SynthParams::default() };
        let mut e = engine(playing(0.0), vec![track(Instrument::Synth(p), Some(clip(&[(60, 0.0, 0.25)])), 0.0, None)]);
        run(&mut e, 128);
        assert_ne!(e.voices[0].synth[0].stage, Stage::Idle);
        run(&mut e, 6000 + 4800 + 1024); // note is 0.25 beat = 6000 frames, release 0.1 s = 4800 frames
        // run() restarts at frame 0, so the note retriggers; check the first voice finished meanwhile
        assert!(e.voices[0].synth.iter().any(|v| v.stage == Stage::Idle));
        assert!(e.process(128, 30_000.0).iter().all(|s| s.abs() < 1e-6), "silent between notes");
    }

    #[test]
    fn sounding_note_survives_state_reload() {
        let t = track(Instrument::Synth(SynthParams::default()), Some(clip(&[(60, 0.0, 4.0)])), 0.0, None);
        let mut e = engine(playing(0.0), vec![t.clone()]);
        run(&mut e, 4096);
        let level = e.voices[0].synth[0].level;
        let mut t2 = t;
        t2.volume = 0.5;
        e.load(EngineState { transport: playing(0.0), tracks: vec![t2] });
        assert_eq!(e.voices[0].synth[0].level, level);
        assert_ne!(e.voices[0].synth[0].stage, Stage::Idle);
    }

    #[test]
    fn malformed_state_keeps_previous() {
        let mut e = engine(playing(0.0), vec![track(Instrument::DrumKit, Some(clip(&[(36, 0.0, 0.25)])), 0.0, None)]);
        assert!(!e.load_bytes(&[0xff, 0xff, 0xff]));
        run(&mut e, 128);
        assert_eq!(e.log, vec![(0, 36)], "still playing the old state");
        assert!(e.load_bytes(&encode(&EngineState::default())));
    }

    #[test]
    fn muted_track_is_silent() {
        let mut t = track(Instrument::DrumKit, Some(clip(&[(36, 0.0, 0.25)])), 0.0, None);
        t.mute = true;
        let mut e = engine(playing(0.0), vec![t]);
        assert!(e.process(512, 0.0).iter().all(|s| *s == 0.0));
    }
}
