use futures_util::{SinkExt, StreamExt};
use protocol::Signal;
use tokio_tungstenite::tungstenite::Message;

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(axum::serve(listener, signaling::router("does-not-exist")).into_future());
    format!("ws://{addr}/ws?room=r1")
}

async fn join(url: &str) -> Ws {
    tokio_tungstenite::connect_async(url).await.unwrap().0
}

async fn recv(ws: &mut Ws) -> Signal {
    loop {
        match ws.next().await.unwrap().unwrap() {
            Message::Text(t) => return serde_json::from_str(&t).unwrap(),
            _ => continue,
        }
    }
}

#[tokio::test]
async fn pairs_relays_rejects_third_and_reports_leave() {
    let url = server().await;
    let mut a = join(&url).await;
    assert_eq!(recv(&mut a).await, Signal::Joined { peer_count: 1 });

    let mut b = join(&url).await;
    assert_eq!(recv(&mut b).await, Signal::Joined { peer_count: 2 });
    assert_eq!(recv(&mut a).await, Signal::PeerJoined);

    let offer = Signal::Offer { sdp: "v=0 test".into() };
    b.send(Message::Text(offer.to_json().into())).await.unwrap();
    assert_eq!(recv(&mut a).await, offer, "relayed verbatim to the other peer");

    let mut c = join(&url).await;
    assert_eq!(recv(&mut c).await, Signal::RoomFull);
    assert!(matches!(c.next().await, None | Some(Ok(Message::Close(_))) | Some(Err(_))), "third peer is closed");

    b.close(None).await.unwrap();
    assert_eq!(recv(&mut a).await, Signal::PeerLeft);

    // freed slot can be reused (reconnect)
    let mut b2 = join(&url).await;
    assert_eq!(recv(&mut b2).await, Signal::Joined { peer_count: 2 });
}
