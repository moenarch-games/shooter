//! The grapple hook (Atlas D2.5): fired at arena geometry, it reels the player toward the hit
//! point while the rope keeps them within its length, so momentum across the rope swings them
//! round the anchor.
//!
//! physics-engine cannot yet pull or swing a kinematic character on a rope
//! (moritzbrantner/physics-engine#557), so this is a deterministic stand-in: each tick before the
//! world steps, [`Hook::pull`] adds the reel's acceleration toward the anchor and removes any
//! velocity that would stretch the rope. The physics world's character move still resolves every
//! collision. [`Hook::pull`] is the whole seam: when the engine has rope joints on characters,
//! its body becomes that joint and nothing else changes.
//!
//! Everything is integer: positions and velocities in subunits, lengths integer square roots.

use crate::arena::FIRST_PLAYER_BODY;
use crate::weapons::{self, subunits};
use sim_foundation::physics::{BodyId, PhysicsError, RigidBody, SUBUNITS_PER_UNIT, Vec3i, World};
use sim_foundation::{subunits_per_tick, subunits_per_tick_squared, trig};

/// The hook reaches 30 m.
pub const RANGE_UNITS: i32 = 3_000;
/// The reel pulls at 45 m/s², two and a half times gravity.
pub const PULL_SUBUNITS_PER_TICK: i32 = subunits_per_tick_squared(4_500);
/// The pull never takes the player past 24 m/s (swings may carry more from a fall).
pub const MAX_PULL_SPEED_SUBUNITS_PER_TICK: i32 = subunits_per_tick(2_400);
/// The reel stops 1 m from the anchor; the player hangs there until they let go.
pub const MIN_LENGTH_UNITS: i32 = 100;

/// A hook set in the arena: part of the simulation state a server owns and a client predicts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Hook {
    /// Where it holds, in subunits.
    pub anchor: [i32; 3],
    /// The rope's length, in subunits: it only ever shortens.
    pub length: i32,
}

/// Whether a hook can hold on `body`: arena geometry only, never fighters or dummies.
#[must_use]
pub const fn holds_on(body: BodyId) -> bool {
    body.0 < FIRST_PLAYER_BODY
}

fn distance(offset: [i64; 3]) -> i64 {
    #[allow(clippy::cast_sign_loss)]
    let squared = offset.iter().map(|axis| axis * axis).sum::<i64>() as u64;
    #[allow(clippy::cast_possible_wrap)]
    let length = trig::isqrt(squared) as i64;
    length
}

fn narrow(value: i64) -> i32 {
    i32::try_from(value).unwrap_or(if value < 0 { i32::MIN } else { i32::MAX })
}

impl Hook {
    /// Fires a hook from `eye` (units) along the Q16 `direction`, ignoring `owner`: set where it
    /// first meets arena geometry within [`RANGE_UNITS`], or `None` with where the shot ended
    /// (a miss, or a fighter or dummy in the way).
    #[must_use]
    pub fn fire(
        world: &World,
        eye: [f32; 3],
        direction: [i32; 3],
        owner: BodyId,
    ) -> (Option<Self>, weapons::Trace) {
        let trace = weapons::trace(world, eye, direction, RANGE_UNITS, owner, |_| true);
        let hook = trace.hit.filter(|hit| holds_on(hit.body)).map(|_| {
            let anchor = subunits(trace.end);
            let from = subunits(eye);
            let offset = [0, 1, 2].map(|axis| i64::from(anchor[axis] - from[axis]));
            Self {
                anchor,
                length: narrow(distance(offset)),
            }
        });
        (hook, trace)
    }

    /// The anchor in units.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn anchor_units(&self) -> [f32; 3] {
        self.anchor
            .map(|axis| axis as f32 / SUBUNITS_PER_UNIT as f32)
    }

    /// One tick of rope on `body`, whose eye is `eye` (units): reels the rope in, pulls toward
    /// the anchor and takes out any velocity that would stretch the rope past its length. Call
    /// after the body's own movement and before the world steps.
    pub fn pull(
        &mut self,
        world: &mut World,
        body: BodyId,
        eye: [f32; 3],
    ) -> Result<(), PhysicsError> {
        let velocity = world
            .body(body)
            .map(RigidBody::velocity)
            .unwrap_or_default();
        let from = subunits(eye);
        let offset = [0, 1, 2].map(|axis| i64::from(self.anchor[axis] - from[axis]));
        let span = distance(offset);
        if span == 0 {
            return Ok(());
        }
        let min_length = i64::from(MIN_LENGTH_UNITS * SUBUNITS_PER_UNIT);
        let mut v = [velocity.x, velocity.y, velocity.z].map(i64::from);
        // The reel: pull toward the anchor until the pull's own speed limit.
        let toward = |v: &[i64; 3]| v.iter().zip(&offset).map(|(v, o)| v * o).sum::<i64>() / span;
        if span > min_length && toward(&v) < i64::from(MAX_PULL_SPEED_SUBUNITS_PER_TICK) {
            for axis in 0..3 {
                v[axis] += offset[axis] * i64::from(PULL_SUBUNITS_PER_TICK) / span;
            }
        }
        // The rope: never further than its length. Outward speed goes (the swing keeps the
        // rest), and a rope stretched by last tick's gravity draws the body back in.
        let length = i64::from(self.length);
        if span >= length {
            let inward = toward(&v);
            let wanted = (span - length).min(i64::from(MAX_PULL_SPEED_SUBUNITS_PER_TICK));
            if inward < wanted {
                for axis in 0..3 {
                    v[axis] += offset[axis] * (wanted - inward) / span;
                }
            }
        }
        // Reel in: the rope shortens to the distance reached, down to its minimum.
        self.length = narrow(span.max(min_length).min(length));
        world.set_velocity(body, Vec3i::new(narrow(v[0]), narrow(v[1]), narrow(v[2])))
    }
}
