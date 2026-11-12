//! The energy blade (Atlas D2.6; the Q3 defaults): a quick melee swing on its own key, whatever
//! gun is held. A swing covers every fighter and dummy whose body is within the melee reach, arc
//! and height band (2.5 m centre distance across the ground, 50° either side of the facing,
//! bodies overlapping 1 m below to 2 m above the swinger's feet; the values of mmorpg's player
//! swing) and in the swinger's line of sight past the arena's geometry. Online the fighters are
//! where the swinger saw them, like hitscan ([`crate::session`]).

use crate::fixed::{SUBUNITS_PER_UNIT, subunits_per_tick};
use crate::physics::Vec3i;
use crate::player::PLAYER_HALF_EXTENTS;
use crate::trig;
use crate::weapons::subunits;

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
pub const REACH_UNITS: i32 = 250;
/// Half the horizontal arc a swing covers: 50° either side of the facing (yaw units: a quarter
/// turn is 16 384).
pub const ARC_HALF_WIDTH: u16 = 9_102;
/// The inclusive height band above the swinger's feet, in units, that a swing covers: a body is
/// covered when its vertical extent overlaps it.
pub const BAND_UNITS: [i32; 2] = [-100, 200];

/// Whether a swing from a fighter whose feet are at `feet` (units), facing `yaw`, covers a
/// player-sized body centred at `center` (units), in subunits so positions keep their
/// sub-centimetre precision.
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
        ARC_HALF_WIDTH,
        BAND_UNITS.map(|band| band * SUBUNITS_PER_UNIT),
    )
}

/// Whether a target centred at `target` with half extents `target_half` lies within `reach`
/// (XZ centre distance) of `feet`, inside `half_width` either side of `facing`, and overlaps the
/// inclusive `band` above `feet`. A target centred on the swinger is always inside the arc.
fn swing_covers(
    feet: Vec3i,
    facing: u16,
    target: Vec3i,
    target_half: Vec3i,
    reach: i32,
    half_width: u16,
    band: [i32; 2],
) -> bool {
    let bottom = i64::from(target.y) - i64::from(target_half.y) - i64::from(feet.y);
    let top = i64::from(target.y) + i64::from(target_half.y) - i64::from(feet.y);
    if top < i64::from(band[0]) || bottom > i64::from(band[1]) {
        return false;
    }
    let [dx, dz] = [
        i64::from(target.x) - i64::from(feet.x),
        i64::from(target.z) - i64::from(feet.z),
    ];
    // Saturates only for positions near opposite ends of the `i32` range, out of every reach.
    let distance_squared = dx.saturating_mul(dx).saturating_add(dz.saturating_mul(dz));
    if distance_squared > i64::from(reach).pow(2) {
        return false;
    }
    // Within reach, so the offset fits `i32` again.
    #[allow(clippy::cast_possible_truncation)]
    let (dx, dz) = (dx as i32, dz as i32);
    let (dir_x, dir_z) = trig::direction(facing);
    // |v| cos(angle to the facing) >= |v| cos(half width), both Q16.
    let along = i64::from(dx) * i64::from(dir_x) + i64::from(dz) * i64::from(dir_z);
    #[allow(clippy::cast_possible_wrap)]
    let length = trig::xz_length(dx, dz) as i64;
    along >= length * i64::from(trig::cos(half_width))
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
