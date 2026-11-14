//! The online path end to end on loopback: a game-server host with the deathmatch, and client
//! sessions (`game::online`) that connect over WebTransport, take slots from bots, move with
//! predicted movement, see each other in the snapshots and resume their slot after a lost
//! connection.
#![cfg(not(target_arch = "wasm32"))]

use game_server::{
    BrowserRoutePrefix, MatchHostWebTransportConfig, serve_match_host_with_shutdown,
};
use shooter_client::game::Game;
use shooter_client::game::online::Online;
use shooter_sim::robot::{SKIRMISHER, STANDARD};
use shooter_sim::{Controller, Intent, MapId};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use wtransport::Identity;

/// A host on loopback serving the `dm` match until dropped.
struct Host {
    url: String,
    cert: PathBuf,
    dir: PathBuf,
    stop: tokio::sync::mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Host {
    /// `lane` keeps parallel tests on their own ports and files.
    fn start(lane: u16) -> Self {
        let dir =
            std::env::temp_dir().join(format!("shooter-online-{}-{lane}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let identity = Identity::self_signed(["localhost", "127.0.0.1"]).unwrap();
        let (cert, key) = (dir.join("cert.pem"), dir.join("key.pem"));
        std::fs::write(&cert, identity.certificate_chain().as_slice()[0].to_pem()).unwrap();
        std::fs::write(&key, identity.private_key().to_secret_pem()).unwrap();
        let port = 4_600 + 2 * u16::try_from(std::process::id() % 400).unwrap() + lane;
        let config = MatchHostWebTransportConfig {
            port,
            certificate_pem: cert.clone(),
            private_key_pem: key,
            route_prefix: BrowserRoutePrefix::new("/shooter").unwrap(),
            allowed_origins: None,
            drain_grace: Duration::from_millis(100),
        };
        let (stop, requests) = tokio::sync::mpsc::channel(1);
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            runtime.block_on(async {
                // A 5 s reconnect grace.
                let host = shooter_server::build_host(&["dm".parse().unwrap()], 300).unwrap();
                serve_match_host_with_shutdown(host, config, requests)
                    .await
                    .unwrap();
            });
        });
        std::thread::sleep(Duration::from_millis(500));
        Self {
            url: format!("https://localhost:{port}/shooter/matches/dm"),
            cert,
            dir,
            stop,
            thread: Some(thread),
        }
    }

    fn connect(&self, name: Option<&str>) -> Game {
        Game::Online(Box::new(Online::connect(
            self.url.clone(),
            Some(self.cert.clone()),
            name,
        )))
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.stop.blocking_send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const FORWARD: Intent = Intent {
    forward: 1,
    ..Intent::IDLE
};

#[test]
fn two_clients_play_a_hosted_deathmatch() {
    let host = Host::start(0);
    let (mut first, mut second) = (
        host.connect(Some("Ripley")).with_loadout(SKIRMISHER),
        host.connect(None),
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut start = None;
    let mut moved = 0.0f32;
    while Instant::now() < deadline {
        first.tick(FORWARD).unwrap();
        second.tick(Intent::IDLE).unwrap();
        if let Some(center) = first.local_center() {
            let origin = *start.get_or_insert(center);
            moved = moved.max((center[0] - origin[0]).hypot(center[2] - origin[2]));
        }
        let both = first.local_slot().is_some() && second.local_slot().is_some();
        if both && moved > 300.0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    assert!(first.status().is_none(), "{:?}", first.status());
    // Both follow the host's map, which the snapshots name.
    assert_eq!(first.map(), Some(MapId::Greybox));
    assert_eq!(second.map(), Some(MapId::Greybox));
    let (a, b) = (first.local_slot().unwrap(), second.local_slot().unwrap());
    assert_ne!(a, b);
    assert!(moved > 300.0, "the first client walked {moved} units");
    let snapshot = second.snapshot().unwrap();
    let humans = snapshot
        .fighters
        .iter()
        .filter(|fighter| matches!(fighter.controller, Controller::Human(_)))
        .count();
    assert_eq!(humans, 2);
    let walker = snapshot.slot(a).unwrap();
    assert!(walker.ack > 0);
    // The first client's name reached the host and everyone's snapshots.
    assert_eq!(snapshot.name(a), Some("Ripley"));
    assert_eq!(snapshot.name(b), None);
    // So did its loadout, validated, for its next life; the second asked for none.
    assert_eq!(walker.configured, SKIRMISHER);
    assert_eq!(snapshot.slot(b).unwrap().configured, STANDARD);
    // The second client draws the walker from its interpolation buffer.
    assert!(second.other_center(a, 0.5).is_some());
}

/// A connection lost mid-match (not closed) resumes at game-server's reconnect route: the HUD
/// shows "reconnecting", then the client is back in its own slot, predicting and moving again.
#[test]
fn a_lost_connection_rejoins_its_slot() {
    let host = Host::start(1);
    let mut game = host.connect(None);
    let play = |game: &mut Game, until: &dyn Fn(&Game) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline && !until(game) {
            game.tick(FORWARD).unwrap();
            std::thread::sleep(Duration::from_millis(16));
        }
    };
    play(&mut game, &|game| game.local_center().is_some());
    let slot = game.local_slot().expect("admitted");
    let Game::Online(online) = &game else {
        unreachable!()
    };
    let player = online.player_id().unwrap();
    assert_eq!(online.resumed(), 0);
    online.sever();

    let resumed = |game: &Game| match game {
        Game::Online(online) => online.resumed(),
        Game::Practice { .. } => 0,
    };
    play(&mut game, &|game| {
        resumed(game) == 1 && game.status().is_none() && game.local_center().is_some()
    });
    assert_eq!(resumed(&game), 1, "{:?}", game.status());
    assert!(game.status().is_none(), "{:?}", game.status());
    let Game::Online(online) = &game else {
        unreachable!()
    };
    assert_eq!(online.player_id(), Some(player));
    assert_eq!(game.local_slot(), Some(slot));
    let snapshot = game.snapshot().unwrap();
    assert_eq!(
        snapshot.slot(slot).unwrap().controller,
        Controller::Human(player)
    );
}
