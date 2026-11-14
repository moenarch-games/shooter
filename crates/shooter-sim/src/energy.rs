//! The shared robot energy pool (Atlas D2.i70.3, D2.i70.4): one pool a robot, no wiring graph,
//! no circuits, no heat.
//!
//! **Capacity and regeneration.** The control core holds a baseline capacity and regenerates it;
//! each battery adds stored capacity only ([`crate::robot::Kind::Battery`]). A robot's pool is its
//! build's ([`Pool::of`]); it comes to life full and refills by [`Pool::regen`] every tick, never
//! past its capacity. A battery the robot loses ([`crate::damage`]) takes its capacity with it
//! ([`Pool::attached`]) and its share of the charge, once ([`Pool::shrink`]). Everything is
//! integer: charges and costs are milli-units ([`MILLI`] to one energy unit), regeneration a
//! whole number of milli-units a tick.
//!
//! **Spending.** Every activation asks the pool for its whole cost ([`spend`]): a weapon's shot,
//! swing, hook or continuous tick ([`crate::robot::Tuning::energy`], its weapon mod and any
//! carried module included), and a thruster jump ([`crate::pickups`]). A request is granted whole
//! or not at all: the charge never goes negative and nothing is paid twice. A refused request
//! simply does not happen (no shot, no cooldown spent, a plain jump instead of a boosted one);
//! the same request a later tick succeeds once regeneration has refilled enough. There is no
//! automatic equalisation and no player-set priority.
//!
//! **Order within a tick** (first come, first served; provisional, Atlas Q2.i70.4): a robot's
//! requests are granted in the order the tick resolves its actions: its jump (a thruster jump,
//! during movement), then its blade swing, then its gun (one fires a tick; a refused swing
//! leaves the gun its turn). Regeneration comes after every request of the tick. Robots resolve
//! in slot order, but each has its own pool, so no robot's request ever waits on another's.
//! A mechanical robot's motors ([`crate::mechanics`]) ask for their draw each commanded tick, in
//! part-ID order, before its pool regenerates; repairs (shooter#61) will join this order.
//!
//! The capacities, regeneration and costs are provisional defaults (Atlas Q2.i70.1), not
//! owner-approved balance.

use crate::TICK_HZ;
use crate::damage::PartDamage;
use crate::robot::{Assembly, BETA};

/// Milli-units in one energy unit: the fixed point of charges and costs.
pub const MILLI: u32 = 1_000;

/// A robot's energy pool: its capacity and its regeneration, from its build.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Pool {
    /// Milli-units.
    pub capacity: u32,
    /// Milli-units a tick.
    pub regen: u32,
}

impl Pool {
    /// The pool `build` gives (its parts in [`BETA`]): capacity in milli-units, and the core's
    /// regeneration a second spread over the ticks of a second (rounded down).
    #[must_use]
    pub fn of(build: &Assembly) -> Self {
        let budget = BETA.budget(build);
        Self {
            capacity: budget.capacity * MILLI,
            regen: budget.regen * MILLI / u32::from(TICK_HZ),
        }
    }

    /// The pool of `build`'s parts still connected ([`crate::damage`]): a lost battery's
    /// capacity (and a lost core's regeneration) is gone.
    #[must_use]
    pub fn attached(build: &Assembly, damage: &PartDamage) -> Self {
        let (capacity, regen) = build
            .parts()
            .iter()
            .enumerate()
            .filter(|(index, _)| damage.attached(*index))
            .filter_map(|(_, part)| BETA.part_type(part.kind))
            .fold((0, 0), |(capacity, regen), part| {
                (
                    capacity + u32::from(part.capacity),
                    regen + u32::from(part.regen),
                )
            });
        Self {
            capacity: capacity * MILLI,
            regen: regen * MILLI / u32::from(TICK_HZ),
        }
    }

    /// The charge left when the pool shrinks from `self` to `smaller` (a battery lost): the
    /// lost capacity's share of `charge` goes with it (rounded up), and what remains fits the
    /// smaller pool.
    #[must_use]
    pub fn shrink(self, smaller: Self, charge: u32) -> u32 {
        if smaller.capacity >= self.capacity || self.capacity == 0 {
            return charge.min(smaller.capacity);
        }
        let gone = u64::from(self.capacity - smaller.capacity);
        let share = (u64::from(charge) * gone).div_ceil(u64::from(self.capacity));
        let left = u64::from(charge).saturating_sub(share);
        u32::try_from(left)
            .unwrap_or(u32::MAX)
            .min(smaller.capacity)
    }

    /// One tick of regeneration on `charge`, never past the capacity.
    pub fn regenerate(self, charge: &mut u32) {
        *charge = charge.saturating_add(self.regen).min(self.capacity);
    }
}

/// Grants a request for `cost` milli-units from `charge` whole, or refuses it and leaves the
/// charge as it was. Whether it was granted.
pub fn spend(charge: &mut u32, cost: u32) -> bool {
    match charge.checked_sub(cost) {
        Some(left) => {
            *charge = left;
            true
        }
        None => false,
    }
}
