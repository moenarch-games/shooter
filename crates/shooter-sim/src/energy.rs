//! A robot's resources (Atlas D2.i70.3 as D2.i89.1 and D2.i123.1–.4 amend it, D2.i70.4;
//! shooter#72, migrated in place by shooter#128): two shared stores a robot, electricity and fuel
//! ([`Resource`]), no wiring graph, no circuits, no heat.
//!
//! **Stores.** The root chassis stores a baseline of electricity ([`crate::robot::ROOT_CAPACITY`])
//! and generates none; each battery adds stored electricity and each fuel tank stored fuel
//! ([`crate::robot::Kind::Battery`], [`crate::robot::Kind::FuelTank`]). A robot's [`Pool`] is its
//! build's ([`Pool::of`]); it comes to life with both stores full. A store-carrying part the robot
//! loses ([`crate::damage`]) takes its capacity with it ([`Pool::attached`]) and its share of
//! what is stored, once ([`Pool::shrink`], [`Pool::shrink_fuel`]). Everything is integer:
//! amounts and costs are milli-units ([`MILLI`] to one unit).
//!
//! **Generation** (D2.i123.1). Nothing refills a store for free: after every request of the tick
//! ([`Pool::generate`]) each connected solar panel ([`crate::robot::Kind::Solar`]) adds its small
//! output, then the connected generators ([`crate::robot::Kind::Generator`]) burn fuel for the
//! electricity there is room for, at most their output, at their fixed rate (10 electricity a
//! unit of fuel, rounded against the robot): an empty tank stops them, a full battery idles them,
//! and a generator never makes electricity without burning its fuel. A robot without generation
//! keeps what it stores until it spends it; a respawn starts a new robot full. Arena resupply
//! stations are shooter#129's.
//!
//! **Spending** (D2.i123.2, D2.i123.4). Every activation asks the store of its resource for its
//! whole cost ([`request`]): a weapon's shot, swing, hook or continuous tick
//! ([`crate::robot::Tuning::energy`] of [`crate::weapons::Weapon::resource`]: the flamethrower
//! burns fuel, everything else is electric), a thruster jump ([`crate::pickups`]), a repair tick
//! ([`crate::repair`]) and a working motor ([`crate::mechanics`]). A request is granted whole or
//! not at all: a store never goes negative and nothing is paid twice. A refused request simply
//! does not happen (no shot, no cooldown spent, a plain jump, an unpowered motor), and refuses
//! only what depends on that store: an empty fuel tank stops the flamethrower and the generators
//! while the electric guns and motors work on, and an empty battery the reverse. The robot never
//! dies of it and nothing limps; the same request a later tick succeeds once generation has
//! refilled enough. There is no automatic equalisation and no player-set priority.
//!
//! **The one order** (first come, first served; provisional, Atlas Q2.i70.4): a robot's requests
//! are granted in the order the tick resolves its actions, against the same two stores: its jump
//! (a thruster jump, during movement), then its grapple module's hook, then its blade swing, then
//! its guns by part ID, then a field repair (after every hit of the tick), then its motors by
//! part ID, and last its generation. Robots resolve in slot order, but each has its own stores,
//! so no robot's request ever waits on another's. A motor asks for its full draw before the
//! physics step and is refunded what its load and speed did not use after it
//! ([`crate::mechanics`]): a resting motor costs nothing, and a working one pays for the torque
//! and speed it gave.
//!
//! The capacities, rates and costs are provisional defaults (Atlas Q2.i123.2), not owner-approved
//! balance.

use crate::TICK_HZ;
use crate::damage::PartDamage;
use crate::robot::{Assembly, BETA, Supply, supply};

/// Milli-units in one unit: the fixed point of stores and costs.
pub const MILLI: u32 = 1_000;

/// What a robot stores and spends (D2.i123.3).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Resource {
    /// Batteries and the root chassis store it; generators and solar panels make it; every
    /// weapon but the flamethrower, the grapple, repairs, thruster jumps and motors spend it.
    Electricity,
    /// Fuel tanks store it; the generators and the flamethrower burn it.
    Fuel,
}

/// A robot's stores and generation, from its build.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Pool {
    /// The electricity it stores, milli-units.
    pub capacity: u32,
    /// The fuel it stores, milli-units.
    pub fuel: u32,
    /// The electricity its solar panels add a tick, milli-units.
    pub solar: u32,
    /// The electricity its generators give a second at full output (whole units).
    pub output: u32,
    /// The fuel its generators burn a second at full output (whole units).
    pub burn: u32,
}

/// What one tick of generation did ([`Pool::generate`]), milli-units.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Generated {
    /// Electricity added (the solar panels' and the generators').
    pub electricity: u32,
    /// Fuel the generators burned for it.
    pub burned: u32,
    /// The generators had room to fill but no fuel to burn: unable to operate.
    pub starved: bool,
}

impl Pool {
    /// The pool `build` gives (its parts in [`BETA`]).
    #[must_use]
    pub fn of(build: &Assembly) -> Self {
        Self::from_parts(build, |_| true)
    }

    /// The pool of `build`'s parts still connected ([`crate::damage`]): a lost battery's or
    /// tank's store is gone, and a lost generator or solar panel generates nothing.
    #[must_use]
    pub fn attached(build: &Assembly, damage: &PartDamage) -> Self {
        Self::from_parts(build, |index| damage.attached(index))
    }

    fn from_parts(build: &Assembly, keep: impl Fn(usize) -> bool) -> Self {
        let mut pool = Self::default();
        for (index, part) in build.parts().iter().enumerate() {
            if !keep(index) {
                continue;
            }
            let Some(kind) = BETA.part_type(part.kind) else {
                continue;
            };
            let Supply {
                capacity,
                fuel,
                output,
                burn,
            } = supply(part, kind);
            pool.capacity += u32::from(capacity) * MILLI;
            pool.fuel += u32::from(fuel) * MILLI;
            if burn == 0 {
                pool.solar += u32::from(output) * MILLI / u32::from(TICK_HZ);
            } else {
                pool.output += u32::from(output);
                pool.burn += u32::from(burn);
            }
        }
        pool
    }

    /// The capacity of its store of `resource`, milli-units.
    #[must_use]
    pub const fn capacity_of(self, resource: Resource) -> u32 {
        match resource {
            Resource::Electricity => self.capacity,
            Resource::Fuel => self.fuel,
        }
    }

    /// The electricity left when the pool shrinks from `self` to `smaller` (a battery lost): the
    /// lost capacity's share of `charge` goes with it (rounded up), and what remains fits the
    /// smaller pool.
    #[must_use]
    pub fn shrink(self, smaller: Self, charge: u32) -> u32 {
        shrunk(self.capacity, smaller.capacity, charge)
    }

    /// The fuel left when the pool shrinks from `self` to `smaller` (a tank lost), as
    /// [`Pool::shrink`] does the electricity.
    #[must_use]
    pub fn shrink_fuel(self, smaller: Self, fuel: u32) -> u32 {
        shrunk(self.fuel, smaller.fuel, fuel)
    }

    /// One tick of generation after the tick's requests: the solar panels add theirs, then the
    /// generators burn `fuel` for the room left in `charge`, at most their output a tick, never
    /// more electricity than the fuel burned buys (the burn rounds up) and never past the
    /// capacity.
    pub fn generate(self, charge: &mut u32, fuel: &mut u32) -> Generated {
        let before = *charge;
        *charge = charge.saturating_add(self.solar).min(self.capacity);
        let mut burned = 0;
        let mut starved = false;
        let room = self.capacity.saturating_sub(*charge);
        if self.output > 0 && self.burn > 0 && room > 0 {
            let output = u64::from(self.output);
            let burn = u64::from(self.burn);
            let full = output * u64::from(MILLI) / u64::from(TICK_HZ);
            let affordable = u64::from(*fuel) * output / burn;
            let made = full.min(u64::from(room)).min(affordable);
            let cost = (made * burn).div_ceil(output);
            starved = made == 0;
            // Both fit: `made` is at most the room, and `cost` at most the fuel (`made` is what
            // the fuel affords, rounded down).
            *charge += u32::try_from(made).unwrap_or(0);
            let cost = u32::try_from(cost).unwrap_or(u32::MAX);
            *fuel = fuel.saturating_sub(cost);
            burned = cost;
        }
        Generated {
            electricity: *charge - before,
            burned,
            starved,
        }
    }
}

/// `stored` of a store shrinking from `whole` to `smaller` capacity: the lost capacity's share
/// goes (rounded up) and the rest fits.
fn shrunk(whole: u32, smaller: u32, stored: u32) -> u32 {
    if smaller >= whole || whole == 0 {
        return stored.min(smaller);
    }
    let gone = u64::from(whole - smaller);
    let share = (u64::from(stored) * gone).div_ceil(u64::from(whole));
    let left = u64::from(stored).saturating_sub(share);
    u32::try_from(left).unwrap_or(u32::MAX).min(smaller)
}

/// Grants a request for `cost` milli-units from `stored` whole, or refuses it and leaves the
/// store as it was. Whether it was granted.
pub fn spend(stored: &mut u32, cost: u32) -> bool {
    match stored.checked_sub(cost) {
        Some(left) => {
            *stored = left;
            true
        }
        None => false,
    }
}

/// Grants a request for `cost` milli-units of `resource` from the robot's stores (`charge`, its
/// electricity, and `fuel`) whole, or refuses it ([`spend`]). Whether it was granted.
pub fn request(charge: &mut u32, fuel: &mut u32, resource: Resource, cost: u32) -> bool {
    match resource {
        Resource::Electricity => spend(charge, cost),
        Resource::Fuel => spend(fuel, cost),
    }
}

/// What a robot's resources did over its last tick, for the HUD (shooter#128): consumption,
/// generation and what could not operate. Display only: the host and the prediction derive it
/// alike, but nothing in the simulation reads it back and the wire does not carry it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Flow {
    /// Electricity spent last tick (milli-units).
    pub spent: u32,
    /// Fuel spent last tick by its weapons, and burned by its generators (milli-units).
    pub fuel_spent: u32,
    /// Generation last tick ([`Pool::generate`]).
    pub generated: Generated,
    /// Of `spent`, what its motors used (milli-units).
    pub motors: u32,
    /// A commanded motor went unpowered last tick: its electricity ran out.
    pub motors_starved: bool,
    /// Smoothed over about a quarter second, per second (milli-units): electricity spent, fuel
    /// spent (weapons and generators) and electricity generated.
    pub rates: [u32; 3],
}

impl Flow {
    /// Closes a tick: `spent` electricity and `fuel_spent` fuel by requests, `generated`, and
    /// whether a motor starved and what the motors used; the smoothed rates follow.
    pub fn close(
        &mut self,
        spent: u32,
        fuel_spent: u32,
        generated: Generated,
        (starved, motors): (bool, u32),
    ) {
        self.spent = spent;
        self.fuel_spent = fuel_spent + generated.burned;
        self.generated = generated;
        self.motors = motors;
        self.motors_starved = starved;
        let now = [self.spent, self.fuel_spent, generated.electricity];
        for (rate, now) in self.rates.iter_mut().zip(now) {
            let per_second = u64::from(now) * u64::from(TICK_HZ);
            let smoothed = (u64::from(*rate) * 15 + per_second) / 16;
            *rate = u32::try_from(smoothed).unwrap_or(u32::MAX);
        }
    }
}
