//! The browser connection: game-server's `BrowserSession` over the page's WebTransport. It runs
//! on the page's event loop (`spawn_local`); the game loop drains what arrived each tick. A lost
//! session is resumed at the reconnect route (see [`super`]).

use super::{NetEvent, RETRY_INTERVAL, grace, reconnect_url};
use game_server::{BrowserConnectOptions, BrowserSession, BrowserSessionEnd, Welcome};
use shooter_sim::wire::decode_snapshot;
use std::cell::RefCell;
use std::rc::Rc;
use web_time::Instant;

/// The SHA-256 of a development host's self-signed certificate, else the browser's roots.
pub type Trust = Option<[u8; 32]>;

#[derive(Default)]
struct Shared {
    session: Option<BrowserSession>,
    /// The last welcome: its token resumes a lost session.
    welcome: Option<Welcome>,
    events: Vec<NetEvent>,
}

pub struct Connection {
    url: String,
    certificate_hash: Trust,
    shared: Rc<RefCell<Shared>>,
}

/// Connects at `url`; `Ok` once admitted (the session and welcome are in `shared`).
async fn admit(
    url: &str,
    certificate_hash: Trust,
    shared: &Rc<RefCell<Shared>>,
) -> Result<(), String> {
    let options = BrowserConnectOptions {
        server_certificate_hash: certificate_hash,
    };
    let session = BrowserSession::connect(url, options)
        .await
        .map_err(|error| error.to_string())?;
    let welcome = session.welcome();
    if welcome.tick_hz != shooter_sim::TICK_HZ {
        return Err("the server runs another tick rate".to_owned());
    }
    let mut shared = shared.borrow_mut();
    shared.session = Some(session);
    shared.welcome = Some(welcome);
    shared.events.push(NetEvent::Welcome {
        player_id: welcome.player_id,
    });
    Ok(())
}

/// Waits `RETRY_INTERVAL` on the page's timer.
async fn pause() {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            #[allow(clippy::cast_possible_truncation)]
            let millis = RETRY_INTERVAL.as_millis() as i32;
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, millis);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

impl Connection {
    /// Starts connecting to `url` (`https://host:port/<prefix>/matches/<id>`).
    #[must_use]
    pub fn open(url: String, certificate_hash: Trust) -> Self {
        let shared = Rc::new(RefCell::new(Shared::default()));
        let pending = Rc::clone(&shared);
        let target = url.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(reason) = admit(&target, certificate_hash, &pending).await {
                pending.borrow_mut().events.push(NetEvent::Closed(reason));
            }
        });
        Self {
            url,
            certificate_hash,
            shared,
        }
    }

    /// Resumes the session lost because of `reason` at the reconnect route, until the grace of
    /// the last welcome runs out.
    fn resume(&self, reason: String, shared: &mut Shared) {
        let Some((welcome, target)) = shared
            .welcome
            .and_then(|welcome| Some((welcome, reconnect_url(&self.url, &welcome)?)))
        else {
            shared.events.push(NetEvent::Closed(reason));
            return;
        };
        shared.events.push(NetEvent::Reconnecting(reason.clone()));
        let deadline = Instant::now() + grace(&welcome);
        let pending = Rc::clone(&self.shared);
        let certificate_hash = self.certificate_hash;
        wasm_bindgen_futures::spawn_local(async move {
            while Instant::now() < deadline {
                if admit(&target, certificate_hash, &pending).await.is_ok() {
                    return;
                }
                pause().await;
            }
            pending.borrow_mut().events.push(NetEvent::Closed(format!(
                "{reason}; reconnecting timed out"
            )));
        });
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
        let Some(session) = &shared.session else {
            return std::mem::take(&mut shared.events);
        };
        // A rejected input is a lost one: the next input replaces it, as on native.
        let _ = session.take_rejections();
        let mut arrived = Vec::new();
        for frame in session.take_snapshots() {
            match decode_snapshot(&frame.payload) {
                Ok(snapshot) => arrived.push(NetEvent::Snapshot(Box::new(snapshot))),
                Err(error) => arrived.push(NetEvent::Closed(error.to_string())),
            }
        }
        let end = session.end();
        shared.events.append(&mut arrived);
        match end {
            None => {}
            Some(BrowserSessionEnd::Lost(reason)) => {
                shared.session = None;
                self.resume(reason, &mut shared);
            }
            Some(BrowserSessionEnd::Closed { reason, .. }) => {
                shared.session = None;
                shared.events.push(NetEvent::Closed(if reason.is_empty() {
                    "the game closed the connection".to_owned()
                } else {
                    reason
                }));
            }
        }
        std::mem::take(&mut shared.events)
    }
}
