//! A robot's resources (Atlas D2.i70.3, D2.i70.4 as D2.i123.1–4 amend them, shooter#128):
//! electricity and fuel, one pool of each a robot, no wiring graph, no circuits, no heat.
//!
//! **Storage.** The root chassis holds a baseline of electricity (Atlas D2.i89.1,
//! [`crate::robot::ROOT_CAPACITY`]); each battery adds electricity storage
//! ([`crate::robot::Kind::Battery`]) and each fuel tank fuel storage
//! ([`crate::robot::Kind::FuelTank`]). A robot's pools are its build's ([`Pool::of`]); it comes
//! to life with both full. A battery or tank the robot loses ([`crate::damage`]) takes its
//! storage with it ([`Pool::attached`]) and its share of the stored amount, once
//! ([`Pool::shrink`], [`Pool::shrink_fuel`]). Everything is integer: amounts and costs are
//! milli-units ([`MILLI`] to one unit).
//!
//! **Recovery is component-driven** (D2.i123.1): nothing refills by itself, and there is no idle
//! drain. Each connected generator ([`crate::robot::Kind::Generator`]) burns fuel from the tanks
//! and makes [`ELECTRICITY_PER_FUEL`] milli-units of electricity for each milli-unit of fuel
//! ([`Pool::generate`]): at most its burn rate a tick, never more than the fuel there is, and only
//! as much as the electricity storage has room for, so a full robot burns nothing and no
//! electricity comes from nothing. A robot without a connected generator, or without fuel, never
//! recharges (until its next life; arena stations are shooter#129).
//!
//! **Spending.** Every consumer asks for its whole cost ([`spend`]): a weapon's shot, swing, hook
//! or continuous tick ([`crate::robot::Tuning::energy`], its weapon mod and any carried module
//! included), a thruster jump ([`crate::pickups`]), a repair tick ([`crate::repair`]) and the
//! motors' work ([`crate::mechanics::draw`]: by activity and load, nothing at rest). Every
//! consumer today runs on electricity; fuel only feeds the generators. A request is granted
//! whole or not at all: an amount never goes negative and nothing is paid twice. A refused
//! request simply does not happen (no shot, no cooldown spent, a plain jump instead of a boosted
//! one, a motor that stays limp, a walk that does not start): only the consumer that was refused
//! stops (D2.i123.4); there is no zero-energy death or limp mode.
//!
//! **Order within a tick** (first come, first served; provisional, Atlas Q2.i70.4): a robot's
//! requests are granted in the order the tick resolves them: its motors (the locomotion's walk,
//! then its joints in part-ID order), its jump (a thruster jump, during movement), then its
//! grapple module's hook, then its blade swing, then its guns (by part ID), then a field repair
//! (after every hit of the tick). The generators run after every request of the tick. Robots
//! resolve in slot order, but each has its own pools, so no robot's request ever waits on
//! another's.
//!
//! The storage, rates and costs are provisional defaults (Atlas Q2.i70.1), not owner-approved
//! balance.

use crate::TICK_HZ;
use crate::damage::PartDamage;
use crate::robot::{Assembly, BETA, capacity};

/// Milli-units in one unit: the fixed point of amounts and costs.
pub const MILLI: u32 = 1_000;

/// The electricity a generator makes from one unit of fuel (provisional, Atlas Q2.i70.1).
pub const ELECTRICITY_PER_FUEL: u32 = 2;

/// A robot's pools: how much electricity and fuel it can store and how much fuel its generators
/// may burn, from its build.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Pool {
    /// Electricity storage, milli-units.
    pub capacity: u32,
    /// Fuel storage, milli-units.
    pub fuel: u32,
    /// Fuel the generators burn a tick at most, milli-units.
    pub burn: u32,
}

/// What a robot's resources did in one tick: a report for the HUD, not state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Flow {
    /// Electricity granted to its consumers, milli-units.
    pub used: u32,
    /// Electricity its generators made, milli-units.
    pub made: u32,
    /// Fuel its generators burned, milli-units.
    pub burned: u32,
    /// The locomotion was refused the electricity to walk.
    pub stalled: bool,
}

impl Pool {
    /// The pools `build` gives (its parts in [`BETA`]).
    #[must_use]
    pub fn of(build: &Assembly) -> Self {
        let budget = BETA.budget(build);
        Self::from_units(budget.capacity, budget.fuel, budget.burn)
    }

    /// The pools of `build`'s parts still connected ([`crate::damage`]): a lost battery's or
    /// tank's storage and a lost generator's burn are gone (and a destroyed root chassis'
    /// baseline, though that is the kill).
    #[must_use]
    pub fn attached(build: &Assembly, damage: &PartDamage) -> Self {
        Self::connected(build, |index| damage.attached(index))
    }

    /// The pools of `build`'s parts that are `attached` (by index).
    #[must_use]
    pub fn connected(build: &Assembly, attached: impl Fn(usize) -> bool) -> Self {
        let (stored, fuel, burn) = build
            .parts()
            .iter()
            .enumerate()
            .filter(|(index, _)| attached(*index))
            .filter_map(|(_, part)| Some((part, BETA.part_type(part.kind)?)))
            .fold((0, 0, 0), |(stored, fuel, burn), (part, kind)| {
                (
                    stored + u32::from(capacity(part, kind)),
                    fuel + u32::from(kind.fuel),
                    burn + u32::from(kind.burn),
                )
            });
        Self::from_units(stored, fuel, burn)
    }

    fn from_units(capacity: u32, fuel: u32, burn: u32) -> Self {
        Self {
            capacity: capacity * MILLI,
            fuel: fuel * MILLI,
            burn: burn * MILLI / u32::from(TICK_HZ),
        }
    }

    /// The electricity left when the pool shrinks from `self` to `smaller` (a battery lost):
    /// the lost storage's share of `charge` goes with it (rounded up), and what remains fits the
    /// smaller pool.
    #[must_use]
    pub fn shrink(self, smaller: Self, charge: u32) -> u32 {
        shrink(self.capacity, smaller.capacity, charge)
    }

    /// The fuel left when the pool shrinks from `self` to `smaller` (a tank lost), as
    /// [`Pool::shrink`] does the electricity.
    #[must_use]
    pub fn shrink_fuel(self, smaller: Self, fuel: u32) -> u32 {
        shrink(self.fuel, smaller.fuel, fuel)
    }

    /// One tick of the generators: they burn fuel for the electricity storage's room, at most
    /// their burn a tick and never more fuel than there is, and make [`ELECTRICITY_PER_FUEL`]
    /// times as much electricity. What they burned and made.
    pub fn generate(self, charge: &mut u32, fuel: &mut u32) -> (u32, u32) {
        let room = self.capacity.saturating_sub(*charge);
        let burned = self.burn.min(*fuel).min(room / ELECTRICITY_PER_FUEL);
        let made = burned * ELECTRICITY_PER_FUEL;
        *fuel -= burned;
        *charge += made;
        (burned, made)
    }
}

/// `amount` of a store shrinking from `whole` to `left`: the lost share goes (rounded up) and
/// the rest fits.
fn shrink(whole: u32, left: u32, amount: u32) -> u32 {
    if left >= whole || whole == 0 {
        return amount.min(left);
    }
    let gone = u64::from(whole - left);
    let share = (u64::from(amount) * gone).div_ceil(u64::from(whole));
    let rest = u64::from(amount).saturating_sub(share);
    u32::try_from(rest).unwrap_or(u32::MAX).min(left)
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
