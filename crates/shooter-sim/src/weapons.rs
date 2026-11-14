//! The energy weapons: a hitscan energy rifle, a high-damage rail gun, a rocket launcher whose
//! rockets fly as projectiles and explode with splash damage and knockback (rocket jumping), a
//! shotgun whose blast is a fixed pattern of fast pellets that slow down (and hurt less) as they
//! fly, a continuous laser that burns the first target in its beam every tick the trigger is held,
//! and a flamethrower whose flame, after a short ignition, is a volume cut short by walls; the
//! grapple hook in the same rotation ([`crate::grapple`]); and the energy
//! blade, a quick melee swing on its own key whatever gun is held ([`crate::melee`]).
//!
//! Everything a shot decides is integer: the aim is a Q16 direction from the yaw and pitch
//! steps (the shotgun's pellets add fixed yaw and pitch offsets, [`PELLETS`]), rockets fly in
//! rockets and pellets fly in subunits per tick (a pellet's drag and damage falloff are integer
//! fractions of its speed), splash distances are integer square roots. Rays and rocket and pellet
//! sweeps go through the physics world's `cast_ray` and the flame through its `overlap_box`, so
//! physics-engine and the Rapier oracle answer them alike.

use crate::fixed::subunits_per_tick;
use crate::physics::{BodyId, PhysicsError, RayHit, SUBUNITS_PER_UNIT, World};
use crate::trig;

/// The guns a player carries.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Weapon {
    /// Fast hitscan bolts, low damage each.
    #[default]
    EnergyRifle,
    /// One slow, heavy hitscan beam.
    RailGun,
    /// A slow rocket that explodes on contact: splash damage and knockback.
    RocketLauncher,
    /// A movement tool: hooks arena geometry and reels the player in while the trigger is held
    /// ([`crate::grapple`]).
    Grapple,
    /// The quick-melee energy blade: never held, swung on its own key ([`crate::melee`]).
    EnergyBlade,
    /// A blast of [`PELLETS`] [`Pellet`]s in a fixed spread.
    Shotgun,
    /// A continuous beam: damage every tick the trigger is held.
    Laser,
    /// A short flame that, after [`FLAME_IGNITION_TICKS`], burns every fighter and dummy in its
    /// volume each tick it is held ([`flame`]).
    Flamethrower,
}

/// How a weapon's shot reaches its target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Delivery {
    /// An instant ray: `damage` to the first target within `range_units`.
    Hitscan { damage: i32, range_units: i32 },
    /// A [`Rocket`].
    Rocket,
    /// A [`crate::grapple::Hook`].
    Grapple,
    /// A [`crate::melee`] swing.
    Melee,
    /// [`PELLETS`] [`Pellet`]s, each `damage` at the muzzle's speed.
    Pellets { damage: i32 },
    /// A flame ([`flame`]): `damage` to everything in it.
    Flame { damage: i32 },
}

impl Weapon {
    /// Every weapon, in [`Weapon::index`] order.
    pub const ALL: [Self; 8] = [
        Self::EnergyRifle,
        Self::RailGun,
        Self::RocketLauncher,
        Self::Grapple,
        Self::EnergyBlade,
        Self::Shotgun,
        Self::Laser,
        Self::Flamethrower,
    ];
    /// The weapons a player can hold (the 1 to 7 keys); the blade is swung, never held.
    pub const ROTATION: [Self; 7] = [
        Self::EnergyRifle,
        Self::RailGun,
        Self::RocketLauncher,
        Self::Grapple,
        Self::Shotgun,
        Self::Laser,
        Self::Flamethrower,
    ];

    /// Its place in [`Weapon::ALL`] (the wire).
    #[must_use]
    pub const fn index(self) -> u8 {
        match self {
            Self::EnergyRifle => 0,
            Self::RailGun => 1,
            Self::RocketLauncher => 2,
            Self::Grapple => 3,
            Self::EnergyBlade => 4,
            Self::Shotgun => 5,
            Self::Laser => 6,
            Self::Flamethrower => 7,
        }
    }

    /// The weapon at `index` in [`Weapon::ALL`].
    #[must_use]
    pub const fn from_index(index: u8) -> Option<Self> {
        match index {
            0 => Some(Self::EnergyRifle),
            1 => Some(Self::RailGun),
            2 => Some(Self::RocketLauncher),
            3 => Some(Self::Grapple),
            4 => Some(Self::EnergyBlade),
            5 => Some(Self::Shotgun),
            6 => Some(Self::Laser),
            7 => Some(Self::Flamethrower),
            _ => None,
        }
    }

    /// Ticks between two shots.
    #[must_use]
    pub const fn refire_ticks(self) -> u32 {
        match self {
            // 10 shots a second.
            Self::EnergyRifle => 6,
            // 1.2 s.
            Self::RailGun => 72,
            // 0.8 s.
            Self::RocketLauncher => 48,
            // 0.75 s from one hook to the next.
            Self::Grapple => 45,
            Self::EnergyBlade => crate::melee::SWING_TICKS,
            // 0.9 s.
            Self::Shotgun => 54,
            // Continuous: every tick the trigger is held.
            Self::Laser | Self::Flamethrower => 1,
        }
    }

    #[must_use]
    pub const fn delivery(self) -> Delivery {
        match self {
            Self::EnergyRifle => Delivery::Hitscan {
                damage: 12,
                range_units: 8_000,
            },
            Self::RailGun => Delivery::Hitscan {
                damage: 85,
                range_units: 10_000,
            },
            Self::RocketLauncher => Delivery::Rocket,
            Self::Grapple => Delivery::Grapple,
            Self::EnergyBlade => Delivery::Melee,
            Self::Shotgun => Delivery::Pellets { damage: 9 },
            Self::Laser => Delivery::Hitscan {
                damage: 2,
                range_units: 3_000,
            },
            Self::Flamethrower => Delivery::Flame { damage: 3 },
        }
    }

    /// Whether it fires every tick the trigger is held (a beam or a flame, not single shots).
    #[must_use]
    pub const fn continuous(self) -> bool {
        matches!(self, Self::Laser | Self::Flamethrower)
    }
}

/// The shotgun's pellets: (yaw, pitch) offsets in `trig` angle steps (182 a degree) from the
/// aim. One pellet on the aim, three on a 2° ring and six on a 4.5° ring: a fixed pattern, so a
/// blast needs no randomness.
pub const PELLETS: [(i16, i16); 10] = [
    (0, 0),
    (0, 364),
    (315, -182),
    (-315, -182),
    (819, 0),
    (410, 709),
    (-410, 709),
    (-819, 0),
    (-410, -709),
    (410, -709),
];

/// Pellets leave the muzzle at 240 m/s (4 m a tick)...
pub const PELLET_SPEED_UNITS_PER_SECOND: i32 = 24_000;
pub const PELLET_SPEED_SUBUNITS_PER_TICK: i32 = subunits_per_tick(PELLET_SPEED_UNITS_PER_SECOND);
/// ...lose a sixteenth of their speed every tick (drag)...
pub const PELLET_DRAG_DIVISOR: i32 = 16;
/// ...and are spent once below 150 m/s...
pub const PELLET_MIN_SPEED_SUBUNITS_PER_TICK: i32 = subunits_per_tick(15_000);
/// ...or 25 m out (with this drag, both come about 8 ticks after the blast).
pub const PELLET_RANGE_UNITS: i32 = 2_500;

/// The flamethrower deals no damage for its first 15 ticks (0.25 s) of a continuous burst: it
/// ignites. Letting go of the trigger (or switching away) puts it out again.
pub const FLAME_IGNITION_TICKS: u32 = 15;
/// The flame reaches 6 m from the eye...
pub const FLAME_REACH_UNITS: i32 = 600;
/// ...and is 1.2 m across: everything within 60 units of its axis burns.
pub const FLAME_RADIUS_UNITS: i32 = 60;

/// Ticks after a switch before the new weapon can fire: 0.2 s.
pub const SWITCH_TICKS: u32 = 12;
/// Rocket speed: 24 m/s.
pub const ROCKET_SPEED_UNITS_PER_SECOND: i32 = 2_400;
pub const ROCKET_SPEED_SUBUNITS_PER_TICK: i32 = subunits_per_tick(ROCKET_SPEED_UNITS_PER_SECOND);
/// A rocket that has met nothing for 4 s fizzles out.
pub const ROCKET_LIFETIME_TICKS: u32 = 240;
/// Splash reaches 3 m from the blast, falling off linearly with the distance to a body's centre.
pub const SPLASH_RADIUS_UNITS: i32 = 300;
/// Splash damage at the blast's centre.
pub const SPLASH_DAMAGE: i32 = 100;
/// Knockback at the blast's centre: 15 m/s. A rocket at a standing player's feet (0.9 m from
/// its centre) throws it up at 10.5 m/s, about 3 m.
pub const SPLASH_KNOCKBACK_UNITS_PER_SECOND: i32 = 1_500;
pub const SPLASH_KNOCKBACK_SUBUNITS_PER_TICK: i32 =
    subunits_per_tick(SPLASH_KNOCKBACK_UNITS_PER_SECOND);
/// Pitch steps (`crate::trig` angle units) of straight up; straight down is the negative.
pub const MAX_PITCH: i16 = trig::YAW_QUARTER_TURN as i16;

/// The weapon a player holds and when it (and the blade) may fire next: part of the simulation
/// state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Arsenal {
    pub held: Weapon,
    ready_at: u32,
    blade_ready_at: u32,
    /// Ticks in a row the flamethrower has fired.
    flaming: u32,
}

impl Arsenal {
    /// Applies a switch request, the melee key and the trigger at `tick`: the weapon that fires,
    /// if any. A swing goes first and keeps the gun down for [`crate::melee::GUN_LOCKOUT_TICKS`].
    pub fn trigger(
        &mut self,
        select: Option<Weapon>,
        melee: bool,
        fire: bool,
        tick: u32,
    ) -> Option<Weapon> {
        self.trigger_tuned(select, melee, fire, tick, Weapon::refire_ticks)
    }

    /// [`Arsenal::trigger`] with each weapon's refire from `refire` (its mod's,
    /// [`crate::robot::Tuning::refire_ticks`]).
    pub fn trigger_tuned(
        &mut self,
        select: Option<Weapon>,
        melee: bool,
        fire: bool,
        tick: u32,
        refire: impl Fn(Weapon) -> u32,
    ) -> Option<Weapon> {
        if let Some(weapon) =
            select.filter(|weapon| *weapon != self.held && Weapon::ROTATION.contains(weapon))
        {
            self.held = weapon;
            self.ready_at = self.ready_at.max(tick + SWITCH_TICKS);
        }
        let fired = if melee && tick >= self.blade_ready_at {
            self.blade_ready_at = tick + refire(Weapon::EnergyBlade);
            self.ready_at = self.ready_at.max(tick + crate::melee::GUN_LOCKOUT_TICKS);
            Some(Weapon::EnergyBlade)
        } else if fire && tick >= self.ready_at {
            self.ready_at = tick + refire(self.held);
            Some(self.held)
        } else {
            None
        };
        self.flaming = if fired == Some(Weapon::Flamethrower) {
            self.flaming.saturating_add(1)
        } else {
            0
        };
        fired
    }

    /// Whether the flame burns this tick: it has fired for more than [`FLAME_IGNITION_TICKS`]
    /// ticks in a row. Before that it is still igniting.
    #[must_use]
    pub const fn ignited(&self) -> bool {
        self.flaming > FLAME_IGNITION_TICKS
    }
}

/// The Q16 directions of a shotgun blast's [`PELLETS`] aimed at `yaw` and `pitch`.
#[must_use]
pub fn pellets(yaw: u16, pitch: i16) -> [[i32; 3]; PELLETS.len()] {
    PELLETS.map(|(turn, rise)| {
        #[allow(clippy::cast_sign_loss)]
        let yaw = yaw.wrapping_add(turn as u16);
        aim(yaw, pitch.saturating_add(rise))
    })
}

/// The Q16 unit aim direction `(x, y, z)` of a yaw and a pitch (up positive).
#[must_use]
pub fn aim(yaw: u16, pitch: i16) -> [i32; 3] {
    // A negative pitch wraps to the same angle below the horizon.
    #[allow(clippy::cast_sign_loss)]
    let pitch = pitch.clamp(-MAX_PITCH, MAX_PITCH) as u16;
    let (sine, cosine) = (trig::sin(pitch), trig::cos(pitch));
    let (x, z) = trig::direction(yaw);
    let scale = |unit| trig::checked_scale(cosine, unit).unwrap_or(0);
    [scale(x), sine, scale(z)]
}

#[allow(clippy::cast_precision_loss)]
fn unit_f32(direction: [i32; 3]) -> [f32; 3] {
    direction.map(|axis| axis as f32 / trig::TRIG_ONE as f32)
}

fn along(origin: [f32; 3], translation: [f32; 3], fraction: f32) -> [f32; 3] {
    [0, 1, 2].map(|axis| origin[axis] + translation[axis] * fraction)
}

/// What a hitscan shot met.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trace {
    /// Where the ray ended: the hit point, or the end of its range.
    pub end: [f32; 3],
    pub hit: Option<RayHit>,
}

/// A hitscan ray from `eye` (units) along the Q16 `direction` for `range_units`, ignoring
/// `owner` and every body `accept` refuses.
#[must_use]
pub fn trace(
    world: &World,
    eye: [f32; 3],
    direction: [i32; 3],
    range_units: i32,
    owner: BodyId,
    accept: impl Fn(BodyId) -> bool,
) -> Trace {
    #[allow(clippy::cast_precision_loss)]
    let translation = unit_f32(direction).map(|axis| axis * range_units as f32);
    let hit = world.cast_ray(eye, translation, |body| body != owner && accept(body));
    Trace {
        end: along(eye, translation, hit.map_or(1.0, |hit| hit.fraction)),
        hit,
    }
}

/// A flame from `eye` (units) along the Q16 `direction`, from `owner`: where it ends (the
/// first body `burns` refuses, the arena's geometry, cuts it short within [`FLAME_REACH_UNITS`])
/// and every body `burns` accepts in its volume, in ID order. The volume is the physics world's
/// `overlap_box` round the flame, narrowed to bodies whose box comes within
/// [`FLAME_RADIUS_UNITS`] of the flame's axis.
pub fn flame(
    world: &World,
    eye: [f32; 3],
    direction: [i32; 3],
    owner: BodyId,
    burns: impl Fn(BodyId) -> bool,
) -> Result<(Trace, Vec<BodyId>), PhysicsError> {
    let wall = trace(world, eye, direction, FLAME_REACH_UNITS, owner, |body| {
        !burns(body)
    });
    let axis = [0, 1, 2].map(|axis| wall.end[axis] - eye[axis]);
    #[allow(clippy::cast_precision_loss)]
    let radius = FLAME_RADIUS_UNITS as f32;
    let centre = [0, 1, 2].map(|index| eye[index] + axis[index] * 0.5);
    let half = axis.map(|length| length.abs() * 0.5 + radius);
    let burning = world
        .overlap_box(centre, half)?
        .into_iter()
        .filter(|body| *body != owner && burns(*body))
        .filter(|body| {
            world.body(*body).is_some_and(|rigid| {
                let extents = rigid.half_extents();
                #[allow(clippy::cast_precision_loss)]
                let inflated = [extents.x, extents.y, extents.z].map(|e| e as f32 + radius);
                ray_box(eye, axis, rigid.exact_position(), inflated).is_some()
            })
        })
        .collect();
    Ok((wall, burning))
}

/// Where along `translation` (0..=1) a ray from `origin` enters the box at `center` with
/// `half` extents, if it does (0 when it starts inside). A slab test: fighters are boxes to
/// lag-compensated shots, wherever they stood at the rewound tick.
#[must_use]
pub fn ray_box(
    origin: [f32; 3],
    translation: [f32; 3],
    center: [f32; 3],
    half: [f32; 3],
) -> Option<f32> {
    let (mut enter, mut exit) = (0.0f32, 1.0f32);
    for axis in 0..3 {
        let (low, high) = (center[axis] - half[axis], center[axis] + half[axis]);
        let (start, step) = (origin[axis], translation[axis]);
        if step == 0.0 {
            if start < low || start > high {
                return None;
            }
            continue;
        }
        let (a, b) = ((low - start) / step, (high - start) / step);
        enter = enter.max(a.min(b));
        exit = exit.min(a.max(b));
        if enter > exit {
            return None;
        }
    }
    Some(enter)
}

/// A rocket in flight, in subunits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rocket {
    pub owner: BodyId,
    pub position: [i32; 3],
    /// Subunits per tick.
    pub velocity: [i32; 3],
    pub age: u32,
}

/// Where a rocket went this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Flight {
    Flying,
    /// It met a body: the blast point (units), pulled back off the surface by a unit.
    Exploded {
        at: [f32; 3],
        body: BodyId,
    },
    Fizzled,
}

#[allow(clippy::cast_precision_loss)]
fn units(subunits: [i32; 3]) -> [f32; 3] {
    subunits.map(|axis| axis as f32 / SUBUNITS_PER_UNIT as f32)
}

/// Units to subunits, rounded.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn subunits(units: [f32; 3]) -> [i32; 3] {
    #[allow(clippy::cast_precision_loss)]
    units.map(|axis| (axis * SUBUNITS_PER_UNIT as f32).round() as i32)
}

impl Rocket {
    /// A rocket leaving `eye` (units) along the Q16 `direction`.
    #[must_use]
    pub fn launch(owner: BodyId, eye: [f32; 3], direction: [i32; 3]) -> Self {
        Self {
            owner,
            position: subunits(eye),
            velocity: direction
                .map(|unit| trig::checked_scale(ROCKET_SPEED_SUBUNITS_PER_TICK, unit).unwrap_or(0)),
            age: 0,
        }
    }

    /// The rocket's position in units.
    #[must_use]
    pub fn position_units(&self) -> [f32; 3] {
        units(self.position)
    }

    /// One tick of flight: sweeps this tick's path for the first body other than the owner.
    pub fn fly(&mut self, world: &World) -> Flight {
        let origin = self.position_units();
        let translation = units(self.velocity);
        if let Some(hit) = world.cast_ray(origin, translation, |body| body != self.owner) {
            let point = along(origin, translation, hit.fraction);
            let at = [0, 1, 2].map(|axis| point[axis] + hit.normal[axis]);
            return Flight::Exploded { at, body: hit.body };
        }
        self.position = [0, 1, 2].map(|axis| self.position[axis] + self.velocity[axis]);
        self.age += 1;
        if self.age >= ROCKET_LIFETIME_TICKS {
            Flight::Fizzled
        } else {
            Flight::Flying
        }
    }
}

/// A shotgun pellet in flight, in subunits: a fast projectile that slows down with drag and
/// hurts less the slower it goes ([`Pellet::damage`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pellet {
    pub owner: BodyId,
    pub position: [i32; 3],
    /// Q16 unit direction.
    pub direction: [i32; 3],
    /// Subunits per tick.
    pub speed: i32,
    /// Subunits flown so far.
    pub travelled: i32,
    /// Its full damage, at the muzzle's speed.
    pub damage: i32,
    /// Where this tick's flight began (subunits): the eye on the tick it is fired.
    pub from: [i32; 3],
    /// Ticks flown.
    pub age: u32,
}

/// Where a pellet went this tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Travel {
    Flying,
    /// It met a body at `at` (units) and deals `damage` there.
    Struck {
        at: [f32; 3],
        normal: [f32; 3],
        body: BodyId,
        damage: i32,
    },
    /// Out of range or too slow: gone.
    Spent,
}

impl Pellet {
    /// A pellet of `damage` leaving `eye` (units) along the Q16 `direction`.
    #[must_use]
    pub fn launch(owner: BodyId, eye: [f32; 3], direction: [i32; 3], damage: i32) -> Self {
        let position = subunits(eye);
        Self {
            owner,
            position,
            direction,
            speed: PELLET_SPEED_SUBUNITS_PER_TICK,
            travelled: 0,
            damage,
            from: position,
            age: 0,
        }
    }

    #[must_use]
    pub fn position_units(&self) -> [f32; 3] {
        units(self.position)
    }

    #[must_use]
    pub fn from_units(&self) -> [f32; 3] {
        units(self.from)
    }

    /// What it deals at its current speed: its full damage scaled by the speed's share of the
    /// muzzle's, rounded up (so a point-blank pellet deals all of it).
    #[must_use]
    pub fn damage(&self) -> i32 {
        let scaled = i64::from(self.damage) * i64::from(self.speed);
        let muzzle = i64::from(PELLET_SPEED_SUBUNITS_PER_TICK);
        i32::try_from((scaled + muzzle - 1) / muzzle).unwrap_or(0)
    }

    /// One tick of flight: sweeps this tick's path (cut at [`PELLET_RANGE_UNITS`]) for the first
    /// body other than the owner, then slows down by drag.
    pub fn fly(&mut self, world: &World) -> Travel {
        let range = PELLET_RANGE_UNITS * SUBUNITS_PER_UNIT;
        let step = self.speed.min(range - self.travelled).max(0);
        let velocity = self
            .direction
            .map(|unit| trig::checked_scale(step, unit).unwrap_or(0));
        let origin = self.position_units();
        let translation = units(velocity);
        self.from = self.position;
        if let Some(hit) = world.cast_ray(origin, translation, |body| body != self.owner) {
            return Travel::Struck {
                at: along(origin, translation, hit.fraction),
                normal: hit.normal,
                body: hit.body,
                damage: self.damage(),
            };
        }
        self.position = [0, 1, 2].map(|axis| self.position[axis] + velocity[axis]);
        self.travelled += step;
        self.speed -= self.speed / PELLET_DRAG_DIVISOR;
        self.age += 1;
        if self.travelled >= range || self.speed < PELLET_MIN_SPEED_SUBUNITS_PER_TICK {
            Travel::Spent
        } else {
            Travel::Flying
        }
    }
}

/// What a blast does to a body whose centre is at `center` (units): damage and the knockback
/// velocity (subunits per tick), or `None` out of reach.
#[must_use]
pub fn splash(blast: [f32; 3], center: [f32; 3]) -> Option<(i32, [i32; 3])> {
    let blast = subunits(blast);
    let center = subunits(center);
    let offset: [i64; 3] = [0, 1, 2].map(|axis| i64::from(center[axis] - blast[axis]));
    #[allow(clippy::cast_sign_loss)]
    let squared = offset.iter().map(|axis| axis * axis).sum::<i64>() as u64;
    #[allow(clippy::cast_possible_wrap)]
    let distance = trig::isqrt(squared) as i64;
    let radius = i64::from(SPLASH_RADIUS_UNITS * SUBUNITS_PER_UNIT);
    if distance >= radius {
        return None;
    }
    let reach = radius - distance;
    let damage = i64::from(SPLASH_DAMAGE) * reach / radius;
    let push = i64::from(SPLASH_KNOCKBACK_SUBUNITS_PER_TICK) * reach / radius;
    let knockback = if distance == 0 {
        [0, push, 0]
    } else {
        offset.map(|axis| axis * push / distance)
    };
    let narrow = |value: i64| i32::try_from(value).unwrap_or(0);
    Some((narrow(damage), knockback.map(narrow)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aim_points_where_the_yaw_and_pitch_say() {
        assert_eq!(aim(0, 0), [0, 0, trig::TRIG_ONE]);
        assert_eq!(aim(0, -MAX_PITCH), [0, -trig::TRIG_ONE, 0]);
        let [x, y, z] = aim(trig::YAW_QUARTER_TURN, 0);
        assert_eq!((x, y, z), (trig::TRIG_ONE, 0, 0));
    }

    #[test]
    fn the_trigger_respects_refire_and_switch_delays() {
        let mut arsenal = Arsenal::default();
        assert_eq!(
            arsenal.trigger(None, false, true, 0),
            Some(Weapon::EnergyRifle)
        );
        assert_eq!(arsenal.trigger(None, false, true, 5), None);
        assert_eq!(
            arsenal.trigger(None, false, true, 6),
            Some(Weapon::EnergyRifle)
        );
        assert_eq!(arsenal.trigger(Some(Weapon::RailGun), false, true, 7), None);
        assert_eq!(
            arsenal.trigger(None, false, true, 19),
            Some(Weapon::RailGun)
        );
        assert_eq!(arsenal.trigger(None, false, true, 20), None);
    }

    #[test]
    fn the_blade_swings_over_any_gun_and_is_never_held() {
        let mut arsenal = Arsenal::default();
        let blade = Some(Weapon::EnergyBlade);
        assert_eq!(arsenal.trigger(blade, true, true, 0), blade);
        assert_eq!(arsenal.held, Weapon::EnergyRifle);
        // The gun waits out the lockout; the blade its own refire.
        assert_eq!(arsenal.trigger(None, false, true, 17), None);
        assert_eq!(
            arsenal.trigger(None, true, true, 18),
            Some(Weapon::EnergyRifle)
        );
        assert_eq!(arsenal.trigger(None, true, false, 29), None);
        assert_eq!(arsenal.trigger(None, true, false, 30), blade);
    }

    #[test]
    fn the_flame_ignites_while_the_trigger_stays_down() {
        let mut arsenal = Arsenal::default();
        let flame = Some(Weapon::Flamethrower);
        arsenal.trigger(flame, false, false, 0);
        let mut tick = SWITCH_TICKS;
        for _ in 0..FLAME_IGNITION_TICKS {
            assert_eq!(arsenal.trigger(None, false, true, tick), flame);
            assert!(!arsenal.ignited(), "tick {tick}");
            tick += 1;
        }
        assert_eq!(arsenal.trigger(None, false, true, tick), flame);
        assert!(arsenal.ignited());
        // Letting go puts it out: the next burst ignites from the start.
        arsenal.trigger(None, false, false, tick + 1);
        assert_eq!(arsenal.trigger(None, false, true, tick + 2), flame);
        assert!(!arsenal.ignited());
    }

    #[test]
    fn pellets_hurt_less_as_they_slow_down() {
        let mut pellet = Pellet::launch(BodyId(0), [0.0; 3], [0, 0, trig::TRIG_ONE], 9);
        assert_eq!(pellet.damage(), 9);
        let mut damages = vec![pellet.damage()];
        let mut ticks = 0;
        let world = World::new(crate::physics::Vec3i::new(0, 0, 0));
        while pellet.fly(&world) == Travel::Flying {
            damages.push(pellet.damage());
            ticks += 1;
        }
        // Never more on the way, and still two thirds at the end of its range.
        assert!(
            damages.windows(2).all(|pair| pair[1] <= pair[0]),
            "{damages:?}"
        );
        assert_eq!(damages.last(), Some(&6), "{damages:?}");
        assert_eq!(ticks, 7);
        assert_eq!(pellet.travelled, PELLET_RANGE_UNITS * SUBUNITS_PER_UNIT);
    }

    #[test]
    fn splash_falls_off_and_pushes_away() {
        let (damage, push) = splash([0.0; 3], [0.0, 90.0, 0.0]).unwrap();
        assert_eq!(damage, 70);
        assert_eq!(push, [0, SPLASH_KNOCKBACK_SUBUNITS_PER_TICK * 7 / 10, 0]);
        assert!(splash([0.0; 3], [300.0, 0.0, 0.0]).is_none());
    }
}
