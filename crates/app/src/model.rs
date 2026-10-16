//! Shared pattern state, stored in a Yrs map so concurrent edits converge.
//! Layout: root map `pattern` with `bpm` (number) and `cell:{track}:{step}` (bool).

use yrs::{Any, Doc, Map, MapRef, Number, Out, ReadTxn, Transact};

pub const TRACKS: usize = 4;
pub const STEPS: usize = 16;
pub const DEFAULT_BPM: f64 = 120.0;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Pattern {
    pub bpm: f64,
    pub cells: [[bool; STEPS]; TRACKS],
}

#[derive(Clone)]
pub struct PatternDoc {
    pub doc: Doc,
    map: MapRef,
}

fn key(track: usize, step: usize) -> String {
    format!("cell:{track}:{step}")
}

impl Default for PatternDoc {
    fn default() -> Self {
        let doc = Doc::new();
        let map = doc.get_or_insert_map("pattern");
        Self { doc, map }
    }
}

impl PatternDoc {
    pub fn set_cell(&self, track: usize, step: usize, on: bool) {
        self.map.insert(&mut self.doc.transact_mut(), key(track, step), on);
    }

    pub fn set_bpm(&self, bpm: f64) {
        self.map.insert(&mut self.doc.transact_mut(), "bpm", bpm);
    }

    pub fn snapshot(&self) -> Pattern {
        let txn = self.doc.transact();
        let bpm = match self.map.get(&txn, "bpm") {
            Some(Out::Any(Any::Number(Number::Float(n)))) => n,
            Some(Out::Any(Any::Number(Number::Int(n)))) => n as f64,
            _ => DEFAULT_BPM, // defaults are never written, so two fresh peers can't conflict on them
        };
        let mut cells = [[false; STEPS]; TRACKS];
        for (t, row) in cells.iter_mut().enumerate() {
            for (s, cell) in row.iter_mut().enumerate() {
                *cell = is_true(&txn, &self.map, &key(t, s));
            }
        }
        Pattern { bpm, cells }
    }
}

fn is_true<T: ReadTxn>(txn: &T, map: &MapRef, key: &str) -> bool {
    matches!(map.get(txn, key), Some(Out::Any(Any::Bool(true))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_pattern() {
        let p = PatternDoc::default().snapshot();
        assert_eq!(p.bpm, 120.0);
        assert!(p.cells.iter().flatten().all(|c| !c));
    }

    #[test]
    fn set_and_read_back() {
        let d = PatternDoc::default();
        d.set_cell(2, 5, true);
        d.set_bpm(90.0);
        let p = d.snapshot();
        assert!(p.cells[2][5]);
        assert_eq!(p.bpm, 90.0);
        d.set_cell(2, 5, false);
        assert!(!d.snapshot().cells[2][5]);
    }
}
