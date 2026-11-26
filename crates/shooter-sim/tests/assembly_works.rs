//! Playtests of the assembly works (shooter#152), the medium arena and the menu's flagship: a
//! deterministic bot match of 4 and 12 fighters on legged (the named loadouts), wheeled, tracked
//! and mixed builds, read for what a playtest looks for (`tests/playtest`): how soon a fresh
//! spawn meets a fight (and whether it is shot before it can), whether any spawn strands a
//! robot, where kills happen, from where and at what range, and whether every build gets round
//! the map.
//!
//! `cargo test -p shooter-sim --release --test assembly_works -- --nocapture --include-ignored`
//! prints the measurements (the ignored report plays five seeds of two minutes each); the
//! assertions hold the layout's promises.
// The platform's floats only read results here; the simulation uses libm (clippy.toml).
#![allow(clippy::disallowed_methods)]

mod playtest;

use playtest::{Drive, Playtest, drive_route, report, seen_from};
use shooter_sim::Arena;
use shooter_sim::maps::MapId;
use shooter_sim::mechanics::fixtures;
use shooter_sim::physics::Vec3i;

const ALL: [Drive; 5] = [
    Drive::Legs,
    Drive::Wheels,
    Drive::Steered,
    Drive::Tracks,
    Drive::Mixed,
];

/// `fighters` bots on `drive` for `seconds` on the works, seeded `seed`.
fn playtest(fighters: u8, drive: Drive, seed: u64, seconds: u32) -> Playtest {
    playtest::playtest(MapId::AssemblyWorks, zone, fighters, drive, seed, seconds)
}

/// Whether `x`, `z` lies over the north half's mezzanine ramp or (since shooter#152) its loading
/// ramp onto the bridge.
fn on_ramp(x: f32, z: f32) -> bool {
    ((-1_800.0..=-1_400.0).contains(&x) && (-1_400.0..=-200.0).contains(&z))
        || ((200.0..=1_400.0).contains(&x) && (-800.0..=-600.0).contains(&z))
}

/// The part of the works a point lies in (by its feet), the same names before and after a
/// layout change so playtests compare.
fn zone([x, y, z]: [f32; 3]) -> &'static str {
    let (ax, az) = (x.abs(), z.abs());
    if y >= 450.0 {
        "tower"
    } else if y >= 250.0 && ax <= 220.0 && az <= 1_400.0 {
        "bridge"
    } else if y >= 250.0 {
        "mezzanine"
    } else if y >= 50.0 && ax <= 300.0 && az <= 300.0 {
        "cradle"
    } else if y >= 50.0 && az >= 1_900.0 {
        "steps"
    } else if y >= 50.0 && (on_ramp(x, z) || on_ramp(-x, -z)) {
        "ramp"
    } else if y >= 50.0 {
        // In the air, or on cover, crates or a container.
        "raised"
    } else if az >= 1_830.0 {
        "bay"
    } else if ax >= 2_300.0 {
        "side"
    } else {
        "floor"
    }
}

/// The playtest report: five seeds of two minutes for every drive at 4 and 12 fighters, summed.
#[test]
#[ignore = "a report: run with --release --nocapture"]
fn playtest_report() {
    for drive in ALL {
        for fighters in [4, 12] {
            let mut sum = Playtest::default();
            for seed in 1..=5 {
                sum.merge(playtest(fighters, drive, seed, 120));
            }
            report(&format!("{fighters} x {drive:?} (seeds 1-5, 120 s)"), &sum);
        }
    }
}

/// 4 and 12 fighters on each drive for 90 s. On every drive no robot is stranded at a spawn
/// point (shooter#152: the bays' spawn points face their ways out, and no 1 m slot by the crane
/// tower wedges a chassis) and no respawn is killed within 3 s. On legs a fight is found within
/// 20 s at the median, no one position (the open floor aside) makes most of the kills and every
/// fighter gets well away from where it started; twelve on wheels, tracks or mixed builds fight
/// and most of them roam. (Four on wheels may not find each other in 90 s: a skid-steered
/// chassis turns in place only slowly, shooter#170.)
#[test]
fn skirmishes_and_full_matches_find_fights_without_spawn_traps() {
    let seed = 3;
    let mut failures = Vec::new();
    for drive in ALL {
        for fighters in [4, 12] {
            let label = format!("{fighters} x {drive:?} (seed {seed})");
            let test = playtest(fighters, drive, seed, 90);
            report(&label, &test);
            let legs = drive.legs();
            let (top, share) = test.top_position("floor");
            let median = test.median_contact().unwrap_or(f32::INFINITY);
            let roamed = test.farthest.iter().filter(|far| **far > 500.0).count();
            for (broken, what) in [
                (test.stranded() > 0, format!("{} stranded", test.stranded())),
                (
                    test.spawn_deaths > 0,
                    format!("{} spawn deaths", test.spawn_deaths),
                ),
                (legs && median >= 20.0, format!("first contact {median} s")),
                (
                    legs && test.kills >= 6 && share > 0.5,
                    format!("{share} of kills from {top}"),
                ),
                (
                    legs && test.farthest.iter().any(|far| *far < 600.0),
                    format!("roamed {:?}", test.farthest),
                ),
                (
                    !legs
                        && fighters == 12
                        && (test.kills == 0 || roamed * 2 < test.farthest.len()),
                    format!("{} kills, roamed {:?}", test.kills, test.farthest),
                ),
            ] {
                if broken {
                    failures.push(format!("{label}: {what}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The works' high places, by name: the bridge's middle (the rail capacitor's), its north end,
/// the north mezzanine's west, middle and east and the north crane tower's top (the thrusters').
fn high_places() -> [(&'static str, Vec3i); 6] {
    use shooter_sim::assembly_works::{DECK_TOP, TOWER_TOP};
    [
        ("bridge middle", Vec3i::new(0, DECK_TOP, 0)),
        ("bridge north", Vec3i::new(0, DECK_TOP, -1_000)),
        ("mezzanine west", Vec3i::new(-1_700, DECK_TOP, -1_600)),
        ("mezzanine middle", Vec3i::new(-600, DECK_TOP, -1_600)),
        ("mezzanine east", Vec3i::new(500, DECK_TOP, -1_600)),
        ("tower", Vec3i::new(2_200, TOWER_TOP, -2_200)),
    ]
}

/// No high place commands the floor and none hides from it: from each one a fighter sees at most
/// half of the floor within 30 m (a bot's sight, the rail gun's useful reach) and at least a
/// sixth of it (sight is mutual: that much of the floor can shoot back). shooter#152 measured
/// the bridge's middle at 43% before its changes and 42% after, the tower at 27% and 26%.
#[test]
fn the_high_ground_sees_and_is_seen_from_the_floor() {
    let arena = Arena::assembly_works();
    let world = arena.build_world(shooter_sim::player::GRAVITY).unwrap();
    let floor = playtest::floor_points(&arena);
    for (name, at) in high_places() {
        let seen = seen_from(&world, at, &floor, 3_000).len();
        let share = seen as f32 / floor.len() as f32;
        println!(
            "{name}: sees {seen} of {} floor points within 30 m ({:.0}%)",
            floor.len(),
            100.0 * share
        );
        assert!((1.0 / 6.0..=0.5).contains(&share), "{name}: {share}");
    }
}

/// Tracks drive onto the gantry bridge both ways on their own actuators: up the mezzanine's ramp
/// and along its deck, and (shooter#152) up the loading ramp from the floor east of the bridge
/// straight onto its edge; no contested high place needs legs, a jump or the grapple. Wheels
/// climb the kit's 1:4 ramps too since shooter#174 (before it the car got about half a metre up
/// and rolled back): the armed car drives up the loading ramp onto the bridge as the tank does,
/// and so does the steer-car on its steering servos (shooter#170). (By the mezzanine both climb
/// onto the deck but neither makes the deck's turn onto the bridge: the car's skid steer turns
/// slowly, and the steer-car, steering, ends about 0.4 m below the deck by the turn.)
#[test]
fn wheels_and_tracks_drive_onto_the_bridge() {
    use shooter_sim::assembly_works::DECK_TOP;
    let loading = [Vec3i::new(1_600, 0, -700), Vec3i::new(0, DECK_TOP, -700)];
    let mezzanine = [
        Vec3i::new(-1_600, 0, 0),
        Vec3i::new(-1_600, DECK_TOP, -1_600),
        Vec3i::new(0, DECK_TOP, -1_600),
        Vec3i::new(0, DECK_TOP, -800),
    ];
    for build in [
        fixtures::armed_tank(),
        fixtures::armed_car(),
        shooter_sim::robot::STEER_CAR,
    ] {
        let (at, _) = drive_route(
            Arena::assembly_works(),
            build,
            Vec3i::new(2_200, 0, -700),
            &loading,
            25,
        );
        assert!(
            at[1] > (DECK_TOP - 30) as f32,
            "up the loading ramp: {at:?}"
        );
        assert!(at[0].abs() < 250.0, "onto the bridge: {at:?}");
    }
    let (at, _) = drive_route(
        Arena::assembly_works(),
        fixtures::armed_tank(),
        Vec3i::new(-1_600, 0, 400),
        &mezzanine,
        60,
    );
    assert!(
        at[1] > (DECK_TOP - 30) as f32 && at[0].abs() < 250.0,
        "up the mezzanine ramp and onto the bridge: {at:?}"
    );
}
