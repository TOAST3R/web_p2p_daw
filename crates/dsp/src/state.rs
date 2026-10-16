//! Engine state: everything the audio thread needs, derived from the shared project and sent as postcard bytes.

use serde::{Deserialize, Serialize};

pub const BEATS_PER_BAR: f64 = 4.0;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct EngineState {
    pub transport: Transport,
    pub tracks: Vec<TrackState>,
}

/// Shared timeline: `beat = anchor_beat + (shared_ms − anchor_ms) · bpm / 60000` while playing.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Transport {
    pub playing: bool,
    pub anchor_ms: f64,
    pub anchor_beat: f64,
    pub bpm: f64,
}

impl Default for Transport {
    fn default() -> Self {
        Self { playing: false, anchor_ms: 0.0, anchor_beat: 0.0, bpm: 120.0 }
    }
}

impl Transport {
    pub fn beat_at(&self, shared_ms: f64) -> f64 {
        self.anchor_beat + (shared_ms - self.anchor_ms) * self.bpm / 60_000.0
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TrackState {
    pub id: u64,
    pub instrument: Instrument,
    pub volume: f32,
    pub mute: bool,
    /// Clip playing from `at_beat` on (None = stopped from `at_beat`).
    pub now: Option<Clip>,
    pub at_beat: f64,
    /// Clip playing before `at_beat`, launched at `prev_at`.
    pub prev: Option<Clip>,
    pub prev_at: f64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum Instrument {
    Synth(SynthParams),
    DrumKit,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct SynthParams {
    pub square: bool,
    pub cutoff: f32,    // Hz
    pub resonance: f32, // 0..1
    pub attack: f32,    // s
    pub decay: f32,     // s
    pub sustain: f32,   // 0..1
    pub release: f32,   // s
}

impl Default for SynthParams {
    fn default() -> Self {
        Self { square: false, cutoff: 2000.0, resonance: 0.3, attack: 0.005, decay: 0.2, sustain: 0.6, release: 0.3 }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Clip {
    pub length_beats: f64,
    pub notes: Vec<Note>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Note {
    pub pitch: u8,
    pub start: f64, // beats from clip start
    pub len: f64,   // beats
    pub vel: f32,   // 0..1
}

pub fn encode(state: &EngineState) -> Vec<u8> {
    postcard::to_allocvec(state).expect("engine state always serializes")
}

pub fn decode(bytes: &[u8]) -> Result<EngineState, postcard::Error> {
    postcard::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let s = EngineState {
            transport: Transport { playing: true, anchor_ms: 1.7e12, anchor_beat: 3.5, bpm: 97.0 },
            tracks: vec![TrackState {
                id: 42,
                instrument: Instrument::Synth(SynthParams::default()),
                volume: 0.8,
                mute: false,
                now: Some(Clip { length_beats: 4.0, notes: vec![Note { pitch: 60, start: 0.5, len: 0.25, vel: 0.9 }] }),
                at_beat: 8.0,
                prev: None,
                prev_at: 0.0,
            }],
        };
        assert_eq!(decode(&encode(&s)).unwrap(), s);
        assert!(decode(&[0xff, 0xff, 0x01]).is_err());
    }

    #[test]
    fn beat_math() {
        let t = Transport { playing: true, anchor_ms: 1000.0, anchor_beat: 2.0, bpm: 120.0 };
        assert_eq!(t.beat_at(1000.0), 2.0);
        assert_eq!(t.beat_at(1500.0), 3.0);
    }
}
