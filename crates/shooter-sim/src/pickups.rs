//! Arena module pickups (Atlas D2.i46.9): a few powerful weapon and movement modules lie on pads
//! at contested points of a map, and any robot that reaches one takes it. There is no
//! compatibility gate: whatever its build, a fighter that reaches a ready pad collects the
//! module, and the module works on top of the build (a rail capacitor stacks on a rail gun's
//! weapon mod); it never changes the build or its budgets.
//!
//! Where the pads are is map data ([`ModulePad`] anchors on [`crate::Arena::module_pads`]); the
//! host owns everything else, one fixed tick at a time ([`crate::Session`]):
//!
//! - **Spawn.** Every pad is ready at the start of a round (and of a match on a fresh map).
//! - **Collection.** While a round is being played, after the world steps, a ready pad is taken
//!   by the living fighter whose feet are within its reach ([`ModulePad::reach`]); when several
//!   are, the one whose feet are nearest the pad's centre (squared whole-unit distance) wins, and
//!   on a tie the lowest slot. Pads resolve in the map's pad order, so one fighter taking two pads
//!   in a tick keeps the later one.
//! - **Lifetime.** A collected module lasts for the rest of its carrier's life, until the carrier
//!   dies or the round ends (owner decision on #62, 14 Nov 2026), not on a timer. A fighter
//!   carries at most one; taking another replaces it with a full one.
//! - **Uses.** A powerful module is not free: it holds a limited number of uses
//!   ([`ArenaModule::uses`]). The rail capacitor boosts [`RAIL_CAPACITOR_SHOTS`] rail shots, the
//!   thrusters [`THRUSTER_JUMPS`] jumps; each boosted shot or jump spends one, and the last one
//!   spends the module ([`crate::Event::ModuleSpent`]). The shared robot energy pool (#72) does
//!   not exist yet, so no module draws energy.
//! - **Death and round end.** Death removes the module at once and drops nothing; a fresh round
//!   (or a match on another map) clears every module and readies every pad.
//! - **Pad respawn.** A pad comes back [`PAD_RESPAWN_TICKS`] (30 s) after it was taken.
//!
//! The module list, the uses and the respawn time are the shooter's provisional defaults
//! (Atlas Q2.i46.3), chosen to be simple, not owner-approved balance.

use crate::fixed::subunits_per_tick;
use crate::physics::Vec3i;
use crate::player::JUMP_VELOCITY_SUBUNITS_PER_TICK;
use crate::robot::Tuning;
use crate::weapons::Weapon;

/// The rail shots a rail capacitor boosts before it is spent.
pub const RAIL_CAPACITOR_SHOTS: u8 = 5;
/// The jumps (the air jump too) thrusters boost before they are spent.
pub const THRUSTER_JUMPS: u8 = 6;
/// How long a pad stays empty after its module was taken: 30 s.
pub const PAD_RESPAWN_TICKS: u32 = 30 * 60;
/// How far (units, on X and on Z) a fighter's feet may be from a pad's centre and take it.
pub const REACH_UNITS: i32 = 60;
/// How far above (or below) a pad's centre the feet may be and take it: a fighter jumping low
/// over a pad still takes it.
pub const REACH_HEIGHT_UNITS: i32 = 100;
/// A thruster jump: 9 m/s up instead of 6 m/s, a 2.25 m jump (onto the greybox's 2 m platform).
pub const THRUSTER_JUMP_UNITS_PER_SECOND: i32 = 900;
pub const THRUSTER_JUMP_SUBUNITS_PER_TICK: i32 = subunits_per_tick(THRUSTER_JUMP_UNITS_PER_SECOND);
/// The rail capacitor's rail gun: 150 % damage...
pub const RAIL_CAPACITOR_DAMAGE_PERCENT: u16 = 150;
/// ...and 75 % of the time between shots.
pub const RAIL_CAPACITOR_REFIRE_PERCENT: u16 = 75;

/// What a module changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Class {
    Weapon,
    Movement,
}

/// A module a pad holds. Its wire tag ([`ArenaModule::tag`]) and key are stable.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum ArenaModule {
    /// Weapon class: the rail gun hits harder and recharges faster
    /// ([`RAIL_CAPACITOR_DAMAGE_PERCENT`], [`RAIL_CAPACITOR_REFIRE_PERCENT`]).
    RailCapacitor,
    /// Movement class: every jump, the air jump too, leaves at
    /// [`THRUSTER_JUMP_UNITS_PER_SECOND`].
    Thrusters,
}

impl ArenaModule {
    pub const ALL: [Self; 2] = [Self::RailCapacitor, Self::Thrusters];

    /// Its stable data ID.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::RailCapacitor => "pickup.rail-capacitor",
            Self::Thrusters => "pickup.thrusters",
        }
    }

    /// Its stable wire tag.
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::RailCapacitor => 0,
            Self::Thrusters => 1,
        }
    }

    #[must_use]
    pub fn from_tag(tag: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|module| module.tag() == tag)
    }

    #[must_use]
    pub const fn class(self) -> Class {
        match self {
            Self::RailCapacitor => Class::Weapon,
            Self::Thrusters => Class::Movement,
        }
    }

    /// How `weapon` fires with this module carried, from how it fires in the build (`tuning`,
    /// its weapon mod's or the shipped weapon's): the module's percentages multiply the mod's.
    #[must_use]
    pub fn tune(self, weapon: Weapon, tuning: Tuning) -> Tuning {
        let scale = |value: u16, percent: u16| {
            u16::try_from(u32::from(value) * u32::from(percent) / 100).unwrap_or(u16::MAX)
        };
        match (self, weapon) {
            (Self::RailCapacitor, Weapon::RailGun) => Tuning {
                damage_percent: scale(tuning.damage_percent, RAIL_CAPACITOR_DAMAGE_PERCENT),
                refire_percent: scale(tuning.refire_percent, RAIL_CAPACITOR_REFIRE_PERCENT),
                ..tuning
            },
            _ => tuning,
        }
    }

    /// The uses a freshly collected module holds.
    #[must_use]
    pub const fn uses(self) -> u8 {
        match self {
            Self::RailCapacitor => RAIL_CAPACITOR_SHOTS,
            Self::Thrusters => THRUSTER_JUMPS,
        }
    }

    /// Whether firing `weapon` spends one of its uses.
    #[must_use]
    pub const fn spent_by_shot(self, weapon: Weapon) -> bool {
        matches!((self, weapon), (Self::RailCapacitor, Weapon::RailGun))
    }

    /// Whether a jump spends one of its uses.
    #[must_use]
    pub const fn spent_by_jump(self) -> bool {
        matches!(self, Self::Thrusters)
    }

    /// The upward velocity a jump sets (subunits per tick).
    #[must_use]
    pub const fn jump_velocity(self) -> i32 {
        match self {
            Self::Thrusters => THRUSTER_JUMP_SUBUNITS_PER_TICK,
            Self::RailCapacitor => JUMP_VELOCITY_SUBUNITS_PER_TICK,
        }
    }
}

/// A module a fighter carries and the uses it has left (at least one: a spent module is gone).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Carried {
    pub module: ArenaModule,
    pub uses_left: u8,
}

impl Carried {
    /// Just collected.
    #[must_use]
    pub const fn fresh(module: ArenaModule) -> Self {
        Self {
            module,
            uses_left: module.uses(),
        }
    }

    /// One use spent; `None` once the last one is.
    #[must_use]
    pub const fn spend(self) -> Option<Self> {
        match self.uses_left {
            0 | 1 => None,
            left => Some(Self {
                module: self.module,
                uses_left: left - 1,
            }),
        }
    }
}

/// The upward velocity a jump sets for a fighter carrying `carried`.
#[must_use]
pub const fn jump_velocity(carried: Option<Carried>) -> i32 {
    match carried {
        Some(carried) => carried.module.jump_velocity(),
        None => JUMP_VELOCITY_SUBUNITS_PER_TICK,
    }
}

/// Where a module lies on a map: map data, like its spawn points and jump pads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModulePad {
    /// The centre of the pad's top, on the ground it lies on (units).
    pub center: Vec3i,
    pub module: ArenaModule,
}

impl ModulePad {
    /// The squared distance (whole units) from the pad's centre to feet at `feet`, when they are
    /// within its reach.
    #[must_use]
    pub fn reach(&self, feet: Vec3i) -> Option<i64> {
        let d = [
            feet.x - self.center.x,
            feet.y - self.center.y,
            feet.z - self.center.z,
        ];
        (d[0].abs() <= REACH_UNITS && d[2].abs() <= REACH_UNITS && d[1].abs() <= REACH_HEIGHT_UNITS)
            .then(|| {
                d.iter()
                    .map(|axis| i64::from(*axis) * i64::from(*axis))
                    .sum()
            })
    }
}

/// A pad in a match: its anchor and the ticks until its module is back (0: ready).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Pickup {
    pub pad: ModulePad,
    pub ready_in: u32,
}

impl Pickup {
    #[must_use]
    pub const fn ready(&self) -> bool {
        self.ready_in == 0
    }
}
