#[tokio::main]
async fn main() {
    // Resolved from this crate's location so `cargo run` works from any directory.
    let default_dist = concat!(env!("CARGO_MANIFEST_DIR"), "/../app/dist");
    let dist = std::env::var("DIST_DIR").unwrap_or_else(|_| default_dist.into());
    let addr = std::env::var("ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
    let port = listener.local_addr().expect("bound address").port();
    // localhost, not 0.0.0.0: AudioWorklet only runs on a secure origin, and localhost counts as one.
    println!("open http://localhost:{port}  (serving {dist})");
    axum::serve(listener, signaling::router(&dist)).await.unwrap();
}
