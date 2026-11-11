//! The native client: `cargo run -p shooter-client --release`.

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    shooter_client::run();
}
