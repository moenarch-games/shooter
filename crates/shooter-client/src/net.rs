//! The client's connection to a hosted match, natively over wtransport and in the browser over
//! the page's WebTransport (game-server's `BrowserSession`). Both speak game-server's frames: the
//! game loop sends one input per tick and polls the welcome, snapshots and the end of the session.

use shooter_sim::Snapshot;

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(not(target_arch = "wasm32"))]
mod native;

#[cfg(target_arch = "wasm32")]
pub use browser::{Connection, Trust};
#[cfg(not(target_arch = "wasm32"))]
pub use native::{Connection, Trust};

/// What the connection reports to the game loop.
#[derive(Debug)]
pub enum NetEvent {
    /// Admitted as `player_id`.
    Welcome {
        player_id: u32,
    },
    Snapshot(Box<Snapshot>),
    /// The session ended (or never started), and why.
    Closed(String),
}
