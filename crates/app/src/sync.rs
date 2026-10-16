//! Peer sync protocol over the data channel: 1-byte tag + payload.
//! Symmetric per connection: on open both sides send SyncStep1 (state vector), the reply
//! is SyncStep2 (missing diff), and after that each local change is sent as an Update.

use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Doc, Origin, ReadTxn, StateVector, Transact, TransactionMut, Update};

const SYNC_STEP1: u8 = 0;
const SYNC_STEP2: u8 = 1;
const UPDATE: u8 = 2;
const REMOTE: &str = "remote";

fn tagged(tag: u8, body: Vec<u8>) -> Vec<u8> {
    let mut m = Vec::with_capacity(body.len() + 1);
    m.push(tag);
    m.extend(body);
    m
}

pub fn sync_step1(doc: &Doc) -> Vec<u8> {
    tagged(SYNC_STEP1, doc.transact().state_vector().encode_v1())
}

pub fn update(update: &[u8]) -> Vec<u8> {
    tagged(UPDATE, update.to_vec())
}

/// True for transactions that applied a peer's update; those must not be re-broadcast.
pub fn is_remote(txn: &TransactionMut) -> bool {
    txn.origin() == Some(&Origin::from(REMOTE))
}

/// Applies a message from a peer and returns the reply to send, if any.
/// Input comes from the network, so malformed messages are errors, never panics.
pub fn handle(doc: &Doc, msg: &[u8]) -> Result<Option<Vec<u8>>, String> {
    let (&tag, body) = msg.split_first().ok_or("empty message")?;
    match tag {
        SYNC_STEP1 => {
            let sv = StateVector::decode_v1(body).map_err(|e| e.to_string())?;
            Ok(Some(tagged(SYNC_STEP2, doc.transact().encode_diff_v1(&sv))))
        }
        SYNC_STEP2 | UPDATE => {
            let u = Update::decode_v1(body).map_err(|e| e.to_string())?;
            doc.transact_mut_with(REMOTE).apply_update(u).map_err(|e| e.to_string())?;
            Ok(None)
        }
        t => Err(format!("unknown tag {t}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Kind, Project};
    use std::sync::{Arc, Mutex};

    /// A peer whose broadcast Updates land in `outbox`, like the data channel would.
    fn peer() -> (Project, Arc<Mutex<Vec<Vec<u8>>>>) {
        let d = Project::default();
        let outbox = Arc::new(Mutex::new(Vec::new()));
        let o = outbox.clone();
        d.doc
            .observe_update_v1("net", move |txn, e| {
                if !is_remote(txn) {
                    o.lock().unwrap().push(update(&e.update));
                }
            })
            .unwrap();
        (d, outbox)
    }

    fn deliver(from: &Mutex<Vec<Vec<u8>>>, to: &Doc) {
        for m in from.lock().unwrap().drain(..) {
            assert_eq!(handle(to, &m).unwrap(), None);
        }
    }

    fn handshake(a: &Doc, b: &Doc) {
        let reply_b = handle(b, &sync_step1(a)).unwrap().unwrap();
        let reply_a = handle(a, &sync_step1(b)).unwrap().unwrap();
        assert_eq!(handle(a, &reply_b).unwrap(), None);
        assert_eq!(handle(b, &reply_a).unwrap(), None);
    }

    #[test]
    fn late_joiner_and_offline_edits_converge() {
        let (a, out_a) = peer();
        let (b, out_b) = peer();
        let ta = a.add_track(Kind::Synth);
        a.set_bpm(100.0, 0.0);
        let tb = b.add_track(Kind::Drums);
        out_a.lock().unwrap().clear(); // not connected yet: these were never sent
        out_b.lock().unwrap().clear();

        handshake(&a.doc, &b.doc);
        assert_eq!(a.view(), b.view());
        let p = a.view();
        assert!(p.track(&ta).is_some() && p.track(&tb).is_some());
        assert_eq!(p.transport.bpm, 100.0);
        assert!(out_a.lock().unwrap().is_empty(), "applying remote state is not echoed");
        assert!(out_b.lock().unwrap().is_empty(), "applying remote state is not echoed");
    }

    #[test]
    fn live_updates_propagate_and_concurrent_writes_converge() {
        let (a, out_a) = peer();
        let (b, out_b) = peer();
        handshake(&a.doc, &b.doc);

        let t = a.add_track(Kind::Synth);
        deliver(&out_a, &b.doc);
        assert!(b.view().track(&t).is_some());
        assert!(out_b.lock().unwrap().is_empty(), "no echo");

        a.set_volume(&t, 0.2); // concurrent: neither has seen the other's write
        b.set_volume(&t, 0.9);
        deliver(&out_a, &b.doc);
        deliver(&out_b, &a.doc);
        assert_eq!(a.view(), b.view());
    }

    #[test]
    fn malformed_input_is_an_error() {
        let d = Doc::new();
        assert!(handle(&d, &[]).is_err());
        assert!(handle(&d, &[9]).is_err());
        assert!(handle(&d, &[UPDATE, 0xff, 0xff, 0xff]).is_err());
    }
}
