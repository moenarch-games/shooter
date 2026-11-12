//! The native connection: WebTransport (wtransport) through game-server's frames, on a thread of
//! its own with a single-threaded tokio runtime. A lost session is resumed at the reconnect route
//! (see [`super`]).

use super::{NetEvent, RETRY_INTERVAL, grace, reconnect_url};
use game_server::{SnapshotReassembler, WELCOME_BYTES, Welcome};
use shooter_sim::wire::decode_snapshot;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::time::Instant;
use wtransport::error::ConnectionError;
use wtransport::{ClientConfig, Endpoint, tls::Certificate};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// No snapshot for this long loses the session.
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(5);

/// The development host's self-signed certificate (PEM file) to trust, else the system's roots.
pub type Trust = Option<PathBuf>;

enum Command {
    Send(Vec<u8>),
    /// Drop the connection without closing it, as a network loss would.
    Sever,
}

/// How a session ended.
enum End {
    /// The game dropped the connection.
    Dropped,
    /// Closed by the host (or never admitted): not resumed.
    Closed(String),
    /// Lost after the welcome: resumed.
    Lost(String),
}

pub struct Connection {
    outgoing: UnboundedSender<Command>,
    incoming: Receiver<NetEvent>,
}

impl Connection {
    /// Starts connecting to `url` (`https://host:port/<prefix>/matches/<id>`), trusting the
    /// self-signed certificate at `certificate` if given (development), else the system's roots.
    #[must_use]
    pub fn open(url: String, certificate: Trust) -> Self {
        let (outgoing, commands) = unbounded_channel();
        let (events, incoming) = channel();
        std::thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = events.send(NetEvent::Closed(error.to_string()));
                    return;
                }
            };
            if let Some(reason) = runtime.block_on(run(&url, certificate, commands, &events)) {
                let _ = events.send(NetEvent::Closed(reason));
            }
        });
        Self { outgoing, incoming }
    }

    /// Sends one input under game-server's command `sequence`.
    pub fn send(&self, sequence: u32, payload: &[u8]) {
        if let Ok(frame) = game_server::encode_command(sequence, payload) {
            let _ = self.outgoing.send(Command::Send(frame));
        }
    }

    /// Drops the connection without closing it (testing the reconnect).
    #[doc(hidden)]
    pub fn sever(&self) {
        let _ = self.outgoing.send(Command::Sever);
    }

    /// Everything that arrived since the last poll.
    pub fn poll(&self) -> Vec<NetEvent> {
        self.incoming.try_iter().collect()
    }
}

/// Plays sessions until one ends for good; why, unless the game dropped the connection.
async fn run(
    url: &str,
    certificate: Option<PathBuf>,
    mut commands: UnboundedReceiver<Command>,
    events: &Sender<NetEvent>,
) -> Option<String> {
    let mut welcome = None;
    let mut reason = match session(url, &certificate, &mut commands, events, &mut welcome).await {
        End::Dropped => return None,
        End::Closed(reason) => return Some(reason),
        End::Lost(reason) => reason,
    };
    'resume: loop {
        let last = welcome?;
        let resume = reconnect_url(url, &last)?;
        let deadline = Instant::now() + grace(&last);
        let _ = events.send(NetEvent::Reconnecting(reason.clone()));
        while Instant::now() < deadline {
            // Inputs from before the loss are stale.
            while commands.try_recv().is_ok() {}
            match session(&resume, &certificate, &mut commands, events, &mut welcome).await {
                End::Dropped => return None,
                // Not readmitted (yet): the host may not have noticed the loss.
                _ if welcome == Some(last) => tokio::time::sleep(RETRY_INTERVAL).await,
                End::Lost(lost) => {
                    reason = lost;
                    continue 'resume;
                }
                End::Closed(closed) => return Some(closed),
            }
        }
        return Some(format!("{reason}; reconnecting timed out"));
    }
}

/// Whether a connection error means the host closed the session on purpose.
const fn closed_on_purpose(error: &ConnectionError) -> bool {
    matches!(
        error,
        ConnectionError::ApplicationClosed(_) | ConnectionError::LocallyClosed
    )
}

/// Connects to `url`, reports the welcome (and keeps it in `welcome`), then passes snapshots in
/// and inputs out until the session ends.
async fn session(
    url: &str,
    certificate: &Option<PathBuf>,
    commands: &mut UnboundedReceiver<Command>,
    events: &Sender<NetEvent>,
    welcome: &mut Option<Welcome>,
) -> End {
    let text = |error: &dyn std::fmt::Display| error.to_string();
    let connected = tokio::time::timeout(CONNECT_TIMEOUT, async {
        let builder = ClientConfig::builder().with_bind_default();
        let config = match certificate {
            Some(path) => builder.with_server_certificate_hashes([Certificate::load_pemfile(path)
                .await
                .map_err(|e| text(&e))?
                .hash()]),
            None => builder.with_native_certs(),
        }
        .keep_alive_interval(Some(Duration::from_secs(1)))
        .build();
        let endpoint = Endpoint::client(config).map_err(|e| text(&e))?;
        let connection = endpoint.connect(url).await.map_err(|e| text(&e))?;
        let mut stream = connection.accept_uni().await.map_err(|e| text(&e))?;
        let mut bytes = [0; WELCOME_BYTES];
        stream.read_exact(&mut bytes).await.map_err(|e| text(&e))?;
        let welcome = game_server::decode_welcome(&bytes).map_err(|e| text(&e))?;
        Ok::<_, String>((connection, welcome))
    })
    .await;
    let (connection, admitted) = match connected {
        Ok(Ok(connected)) => connected,
        Ok(Err(reason)) => return End::Closed(reason),
        Err(_) => return End::Closed("connecting timed out".to_owned()),
    };
    if admitted.tick_hz != shooter_sim::TICK_HZ {
        return End::Closed("the server runs another tick rate".to_owned());
    }
    *welcome = Some(admitted);
    if events
        .send(NetEvent::Welcome {
            player_id: admitted.player_id,
        })
        .is_err()
    {
        return End::Dropped;
    }
    let mut reassembler = SnapshotReassembler::new();
    loop {
        tokio::select! {
            datagram = tokio::time::timeout(SNAPSHOT_TIMEOUT, connection.receive_datagram()) => {
                let datagram = match datagram {
                    Err(_) => return End::Lost("the server went quiet".to_owned()),
                    Ok(Err(error)) if closed_on_purpose(&error) => return End::Closed(text(&error)),
                    Ok(Err(error)) => return End::Lost(text(&error)),
                    Ok(Ok(datagram)) => datagram,
                };
                let frame = match reassembler.accept(&datagram) {
                    Ok(frame) => frame,
                    Err(error) => return End::Closed(text(&error)),
                };
                if let Some(frame) = frame {
                    let snapshot = match decode_snapshot(&frame.payload) {
                        Ok(snapshot) => snapshot,
                        Err(error) => return End::Closed(text(&error)),
                    };
                    if events.send(NetEvent::Snapshot(Box::new(snapshot))).is_err() {
                        return End::Dropped;
                    }
                }
            }
            command = commands.recv() => match command {
                None => return End::Dropped,
                // Datagrams are unreliable by design; a lost input is replaced by the next.
                Some(Command::Send(frame)) => {
                    let _ = connection.send_datagram(frame);
                }
                Some(Command::Sever) => return End::Lost("the connection was severed".to_owned()),
            },
        }
    }
}
