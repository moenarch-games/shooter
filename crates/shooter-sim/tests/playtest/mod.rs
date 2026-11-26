//! Deterministic bot playtests of a kit-built map (shooter#151, #152, #153), shared by the maps'
//! playtest files: bot matches of some fighters on legged (the named loadouts), wheeled, tracked
//! or mixed builds, read for what a playtest looks for: how soon a fresh spawn meets a fight (and
//! whether it is shot before it can), whether any spawn strands a robot, where kills happen and
//! from where, and whether every build gets round the map. Each map names its zones (by a
//! fighter's feet) so playtests before and after a layout change compare.
// The platform's floats only read results here; the simulation uses libm (clippy.toml).
#![allow(clippy::disallowed_methods, dead_code)]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use shooter_sim::maps::MapId;
use shooter_sim::mechanics::fixtures;
use shooter_sim::physics::Vec3i;
use shooter_sim::{Arena, Assembly, Event, Rules, Session};

/// Ticks a second.
pub const HZ: u32 = 60;

/// The locomotion a playtest gives its fighters.
#[derive(Clone, Copy, Debug)]
pub enum Drive {
    /// The named loadouts the bots pick (legs, on the interim drive).
    Legs,
    Wheels,
    Tracks,
    /// Representative mixed builds: slot by slot, the named loadouts, the armed car and the
    /// armed tank in turn.
    Mixed,
}

impl Drive {
    /// The build `slot` plays (none: the bot's named loadout).
    pub fn build(self, slot: u8) -> Option<Assembly> {
        match self {
            Self::Legs => None,
            Self::Wheels => Some(fixtures::armed_car()),
            Self::Tracks => Some(fixtures::armed_tank()),
            Self::Mixed => match slot % 3 {
                0 => None,
                1 => Some(fixtures::armed_car()),
                _ => Some(fixtures::armed_tank()),
            },
        }
    }

    pub fn legs(self) -> bool {
        matches!(self, Self::Legs)
    }
}

/// How the lives that started on one spawn point went.
#[derive(Clone, Copy, Debug, Default)]
pub struct SpawnStats {
    pub lives: u32,
    pub respawns: u32,
    /// Respawns first hit by another fighter within 2 s (the match's first spawns, all at
    /// once, do not count).
    pub hit_early: u32,
    /// Lived 20 s and never got 3 m from the spawn point.
    pub stranded: u32,
}

/// What playtests measured.
#[derive(Debug, Default)]
pub struct Playtest {
    pub shots: u32,
    pub kills: u32,
    /// Ticks from each spawn to its fighter's first exchange of fire (hit or hit by another).
    pub contact: Vec<u32>,
    /// Respawned lives ended within 3 s.
    pub spawn_deaths: u32,
    /// By spawn point index.
    pub spawns: BTreeMap<usize, SpawnStats>,
    /// Kills by the victim's zone, and by the killer's.
    pub died_in: BTreeMap<&'static str, u32>,
    pub killed_from: BTreeMap<&'static str, u32>,
    /// Ticks alive fighters spent in each zone.
    pub time_in: BTreeMap<&'static str, u32>,
    /// The farthest each fighter got from its first spawn.
    pub farthest: Vec<f32>,
    /// Kills by the distance between killer and victim (whole metres, in 10 m bands).
    pub kill_range: BTreeMap<u32, u32>,
    /// Ticks stepped and the wall time they took.
    pub ticks: u32,
    pub stepping: Duration,
    /// Each early hit on a respawn: its spawn point and where the attacker stood.
    pub early_from: Vec<(usize, [f32; 3])>,
    /// Ticks alive fighters spent in each 4 m square (by its south-west corner, in metres).
    pub cells: BTreeMap<(i32, i32), u32>,
}

impl Playtest {
    pub fn median_contact(&self) -> Option<f32> {
        let mut sorted = self.contact.clone();
        sorted.sort_unstable();
        sorted
            .get(sorted.len() / 2)
            .map(|ticks| *ticks as f32 / HZ as f32)
    }

    pub fn hit_early(&self) -> u32 {
        self.spawns.values().map(|spawn| spawn.hit_early).sum()
    }

    pub fn stranded(&self) -> u32 {
        self.spawns.values().map(|spawn| spawn.stranded).sum()
    }

    pub fn lives(&self) -> u32 {
        self.spawns.values().map(|spawn| spawn.lives).sum()
    }

    /// The largest share of kills made from one position (a zone other than `open`, which is
    /// most of the map rather than a position).
    pub fn top_position(&self, open: &str) -> (&'static str, f32) {
        let total = self.killed_from.values().sum::<u32>().max(1) as f32;
        self.killed_from
            .iter()
            .filter(|(zone, _)| **zone != open)
            .max_by_key(|(_, kills)| **kills)
            .map_or(("", 0.0), |(zone, kills)| (*zone, *kills as f32 / total))
    }

    /// Milliseconds of wall time a tick took.
    pub fn ms_per_tick(&self) -> f32 {
        self.stepping.as_secs_f32() * 1_000.0 / self.ticks.max(1) as f32
    }

    pub fn merge(&mut self, other: Self) {
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
        for (band, n) in other.kill_range {
            *self.kill_range.entry(band).or_default() += n;
        }
        self.farthest.extend(other.farthest);
        self.ticks += other.ticks;
        self.stepping += other.stepping;
        self.early_from.extend(other.early_from);
        for (cell, n) in other.cells {
            *self.cells.entry(cell).or_default() += n;
        }
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

fn flat(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).hypot(a[2] - b[2])
}

/// `fighters` bots on `drive` for `seconds` on `map`, seeded `seed`, zoned by `zone`.
pub fn playtest(
    map: MapId,
    zone: fn([f32; 3]) -> &'static str,
    fighters: u8,
    drive: Drive,
    seed: u64,
    seconds: u32,
) -> Playtest {
    let rules = Rules {
        slots: fighters,
        frag_limit: 0,
        time_limit_ticks: 0,
        ..Rules::deathmatch(seed)
    };
    let mut session = Session::on_map(map, rules).unwrap();
    for slot in 0..fighters {
        if let Some(build) = drive.build(slot) {
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
        let started = Instant::now();
        session.step().unwrap();
        out.stepping += started.elapsed();
        out.ticks += 1;
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
                                let from = session.fighters[usize::from(attacker)].feet();
                                out.early_from.push((life.spawn, from));
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
                        let at = session.fighters[usize::from(killer)].feet();
                        *out.killed_from.entry(zone(at)).or_default() += 1;
                        let band = (flat(at, last[victim]) / 1_000.0) as u32 * 10;
                        *out.kill_range.entry(band).or_default() += 1;
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
                let cell = (
                    (feet[0] / 400.0).floor() as i32 * 4,
                    (feet[2] / 400.0).floor() as i32 * 4,
                );
                *out.cells.entry(cell).or_default() += 1;
                out.farthest[slot] = out.farthest[slot].max(flat(feet, starts[slot]));
                if let Some(life) = lives[slot].as_mut() {
                    life.farthest = life.farthest.max(flat(feet, life.at));
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

pub fn report(label: &str, test: &Playtest) {
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
         deaths (respawned, <3 s), {} respawns hit (<2 s), {} stranded; least roamed {:.0} cm; \
         {:.2} ms a tick",
        test.shots,
        test.kills,
        test.median_contact().unwrap_or(f32::NAN),
        test.lives(),
        test.spawn_deaths,
        test.hit_early(),
        test.stranded(),
        farthest.first().copied().unwrap_or(0.0),
        test.ms_per_tick(),
    );
    println!("  kills from: {}", share(&test.killed_from));
    println!("  deaths in:  {}", share(&test.died_in));
    println!("  time in:    {}", share(&test.time_in));
    let total = test.kill_range.values().sum::<u32>().max(1) as f32;
    let ranges: Vec<String> = test
        .kill_range
        .iter()
        .map(|(band, n)| format!("{band}-{} m {:.0}%", band + 10, 100.0 * *n as f32 / total))
        .collect();
    println!("  kill range: {}", ranges.join(", "));
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

/// A wheeled or tracked robot driven alone on `arena` from `from` (feet) toward each of `route`
/// in turn, steered as a bot steers ([`shooter_sim::bots::steer`]) through the build's preset:
/// where it ends up after at most `seconds`, and the highest it got.
pub fn drive_route(
    arena: Arena,
    build: Assembly,
    from: Vec3i,
    route: &[Vec3i],
    seconds: u32,
) -> ([f32; 3], f32) {
    use shooter_sim::bots::steer;
    use shooter_sim::session::LOCAL_PLAYER;
    use shooter_sim::{Intent, Spawn};
    let first = route[0];
    let arena = Arena {
        spawn: from,
        spawn_yaw: shooter_sim::trig::yaw_from_vector(first.x - from.x, first.z - from.z).unwrap(),
        spawns: Vec::<Spawn>::new(),
        ..arena
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

/// Where a fighter can stand on `arena`'s floor, on a 2 m grid: no piece taller than a step
/// stands there.
pub fn floor_points(arena: &Arena) -> Vec<Vec3i> {
    let half = arena.half_size - 100;
    let mut out = Vec::new();
    let mut x = -half;
    while x <= half {
        let mut z = -half;
        while z <= half {
            let point = Vec3i::new(x, 0, z);
            if arena.clear_walk(point, point) {
                out.push(point);
            }
            z += 200;
        }
        x += 200;
    }
    out
}

/// Which of `points` (feet) a fighter's eye at `from` (feet) sees an eye at within `range`
/// (units), by their indices.
pub fn seen_from(
    world: &shooter_sim::physics::World,
    from: Vec3i,
    points: &[Vec3i],
    range: i32,
) -> Vec<usize> {
    use shooter_sim::player::EYE_HEIGHT_UNITS;
    let eye = |p: Vec3i| [p.x, p.y + EYE_HEIGHT_UNITS, p.z].map(|axis| axis as f32);
    let start = eye(from);
    (0..points.len())
        .filter(|index| {
            let to = eye(points[*index]);
            let along = [to[0] - start[0], to[1] - start[1], to[2] - start[2]];
            along[0].hypot(along[2]) <= range as f32
                && world.cast_ray(start, along, |_| true).is_none()
        })
        .collect()
}
