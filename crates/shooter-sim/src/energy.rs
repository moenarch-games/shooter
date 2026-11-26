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
//! consumer asks its own resource ([`Resource`], [`pay`]).
//!
//! **Powertrains** (D2.i123.3, shooter#130): the root chassis' [`crate::robot::Power`] is the
//! robot's. An electric root stores the baseline electricity and its locomotion draws
//! electricity; a combustion root stores nothing, and its locomotion burns fuel through a
//! connected engine ([`crate::robot::Kind::Engine`], [`Pool::drive`]) or, without one, cannot
//! walk. The flamethrower burns fuel ([`crate::weapons::Weapon::resource`]); every other weapon,
//! the joints, repairs and the thruster jump draw electricity. The generator is the only
//! converter, at its chassis' rate ([`Pool::electricity_per_fuel`]: an electric chassis
//! recharges twice as fast). A hybrid is just a build with both stores: an electric robot's
//! flamethrower needs a tank, a combustion robot's electrical consumers a battery and a
//! generator. A request is granted whole or not at all: an amount never goes negative and nothing is paid twice. A refused
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
use crate::robot::{Assembly, BETA, Kind, Power, capacity};

/// Milli-units in one unit: the fixed point of amounts and costs.
pub const MILLI: u32 = 1_000;

/// The electricity a generator on an electric chassis makes from one unit of fuel
/// (provisional, Atlas Q2.i70.1, Q2.i123.2).
pub const ELECTRICITY_PER_FUEL: u32 = 2;
/// The electricity a generator on a combustion chassis makes from one unit of fuel: half the
/// electric chassis' rate, so electric builds recharge faster (provisional, Atlas Q2.i123.2).
pub const COMBUSTION_ELECTRICITY_PER_FUEL: u32 = 1;

/// The two resources a robot stores (D2.i123.3).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Resource {
    Electricity,
    Fuel,
}

/// A robot's pools: how much electricity and fuel it can store and how much fuel its generators
/// may burn, from its build, and its powertrain: its root chassis' power and whether an engine is
/// connected.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Pool {
    /// Electricity storage, milli-units.
    pub capacity: u32,
    /// Fuel storage, milli-units.
    pub fuel: u32,
    /// Fuel the generators burn a tick at most, milli-units.
    pub burn: u32,
    /// Its root chassis' native power ([`Power`]).
    pub power: Power,
    /// An engine is connected ([`Kind::Engine`]).
    pub engine: bool,
}

/// What a robot's resources did in one tick: a report for the HUD, not state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Flow {
    /// Electricity granted to its consumers, milli-units.
    pub used: u32,
    /// Fuel granted to its consumers (a combustion drive, the flamethrower), milli-units; the
    /// generators' is [`Flow::burned`].
    pub fueled: u32,
    /// Electricity its generators made, milli-units.
    pub made: u32,
    /// Fuel its generators burned, milli-units.
    pub burned: u32,
    /// The locomotion was refused what it walks on: electricity, or fuel or an engine
    /// ([`Pool::drive`]).
    pub stalled: bool,
}

impl Pool {
    /// The pools `build` gives (its parts in [`BETA`]).
    #[must_use]
    pub fn of(build: &Assembly) -> Self {
        let budget = BETA.budget(build);
        Self {
            power: budget.power,
            engine: budget.engines > 0,
            ..Self::from_units(budget.capacity, budget.fuel, budget.burn)
        }
    }

    /// What its locomotion runs on (D2.i123.3): electricity on an electric chassis; fuel on a
    /// combustion chassis while an engine is connected; nothing on a combustion chassis without
    /// one (it cannot walk).
    #[must_use]
    pub const fn drive(self) -> Option<Resource> {
        match self.power {
            Power::Electric => Some(Resource::Electricity),
            Power::Combustion if self.engine => Some(Resource::Fuel),
            Power::Combustion => None,
        }
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
        let (mut stored, mut fuel, mut burn) = (0, 0, 0);
        let (mut power, mut engine) = (Power::default(), false);
        for (part, kind) in build
            .parts()
            .iter()
            .enumerate()
            .filter(|(index, _)| attached(*index))
            .filter_map(|(_, part)| Some((part, BETA.part_type(part.kind)?)))
        {
            stored += u32::from(capacity(part, kind));
            fuel += u32::from(kind.fuel);
            burn += u32::from(kind.burn);
            match kind.kind {
                Kind::Chassis(native) if part.mount.is_none() => power = native,
                Kind::Engine => engine = true,
                _ => {}
            }
        }
        Self {
            power,
            engine,
            ..Self::from_units(stored, fuel, burn)
        }
    }

    fn from_units(capacity: u32, fuel: u32, burn: u32) -> Self {
        Self {
            capacity: capacity * MILLI,
            fuel: fuel * MILLI,
            burn: burn * MILLI / u32::from(TICK_HZ),
            power: Power::Electric,
            engine: false,
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

    /// The electricity its generators make from one unit of fuel: its root chassis' rate
    /// ([`ELECTRICITY_PER_FUEL`] electric, [`COMBUSTION_ELECTRICITY_PER_FUEL`] combustion).
    #[must_use]
    pub const fn electricity_per_fuel(self) -> u32 {
        match self.power {
            Power::Electric => ELECTRICITY_PER_FUEL,
            Power::Combustion => COMBUSTION_ELECTRICITY_PER_FUEL,
        }
    }

    /// One tick of the generators: they burn fuel for the electricity storage's room, at most
    /// their burn a tick and never more fuel than there is, and make
    /// [`Pool::electricity_per_fuel`] times as much electricity. What they burned and made.
    pub fn generate(self, charge: &mut u32, fuel: &mut u32) -> (u32, u32) {
        let rate = self.electricity_per_fuel();
        let room = self.capacity.saturating_sub(*charge);
        let burned = self.burn.min(*fuel).min(room / rate);
        let made = burned * rate;
        *fuel -= burned;
        *charge += made;
        (burned, made)
    }
}

/// Grants a request for `cost` milli-units of `resource` (from `charge` for electricity, from
/// `fuel` for fuel) whole, or refuses it and leaves both as they were ([`spend`]). A request for
/// no resource (`None`: a combustion drive without an engine) is granted only when it costs
/// nothing. Whether it was granted.
pub fn pay(resource: Option<Resource>, charge: &mut u32, fuel: &mut u32, cost: u32) -> bool {
    match resource {
        Some(Resource::Electricity) => spend(charge, cost),
        Some(Resource::Fuel) => spend(fuel, cost),
        None => cost == 0,
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
