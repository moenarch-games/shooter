//! Bots: simple, beatable and deterministic. A bot walks from spawn point to spawn point, looks
//! for the nearest fighter it can see, turns toward it at a limited rate, waits a reaction delay,
//! then fires with an aim error it re-rolls every few hundred milliseconds, strafing as it does.
//! It burns anyone within a few metres with the flamethrower, picks the rifle (even slots) or the
//! shotgun (odd slots) up close, rockets (even) or the laser (odd) at mid range and the rail gun
//! far off, and rushes in to swing the energy blade when someone comes within a few metres. Every random
//! choice comes from its own seeded stream (`sim_foundation`'s SplitMix64), so a match replays
//! exactly.

use crate::arena::Arena;
use crate::player::Intent;
use crate::session::slot_of;
use crate::weapons::{MAX_PITCH, Weapon};
use sim_foundation::physics::{Vec3i, World};
use sim_foundation::rng::ZoneRng;
use sim_foundation::trig;

/// A bot looks for targets every 6 ticks (0.1 s).
pub const SCAN_TICKS: u32 = 6;
/// It sees 30 m.
pub const SIGHT_UNITS: i64 = 3_000;
/// It waits 0.35 s after spotting someone before it fires.
pub const REACTION_TICKS: u32 = 21;
/// It turns at most 6° a tick (360°/s).
pub const TURN_PER_TICK: i32 = 1_092;
/// Its aim wanders up to ±3.3° in yaw and ±2° in pitch...
pub const AIM_ERROR_YAW: u32 = 600;
pub const AIM_ERROR_PITCH: u32 = 360;
/// ...re-rolled every 0.3 s.
pub const AIM_ERROR_TICKS: u32 = 18;
/// It fires once its aim is within 4° of where it wants it.
pub const FIRE_CONE: i32 = 730;
/// The flamethrower inside 5 m, the rifle or shotgun inside 7 m, rockets or the laser to 18 m,
/// the rail gun beyond.
pub const FLAME_RANGE_UNITS: i64 = 500;
pub const RIFLE_RANGE_UNITS: i64 = 700;
pub const ROCKET_RANGE_UNITS: i64 = 1_800;
/// It closes in beyond 12 m and backs off inside 5 m.
pub const CLOSE_IN_UNITS: i64 = 1_200;
pub const BACK_OFF_UNITS: i64 = 500;
/// Inside 3 m it rushes in and swings the blade once its target is within 2.2 m across the
/// ground (a little inside the blade's reach).
pub const RUSH_UNITS: i64 = 300;
pub const MELEE_UNITS: i32 = crate::melee::REACH_UNITS - 30;
/// It counts as at a waypoint within 1.5 m.
pub const WAYPOINT_REACH_UNITS: i64 = 150;
/// After 0.5 s barely moving while it wants to walk, it jumps and picks another waypoint.
pub const STUCK_TICKS: u32 = 30;

/// What a bot knows of one slot: where its fighter stands (units), if it is alive.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Sight {
    pub center: Option<Vec3i>,
    pub eye: Option<Vec3i>,
    pub health: i32,
}

/// One bot's mind.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Bot {
    rng: ZoneRng,
    yaw: u16,
    pitch: i16,
    target: Option<usize>,
    spotted_at: u32,
    error: (i32, i32),
    error_until: u32,
    strafe: i8,
    strafe_until: u32,
    waypoint: Option<usize>,
    stuck: u32,
    last: Option<Vec3i>,
}

const fn mix(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// The signed turn from `from` to `to`, shortest way round.
const fn turn(from: u16, to: u16) -> i32 {
    to.wrapping_sub(from) as i16 as i32
}

const fn turned(from: u16, by: i32) -> u16 {
    from.wrapping_add(by as i16 as u16)
}

/// The pitch steps (`trig` angle, up positive) looking `rise` up over `run` across.
fn pitch_of(rise: i32, run: i32) -> i16 {
    let angle = trig::yaw_from_vector(rise, run.max(1)).unwrap_or(0);
    (angle as i16).clamp(-MAX_PITCH, MAX_PITCH)
}

impl Bot {
    /// The bot for `slot` in a match seeded `seed`, created at `tick`.
    #[must_use]
    pub const fn new(seed: u64, slot: u8, tick: u32) -> Self {
        Self {
            rng: ZoneRng::from_state(mix(seed ^ mix(slot as u64) ^ mix((tick as u64) << 8))),
            yaw: 0,
            pitch: 0,
            target: None,
            spotted_at: 0,
            error: (0, 0),
            error_until: 0,
            strafe: 1,
            strafe_until: 0,
            waypoint: None,
            stuck: 0,
            last: None,
        }
    }

    /// It appeared at a spawn point facing `yaw`.
    pub const fn spawned(&mut self, yaw: u16) {
        self.yaw = yaw;
        self.pitch = 0;
        self.target = None;
        self.waypoint = None;
        self.stuck = 0;
        self.last = None;
    }

    fn signed(&mut self, bound: u32) -> i32 {
        let span = self.rng.below(2 * bound + 1);
        i32::try_from(span).unwrap_or(0) - i32::try_from(bound).unwrap_or(0)
    }

    /// The nearest living fighter in sight of `eye`, if any.
    fn spot(me: usize, eye: Vec3i, sights: &[Sight], world: &World) -> Option<usize> {
        let mut best: Option<(usize, i64)> = None;
        for (other, sight) in sights.iter().enumerate() {
            let Some(center) = sight.center.filter(|_| other != me) else {
                continue;
            };
            let d = [center.x - eye.x, center.y - eye.y, center.z - eye.z].map(i64::from);
            let squared = d.iter().map(|axis| axis * axis).sum::<i64>();
            if squared > SIGHT_UNITS * SIGHT_UNITS || best.is_some_and(|(_, b)| b <= squared) {
                continue;
            }
            #[allow(clippy::cast_precision_loss)]
            let (from, along) = (
                [eye.x, eye.y, eye.z].map(|axis| axis as f32),
                d.map(|axis| axis as f32),
            );
            // Only the arena blocks sight; fighters do not.
            if world
                .cast_ray(from, along, |body| slot_of(body).is_none())
                .is_none()
            {
                best = Some((other, squared));
            }
        }
        best.map(|(other, _)| other)
    }

    /// This tick's intent for the bot in slot `me`.
    pub fn think(
        &mut self,
        me: usize,
        sights: &[Sight],
        world: &World,
        arena: &Arena,
        held: Weapon,
        tick: u32,
    ) -> Intent {
        let (Some(center), Some(eye)) = (sights[me].center, sights[me].eye) else {
            return Intent::IDLE;
        };
        if (tick + me as u32).is_multiple_of(SCAN_TICKS) {
            let spotted = Self::spot(me, eye, sights, world);
            if spotted != self.target {
                self.target = spotted;
                self.spotted_at = tick;
            }
        }
        if self
            .target
            .is_some_and(|target| sights[target].center.is_none())
        {
            self.target = None;
        }
        let mut intent = match self.target.and_then(|target| sights[target].center) {
            Some(target) => self.fight(me, eye, target, held, tick),
            None => self.wander(center, arena),
        };
        // Stuck against something while walking: hop and head elsewhere.
        let moved = self
            .last
            .is_some_and(|last| (last.x - center.x).abs() + (last.z - center.z).abs() > 1);
        self.last = Some(center);
        if intent.forward != 0 && !moved {
            self.stuck += 1;
            if self.stuck >= STUCK_TICKS {
                self.stuck = 0;
                self.waypoint = None;
                intent.jump = true;
            }
        } else {
            self.stuck = 0;
        }
        intent
    }

    fn face(&mut self, yaw: u16, pitch: i16) {
        self.yaw = turned(
            self.yaw,
            turn(self.yaw, yaw).clamp(-TURN_PER_TICK, TURN_PER_TICK),
        );
        let step = (i32::from(pitch) - i32::from(self.pitch)).clamp(-TURN_PER_TICK, TURN_PER_TICK);
        self.pitch = i16::try_from(i32::from(self.pitch) + step).unwrap_or(0);
    }

    fn fight(&mut self, me: usize, eye: Vec3i, target: Vec3i, held: Weapon, tick: u32) -> Intent {
        let (dx, dy, dz) = (target.x - eye.x, target.y - eye.y, target.z - eye.z);
        let run = i32::try_from(trig::xz_length(dx, dz)).unwrap_or(i32::MAX);
        let distance = i64::from(run).max(i64::from(dy.abs()));
        if tick >= self.error_until {
            self.error = (self.signed(AIM_ERROR_YAW), self.signed(AIM_ERROR_PITCH));
            self.error_until = tick + AIM_ERROR_TICKS;
        }
        let yaw = turned(
            trig::yaw_from_vector(dx, dz).unwrap_or(self.yaw),
            self.error.0,
        );
        let pitch = i16::try_from(i32::from(pitch_of(dy, run)) + self.error.1)
            .unwrap_or(0)
            .clamp(-MAX_PITCH, MAX_PITCH);
        self.face(yaw, pitch);
        let odd = me % 2 == 1;
        let weapon = if distance < FLAME_RANGE_UNITS {
            Weapon::Flamethrower
        } else if distance < RIFLE_RANGE_UNITS {
            if odd {
                Weapon::Shotgun
            } else {
                Weapon::EnergyRifle
            }
        } else if distance < ROCKET_RANGE_UNITS {
            if odd {
                Weapon::Laser
            } else {
                Weapon::RocketLauncher
            }
        } else {
            Weapon::RailGun
        };
        if tick >= self.strafe_until {
            self.strafe = if self.rng.below(2) == 0 { -1 } else { 1 };
            self.strafe_until = tick + 40 + self.rng.below(60);
        }
        let on_target = turn(self.yaw, yaw).abs() < FIRE_CONE;
        let ready = on_target && tick >= self.spotted_at + REACTION_TICKS;
        Intent {
            forward: if !(RUSH_UNITS..=CLOSE_IN_UNITS).contains(&distance) {
                1
            } else if distance < BACK_OFF_UNITS {
                -1
            } else {
                0
            },
            strafe: self.strafe,
            yaw: self.yaw,
            pitch: self.pitch,
            jump: self.rng.below(90) == 0,
            sprint: false,
            crouch: false,
            fire: weapon == held && ready,
            melee: ready && run < MELEE_UNITS && dy.abs() < crate::player::EYE_HEIGHT_UNITS,
            select: Some(weapon),
        }
    }

    fn wander(&mut self, center: Vec3i, arena: &Arena) -> Intent {
        let points = arena.spawn_points();
        let feet = Vec3i::new(
            center.x,
            center.y - crate::player::PLAYER_HALF_EXTENTS.y,
            center.z,
        );
        let reached = self.waypoint.is_some_and(|index| {
            let point = points[index].feet;
            i64::try_from(trig::xz_length(point.x - feet.x, point.z - feet.z)).unwrap_or(0)
                < WAYPOINT_REACH_UNITS
        });
        if reached || self.waypoint.is_none_or(|index| index >= points.len()) {
            let open: Vec<usize> = (0..points.len())
                .filter(|index| Some(*index) != self.waypoint)
                .filter(|index| arena.clear_walk(feet, points[*index].feet))
                .collect();
            let choices = if open.is_empty() {
                (0..points.len()).collect()
            } else {
                open
            };
            let pick = self.rng.below(u32::try_from(choices.len()).unwrap_or(1)) as usize;
            self.waypoint = choices.get(pick).copied();
        }
        let point = points[self.waypoint.unwrap_or(0)].feet;
        let yaw = trig::yaw_from_vector(point.x - feet.x, point.z - feet.z).unwrap_or(self.yaw);
        self.face(yaw, 0);
        Intent {
            forward: 1,
            yaw: self.yaw,
            pitch: self.pitch,
            sprint: true,
            ..Intent::IDLE
        }
    }
}
