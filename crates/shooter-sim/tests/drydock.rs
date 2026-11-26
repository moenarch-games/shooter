//! Playtests of the drydock (shooter#153), the large arena: a deterministic bot match of 8 and
//! 12 fighters on legged (the named loadouts), wheeled, tracked and mixed builds, read for what a
//! playtest looks for (`tests/playtest`): how soon a fresh spawn meets a fight (and whether it is
//! shot before it can), whether any spawn strands a robot, where kills happen, from where and at
//! what range, and whether every build gets round the dock.
//!
//! `cargo test -p shooter-sim --release --test drydock -- --nocapture --include-ignored` prints
//! the measurements (the ignored report plays five seeds of two minutes each); the assertions
//! hold the layout's promises.
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

/// `fighters` bots on `drive` for `seconds` on the dock, seeded `seed`.
fn playtest(fighters: u8, drive: Drive, seed: u64, seconds: u32) -> Playtest {
    playtest::playtest(MapId::Drydock, zone, fighters, drive, seed, seconds)
}

/// The part of the dock a point lies in (by its feet), the same names before and after a layout
/// change so playtests compare. The ships' centre lines are 22 m off the dock's, their hulls
/// 12 m wide; the gantries run between the hulls and the corridor.
fn zone([x, y, z]: [f32; 3]) -> &'static str {
    let (ax, az) = (x.abs(), z.abs());
    let by_hull = (1_600.0..=2_800.0).contains(&ax) && az <= 2_800.0;
    if y >= 550.0 && by_hull {
        "upper deck"
    } else if y >= 350.0 && by_hull {
        "foredeck"
    } else if y >= 350.0 && (1_100.0..1_600.0).contains(&ax) {
        "gantry"
    } else if y >= 350.0 {
        // On the pylon, or thrown or swinging.
        "high"
    } else if y >= 50.0 && (1_100.0..1_600.0).contains(&ax) && az >= 1_300.0 {
        "gantry ramp"
    } else if y >= 50.0 {
        // In the air, or on cover, crates, a container or a hull's side.
        "raised"
    } else if az >= 2_800.0 {
        "end lane"
    } else if ax >= 2_800.0 {
        "outer lane"
    } else {
        "corridor"
    }
}

/// The dock's high places, by name: each ship's 6 m deck (the freighter's stern deck, the
/// cutter's bridge), foredeck and gantry, the cutter's prow step and the crane pylon's foot.
fn high_places() -> [(&'static str, Vec3i); 7] {
    use shooter_sim::drydock::{FOREDECK_TOP, GANTRY_TOP, GANTRY_X, PROW_TOP, SHIP_X, STERN_TOP};
    [
        ("freighter stern deck", Vec3i::new(SHIP_X, STERN_TOP, 2_000)),
        ("cutter bridge", Vec3i::new(-SHIP_X, STERN_TOP, -800)),
        (
            "freighter foredeck",
            Vec3i::new(SHIP_X, FOREDECK_TOP, -1_000),
        ),
        ("cutter foredeck", Vec3i::new(-SHIP_X, FOREDECK_TOP, 800)),
        ("freighter gantry", Vec3i::new(GANTRY_X, GANTRY_TOP, 0)),
        ("cutter gantry", Vec3i::new(-GANTRY_X, GANTRY_TOP, 0)),
        ("cutter prow", Vec3i::new(-SHIP_X, PROW_TOP, 2_600)),
    ]
}

/// The share of the dock floor each high place sees within `range`.
fn sightlines(arena: &Arena, range: i32) -> Vec<(&'static str, f32)> {
    let world = arena.build_world(shooter_sim::player::GRAVITY).unwrap();
    let floor = playtest::floor_points(arena);
    high_places()
        .into_iter()
        .map(|(name, at)| {
            let seen = seen_from(&world, at, &floor, range).len();
            (name, seen as f32 / floor.len() as f32)
        })
        .collect()
}

/// No high place commands the dock floor and none hides from it: from each one a fighter sees at
/// most half of the floor within 40 m (half the dock across) and at least a tenth of it (sight
/// is mutual: that much of the floor can shoot back). Hybrid symmetry (D2.i55.2): the two ships'
/// 6 m decks, foredecks and gantries each see about as much as their twin on the other ship,
/// within 10 percentage points, though the ships differ.
#[test]
fn the_high_ground_sees_and_is_seen_from_the_floor() {
    let seen = sightlines(&Arena::drydock(), 4_000);
    for (name, share) in &seen {
        println!(
            "{name}: sees {:.0}% of the floor within 40 m",
            100.0 * share
        );
    }
    for (name, share) in &seen {
        assert!((0.1..=0.5).contains(share), "{name}: {share}");
    }
    for pair in seen[..6].chunks(2) {
        let [(a, x), (b, y)] = [pair[0], pair[1]];
        assert!((x - y).abs() <= 0.1, "{a} {x} against {b} {y}");
    }
}

/// The playtest report: five seeds of two minutes for every drive at 8 and 12 fighters, summed.
#[test]
#[ignore = "a report: run with --release --nocapture"]
fn playtest_report() {
    for drive in ALL {
        for fighters in [8, 12] {
            let mut sum = Playtest::default();
            for seed in 1..=5 {
                sum.merge(playtest(fighters, drive, seed, 120));
            }
            report(&format!("{fighters} x {drive:?} (seeds 1-5, 120 s)"), &sum);
            let mut early: Vec<String> = sum
                .early_from
                .iter()
                .map(|(spawn, from)| format!("#{spawn} from ({:.0}, {:.0})", from[0], from[2]))
                .collect();
            early.sort();
            if !early.is_empty() {
                println!("  early from: {}", early.join("; "));
            }
        }
    }
}

/// 8 and 12 fighters on each drive for 90 s. On every drive no respawn is killed within 3 s, and
/// on legs, tracks and mixed builds no robot is stranded at a spawn point (shooter#153: the
/// corridor's spawn points face their ways out and the one in the pylon's lee stands clear of
/// it). On legs a fight is found within 20 s at the median, no one position (the open corridor
/// aside) makes most of the kills and every fighter gets well away from where it started; on
/// wheels, tracks or mixed builds the fighters fight and most of them roam. A wheeled robot may
/// still sit out a spawn turning toward a waypoint behind it (a skid-steered chassis turns in
/// place only slowly, shooter#170): at most one of its lives in the match.
#[test]
fn skirmishes_and_full_matches_find_fights_without_spawn_traps() {
    let seed = 3;
    let mut failures = Vec::new();
    for drive in ALL {
        for fighters in [8, 12] {
            let label = format!("{fighters} x {drive:?} (seed {seed})");
            let test = playtest(fighters, drive, seed, 90);
            report(&label, &test);
            let legs = drive.legs();
            let wheels = matches!(drive, Drive::Wheels);
            let (top, share) = test.top_position("corridor");
            let median = test.median_contact().unwrap_or(f32::INFINITY);
            let roamed = test.farthest.iter().filter(|far| **far > 500.0).count();
            for (broken, what) in [
                (
                    test.stranded() > u32::from(wheels),
                    format!("{} stranded", test.stranded()),
                ),
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
                    !legs && (test.kills == 0 || roamed * 2 < test.farthest.len()),
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

/// Tracks drive onto both ships' hulls on their own actuators, by the routes the cutter's
/// shooter#153 layout adds beside the gantries' ramps and gangways: up its gantry's bow ramp,
/// across onto the prow step and up the prow's ramp onto the foredeck; and from the foredeck up
/// the bridge's fore ramp onto the 6 m bridge. No hull top needs legs, a jump, a pad or the
/// grapple. Wheels climb the kit's 1:4 ramps too since shooter#174 (before it the car got about
/// half a metre up and rolled back): the armed car drives up the gantry's 16 m bow ramp onto its
/// 4 m deck and from the foredeck up the fore ramp onto the bridge, the latter as the steer-car
/// does on its steering servos (shooter#170). (From the gantry across onto the prow step the car
/// does not steer: its skid steer turns slowly and it ends on the floor. The steer-car gets 2 m
/// up, at the prow step's height, but not up the prow's ramp onto the foredeck.)
#[test]
fn wheels_and_tracks_drive_onto_the_cutter() {
    use shooter_sim::drydock::{FOREDECK_TOP, GANTRY_TOP, GANTRY_X, PROW_TOP, SHIP_X, STERN_TOP};
    let (cutter, gantry) = (-SHIP_X, -GANTRY_X);
    let prow = [
        Vec3i::new(gantry, PROW_TOP, 1_950),
        Vec3i::new(cutter, PROW_TOP, 2_350),
        Vec3i::new(cutter, FOREDECK_TOP, 900),
    ];
    let (at, _) = drive_route(
        Arena::drydock(),
        fixtures::armed_tank(),
        Vec3i::new(gantry, 0, 3_400),
        &prow,
        60,
    );
    assert!(
        at[1] > (FOREDECK_TOP - 30) as f32 && (at[0] - cutter as f32).abs() < 400.0,
        "up the prow onto the foredeck: {at:?}"
    );
    let bridge = [
        Vec3i::new(cutter, FOREDECK_TOP, 500),
        Vec3i::new(cutter, STERN_TOP, -800),
    ];
    for build in [
        fixtures::armed_tank(),
        fixtures::armed_car(),
        shooter_sim::robot::STEER_CAR,
    ] {
        let (at, _) = drive_route(
            Arena::drydock(),
            build,
            Vec3i::new(cutter, FOREDECK_TOP, 1_100),
            &bridge,
            30,
        );
        assert!(
            at[1] > (STERN_TOP - 30) as f32 && (at[0] - cutter as f32).abs() < 200.0,
            "up the fore ramp onto the bridge: {at:?}"
        );
    }
    let (at, _) = drive_route(
        Arena::drydock(),
        fixtures::armed_car(),
        Vec3i::new(gantry, 0, 3_400),
        &[Vec3i::new(gantry, GANTRY_TOP, 1_000)],
        20,
    );
    assert!(
        at[1] > (GANTRY_TOP - 30) as f32 && (at[0] - gantry as f32).abs() < 200.0,
        "the car up the gantry's bow ramp onto its deck: {at:?}"
    );
}
