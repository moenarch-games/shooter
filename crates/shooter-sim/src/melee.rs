//! The energy blade (Atlas D2.6; the Q3 defaults): a quick melee swing on its own key, whatever
//! gun is held. A swing covers every fighter and dummy whose body is within the shared melee
//! reach, arc and height band of `sim-combat` (mmorpg's player swing: 2.5 m centre distance
//! across the ground, 50° either side of the facing, bodies overlapping 1 m below to 2 m above
//! the swinger's feet) and in the swinger's line of sight past the arena's geometry. Online the
//! fighters are where the swinger saw them, like hitscan ([`crate::session`]).
//!
//! `sim-combat` shares both the constants and the coverage test ([`swing_covers`]).

use crate::player::PLAYER_HALF_EXTENTS;
use crate::weapons::subunits;
use sim_combat::swing::swing_covers;
use sim_combat::unit::{PLAYER_REACH_UNITS, SWING_ARC_HALF_WIDTH, SWING_BAND_UNITS};
use sim_foundation::physics::{SUBUNITS_PER_UNIT, Vec3i};
use sim_foundation::{subunits_per_tick, trig};

/// Damage of one swing: four take a full health pool.
pub const DAMAGE: i32 = 50;
/// Ticks between two swings: 0.5 s.
pub const SWING_TICKS: u32 = 30;
/// Ticks after a swing before the gun fires again: 0.3 s.
pub const GUN_LOCKOUT_TICKS: u32 = 18;
/// A struck body is shoved 6 m/s along the swing's facing...
pub const KNOCKBACK_SUBUNITS_PER_TICK: i32 = subunits_per_tick(600);
/// ...and lifted 2.4 m/s.
pub const LIFT_SUBUNITS_PER_TICK: i32 = subunits_per_tick(240);
/// How far a swing reaches across the ground (centre to centre), in units.
pub const REACH_UNITS: i32 = PLAYER_REACH_UNITS;

/// Whether a swing from a fighter whose feet are at `feet` (units), facing `yaw`, covers a
/// player-sized body centred at `center` (units): `sim-combat`'s shared swing coverage, run in
/// subunits so positions keep their sub-centimetre precision.
#[must_use]
pub fn covers(feet: [f32; 3], yaw: u16, center: [f32; 3]) -> bool {
    let point = |units| {
        let [x, y, z] = subunits(units);
        Vec3i::new(x, y, z)
    };
    let half = Vec3i::new(
        PLAYER_HALF_EXTENTS.x * SUBUNITS_PER_UNIT,
        PLAYER_HALF_EXTENTS.y * SUBUNITS_PER_UNIT,
        PLAYER_HALF_EXTENTS.z * SUBUNITS_PER_UNIT,
    );
    swing_covers(
        point(feet),
        yaw,
        point(center),
        half,
        REACH_UNITS * SUBUNITS_PER_UNIT,
        SWING_ARC_HALF_WIDTH,
        SWING_BAND_UNITS.map(|band| band * SUBUNITS_PER_UNIT),
    )
}

/// The knockback (subunits per tick) of a swing facing `yaw`.
#[must_use]
pub fn knockback(yaw: u16) -> [i32; 3] {
    let (x, z) = trig::direction(yaw);
    let scale = |unit| trig::checked_scale(KNOCKBACK_SUBUNITS_PER_TICK, unit).unwrap_or(0);
    [scale(x), LIFT_SUBUNITS_PER_TICK, scale(z)]
}

/// The point of a player-sized box centred at `center` nearest to `eye` (units): where the
/// blade meets it.
#[must_use]
pub fn contact(eye: [f32; 3], center: [f32; 3]) -> [f32; 3] {
    #[allow(clippy::cast_precision_loss)]
    let half = [
        PLAYER_HALF_EXTENTS.x as f32,
        PLAYER_HALF_EXTENTS.y as f32,
        PLAYER_HALF_EXTENTS.z as f32,
    ];
    [0, 1, 2].map(|axis| eye[axis].clamp(center[axis] - half[axis], center[axis] + half[axis]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_swing_covers_reach_arc_and_band() {
        let feet = [0.0, 0.0, 0.0];
        // Yaw 0 faces +Z; a body 2 m ahead standing on the same floor.
        assert!(covers(feet, 0, [0.0, 90.0, 200.0]));
        // Beyond 2.5 m, behind, or 70° off to the side: not covered.
        assert!(!covers(feet, 0, [0.0, 90.0, 260.0]));
        assert!(!covers(feet, 0, [0.0, 90.0, -100.0]));
        assert!(!covers(feet, 0, [190.0, 90.0, 70.0]));
        // 40° off to the side is inside the 50° half arc.
        assert!(covers(feet, 0, [130.0, 90.0, 155.0]));
        // Far above the 2 m band (its feet 2.5 m up) or below it.
        assert!(!covers(feet, 0, [0.0, 340.0, 100.0]));
        assert!(!covers(feet, 0, [0.0, -200.0, 100.0]));
        // Standing on the swinger: covered.
        assert!(covers(feet, 0, [0.0, 90.0, 0.0]));
    }

    #[test]
    fn knockback_follows_the_facing() {
        let [x, y, z] = knockback(0);
        assert_eq!((x, y), (0, LIFT_SUBUNITS_PER_TICK));
        assert_eq!(z, KNOCKBACK_SUBUNITS_PER_TICK);
    }
}
