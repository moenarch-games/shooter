//! The energy weapons: a hitscan energy rifle, a high-damage rail gun and a rocket launcher whose
//! rockets fly as projectiles and explode with splash damage and knockback (rocket jumping).
//!
//! Everything a shot decides is integer: the aim is a Q16 direction from the yaw and pitch
//! steps, rockets fly in subunits per tick, splash distances are integer square roots. Rays and
//! rocket sweeps go through the physics world's `cast_ray`, so physics-engine and the Rapier
//! oracle answer them alike.

use sim_foundation::physics::{BodyId, RayHit, SUBUNITS_PER_UNIT, World};
use sim_foundation::{subunits_per_tick, trig};

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
}

/// How a weapon's shot reaches its target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Delivery {
    /// An instant ray: `damage` to the first target within `range_units`.
    Hitscan { damage: i32, range_units: i32 },
    /// A [`Rocket`].
    Rocket,
}

impl Weapon {
    pub const ALL: [Self; 3] = [Self::EnergyRifle, Self::RailGun, Self::RocketLauncher];

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
        }
    }
}

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
/// Pitch steps (`sim_foundation::trig` angle units) of straight up; straight down is the negative.
pub const MAX_PITCH: i16 = trig::YAW_QUARTER_TURN as i16;

/// The weapon a player holds and when it may fire next: part of the simulation state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Arsenal {
    pub held: Weapon,
    ready_at: u32,
}

impl Arsenal {
    /// Applies a switch request and the trigger at `tick`: the weapon that fires, if any.
    pub fn trigger(&mut self, select: Option<Weapon>, fire: bool, tick: u32) -> Option<Weapon> {
        if let Some(weapon) = select.filter(|weapon| *weapon != self.held) {
            self.held = weapon;
            self.ready_at = self.ready_at.max(tick + SWITCH_TICKS);
        }
        if fire && tick >= self.ready_at {
            self.ready_at = tick + self.held.refire_ticks();
            Some(self.held)
        } else {
            None
        }
    }
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
/// `owner`.
#[must_use]
pub fn trace(
    world: &World,
    eye: [f32; 3],
    direction: [i32; 3],
    range_units: i32,
    owner: BodyId,
) -> Trace {
    #[allow(clippy::cast_precision_loss)]
    let translation = unit_f32(direction).map(|axis| axis * range_units as f32);
    let hit = world.cast_ray(eye, translation, |body| body != owner);
    Trace {
        end: along(eye, translation, hit.map_or(1.0, |hit| hit.fraction)),
        hit,
    }
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
        assert_eq!(arsenal.trigger(None, true, 0), Some(Weapon::EnergyRifle));
        assert_eq!(arsenal.trigger(None, true, 5), None);
        assert_eq!(arsenal.trigger(None, true, 6), Some(Weapon::EnergyRifle));
        assert_eq!(arsenal.trigger(Some(Weapon::RailGun), true, 7), None);
        assert_eq!(arsenal.trigger(None, true, 19), Some(Weapon::RailGun));
        assert_eq!(arsenal.trigger(None, true, 20), None);
    }

    #[test]
    fn splash_falls_off_and_pushes_away() {
        let (damage, push) = splash([0.0; 3], [0.0, 90.0, 0.0]).unwrap();
        assert_eq!(damage, 70);
        assert_eq!(push, [0, SPLASH_KNOCKBACK_SUBUNITS_PER_TICK * 7 / 10, 0]);
        assert!(splash([0.0; 3], [300.0, 0.0, 0.0]).is_none());
    }
}
