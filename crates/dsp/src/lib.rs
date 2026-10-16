//! Pure-Rust sequencer + drum synth. Runs natively (tests) and as a raw wasm module inside the AudioWorklet.

pub const TRACKS: usize = 4;
pub const STEPS: usize = 16;
const MAX_FRAMES: usize = 1024;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Pattern {
    pub bpm: f32,
    pub cells: [[bool; STEPS]; TRACKS],
}

impl Default for Pattern {
    fn default() -> Self {
        Self { bpm: 120.0, cells: [[false; STEPS]; TRACKS] }
    }
}

/// 16th-note step length.
pub fn samples_per_step(bpm: f32, sample_rate: f32) -> f64 {
    sample_rate as f64 * 60.0 / bpm as f64 / 4.0
}

#[derive(Clone, Copy)]
struct Voice {
    t: f32, // seconds since trigger; < 0 means idle
    phase: f32,
}

pub struct Engine {
    sr: f32,
    pub pattern: Pattern,
    playing: bool,
    next_step: usize,
    current_step: Option<usize>,
    until_next: f64, // samples until next step fires
    voices: [Voice; TRACKS],
    noise: u32,
    out: [f32; MAX_FRAMES],
}

impl Engine {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sr: sample_rate,
            pattern: Pattern::default(),
            playing: false,
            next_step: 0,
            current_step: None,
            until_next: 0.0,
            voices: [Voice { t: -1.0, phase: 0.0 }; TRACKS],
            noise: 0x1234_5678,
            out: [0.0; MAX_FRAMES],
        }
    }

    pub fn set_playing(&mut self, playing: bool) {
        if playing == self.playing {
            return;
        }
        self.playing = playing;
        if !playing {
            self.next_step = 0;
            self.current_step = None;
            self.until_next = 0.0;
        }
    }

    pub fn current_step(&self) -> Option<usize> {
        self.current_step
    }

    /// Advance one sample; returns the step triggered on this sample, if any.
    fn tick(&mut self) -> Option<usize> {
        if !self.playing {
            return None;
        }
        let mut fired = None;
        if self.until_next <= 0.0 {
            let step = self.next_step;
            for track in 0..TRACKS {
                if self.pattern.cells[track][step] {
                    self.voices[track] = Voice { t: 0.0, phase: 0.0 };
                }
            }
            self.current_step = Some(step);
            self.next_step = (step + 1) % STEPS;
            self.until_next += samples_per_step(self.pattern.bpm, self.sr);
            fired = Some(step);
        }
        self.until_next -= 1.0;
        fired
    }

    fn white(&mut self) -> f32 {
        // xorshift32
        self.noise ^= self.noise << 13;
        self.noise ^= self.noise >> 17;
        self.noise ^= self.noise << 5;
        self.noise as f32 / u32::MAX as f32 * 2.0 - 1.0
    }

    fn sample(&mut self) -> f32 {
        let dt = 1.0 / self.sr;
        let mut mix = 0.0;
        for track in 0..TRACKS {
            let Voice { t, phase } = self.voices[track];
            if t < 0.0 {
                continue;
            }
            let (s, len) = match track {
                // kick: sine sweeping 150 -> 45 Hz
                0 => {
                    let f = 45.0 + 105.0 * (-t * 30.0).exp();
                    self.voices[0].phase = phase + f * dt;
                    ((phase * std::f32::consts::TAU).sin() * (-t * 8.0).exp(), 0.5)
                }
                // snare: noise + 180 Hz body
                1 => {
                    let n = self.white();
                    self.voices[1].phase = phase + 180.0 * dt;
                    let body = (phase * std::f32::consts::TAU).sin() * 0.4;
                    ((n * 0.6 + body) * (-t * 20.0).exp(), 0.25)
                }
                // hat: short noise burst
                2 => (self.white() * 0.4 * (-t * 60.0).exp(), 0.1),
                // bass: 55 Hz square-ish blip
                _ => {
                    self.voices[3].phase = (phase + 55.0 * dt).fract();
                    let sq = if phase < 0.5 { 1.0 } else { -1.0 };
                    (sq * 0.3 * (-t * 6.0).exp(), 0.4)
                }
            };
            mix += s;
            let nt = t + dt;
            self.voices[track].t = if nt > len { -1.0 } else { nt };
        }
        mix * 0.5
    }

    /// Render `frames` mono samples into the internal buffer (clamped to its size).
    pub fn process(&mut self, frames: usize) -> &[f32] {
        let n = frames.min(MAX_FRAMES);
        for i in 0..n {
            self.tick();
            self.out[i] = self.sample();
        }
        &self.out[..n]
    }
}

/// Raw exports for the AudioWorklet (no wasm-bindgen; see crates/app/worklet.js).
#[cfg(target_arch = "wasm32")]
mod exports {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        static ENGINE: RefCell<Engine> = RefCell::new(Engine::new(48_000.0));
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn init(sample_rate: f32) {
        ENGINE.with(|e| *e.borrow_mut() = Engine::new(sample_rate));
    }

    /// Returns the current step, or -1 when stopped.
    #[unsafe(no_mangle)]
    pub extern "C" fn process(frames: usize) -> i32 {
        ENGINE.with(|e| {
            let mut e = e.borrow_mut();
            e.process(frames);
            e.current_step().map_or(-1, |s| s as i32)
        })
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn out_ptr() -> *const f32 {
        ENGINE.with(|e| e.borrow().out.as_ptr())
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn set_playing(on: i32) {
        ENGINE.with(|e| e.borrow_mut().set_playing(on != 0));
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn set_bpm(bpm: f32) {
        ENGINE.with(|e| e.borrow_mut().pattern.bpm = bpm.clamp(20.0, 400.0));
    }

    /// `index` = track * STEPS + step.
    #[unsafe(no_mangle)]
    pub extern "C" fn set_cell(index: usize, on: i32) {
        if index < TRACKS * STEPS {
            ENGINE.with(|e| e.borrow_mut().pattern.cells[index / STEPS][index % STEPS] = on != 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trigger_samples(e: &mut Engine, n: usize) -> Vec<(usize, usize)> {
        (0..n).filter_map(|i| e.tick().map(|s| (i, s))).collect()
    }

    #[test]
    fn step_is_6000_samples_at_120bpm_48k() {
        assert_eq!(samples_per_step(120.0, 48_000.0), 6000.0);
        let mut e = Engine::new(48_000.0);
        e.set_playing(true);
        assert_eq!(trigger_samples(&mut e, 12_001), vec![(0, 0), (6000, 1), (12_000, 2)]);
    }

    #[test]
    fn stop_resets_to_step_zero_and_silences() {
        let mut e = Engine::new(48_000.0);
        e.pattern.cells[0] = [true; STEPS];
        e.set_playing(true);
        trigger_samples(&mut e, 20_000);
        e.set_playing(false);
        assert_eq!(e.current_step(), None);
        assert!(trigger_samples(&mut e, 20_000).is_empty());
        for _ in 0..50 {
            e.process(1024); // ~1 s, longer than any voice
        }
        assert!(e.process(128).iter().all(|s| *s == 0.0), "voices decayed, no new notes");
        e.set_playing(true);
        assert_eq!(trigger_samples(&mut e, 1), vec![(0, 0)]);
    }

    #[test]
    fn pattern_change_applies_from_next_step_without_restart() {
        let mut e = Engine::new(48_000.0);
        e.set_playing(true);
        trigger_samples(&mut e, 6001); // steps 0, 1 fired
        e.pattern.cells[2][2] = true;
        assert_eq!(trigger_samples(&mut e, 6000), vec![(5999, 2)]); // timing unchanged
        assert!(e.voices[2].t >= 0.0, "hat triggered on step 2");
    }

    #[test]
    fn active_cell_makes_sound() {
        let mut e = Engine::new(48_000.0);
        e.pattern.cells[0][0] = true;
        e.set_playing(true);
        assert!(e.process(512).iter().any(|s| s.abs() > 0.01));
    }
}
