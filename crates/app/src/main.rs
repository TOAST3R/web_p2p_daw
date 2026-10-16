fn main() {
    #[cfg(target_arch = "wasm32")]
    app::web::start();
}
