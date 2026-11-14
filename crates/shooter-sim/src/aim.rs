//! Where a shot lands: the one answer the host's hitscan and the clients' reticle share.
//!
//! Every shot leaves the fighter's eye ([`crate::Player::eye`]) along the Q16 aim of its yaw and
//! pitch ([`crate::weapons::aim`]), never the camera. [`sight`] is the hitscan's first contact:
//! the arena as the physics world has it now and the fighters where the shooter sees them. The
//! host resolves every hitscan beam with it, and a third-person client draws its reticle where
//! [`sight`] ends for the same eye and aim, so what the reticle covers is what the shot meets,
//! cover in front of the fighter included, wherever the camera sits.

use crate::physics::{BodyId, World};
use crate::session::HALF;
use crate::weapons::{self, Trace};

/// How far (units) a client looks along its aim for the reticle: further than any arena.
pub const RETICLE_RANGE_UNITS: i32 = 20_000;

/// What a ray along a fighter's aim met first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sight {
    /// Where the ray ended: on the first fighter, else on the scenery, else at its range.
    pub end: [f32; 3],
    /// The fighter it met first (its index in the caller's list) and the fraction of the range
    /// it met it at, if it met one before the scenery.
    pub fighter: Option<(usize, f32)>,
    /// What the ray met among the scenery, fighters ignored.
    pub scenery: Trace,
}

/// A ray from `eye` (units) along the Q16 `direction` for `range_units`: the first of the
/// scenery (the bodies of `world` other than `owner` that `scenery` accepts) and the `fighters`
/// (an index and a body centre in units, each a box of [`HALF`]).
#[must_use]
pub fn sight(
    world: &World,
    eye: [f32; 3],
    direction: [i32; 3],
    range_units: i32,
    owner: BodyId,
    scenery: impl Fn(BodyId) -> bool,
    fighters: impl IntoIterator<Item = (usize, [f32; 3])>,
) -> Sight {
    #[allow(clippy::cast_precision_loss)]
    let unit = direction.map(|axis| axis as f32 / crate::trig::TRIG_ONE as f32);
    let scenery = weapons::trace(world, eye, direction, range_units, owner, scenery);
    #[allow(clippy::cast_precision_loss)]
    let translation = unit.map(|axis| axis * range_units as f32);
    let reach = scenery.hit.map_or(1.0, |hit| hit.fraction);
    let mut target: Option<(usize, f32)> = None;
    for (index, center) in fighters {
        if let Some(fraction) = weapons::ray_box(eye, translation, center, HALF)
            && fraction < reach
            && target.is_none_or(|(_, best)| fraction < best)
        {
            target = Some((index, fraction));
        }
    }
    let end = target.map_or(scenery.end, |(_, fraction)| {
        [0, 1, 2].map(|axis| eye[axis] + translation[axis] * fraction)
    });
    Sight {
        end,
        fighter: target,
        scenery,
    }
}
