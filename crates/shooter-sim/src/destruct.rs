//! Deliberate self-destruct (Atlas D2.i123.10, D2.i123.4; shooter#131): every robot can blow
//! itself up, stranded or not, after a warning everyone sees, and how hard it blows depends on
//! the volatile contents it still carries.
//!
//! **Arming** ([`crate::player::Intent::destruct`], a press sent once like a repair order): a
//! living robot's press while the round is played arms it, and the host counts down
//! [`COUNTDOWN_TICKS`]; another press during the countdown cancels it. Nothing else is needed:
//! no energy, no fuel, no locomotion, no ground under it (a robot whose propulsion is gone or
//! empty can still end its life, D2.i123.4). Death cancels a countdown (a robot destroyed before
//! it goes off does not go off: the counterplay), and so does the end of the round; a dead robot
//! or one between rounds cannot arm. The countdown is host state on the wire
//! ([`crate::wire::FighterState::destruct_in`]), so every client shows it and a predicting client
//! and a replay count it alike.
//!
//! **The blast** ([`Charge`]). When the countdown ends the robot dies at once, its own kill, and
//! its blast goes off at its centre. The blast's strength is the volatile contents of the parts
//! still connected at that tick: the fuel left in its connected tanks ([`FUEL_PER_DAMAGE`]) and
//! the electricity held in its connected batteries ([`CHARGE_PER_DAMAGE`]; the batteries' share
//! of the pool, not the root chassis' baseline), up to [`MAX_DAMAGE`], and its reach grows with
//! it ([`Charge::radius_units`]). A tank or battery lost before then took its contents with it
//! ([`crate::energy::Pool::shrink`]); a destroyed battery already set off its own small blast
//! ([`crate::damage`]) and adds nothing more; the robot's own death sets off no battery blast. So
//! a full robot blows hardest, an empty one, or one that lost its tanks and batteries, barely or
//! not at all, and nothing is counted twice. There is no explosive module and no ammunition yet
//! (shooter#129), so fuel and battery charge are the only volatile contents.
//!
//! The blast resolves through the existing damage ([`crate::session`]): every other living
//! robot's connected parts whose box comes within reach take the linear falloff to their
//! nearest point ([`crate::damage::Hitboxes::blast`]), but only where the arena's geometry does
//! not stand between the blast and that point (cover shelters what is behind it); a battery it
//! destroys blasts in turn as any destroyed battery does. Damageable placeables in reach take it
//! by their nearest point (the cover in the way among them) and target dummies in sight by their
//! centre. It pushes nothing.
//!
//! **Scoring**: the robot's own death is a suicide (it loses a frag, as every self-kill does);
//! each robot its blast kills is a frag for it, as any kill is. So a self-destruct that takes one
//! opponent with it scores nothing net; only taking two pays. Bots never self-destruct.
//!
//! Every number here is provisional (Atlas Q2.i123.2).

use crate::TICK_HZ;
use crate::damage::PartDamage;
use crate::energy::{MILLI, Pool};
use crate::robot::{Assembly, BETA, Kind};

/// The warning before the blast: 3 s.
pub const COUNTDOWN_TICKS: u32 = 3 * TICK_HZ as u32;
/// Units of fuel for one point of blast damage: a full tank (1000 fuel) is 100.
pub const FUEL_PER_DAMAGE: u32 = 10;
/// Units of battery charge for one point of blast damage: a full battery (60) is 60.
pub const CHARGE_PER_DAMAGE: u32 = 1;
/// The most damage a blast deals at its centre.
pub const MAX_DAMAGE: i32 = 300;
/// A blast's reach (units) is this plus its damage...
pub const BASE_RADIUS_UNITS: i32 = 150;
/// ...and at most this.
pub const MAX_RADIUS_UNITS: i32 = 450;

/// The volatile contents a robot carries: what its self-destruct blast is made of.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Charge {
    /// The fuel in its connected tanks, milli-units.
    pub fuel: u32,
    /// The electricity its connected batteries hold, milli-units: their share of the pool.
    pub electricity: u32,
}

impl Charge {
    /// What `build`, with `parts` connected, holding `energy` electricity and `fuel` fuel
    /// (milli-units), would blow up with now.
    #[must_use]
    pub fn of(build: &Assembly, parts: &PartDamage, energy: u32, fuel: u32) -> Self {
        let pool = Pool::attached(build, parts);
        let batteries: u32 = build
            .parts()
            .iter()
            .enumerate()
            .filter(|(index, _)| parts.attached(*index))
            .filter_map(|(_, part)| BETA.part_type(part.kind))
            .filter(|kind| kind.kind == Kind::Battery)
            .map(|kind| u32::from(kind.capacity) * MILLI)
            .sum();
        let electricity = if pool.capacity == 0 {
            0
        } else {
            let share = u64::from(energy.min(pool.capacity)) * u64::from(batteries)
                / u64::from(pool.capacity);
            u32::try_from(share).unwrap_or(u32::MAX)
        };
        Self {
            fuel: fuel.min(pool.fuel),
            electricity,
        }
    }

    /// The damage its blast deals at its centre (0: it only dies).
    #[must_use]
    pub fn damage(self) -> i32 {
        let fuel = self.fuel / (FUEL_PER_DAMAGE * MILLI);
        let electricity = self.electricity / (CHARGE_PER_DAMAGE * MILLI);
        i32::try_from(fuel + electricity)
            .unwrap_or(MAX_DAMAGE)
            .min(MAX_DAMAGE)
    }

    /// How far (units) its blast reaches: [`BASE_RADIUS_UNITS`] plus its damage, at most
    /// [`MAX_RADIUS_UNITS`]; nothing without damage.
    #[must_use]
    pub fn radius_units(self) -> i32 {
        match self.damage() {
            0 => 0,
            damage => (BASE_RADIUS_UNITS + damage).min(MAX_RADIUS_UNITS),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::robot::{HEAVY, STANDARD};

    #[test]
    fn a_full_robot_blows_by_its_tank_and_batteries_and_an_empty_one_not_at_all() {
        let parts = PartDamage::fresh(&HEAVY);
        let pool = Pool::of(&HEAVY);
        let full = Charge::of(&HEAVY, &parts, pool.capacity, pool.fuel);
        assert!(full.fuel > 0 && full.electricity > 0, "{full:?}");
        assert!(full.damage() > 100, "{}", full.damage());
        assert_eq!(Charge::of(&HEAVY, &parts, 0, 0).damage(), 0);
        assert_eq!(Charge::of(&HEAVY, &parts, 0, 0).radius_units(), 0);
        // The standard build has a tank and no battery: its charge is only fuel.
        let parts = PartDamage::fresh(&STANDARD);
        let pool = Pool::of(&STANDARD);
        let standard = Charge::of(&STANDARD, &parts, pool.capacity, pool.fuel);
        assert_eq!(standard.electricity, 0);
        assert_eq!(standard.damage(), 100);
        assert_eq!(standard.radius_units(), 250);
    }
}
