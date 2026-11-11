//! The online path end to end on loopback: a game-server host with the deathmatch, and two
//! client sessions (`game::online`) that connect over WebTransport, take slots from bots, move
//! with predicted movement and see each other in the snapshots.
#![cfg(not(target_arch = "wasm32"))]

use game_server::{
    BrowserRoutePrefix, MatchHostWebTransportConfig, serve_match_host_with_shutdown,
};
use shooter_client::game::Game;
use shooter_client::game::online::Online;
use shooter_sim::{Controller, Intent};
use std::time::{Duration, Instant};
use wtransport::Identity;

#[test]
fn two_clients_play_a_hosted_deathmatch() {
    let dir = std::env::temp_dir().join(format!("shooter-online-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let identity = Identity::self_signed(["localhost", "127.0.0.1"]).unwrap();
    let (cert, key) = (dir.join("cert.pem"), dir.join("key.pem"));
    std::fs::write(&cert, identity.certificate_chain().as_slice()[0].to_pem()).unwrap();
    std::fs::write(&key, identity.private_key().to_secret_pem()).unwrap();
    let port = 4_600 + u16::try_from(std::process::id() % 400).unwrap();

    let config = MatchHostWebTransportConfig {
        port,
        certificate_pem: cert.clone(),
        private_key_pem: key,
        route_prefix: BrowserRoutePrefix::new("/shooter").unwrap(),
        allowed_origins: None,
        drain_grace: Duration::from_millis(100),
    };
    let (stop, requests) = tokio::sync::mpsc::channel(1);
    let host = std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let host = shooter_server::build_host(&["dm".to_owned()], 300).unwrap();
            serve_match_host_with_shutdown(host, config, requests)
                .await
                .unwrap();
        });
    });
    std::thread::sleep(Duration::from_millis(500));

    let url = format!("https://localhost:{port}/shooter/matches/dm");
    let connect = || {
        Game::Online(Box::new(
            Online::connect(url.clone(), Some(cert.clone())).unwrap(),
        ))
    };
    let (mut first, mut second) = (connect(), connect());
    let forward = Intent {
        forward: 1,
        ..Intent::IDLE
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut start = None;
    let mut moved = 0.0f32;
    while Instant::now() < deadline {
        first.tick(forward).unwrap();
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
    stop.blocking_send(()).unwrap();
    host.join().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}
