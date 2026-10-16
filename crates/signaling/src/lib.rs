//! Stateless signaling: pairs up to 2 peers per room and relays their SDP/ICE text verbatim.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::Response;
use axum::routing::get;
use protocol::Signal;
use serde::Deserialize;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tower_http::services::ServeDir;

const MAX_PEERS: usize = 2;

type Rooms = Arc<Mutex<HashMap<String, Vec<(u64, UnboundedSender<Message>)>>>>;

#[derive(Deserialize)]
struct Join {
    room: String,
}

pub fn router(dist_dir: &str) -> Router {
    Router::new()
        .route("/ws", get(ws))
        .fallback_service(ServeDir::new(dist_dir))
        .with_state(Rooms::default())
}

async fn ws(upgrade: WebSocketUpgrade, Query(join): Query<Join>, State(rooms): State<Rooms>) -> Response {
    upgrade.on_upgrade(move |socket| peer(socket, join.room, rooms))
}

fn text(s: Signal) -> Message {
    Message::Text(s.to_json().into())
}

async fn peer(mut socket: WebSocket, room: String, rooms: Rooms) {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let (tx, mut rx) = unbounded_channel();

    let joined = {
        let mut rooms = rooms.lock().unwrap();
        let peers = rooms.entry(room.clone()).or_default();
        if peers.len() >= MAX_PEERS {
            None
        } else {
            for (_, p) in peers.iter() {
                let _ = p.send(text(Signal::PeerJoined));
            }
            peers.push((id, tx));
            Some(peers.len())
        }
    };
    let Some(peer_count) = joined else {
        let _ = socket.send(text(Signal::RoomFull)).await;
        return; // dropping the socket closes it
    };
    if socket.send(text(Signal::Joined { peer_count })).await.is_err() {
        return leave(&rooms, &room, id);
    }

    loop {
        tokio::select! {
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(t))) => {
                    let rooms = rooms.lock().unwrap();
                    for (pid, p) in rooms.get(&room).into_iter().flatten() {
                        if *pid != id {
                            let _ = p.send(Message::Text(t.clone()));
                        }
                    }
                }
                Some(Ok(_)) => {}
                _ => break,
            },
            Some(out) = rx.recv() => if socket.send(out).await.is_err() { break },
        }
    }
    leave(&rooms, &room, id);
}

fn leave(rooms: &Rooms, room: &str, id: u64) {
    let mut rooms = rooms.lock().unwrap();
    if let Some(peers) = rooms.get_mut(room) {
        peers.retain(|(pid, _)| *pid != id);
        for (_, p) in peers.iter() {
            let _ = p.send(text(Signal::PeerLeft));
        }
        if peers.is_empty() {
            rooms.remove(room);
        }
    }
}
