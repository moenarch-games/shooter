//! The native client's connection to a hosted match: WebTransport (wtransport) through
//! game-server's frames, on a thread of its own with a single-threaded tokio runtime. The game
//! loop sends one input per tick and polls the welcome, snapshots and the end of the session.

use game_server::{SnapshotReassembler, WELCOME_BYTES};
use shooter_sim::Snapshot;
use shooter_sim::wire::decode_snapshot;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use wtransport::{ClientConfig, Endpoint, tls::Certificate};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// No snapshot for this long ends the session.
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(5);

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

pub struct Connection {
    outgoing: UnboundedSender<Vec<u8>>,
    incoming: Receiver<NetEvent>,
}

impl Connection {
    /// Starts connecting to `url` (`https://host:port/<prefix>/matches/<id>`), trusting the
    /// self-signed certificate at `certificate` if given (development), else the system's roots.
    #[must_use]
    pub fn open(url: String, certificate: Option<PathBuf>) -> Self {
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
            let reason = runtime
                .block_on(session(&url, certificate, commands, &events))
                .err()
                .unwrap_or_else(|| "the game closed the connection".to_owned());
            let _ = events.send(NetEvent::Closed(reason));
        });
        Self { outgoing, incoming }
    }

    /// Sends one input under game-server's command `sequence`.
    pub fn send(&self, sequence: u32, payload: &[u8]) {
        if let Ok(frame) = game_server::encode_command(sequence, payload) {
            let _ = self.outgoing.send(frame);
        }
    }

    /// Everything that arrived since the last poll.
    pub fn poll(&self) -> Vec<NetEvent> {
        self.incoming.try_iter().collect()
    }
}

async fn session(
    url: &str,
    certificate: Option<PathBuf>,
    mut commands: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    events: &Sender<NetEvent>,
) -> Result<(), String> {
    let text = |error: &dyn std::fmt::Display| error.to_string();
    let (connection, welcome) = tokio::time::timeout(CONNECT_TIMEOUT, async {
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
    .await
    .map_err(|_| "connecting timed out".to_owned())??;
    if welcome.tick_hz != shooter_sim::TICK_HZ {
        return Err("the server runs another tick rate".to_owned());
    }
    let _ = events.send(NetEvent::Welcome {
        player_id: welcome.player_id,
    });
    let mut reassembler = SnapshotReassembler::new();
    loop {
        tokio::select! {
            datagram = tokio::time::timeout(SNAPSHOT_TIMEOUT, connection.receive_datagram()) => {
                let datagram = datagram
                    .map_err(|_| "the server went quiet".to_owned())?
                    .map_err(|e| text(&e))?;
                if let Some(frame) = reassembler.accept(&datagram).map_err(|e| text(&e))? {
                    let snapshot = decode_snapshot(&frame.payload).map_err(|e| text(&e))?;
                    if events.send(NetEvent::Snapshot(Box::new(snapshot))).is_err() {
                        return Ok(());
                    }
                }
            }
            command = commands.recv() => {
                let Some(frame) = command else { return Ok(()) };
                // Datagrams are unreliable by design; a lost input is replaced by the next.
                let _ = connection.send_datagram(frame);
            }
        }
    }
}
