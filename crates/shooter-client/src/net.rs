//! The client's connection to a hosted match, natively over wtransport and in the browser over
//! the page's WebTransport (game-server's `BrowserSession`). Both speak game-server's frames: the
//! game loop sends one input per tick and polls the welcome, snapshots and the end of the session.
//!
//! A session that is lost (not closed by either side) is resumed: the connection reconnects at
//! game-server's reconnect route with the token from the last welcome, retrying every
//! [`RETRY_INTERVAL`] until the welcome's reconnect grace runs out, and the host gives back the
//! same player and slot.

use game_server::{BrowserRoutePrefix, MatchId, ReconnectToken, Welcome};
use shooter_sim::Snapshot;
use web_time::Duration;

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(not(target_arch = "wasm32"))]
mod native;

#[cfg(target_arch = "wasm32")]
pub use browser::{Connection, Trust};
#[cfg(not(target_arch = "wasm32"))]
pub use native::{Connection, Trust};

/// Between reconnect attempts (the host refuses one until it has noticed the loss).
pub const RETRY_INTERVAL: Duration = Duration::from_millis(500);

/// What the connection reports to the game loop.
#[derive(Debug)]
pub enum NetEvent {
    /// Admitted (or readmitted after a loss) as `player_id`.
    Welcome {
        player_id: u32,
    },
    Snapshot(Box<Snapshot>),
    /// The session was lost and is being resumed, and why it was lost.
    Reconnecting(String),
    /// The session ended (or never started), and why.
    Closed(String),
}

/// How long after a loss the host keeps the slot, from `welcome`.
fn grace(welcome: &Welcome) -> Duration {
    let ticks = welcome.reconnect_grace_ticks.min(u64::from(u32::MAX));
    Duration::from_secs(ticks) / u32::from(welcome.tick_hz.max(1))
}

/// The reconnect URL for the match at `url` (`https://host:port/<prefix>/matches/<id>`) with
/// the token from `welcome`.
fn reconnect_url(url: &str, welcome: &Welcome) -> Option<String> {
    let path_start = url.find("://").map_or(0, |scheme| scheme + 3);
    let path_start = path_start + url[path_start..].find('/')?;
    let (origin, path) = url.split_at(path_start);
    let (prefix, id) = path.trim_end_matches('/').rsplit_once("/matches/")?;
    let prefix = BrowserRoutePrefix::new(prefix).ok()?;
    let id = MatchId::new(id.to_owned()).ok()?;
    let token = ReconnectToken(welcome.reconnect_token);
    Some(format!("{origin}{}", prefix.reconnect_path(&id, token)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_server::RECONNECT_TOKEN_BYTES;

    #[test]
    fn reconnects_at_the_match_reconnect_route_within_the_grace() {
        let welcome = Welcome {
            player_id: 3,
            tick_hz: 60,
            max_players: 6,
            current_tick: 0,
            connection_epoch: 1,
            reconnect_token: [0xab; RECONNECT_TOKEN_BYTES],
            reconnect_grace_ticks: 300,
        };
        assert_eq!(
            reconnect_url("https://localhost:4433/shooter/matches/dm", &welcome).unwrap(),
            format!(
                "https://localhost:4433/shooter/matches/dm/reconnect/{}",
                "ab".repeat(RECONNECT_TOKEN_BYTES)
            )
        );
        assert!(reconnect_url("https://localhost:4433/elsewhere", &welcome).is_none());
        assert_eq!(grace(&welcome), Duration::from_secs(5));
    }
}
