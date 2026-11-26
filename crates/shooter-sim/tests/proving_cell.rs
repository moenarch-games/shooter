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

use std::collections::BTreeMap;

use shooter_sim::maps::MapId;
use shooter_sim::mechanics::fixtures;
use shooter_sim::physics::Vec3i;
use shooter_sim::{Arena, Assembly, Event, Rules, Session};

/// Ticks a second.
const HZ: u32 = 60;

/// The locomotion a playtest gives every fighter.
#[derive(Clone, Copy, Debug)]
enum Drive {
    /// The named loadouts the bots pick (legs, on the interim drive).
    Legs,
    Wheels,
    Tracks,
}

impl Drive {
    const ALL: [Self; 3] = [Self::Legs, Self::Wheels, Self::Tracks];

    fn build(self) -> Option<Assembly> {
        match self {
            Self::Legs => None,
            Self::Wheels => Some(fixtures::armed_car()),
            Self::Tracks => Some(fixtures::armed_tank()),
        }
    }
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

/// How the lives that started on one spawn point went.
#[derive(Clone, Copy, Debug, Default)]
struct SpawnStats {
    lives: u32,
    respawns: u32,
    /// Respawns first hit by another fighter within 2 s (the match's first spawns, all at
    /// once, do not count).
    hit_early: u32,
    /// Lived 20 s and never got 3 m from the spawn point.
    stranded: u32,
}

/// What playtests measured.
#[derive(Debug, Default)]
struct Playtest {
    shots: u32,
    kills: u32,
    /// Ticks from each spawn to its fighter's first exchange of fire (hit or hit by another).
    contact: Vec<u32>,
    /// Respawned lives ended within 3 s.
    spawn_deaths: u32,
    /// By spawn point index.
    spawns: BTreeMap<usize, SpawnStats>,
    /// Kills by the victim's zone, and by the killer's.
    died_in: BTreeMap<&'static str, u32>,
    killed_from: BTreeMap<&'static str, u32>,
    /// Ticks alive fighters spent in each zone.
    time_in: BTreeMap<&'static str, u32>,
    /// The farthest each fighter got from its first spawn.
    farthest: Vec<f32>,
}

impl Playtest {
    fn median_contact(&self) -> Option<f32> {
        let mut sorted = self.contact.clone();
        sorted.sort_unstable();
        sorted
            .get(sorted.len() / 2)
            .map(|ticks| *ticks as f32 / HZ as f32)
    }

    fn hit_early(&self) -> u32 {
        self.spawns.values().map(|spawn| spawn.hit_early).sum()
    }

    fn stranded(&self) -> u32 {
        self.spawns.values().map(|spawn| spawn.stranded).sum()
    }

    fn lives(&self) -> u32 {
        self.spawns.values().map(|spawn| spawn.lives).sum()
    }

    /// The largest share of kills made from one position (a zone other than the open floor,
    /// which is most of the cell rather than a position).
    fn top_position(&self) -> (&'static str, f32) {
        let total = self.killed_from.values().sum::<u32>().max(1) as f32;
        self.killed_from
            .iter()
            .filter(|(zone, _)| **zone != "floor")
            .max_by_key(|(_, kills)| **kills)
            .map_or(("", 0.0), |(zone, kills)| (*zone, *kills as f32 / total))
    }

    fn merge(&mut self, other: Self) {
        self.shots += other.shots;
        self.kills += other.kills;
        self.contact.extend(other.contact);
        self.spawn_deaths += other.spawn_deaths;
        for (spawn, stats) in other.spawns {
            let into = self.spawns.entry(spawn).or_default();
            into.lives += stats.lives;
            into.respawns += stats.respawns;
            into.hit_early += stats.hit_early;
            into.stranded += stats.stranded;
        }
        for (into, from) in [
            (&mut self.died_in, other.died_in),
            (&mut self.killed_from, other.killed_from),
            (&mut self.time_in, other.time_in),
        ] {
            for (zone, n) in from {
                *into.entry(zone).or_default() += n;
            }
        }
        self.farthest.extend(other.farthest);
    }
}

/// The spawn point `feet` stands on.
fn spawn_index(arena: &Arena, feet: [f32; 3]) -> usize {
    let points = arena.spawn_points();
    (0..points.len())
        .min_by_key(|index| {
            let point = points[*index].feet;
            ((point.x as f32 - feet[0]).hypot(point.z as f32 - feet[2])) as i64
        })
        .unwrap()
}

/// One life in progress.
#[derive(Clone, Copy)]
struct Life {
    spawn: usize,
    at: [f32; 3],
    tick: u32,
    met: bool,
    farthest: f32,
}

/// `fighters` bots on `drive` for `seconds` on the proving cell, seeded `seed`.
fn playtest(fighters: u8, drive: Drive, seed: u64, seconds: u32) -> Playtest {
    let rules = Rules {
        slots: fighters,
        frag_limit: 0,
        time_limit_ticks: 0,
        ..Rules::deathmatch(seed)
    };
    let mut session = Session::on_map(MapId::ProvingCell, rules).unwrap();
    if let Some(build) = drive.build() {
        for slot in 0..fighters {
            session.rebuild(slot, build).unwrap();
        }
    }
    let arena = session.arena.clone();
    let slots = usize::from(fighters);
    let mut out = Playtest {
        farthest: vec![0.0; slots],
        ..Playtest::default()
    };
    let starts: Vec<[f32; 3]> = session.fighters.iter().map(|f| f.feet()).collect();
    let mut lives: Vec<Option<Life>> = starts
        .iter()
        .map(|feet| {
            Some(Life {
                spawn: spawn_index(&arena, *feet),
                at: *feet,
                tick: 0,
                met: false,
                farthest: 0.0,
            })
        })
        .collect();
    // Where each fighter stood on the last tick it lived (a dead robot's feet are not its last).
    let mut last = starts.clone();
    let end = |out: &mut Playtest, life: Life, tick: u32| {
        let stats = out.spawns.entry(life.spawn).or_default();
        stats.lives += 1;
        stats.respawns += u32::from(life.tick > 0);
        if tick - life.tick >= 20 * HZ && life.farthest < 300.0 {
            stats.stranded += 1;
        }
    };
    for _ in 0..seconds * HZ {
        session.step().unwrap();
        let tick = session.ticks();
        for event in &session.events {
            match *event {
                Event::Fired { .. } => out.shots += 1,
                Event::Spawned { slot, .. } => {
                    let feet = session.fighters[usize::from(slot)].feet();
                    lives[usize::from(slot)] = Some(Life {
                        spawn: spawn_index(&arena, feet),
                        at: feet,
                        tick,
                        met: false,
                        farthest: 0.0,
                    });
                }
                Event::Hit {
                    victim,
                    attacker: Some(attacker),
                    ..
                } if attacker != victim => {
                    for (slot, was_hit) in [(victim, true), (attacker, false)] {
                        if let Some(life) = lives[usize::from(slot)].as_mut().filter(|l| !l.met) {
                            life.met = true;
                            out.contact.push(tick - life.tick);
                            if was_hit && life.tick > 0 && tick - life.tick < 2 * HZ {
                                out.spawns.entry(life.spawn).or_default().hit_early += 1;
                            }
                        }
                    }
                }
                Event::Killed { victim, killer, .. } => {
                    out.kills += 1;
                    let victim = usize::from(victim);
                    if let Some(life) = lives[victim].take() {
                        if life.tick > 0 && tick - life.tick < 3 * HZ {
                            out.spawn_deaths += 1;
                        }
                        end(&mut out, life, tick);
                    }
                    *out.died_in.entry(zone(last[victim])).or_default() += 1;
                    if let Some(killer) = killer.filter(|killer| usize::from(*killer) != victim) {
                        *out.killed_from
                            .entry(zone(session.fighters[usize::from(killer)].feet()))
                            .or_default() += 1;
                    }
                }
                _ => {}
            }
        }
        for (slot, fighter) in session.fighters.iter().enumerate() {
            if fighter.alive() {
                let feet = fighter.feet();
                last[slot] = feet;
                *out.time_in.entry(zone(feet)).or_default() += 1;
                let from = starts[slot];
                out.farthest[slot] =
                    out.farthest[slot].max((feet[0] - from[0]).hypot(feet[2] - from[2]));
                if let Some(life) = lives[slot].as_mut() {
                    life.farthest = life
                        .farthest
                        .max((feet[0] - life.at[0]).hypot(feet[2] - life.at[2]));
                }
            }
        }
    }
    let tick = session.ticks();
    for life in lives.into_iter().flatten() {
        end(&mut out, life, tick);
    }
    out
}

fn report(label: &str, test: &Playtest) {
    let share = |map: &BTreeMap<&str, u32>| {
        let total = map.values().sum::<u32>().max(1) as f32;
        map.iter()
            .map(|(zone, n)| format!("{zone} {:.0}%", 100.0 * *n as f32 / total))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut farthest = test.farthest.clone();
    farthest.sort_by(f32::total_cmp);
    println!(
        "{label}: {} shots, {} kills; first contact median {:.1} s over {} lives; {} spawn \
         deaths (respawned, <3 s), {} respawns hit (<2 s), {} stranded; least roamed {:.0} cm",
        test.shots,
        test.kills,
        test.median_contact().unwrap_or(f32::NAN),
        test.lives(),
        test.spawn_deaths,
        test.hit_early(),
        test.stranded(),
        farthest.first().copied().unwrap_or(0.0),
    );
    println!("  kills from: {}", share(&test.killed_from));
    println!("  deaths in:  {}", share(&test.died_in));
    println!("  time in:    {}", share(&test.time_in));
    let spawns: Vec<String> = test
        .spawns
        .iter()
        .filter(|(_, stats)| stats.hit_early > 0 || stats.stranded > 0)
        .map(|(spawn, stats)| {
            format!(
                "#{spawn} {}/{} respawns hit early, {} stranded",
                stats.hit_early, stats.respawns, stats.stranded
            )
        })
        .collect();
    if !spawns.is_empty() {
        println!("  spawns:     {}", spawns.join("; "));
    }
}

/// 2 and 6 fighters on each drive for 90 s. On every drive no robot is stranded at a spawn
/// point (shooter#151: the booths' and the passage end's spawns held wheels and tracks) and no
/// respawn is killed within 3 s. On legs a fight is found within 20 s at the median (a
/// close-quarters map), no one position (the open floor aside) makes most of the kills and
/// every fighter gets well away from where it started; six on wheels or tracks fight and most
/// of them roam. (Two on wheels or tracks may not find each other: a skid-steered chassis turns
/// in place only slowly, shooter#170, and wheels do not climb the kit's ramps.)
#[test]
fn duels_and_skirmishes_find_fights_without_spawn_traps() {
    let seed = 3;
    let mut failures = Vec::new();
    for drive in Drive::ALL {
        for fighters in [2, 6] {
            let label = format!("{fighters} x {drive:?} (seed {seed})");
            let test = playtest(fighters, drive, seed, 90);
            report(&label, &test);
            let legs = matches!(drive, Drive::Legs);
            let (top, share) = test.top_position();
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
    for drive in Drive::ALL {
        for fighters in [2, 6] {
            let mut sum = Playtest::default();
            for seed in 1..=5 {
                sum.merge(playtest(fighters, drive, seed, 120));
            }
            report(&format!("{fighters} x {drive:?} (seeds 1-5, 120 s)"), &sum);
        }
    }
}

/// A wheeled or tracked robot driven alone from `from` (feet) toward each of `route` in turn,
/// steered as a bot steers ([`shooter_sim::bots::steer`]) through the build's preset: where it
/// ends up after at most `seconds`, and the highest it got.
fn drive_route(build: Assembly, from: Vec3i, route: &[Vec3i], seconds: u32) -> ([f32; 3], f32) {
    use shooter_sim::bots::steer;
    use shooter_sim::session::LOCAL_PLAYER;
    use shooter_sim::{Intent, Spawn};
    let first = route[0];
    let arena = Arena {
        spawn: from,
        spawn_yaw: shooter_sim::trig::yaw_from_vector(first.x - from.x, first.z - from.z).unwrap(),
        spawns: Vec::<Spawn>::new(),
        ..Arena::proving_cell()
    };
    let mut session = Session::with_rules(arena, Rules::solo()).unwrap();
    assert!(session.configure(LOCAL_PLAYER, build));
    let mut next = 0;
    let mut highest = f32::MIN;
    for _ in 0..seconds * HZ {
        let feet = session.local().feet();
        highest = highest.max(feet[1]);
        let goal = route[next];
        let (dx, dz) = (goal.x as f32 - feet[0], goal.z as f32 - feet[2]);
        if dx.hypot(dz) < 100.0 {
            next += 1;
            if next == route.len() {
                break;
            }
            continue;
        }
        let toward = shooter_sim::trig::yaw_from_vector(dx as i32, dz as i32).unwrap_or(0);
        let (forward, strafe) = steer(session.local().heading(), toward, 1);
        let intent = session.with_preset(
            0,
            Intent {
                forward,
                strafe,
                ..Intent::IDLE
            },
        );
        session.tick(intent).unwrap();
    }
    (session.local().feet(), highest)
}

/// Tracks climb the cell's ramps on their own actuators: up the stage's north ramp onto the
/// stage (the rail capacitor's) and up the north gallery's ramp onto its deck (the thrusters');
/// no contested place needs legs, a jump or the grapple. Wheels do not yet: the car (armed or
/// bare) gets about half a metre up the kit's 1:4 ramps and rolls back, on every kit-built map
/// (a locomotion matter, shooter#174, not this layout's); the cell's floor keeps its fights within reach of
/// it.
#[test]
fn tracks_drive_onto_the_stage_and_the_gallery() {
    use shooter_sim::proving_cell::{DECK_TOP, STAGE_TOP};
    let stage = [Vec3i::new(0, 0, -1_000), Vec3i::new(0, STAGE_TOP, -100)];
    let gallery = [
        Vec3i::new(-1_400, 0, 0),
        Vec3i::new(-1_400, DECK_TOP, -1_400),
    ];
    let (at, _) = drive_route(fixtures::armed_tank(), Vec3i::new(0, 0, -1_300), &stage, 20);
    assert!(at[1] > (STAGE_TOP - 30) as f32, "onto the stage: {at:?}");
    let (at, _) = drive_route(
        fixtures::armed_tank(),
        Vec3i::new(-1_400, 0, 150),
        &gallery,
        30,
    );
    assert!(at[1] > (DECK_TOP - 30) as f32, "onto the gallery: {at:?}");
    let (at, highest) = drive_route(fixtures::armed_car(), Vec3i::new(0, 0, -1_300), &stage, 20);
    println!("the car up the stage's ramp: at {at:?}, highest {highest}");
    assert!(
        highest < 100.0,
        "wheels climb the ramp now: drive them onto the stage here"
    );
}
