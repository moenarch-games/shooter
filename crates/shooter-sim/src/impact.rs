//! Collision damage (Atlas D2.i70.10, shooter#75): a high-energy collision between robots, with
//! debris or with the arena, or a high fall, damages the component actually hit, under the same
//! health, detachment and armor rules as a weapon's strike ([`crate::damage`]).
//!
//! **What the impact is.** physics-engine reports which pairs started touching in a step, not
//! the impulse a contact gave (moritzbrantner/physics-engine#575), so the impact is measured on
//! the robot as a whole, deterministically and alike on both backends, from three velocities of
//! its centre of mass: as the last step left it, as the tick's drives set it before this step,
//! and as this step left it. Whatever changed across the step beyond gravity's share is what its
//! contacts gave it (its joints and motors are internal and cancel in the centre of mass, so a
//! leg's servos never count): the impulse a unit of its mass took, `Δv`, pointing away from what
//! it hit. The relative speed and the masses on both sides are in it: a robot rammed by a
//! heavier one takes a larger `Δv` than the heavier one does, a robot that hits a wall or the
//! floor loses its whole closing speed. What it cannot see: simultaneous contacts apart (their
//! sum is one impact) and a robot crushed without its velocity changing (pinned to a wall).
//!
//! **What does not count.** The movement drive ([`crate::mechanics::Mechanics::drive`]) sets a
//! robot's velocity outright, which is not momentum it had: the part of `Δv` that only takes back
//! what the drive put in this tick toward the contact (walking or driving into a wall, leaning on
//! another robot) is not an impact. What is left must reach [`IMPACT_SPEED`] to do any damage, so
//! walking, sprinting, sliding (15 m/s) into a wall, jump and pad landings, the drops of the
//! shipped maps, small bumps and resting contact (gravity's share a tick, about 30 units a
//! second) never damage, and a robot lying still is never hurt again tick after tick: the
//! measure is the change of one step, so a contact that goes on changes nothing once it has
//! stopped the robot. Only the grapple's pull (up to 24 m/s) is faster than the threshold among
//! the shipped moves: reeled into a wall at nearly its top speed, a robot takes a little damage.
//! A hard impact's change mostly lands within one step (a 12 m fall: about 24 m/s in its first
//! step, 2 in the next); one that spreads over steps counts each step's share alone.
//!
//! **Where it lands.** The impact strikes the robot's part reaching furthest toward what it hit
//! (against `Δv`; ties within [`TIE_UNITS`] by part ID, so the choice does not depend on the order
//! the physics met its contacts), along the line from outside through that part's centre, with
//! the weapons' ray resolution and no penetration ([`crate::damage::Hitboxes::pierce`]): an armor
//! plate in the way takes it all. Above the threshold every unit a second of `Δv` deals
//! [`DAMAGE_PER_SPEED`] (hundredths, rounded down). The root chassis at zero is the kill, as for
//! any strike. Thresholds and scaling are provisional (Atlas Q2.i70.5).

use crate::damage::{Blow, Hitboxes};

/// The change of velocity (units a second) a robot's contacts must give it in one step, beyond
/// what only takes back its drive, before it is damaged: 20 m/s, above a slide's 15 and the
/// bots' landings in the shipped maps (at most about 18 m/s measured), about an 11 m fall
/// (Atlas Q2.i70.5, provisional).
pub const IMPACT_SPEED: f32 = 2_000.0;
/// Damage a unit a second of `Δv` above [`IMPACT_SPEED`] deals, in hundredths: 20 a metre a
/// second (Atlas Q2.i70.5, provisional).
pub const DAMAGE_PER_SPEED: i32 = 20;
/// Parts reaching toward the contact within this (units) of the furthest count as tied.
pub const TIE_UNITS: f32 = 0.5;

/// One step's impact on a robot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Impact {
    /// The velocity change that counts (units a second).
    pub speed: f32,
    /// The unit direction of the impulse: away from what it hit.
    pub direction: [f32; 3],
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The impact one step gave a robot whose centre of mass moved at `before` after the last step
/// (units a second), was set moving at `set` by the tick's drives and moved at `after` after the
/// step, under `gravity` (units a second, a step's worth). `None` without any change beyond
/// gravity's.
#[must_use]
pub fn measure(
    before: [f32; 3],
    set: [f32; 3],
    after: [f32; 3],
    gravity: [f32; 3],
) -> Option<Impact> {
    let change: [f32; 3] = std::array::from_fn(|axis| after[axis] - set[axis] - gravity[axis]);
    let size = dot(change, change).sqrt();
    if !size.is_finite() || size <= f32::EPSILON {
        return None;
    }
    let direction = change.map(|axis| axis / size);
    let driven: [f32; 3] = std::array::from_fn(|axis| set[axis] - before[axis]);
    // What the drive pushed toward the contact this tick, taken back by it: not an impact.
    let undone = (-dot(driven, direction)).max(0.0);
    Some(Impact {
        speed: (size - undone).max(0.0),
        direction,
    })
}

/// The damage an impact of `speed` (units a second) deals: none up to [`IMPACT_SPEED`].
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn damage(speed: f32) -> i32 {
    if speed.is_nan() || speed <= IMPACT_SPEED {
        return 0;
    }
    let over = ((speed - IMPACT_SPEED) * DAMAGE_PER_SPEED as f32 / 100.0).floor();
    if over >= i32::MAX as f32 {
        i32::MAX
    } else {
        over as i32
    }
}

/// How far `part`'s box reaches along `toward` (a unit vector).
fn reach(part: &crate::damage::PartBox, toward: [f32; 3]) -> f32 {
    dot(part.center, toward)
        + part
            .axes
            .iter()
            .zip(part.half)
            .map(|(axis, half)| dot(*axis, toward).abs() * half)
            .sum::<f32>()
}

/// The blows an impact of `damage` along `direction` (the impulse's, [`Impact::direction`])
/// deals to a robot whose connected parts are `parts`: the part reaching furthest against
/// `direction` (ties by part ID) is where it comes from, and a ray from outside through that
/// part's centre resolves it as a weapon's strike without penetration.
#[must_use]
pub fn strike(parts: &Hitboxes, direction: [f32; 3], damage: i32) -> Vec<Blow> {
    if damage <= 0 {
        return Vec::new();
    }
    let toward = direction.map(|axis| -axis);
    let furthest = parts
        .boxes
        .iter()
        .map(|part| reach(part, toward))
        .fold(f32::MIN, f32::max);
    // Boxes are in part-ID order: the first within the tie is the lowest ID.
    let Some(part) = parts
        .boxes
        .iter()
        .find(|part| reach(part, toward) >= furthest - TIE_UNITS)
    else {
        return Vec::new();
    };
    let out = reach(part, toward) - dot(part.center, toward) + 10.0;
    let origin: [f32; 3] = std::array::from_fn(|axis| part.center[axis] + toward[axis] * out);
    let translation = direction.map(|axis| axis * out);
    let blows = parts.pierce(origin, translation, damage, 0);
    if blows.is_empty() {
        let at = std::array::from_fn(|axis| part.center[axis] + toward[axis] * (out - 10.0));
        return vec![Blow {
            index: part.index,
            damage,
            at,
        }];
    }
    blows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drive_into_a_wall_is_no_impact() {
        // Pushing at a sprint into a wall it already rests against.
        let impact = measure([0.0; 3], [1_050.0, 0.0, 0.0], [0.0; 3], [0.0; 3]).unwrap();
        assert!(impact.speed.abs() < 1e-3, "{impact:?}");
        // Running into it at a sprint: the whole closing speed, below the threshold.
        let impact = measure([1_050.0, 0.0, 0.0], [1_050.0, 0.0, 0.0], [0.0; 3], [0.0; 3]).unwrap();
        assert!((impact.speed - 1_050.0).abs() < 1e-3);
        assert_eq!(damage(impact.speed), 0);
        assert_eq!(impact.direction, [-1.0, 0.0, 0.0]);
    }

    #[test]
    fn damage_starts_above_the_threshold() {
        assert_eq!(damage(IMPACT_SPEED), 0);
        assert_eq!(damage(IMPACT_SPEED + 100.0), 20);
        assert_eq!(damage(IMPACT_SPEED + 1_000.0), 200);
        assert_eq!(damage(f32::NAN), 0);
    }
}
