//! Playtests of the proving cell (shooter#151), the small close-quarters arena: deterministic bot
//! matches of 2 and 6 fighters on legged (the named loadouts), wheeled and tracked builds, read
//! for what a playtest looks for: how soon a fresh spawn meets a fight (and whether it is shot
//! before it can), whether any spawn strands a robot, where kills happen and from where, and
//! whether every build gets round the map.
//!
//! `cargo test -p shooter-sim --release --test proving_cell -- --nocapture --include-ignored`
//! prints the measurements (the ignored report plays five seeds of two minutes each); the
//! assertions hold the layout's promises.
// The platform's floats only read results here; the simulation uses libm (clippy.toml).
#![allow(clippy::disallowed_methods)]

mod playtest;

use playtest::{Drive, Playtest, drive_route, report};
use shooter_sim::Arena;
use shooter_sim::maps::MapId;
use shooter_sim::mechanics::fixtures;
use shooter_sim::physics::Vec3i;

const ALL: [Drive; 4] = [Drive::Legs, Drive::Wheels, Drive::Steered, Drive::Tracks];

/// `fighters` bots on `drive` for `seconds` on the proving cell, seeded `seed`.
fn playtest(fighters: u8, drive: Drive, seed: u64, seconds: u32) -> Playtest {
    playtest::playtest(MapId::ProvingCell, zone, fighters, drive, seed, seconds)
}

/// The part of the cell a point lies in (by its feet), the same names before and after a layout
/// change so playtests compare.
fn zone([x, y, z]: [f32; 3]) -> &'static str {
    let (ax, az) = (x.abs(), z.abs());
    if y >= 150.0 && ax <= 320.0 && az <= 320.0 {
        "stage"
    } else if y >= 250.0 {
        "gallery"
    } else if y >= 50.0 {
        "raised"
    } else if ax >= 870.0 && az >= 870.0 {
        "corner"
    } else if az >= 1_150.0 {
        "under-gallery"
    } else if ax >= 1_150.0 {
        "side"
    } else {
        "floor"
    }
}

/// 2 and 6 fighters on each drive for 90 s. On every drive no robot is stranded at a spawn
/// point (shooter#151: the booths' and the passage end's spawns held wheels and tracks) and no
/// respawn is killed within 3 s. On legs a fight is found within 20 s at the median (a
/// close-quarters map), no one position (the open floor aside) makes most of the kills and
/// every fighter gets well away from where it started; six on wheels or tracks fight and most
/// of them roam. (Two on wheels or tracks may not find each other: a skid-steered chassis turns
/// in place only slowly, shooter#170.)
#[test]
fn duels_and_skirmishes_find_fights_without_spawn_traps() {
    let seed = 3;
    let mut failures = Vec::new();
    for drive in ALL {
        for fighters in [2, 6] {
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
                    !legs && fighters == 6 && (test.kills == 0 || roamed * 2 < test.farthest.len()),
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

/// The playtest report: five seeds of two minutes for every drive at 2 and 6 fighters, summed.
#[test]
#[ignore = "a report: run with --release --nocapture"]
fn playtest_report() {
    for drive in ALL {
        for fighters in [2, 6] {
            let mut sum = Playtest::default();
            for seed in 1..=5 {
                sum.merge(playtest(fighters, drive, seed, 120));
            }
            report(&format!("{fighters} x {drive:?} (seeds 1-5, 120 s)"), &sum);
        }
    }
}

/// Tracks and wheels climb the cell's ramps on their own actuators: up the stage's north ramp
/// onto the stage (the rail capacitor's) and up the north gallery's ramp onto its deck (the
/// thrusters'); no contested place needs legs, a jump or the grapple. Wheels since shooter#174
/// (before it the car, armed or bare, got about half a metre up the kit's 1:4 ramps and rolled
/// back): the armed tank, the armed car and the bare car all drive up both, and so does the
/// steer-car on its steering servos (shooter#170).
#[test]
fn wheels_and_tracks_drive_onto_the_stage_and_the_gallery() {
    use shooter_sim::proving_cell::{DECK_TOP, STAGE_TOP};
    let stage = [Vec3i::new(0, 0, -1_000), Vec3i::new(0, STAGE_TOP, -100)];
    let gallery = [
        Vec3i::new(-1_400, 0, 0),
        Vec3i::new(-1_400, DECK_TOP, -1_400),
    ];
    for build in [
        fixtures::armed_tank(),
        fixtures::armed_car(),
        fixtures::car(),
        shooter_sim::robot::STEER_CAR,
    ] {
        let (at, _) = drive_route(
            Arena::proving_cell(),
            build,
            Vec3i::new(0, 0, -1_300),
            &stage,
            20,
        );
        assert!(at[1] > (STAGE_TOP - 30) as f32, "onto the stage: {at:?}");
        let (at, _) = drive_route(
            Arena::proving_cell(),
            build,
            Vec3i::new(-1_400, 0, 150),
            &gallery,
            30,
        );
        assert!(at[1] > (DECK_TOP - 30) as f32, "onto the gallery: {at:?}");
    }
}
