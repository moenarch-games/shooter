//! Field repair (Atlas D2.i70.9, shooter#61): a robot's mounted repair modules
//! ([`crate::robot::Kind::Repair`]) restore, when the player asks, one of its own parts that is
//! damaged but still connected, over time and for energy from its shared pool.
//!
//! **Reach.** A module reaches a part when the part's box comes within [`RADIUS_UNITS`] of the
//! module's centre, both where the robot's bodies carry them now ([`Hitboxes`]): where a module
//! is mounted decides what it covers, and a part a joint swings out of reach leaves it. A module
//! does not repair itself; a second module may. Only a connected (so working) module repairs.
//!
//! **Eligible** ([`eligible`]): a connected part below its catalog health that a working module
//! reaches. A destroyed part (zero health) or one that came off is gone for the life: nothing
//! reconstructs it, and respawn alone restores the build.
//!
//! **Activation.** The person picks a part from the eligible ones and its client sends
//! [`Order::Start`] once (or [`Order::Stop`]); the host checks it ([`start`]): an unknown,
//! whole, lost or out-of-reach part is refused and changes nothing. The first module by part ID
//! that reaches the part does the work. Asking again for the part being repaired keeps the work
//! done.
//!
//! **Work** ([`Repair::tick`]), each tick while it is not paused: the repair asks the pool for
//! [`ENERGY_PER_TICK`] (granted whole or refused, [`crate::energy::spend`]); a refused tick does
//! nothing and the repair waits for the generators to refill it. Every [`TICKS_PER_HEALTH`]
//! granted ticks restore one health point. The repair ends when the part is whole, and stops when the part or
//! its module is lost or the part leaves the module's reach. Nothing locks the robot: it walks,
//! jumps and fires while it repairs, and they all draw on the same pool.
//!
//! **Interruption** ([`Repair::struck`]): a blow on the part being repaired or on the module
//! doing the work pauses it for [`PAUSE_TICKS`] (a further blow starts the pause over): no
//! health and no energy while paused; the work done is kept and it resumes by itself afterwards,
//! if the part is still eligible. Blows on any other part do not pause it.
//!
//! The radius, rate, energy and pause are provisional tuning (Atlas Q2.i70.3), not
//! owner-approved balance. There is no cooldown beyond the energy a repair draws.

use crate::damage::{Hitboxes, PartDamage};
use crate::energy;
use crate::robot::{Assembly, BETA, Kind, PartId};

/// How far (units, centimetres) a repair module reaches from its centre to a part's box: 0.6 m,
/// three socket cells (provisional, Atlas Q2.i70.3).
pub const RADIUS_UNITS: f32 = 60.0;
/// Granted ticks of work a health point takes: 10 health a second (provisional).
pub const TICKS_PER_HEALTH: u16 = 6;
/// Milli-units a tick of work asks the pool for: 15 energy a second (provisional).
pub const ENERGY_PER_TICK: u32 = 250;
/// Ticks a direct hit pauses a repair: 0.75 s (provisional).
pub const PAUSE_TICKS: u16 = 45;

/// What a person asks of their repair modules on one tick.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Order {
    /// Repair this part (it must be eligible now).
    Start(PartId),
    /// Stop the repair under way.
    Stop,
}

/// A repair under way: the part, the module doing the work and how far it has come.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Repair {
    pub target: PartId,
    pub module: PartId,
    /// Granted ticks of work toward the next health point (below [`TICKS_PER_HEALTH`]).
    pub progress: u16,
    /// Health it has restored so far.
    pub healed: u16,
    /// Ticks it stays paused by a direct hit (0: working).
    pub paused: u16,
}

/// What one tick of a repair did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    /// It drew its energy and worked (restoring a health point every [`TICKS_PER_HEALTH`]).
    Worked,
    /// A direct hit holds it: no work, no energy.
    Paused,
    /// The pool could not pay this tick: no work.
    Starved,
    /// The part is whole: it is over.
    Done,
    /// The part or the module is lost, or the part out of reach: it is over.
    Stopped,
}

fn index_of(build: &Assembly, id: PartId) -> Option<usize> {
    build.parts().iter().position(|part| part.id == id)
}

/// Part `index` of `build` is a repair module.
#[must_use]
pub fn is_module(build: &Assembly, index: usize) -> bool {
    build
        .parts()
        .get(index)
        .and_then(|part| BETA.part_type(part.kind))
        .is_some_and(|part| part.kind == Kind::Repair)
}

/// Module `module` (an index of `build`) is connected and reaches part `target`, another part,
/// where `boxes` has them.
#[must_use]
pub fn reaches(
    build: &Assembly,
    damage: &PartDamage,
    boxes: &Hitboxes,
    module: usize,
    target: usize,
) -> bool {
    module != target
        && is_module(build, module)
        && damage.attached(module)
        && damage.attached(target)
        && boxes
            .get(module)
            .and_then(|at| boxes.reach(target, at.center))
            .is_some_and(|gap| gap <= RADIUS_UNITS)
}

/// The first module (by part ID) that reaches part `target`.
#[must_use]
pub fn module_for(
    build: &Assembly,
    damage: &PartDamage,
    boxes: &Hitboxes,
    target: usize,
) -> Option<usize> {
    (0..build.parts().len()).find(|&module| reaches(build, damage, boxes, module, target))
}

/// The parts a repair may start on now (bits by index): damaged, still connected, and reached
/// by a working module.
#[must_use]
pub fn eligible(build: &Assembly, damage: &PartDamage, boxes: &Hitboxes) -> u32 {
    (0..build.parts().len())
        .filter(|&target| {
            damage.damaged(build, target) && module_for(build, damage, boxes, target).is_some()
        })
        .fold(0, |bits, target| bits | 1 << target)
}

/// A repair of `target` if it is eligible now ([`eligible`]), by the first module reaching it;
/// `None` refuses the order (an unknown, whole, lost or out-of-reach part).
#[must_use]
pub fn start(
    build: &Assembly,
    damage: &PartDamage,
    boxes: &Hitboxes,
    target: PartId,
) -> Option<Repair> {
    let index = index_of(build, target)?;
    if !damage.damaged(build, index) {
        return None;
    }
    let module = module_for(build, damage, boxes, index)?;
    Some(Repair {
        target,
        module: build.parts()[module].id,
        progress: 0,
        healed: 0,
        paused: 0,
    })
}

impl Repair {
    /// A blow landed on `part`: the target or the module working on it pauses the repair for
    /// [`PAUSE_TICKS`] (again from the start); any other part does not.
    pub fn struck(&mut self, part: PartId) {
        if part == self.target || part == self.module {
            self.paused = PAUSE_TICKS;
        }
    }

    /// One tick of the repair on `damage`, paid from `charge` (milli-units).
    pub fn tick(
        &mut self,
        build: &Assembly,
        damage: &mut PartDamage,
        boxes: &Hitboxes,
        charge: &mut u32,
    ) -> Outcome {
        let (Some(target), Some(module)) =
            (index_of(build, self.target), index_of(build, self.module))
        else {
            return Outcome::Stopped;
        };
        if !reaches(build, damage, boxes, module, target) {
            return Outcome::Stopped;
        }
        if !damage.damaged(build, target) {
            return Outcome::Done;
        }
        if self.paused > 0 {
            self.paused -= 1;
            return Outcome::Paused;
        }
        if !energy::spend(charge, ENERGY_PER_TICK) {
            return Outcome::Starved;
        }
        self.progress += 1;
        if self.progress >= TICKS_PER_HEALTH {
            self.progress = 0;
            self.healed += damage.heal(build, target, 1);
            if !damage.damaged(build, target) {
                return Outcome::Done;
            }
        }
        Outcome::Worked
    }
}
