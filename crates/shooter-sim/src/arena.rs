//! The greybox arena: one floor, perimeter walls and a few blocks and ramps, as fixed physics
//! bodies. The clients draw the same pieces, so what a player sees is what they collide with.
//!
//! Lengths are centimetre units, as in `sim-foundation`. Y is up; the floor's top is at 0.

use sim_foundation::RampRise;
use sim_foundation::physics::{BodyId, PhysicsError, RigidBody, Vec3i, World};

/// One fixed piece of arena geometry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Piece {
    /// An axis-aligned box: centre and half extents.
    Block { center: Vec3i, half: Vec3i },
    /// A wedge filling the lower half of its box, its top rising toward `rises` (a
    /// `sim-foundation` ramp).
    Ramp {
        center: Vec3i,
        half: Vec3i,
        rises: RampRise,
    },
}

impl Piece {
    #[must_use]
    pub const fn center(&self) -> Vec3i {
        match *self {
            Self::Block { center, .. } | Self::Ramp { center, .. } => center,
        }
    }

    #[must_use]
    pub const fn half(&self) -> Vec3i {
        match *self {
            Self::Block { half, .. } | Self::Ramp { half, .. } => half,
        }
    }
}

/// A whole arena: its floor extent, pieces and where a player spawns.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Arena {
    /// The floor spans `±half_size` units on X and Z, inside the perimeter walls.
    pub half_size: i32,
    pub pieces: Vec<Piece>,
    /// A spawning player's feet.
    pub spawn: Vec3i,
    /// The yaw a spawning player faces (`sim_foundation::trig`: 0 faces +Z, increasing toward +X).
    pub spawn_yaw: u16,
}

/// The floor's body; pieces take the IDs after it, players start at [`FIRST_PLAYER_BODY`].
pub const FLOOR_BODY: BodyId = BodyId(1);
pub const FIRST_PLAYER_BODY: u64 = 1_000;

const fn block(center: [i32; 3], half: [i32; 3]) -> Piece {
    Piece::Block {
        center: Vec3i::new(center[0], center[1], center[2]),
        half: Vec3i::new(half[0], half[1], half[2]),
    }
}

const fn ramp(center: [i32; 3], half: [i32; 3], rises: RampRise) -> Piece {
    Piece::Ramp {
        center: Vec3i::new(center[0], center[1], center[2]),
        half: Vec3i::new(half[0], half[1], half[2]),
        rises,
    }
}

impl Arena {
    /// The first greybox: a 40 m square hall with 4 m walls, a central 2 m platform reached by
    /// two ramps, crates to hop onto, pillars and low cover walls.
    #[must_use]
    pub fn greybox() -> Self {
        let half_size = 2_000;
        let wall = 50;
        let outer = half_size + wall;
        let pieces = vec![
            // Perimeter walls, 4 m high.
            block([0, 200, outer], [outer + wall, 200, wall]),
            block([0, 200, -outer], [outer + wall, 200, wall]),
            block([outer, 200, 0], [wall, 200, outer - wall]),
            block([-outer, 200, 0], [wall, 200, outer - wall]),
            // Central platform, top at 2 m.
            block([0, 100, 0], [400, 100, 400]),
            // Ramps onto it from the south (+Z) and the west (-X), 14°.
            ramp([0, 100, 800], [200, 100, 400], RampRise::NegZ),
            ramp([-800, 100, 0], [400, 100, 200], RampRise::PosX),
            // Crates, 80 cm: a jump clears them.
            block([1_000, 40, 1_000], [40, 40, 40]),
            block([1_100, 40, 1_000], [40, 40, 40]),
            block([1_000, 40, -1_000], [40, 40, 40]),
            block([-1_200, 40, 1_200], [40, 40, 40]),
            // Pillars to the ceiling height of the walls.
            block([1_200, 200, -1_400], [60, 200, 60]),
            block([-1_200, 200, -1_400], [60, 200, 60]),
            // Low cover walls, 1.2 m.
            block([1_400, 60, 0], [30, 60, 250]),
            block([0, 60, -1_400], [250, 60, 30]),
        ];
        Self {
            half_size,
            pieces,
            spawn: Vec3i::new(0, 0, 1_600),
            // Half a turn: facing -Z, toward the platform.
            spawn_yaw: 32_768,
        }
    }

    /// A physics world holding this arena's floor and pieces, under `gravity` (subunits per tick²).
    pub fn build_world(&self, gravity: Vec3i) -> Result<World, PhysicsError> {
        let mut world = World::new(gravity);
        world.add_body(RigidBody::floor(FLOOR_BODY, 0))?;
        for (index, piece) in self.pieces.iter().enumerate() {
            let id = BodyId(FLOOR_BODY.0 + 1 + index as u64);
            world.add_body(match *piece {
                Piece::Block { center, half } => RigidBody::fixed(id, center, half),
                Piece::Ramp {
                    center,
                    half,
                    rises,
                } => RigidBody::ramp(id, center, half, rises),
            })?;
        }
        Ok(world)
    }
}
