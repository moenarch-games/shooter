//! Per-tick state hashes of the scripted practice match: the clients' profile run's simulation
//! (practice rules on seed 1, the local fighter played by the replays' scripted person 0). One
//! line per map and tick: the map, the tick, then hashes of the whole snapshot, its fighters,
//! rockets, pellets, debris and scenery. Native and `wasm32` builds must print the same lines
//! (`scripts/cross-build-hashes.sh`, shooter#120).
//!
//! `cargo run -p shooter-sim --example state_hashes -- [MAP|all] [TICKS] [DUMP_TICK]`: every
//! catalogued map by default, 600 ticks each; with `DUMP_TICK`, that tick's whole snapshot is
//! printed too (to see which values differ).

use shooter_sim::{MapId, Rules, Session, scenarios::scripted_intent};

/// The clients' practice seed (`shooter_client::game::PRACTICE_SEED`).
const PRACTICE_SEED: u64 = 1;

/// FNV-1a over the value's `Debug` text, which writes every float exactly.
fn hash(value: &impl std::fmt::Debug) -> u64 {
    format!("{value:?}")
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        })
}

fn main() {
    let mut args = std::env::args().skip(1);
    let maps: Vec<MapId> = match args.next().as_deref() {
        None | Some("all") => MapId::ALL.to_vec(),
        Some(id) => vec![id.parse().expect("a catalogued map")],
    };
    let ticks: u32 = args
        .next()
        .map_or(600, |ticks| ticks.parse().expect("a tick count"));
    let dump: Option<u32> = args
        .next()
        .map(|tick| tick.parse().expect("a tick to dump"));
    for map in maps {
        let mut session =
            Session::on_map(map, Rules::practice(PRACTICE_SEED)).expect("the map builds");
        for tick in 0..ticks {
            session
                .tick(scripted_intent(0, tick))
                .expect("the match ticks");
            let snapshot = session.snapshot();
            if dump == Some(snapshot.tick) {
                println!("{snapshot:#?}");
            }
            println!(
                "{map} {} {:016x} fighters {:016x} rockets {:016x} pellets {:016x} debris {:016x} scenery {:016x}",
                snapshot.tick,
                hash(&snapshot),
                hash(&snapshot.fighters),
                hash(&snapshot.rockets),
                hash(&snapshot.pellets),
                hash(&snapshot.debris),
                hash(&snapshot.scenery),
            );
        }
    }
}
