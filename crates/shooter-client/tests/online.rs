//! The online path end to end on loopback: a game-server host with the deathmatch, and client
//! sessions (`game::online`) that connect over WebTransport, take slots from bots, move with
//! predicted movement, see each other in the snapshots and resume their slot after a lost
//! connection.
#![cfg(not(target_arch = "wasm32"))]

use game_server::{
    BrowserRoutePrefix, MatchHostWebTransportConfig, serve_match_host_with_shutdown,
};
use shooter_client::controls::Selection;
use shooter_client::game::Game;
use shooter_client::game::online::Online;
use shooter_sim::robot::{SKIRMISHER, STANDARD};
use shooter_sim::{Controller, Intent, MapId, Weapon};
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
                // A 5 s reconnect grace, on the greybox fixture: this checks networking, not a map.
                let host =
                    shooter_server::build_host(&["dm=greybox".parse().unwrap()], 300).unwrap();
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
        // Eleven bots may have just killed the walker: wait until the second client draws it.
        let drawn = first
            .local_slot()
            .is_some_and(|walker| second.other_center(walker, 0.5).is_some());
        if both && drawn && moved > 300.0 {
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

/// The weapon cycle goes through the host (shooter#92): two steps from the standard build's
/// rifle reach its shotgun (past the grapple) in the prediction at once and on the host, and a
/// switch to a weapon the robot does not carry, asked for as long as a step is, changes nothing
/// on either.
#[test]
fn the_weapon_cycle_goes_through_the_host() {
    let host = Host::start(2);
    let mut game = host.connect(None);
    let deadline = Instant::now() + Duration::from_secs(30);
    // Ticks `game` with `select` until the host shows `want` held (or `ticks` pass): whether it
    // did, and whether the fighter died meanwhile (a bot can end the life mid-check).
    let hold = |game: &mut Game, select: Option<Weapon>, want: Weapon, ticks: u32| {
        let deaths = |game: &Game| {
            let slot = game.local_slot()?;
            Some(game.snapshot()?.slot(slot)?.deaths)
        };
        let before = deaths(game);
        let mut shown = false;
        for _ in 0..ticks {
            game.tick(Intent {
                select,
                ..Intent::IDLE
            })
            .unwrap();
            let held = game
                .local_slot()
                .and_then(|slot| game.snapshot()?.slot(slot).map(|me| me.held));
            shown = held == Some(want);
            std::thread::sleep(Duration::from_millis(16));
        }
        (shown, deaths(game) == before)
    };
    let mut checked = false;
    while Instant::now() < deadline && !checked {
        game.tick(Intent::IDLE).unwrap();
        std::thread::sleep(Duration::from_millis(16));
        let Some((me, _)) = game.local_fighter() else {
            continue;
        };
        if me.arsenal.held != Weapon::EnergyRifle {
            continue;
        }
        // Two notches down: rifle, grapple, shotgun.
        let mut selection = Selection::default();
        let (build, parts) = (me.build, me.parts);
        let armed = |weapon| parts.armed(&build, weapon);
        let select = selection.intent(None, 2, Weapon::EnergyRifle, armed);
        assert_eq!(select, Some(Weapon::Shotgun));
        game.tick(Intent {
            select,
            ..Intent::IDLE
        })
        .unwrap();
        assert_eq!(
            game.local_fighter().map(|(me, _)| me.arsenal.held),
            Some(Weapon::Shotgun),
            "predicted at once"
        );
        let (shown, alive) = hold(&mut game, select, Weapon::Shotgun, Selection::ASK_TICKS);
        if !alive {
            continue;
        }
        assert!(shown, "the host took the switch");
        // The rail gun is not on the standard build: refused by the host, and the prediction
        // follows the host.
        let (shotgun, alive) = hold(
            &mut game,
            Some(Weapon::RailGun),
            Weapon::Shotgun,
            Selection::ASK_TICKS,
        );
        if !alive {
            continue;
        }
        assert!(shotgun, "the host kept the shotgun");
        assert_eq!(
            game.local_fighter().map(|(me, _)| me.arsenal.held),
            Some(Weapon::Shotgun)
        );
        checked = true;
    }
    assert!(checked, "no life lasted the check");
}

/// The builder through a real host (shooter#57): a build readied before the match (before the
/// first tick connects) is the robot's first life; after a death, Edit Robot holds the respawn
/// however long it takes, a new build readied meanwhile is accepted by the host, and letting go
/// brings the robot back as it.
#[test]
fn the_builder_s_builds_reach_the_host_before_the_match_and_between_lives() {
    use shooter_sim::robot::{HEAVY, PYRO};
    let host = Host::start(3);
    let mut game = host.connect(None);
    game.configure(PYRO);
    let me = |game: &Game| {
        let slot = game.local_slot()?;
        game.snapshot()?.slot(slot).copied()
    };
    let play = |game: &mut Game, intent: Intent, seconds: u64, until: &dyn Fn(&Game) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(seconds);
        while Instant::now() < deadline && !until(game) {
            game.tick(intent).unwrap();
            std::thread::sleep(Duration::from_millis(16));
        }
    };
    play(&mut game, Intent::IDLE, 20, &|game| {
        me(game).is_some_and(|me| me.configured == PYRO)
    });
    let first = me(&game).expect("admitted");
    // Configured before it played: the robot is that build at once, not after a death.
    assert_eq!(
        (first.build, first.configured),
        (PYRO, PYRO),
        "the first life"
    );
    assert_eq!(first.deaths, 0);

    // Eleven bots end an idle robot's life soon enough.
    play(&mut game, Intent::IDLE, 120, &|game| {
        me(game).is_some_and(|me| !me.alive)
    });
    assert!(!me(&game).unwrap().alive, "never killed");
    let editing = Intent {
        edit: true,
        ..Intent::IDLE
    };
    game.configure(HEAVY);
    play(&mut game, editing, 20, &|game| {
        me(game).is_some_and(|me| me.configured == HEAVY)
    });
    // Past the 2 s countdown, still in the builder.
    play(&mut game, editing, 3, &|_| false);
    let waiting = me(&game).unwrap();
    assert!(!waiting.alive, "the respawn waits for Ready");
    assert_eq!(waiting.configured, HEAVY);
    play(&mut game, Intent::IDLE, 20, &|game| {
        me(game).is_some_and(|me| me.alive)
    });
    assert_eq!(me(&game).unwrap().build, HEAVY);
}
