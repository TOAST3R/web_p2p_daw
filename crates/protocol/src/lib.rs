//! Signaling messages (JSON over WebSocket). The server only creates Joined/PeerJoined/PeerLeft/RoomFull
//! and forwards everything else verbatim; pattern data never travels this path.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type")]
pub enum Signal {
    /// Sent to a peer right after it joins. `peer_count == 2` means it should send the offer.
    Joined { peer_count: usize },
    PeerJoined,
    PeerLeft,
    RoomFull,
    Offer { sdp: String },
    Answer { sdp: String },
    Ice { candidate: String, sdp_mid: Option<String>, sdp_m_line_index: Option<u16> },
}

impl Signal {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("Signal always serializes")
    }
}
