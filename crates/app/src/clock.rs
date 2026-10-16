//! Shared clock (design D2): NTP-style offset to the reference peer (lower Yrs client id).
//! Pongs carry the responder's *shared* time, so a peer that inherited the reference role keeps
//! its offset and nobody's timeline jumps.

const PING: u8 = 0x10;
const PONG: u8 = 0x11;
const KEEP_SAMPLES: usize = 8;

#[derive(Debug, PartialEq)]
pub enum ClockMsg {
    Ping { t0: f64, client_id: u64 },
    Pong { t0: f64, t1: f64 },
}

/// Data-channel messages with a tag ≥ 0x10 are clock messages; lower tags are Yrs sync.
pub fn is_clock(msg: &[u8]) -> bool {
    msg.first().is_some_and(|&t| t >= PING)
}

pub fn ping(t0: f64, client_id: u64) -> Vec<u8> {
    [&[PING][..], &t0.to_le_bytes(), &client_id.to_le_bytes()].concat()
}

pub fn pong(t0: f64, t1: f64) -> Vec<u8> {
    [&[PONG][..], &t0.to_le_bytes(), &t1.to_le_bytes()].concat()
}

/// Input comes from the network: malformed messages are errors, never panics.
pub fn decode(msg: &[u8]) -> Result<ClockMsg, String> {
    let word = |i: usize| -> Result<[u8; 8], String> {
        msg.get(1 + 8 * i..9 + 8 * i).and_then(|b| b.try_into().ok()).ok_or_else(|| "short clock message".to_string())
    };
    match msg.first() {
        Some(&PING) if msg.len() == 17 => Ok(ClockMsg::Ping { t0: f64::from_le_bytes(word(0)?), client_id: u64::from_le_bytes(word(1)?) }),
        Some(&PONG) if msg.len() == 17 => Ok(ClockMsg::Pong { t0: f64::from_le_bytes(word(0)?), t1: f64::from_le_bytes(word(1)?) }),
        _ => Err(format!("bad clock message ({} bytes)", msg.len())),
    }
}

pub struct ClockSync {
    my_id: u64,
    peer_id: Option<u64>,
    offset: f64,
    samples: Vec<(f64, f64)>, // (round trip, offset)
}

impl ClockSync {
    pub fn new(my_id: u64) -> Self {
        Self { my_id, peer_id: None, offset: 0.0, samples: Vec::new() }
    }

    pub fn is_reference(&self) -> bool {
        self.peer_id.is_none_or(|p| self.my_id < p)
    }

    pub fn offset(&self) -> f64 {
        self.offset
    }

    pub fn shared(&self, local_ms: f64) -> f64 {
        local_ms + self.offset
    }

    pub fn on_ping(&mut self, peer_id: u64) {
        if self.peer_id != Some(peer_id) {
            self.peer_id = Some(peer_id);
            self.samples.clear();
        }
    }

    /// `t0` our send time, `t1` the reference's shared time, `t3` our receive time.
    pub fn on_pong(&mut self, t0: f64, t1: f64, t3: f64) {
        if self.is_reference() || t3 < t0 {
            return;
        }
        self.samples.push((t3 - t0, t1 - (t0 + t3) / 2.0));
        if self.samples.len() > KEEP_SAMPLES {
            self.samples.remove(0);
        }
        let best = self.samples.iter().min_by(|a, b| a.0.total_cmp(&b.0)).unwrap();
        self.offset = best.1;
    }

    /// The peer is gone: we become the reference and keep our current offset (no jump).
    pub fn peer_left(&mut self) {
        self.peer_id = None;
        self.samples.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_from_symmetric_sample() {
        let mut c = ClockSync::new(2);
        c.on_ping(1); // peer 1 has the lower id → it is the reference
        assert!(!c.is_reference());
        c.on_pong(1000.0, 5050.0, 1100.0);
        assert_eq!(c.offset(), 4000.0);
        assert_eq!(c.shared(1100.0), 5100.0);
    }

    #[test]
    fn lowest_round_trip_wins_over_last_eight() {
        let mut c = ClockSync::new(2);
        c.on_ping(1);
        c.on_pong(0.0, 300.0, 300.0); // rtt 300 → offset 150
        c.on_pong(1000.0, 4010.0, 1020.0); // rtt 20 → offset 3000
        assert_eq!(c.offset(), 3000.0);
        for i in 0..8 {
            let t0 = 2000.0 + i as f64 * 100.0;
            c.on_pong(t0, t0 + 2000.0 + 25.0, t0 + 50.0); // rtt 50 → offset 2000
        }
        assert_eq!(c.offset(), 2000.0, "the rtt-20 sample aged out of the last 8");
    }

    #[test]
    fn reference_ignores_pongs_and_handover_keeps_offset() {
        let mut r = ClockSync::new(1);
        r.on_ping(2);
        assert!(r.is_reference());
        r.on_pong(0.0, 9999.0, 10.0);
        assert_eq!(r.offset(), 0.0);

        let mut c = ClockSync::new(2);
        c.on_ping(1);
        c.on_pong(1000.0, 5050.0, 1100.0);
        c.peer_left();
        assert!(c.is_reference());
        assert_eq!(c.offset(), 4000.0, "timeline does not jump when the reference leaves");
    }

    #[test]
    fn codec_round_trip_and_malformed() {
        assert_eq!(decode(&ping(12.5, 7)).unwrap(), ClockMsg::Ping { t0: 12.5, client_id: 7 });
        assert_eq!(decode(&pong(1.0, 2.0)).unwrap(), ClockMsg::Pong { t0: 1.0, t1: 2.0 });
        assert!(is_clock(&ping(0.0, 0)) && !is_clock(&[0x00, 1]) && !is_clock(&[]));
        assert!(decode(&[PING, 1, 2]).is_err());
        assert!(decode(&[0x12; 17]).is_err());
        assert!(decode(&[]).is_err());
    }
}
