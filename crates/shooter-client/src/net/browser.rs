//! The browser connection: game-server's `BrowserSession` over the page's WebTransport. It runs
//! on the page's event loop (`spawn_local`); the game loop drains what arrived each tick.

use super::NetEvent;
use game_server::{BrowserConnectOptions, BrowserSession, BrowserSessionEnd};
use shooter_sim::wire::decode_snapshot;
use std::cell::RefCell;
use std::rc::Rc;

/// The SHA-256 of a development host's self-signed certificate, else the browser's roots.
pub type Trust = Option<[u8; 32]>;

#[derive(Default)]
struct Shared {
    session: Option<BrowserSession>,
    events: Vec<NetEvent>,
}

pub struct Connection {
    shared: Rc<RefCell<Shared>>,
}

impl Connection {
    /// Starts connecting to `url` (`https://host:port/<prefix>/matches/<id>`).
    #[must_use]
    pub fn open(url: String, certificate_hash: Trust) -> Self {
        let shared = Rc::new(RefCell::new(Shared::default()));
        let pending = Rc::clone(&shared);
        wasm_bindgen_futures::spawn_local(async move {
            let options = BrowserConnectOptions {
                server_certificate_hash: certificate_hash,
            };
            let event = match BrowserSession::connect(&url, options).await {
                Ok(session) if session.welcome().tick_hz != shooter_sim::TICK_HZ => {
                    NetEvent::Closed("the server runs another tick rate".to_owned())
                }
                Ok(session) => {
                    let player_id = session.welcome().player_id;
                    pending.borrow_mut().session = Some(session);
                    NetEvent::Welcome { player_id }
                }
                Err(error) => NetEvent::Closed(error.to_string()),
            };
            pending.borrow_mut().events.push(event);
        });
        Self { shared }
    }

    /// Sends one input under game-server's command `sequence`.
    pub fn send(&self, sequence: u32, payload: &[u8]) {
        if let Some(session) = &self.shared.borrow().session {
            // Datagrams are unreliable by design; a lost input is replaced by the next.
            let _ = session.send_command(sequence, payload);
        }
    }

    /// Everything that arrived since the last poll.
    pub fn poll(&self) -> Vec<NetEvent> {
        let mut shared = self.shared.borrow_mut();
        let mut events = std::mem::take(&mut shared.events);
        let Some(session) = &shared.session else {
            return events;
        };
        // A rejected input is a lost one: the next input replaces it, as on native.
        let _ = session.take_rejections();
        for frame in session.take_snapshots() {
            match decode_snapshot(&frame.payload) {
                Ok(snapshot) => events.push(NetEvent::Snapshot(Box::new(snapshot))),
                Err(error) => events.push(NetEvent::Closed(error.to_string())),
            }
        }
        if let Some(end) = session.end() {
            events.push(NetEvent::Closed(match end {
                BrowserSessionEnd::Closed { reason, .. } if !reason.is_empty() => reason,
                BrowserSessionEnd::Closed { .. } => "the game closed the connection".to_owned(),
                BrowserSessionEnd::Lost(reason) => reason,
            }));
            shared.session = None;
        }
        events
    }
}
