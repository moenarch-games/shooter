//! The greybox arena: one floor, perimeter walls and a few blocks and ramps, as fixed physics
//! bodies. The clients draw the same pieces, so what a player sees is what they collide with.
//!
//! Lengths are centimetre units, as in physics-engine (`length_unit` 100). Y is up; the floor's top is at 0.

use crate::physics::RampRise;
use crate::physics::{BodyId, PhysicsError, RigidBody, Vec3i, World};
use crate::pickups::{ArenaModule, ModulePad};

/// One fixed piece of arena geometry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Piece {
    /// An axis-aligned box: centre and half extents.
    Block { center: Vec3i, half: Vec3i },
    /// A wedge filling the lower half of its box, its top rising toward `rises` (a
    /// [`crate::physics::RigidBody::ramp`]).
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

/// A jump pad: a flat plate on the ground that throws a player standing on it. It has no
/// collider of its own (it is flush with the floor), only its footprint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JumpPad {
    /// The centre of the plate's top, on the ground it lies on.
    pub center: Vec3i,
    /// Half its footprint on X and Z.
    pub half: [i32; 2],
    /// The velocity it sets, units per second (a multiple of 15 per axis, so it is exact in
    /// subunits per tick).
    pub launch: Vec3i,
}

/// How far above or below a pad's top a player's feet may be and still be thrown.
pub const PAD_REACH_UNITS: i32 = 10;

impl JumpPad {
    /// Whether feet at `feet` (units) stand on this pad.
    #[must_use]
    pub const fn under(&self, feet: Vec3i) -> bool {
        (feet.x - self.center.x).abs() <= self.half[0]
            && (feet.z - self.center.z).abs() <= self.half[1]
            && (feet.y - self.center.y).abs() <= PAD_REACH_UNITS
    }

    /// The launch velocity in subunits per tick.
    #[must_use]
    pub const fn launch_velocity(&self) -> Vec3i {
        use crate::fixed::subunits_per_tick;
        Vec3i::new(
            subunits_per_tick(self.launch.x),
            subunits_per_tick(self.launch.y),
            subunits_per_tick(self.launch.z),
        )
    }
}

/// Where a fighter appears: its feet and the yaw it faces (`crate::trig`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Spawn {
    pub feet: Vec3i,
    pub yaw: u16,
}

const fn spawn_at(feet: [i32; 3], yaw: u16) -> Spawn {
    Spawn {
        feet: Vec3i::new(feet[0], feet[1], feet[2]),
        yaw,
    }
}

/// A whole arena: its floor extent, pieces, jump pads and where players spawn.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Arena {
    /// The floor spans `±half_size` units on X and Z, inside the perimeter walls.
    pub half_size: i32,
    pub pieces: Vec<Piece>,
    pub pads: Vec<JumpPad>,
    /// Where target dummies stand.
    pub posts: Vec<Post>,
    /// A spawning player's feet.
    pub spawn: Vec3i,
    /// The yaw a spawning player faces (`crate::trig`: 0 faces +Z, increasing toward +X).
    pub spawn_yaw: u16,
    /// More deathmatch spawn points after the first one ([`Arena::spawn`]).
    pub spawns: Vec<Spawn>,
    /// Where the contested module pickups lie ([`crate::pickups`]), in the order the host
    /// resolves them.
    pub module_pads: Vec<ModulePad>,
}

/// The floor's body; pieces take the IDs after it, players start at [`FIRST_PLAYER_BODY`].
pub const FLOOR_BODY: BodyId = BodyId(1);
pub const FIRST_PLAYER_BODY: u64 = 1_000;
/// Target dummies take the IDs from here, in post order.
pub const FIRST_DUMMY_BODY: u64 = 2_000;

/// Where a target dummy stands: its feet, and how far it walks to either side along X (0 stands
/// still).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Post {
    pub feet: Vec3i,
    pub patrol_units: i32,
}

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
    /// two ramps, crates to hop onto, pillars, low cover walls and two jump pads that throw a
    /// player onto the platform from the east and the north, and two module pickups.
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
        // Both pads throw 12 m/s up (a 4 m apex) and 6 m/s toward the platform: about 1.1 s in
        // the air and 6.8 m across, landing on the platform's near half.
        let pads = vec![
            JumpPad {
                center: Vec3i::new(1_000, 0, 0),
                half: [80, 80],
                launch: Vec3i::new(-600, 1_200, 0),
            },
            JumpPad {
                center: Vec3i::new(0, 0, -1_000),
                half: [80, 80],
                launch: Vec3i::new(0, 1_200, 600),
            },
        ];
        // Deathmatch spawns round the hall, each facing the platform: the north, east and west
        // walls' middles, the four corners, the platform's top and one more along each wall (a
        // pinwheel between the middles and the corners). With the first one that is a point of
        // its own for each of a 12-slot match's fighters at the start of a round, and one spare.
        let spawns = vec![
            spawn_at([0, 0, -1_700], 0),
            spawn_at([1_700, 0, 0], 49_152),
            spawn_at([-1_700, 0, 0], 16_384),
            spawn_at([-1_600, 0, 1_600], 24_576),
            spawn_at([1_600, 0, -1_600], 57_344),
            spawn_at([1_600, 0, 1_600], 40_960),
            spawn_at([-1_600, 0, -1_600], 8_192),
            spawn_at([0, 200, -200], 32_768),
            spawn_at([800, 0, 1_600], 37_604),
            spawn_at([-800, 0, -1_700], 4_588),
            spawn_at([1_700, 0, -800], 53_740),
            spawn_at([-1_700, 0, 800], 20_972),
        ];
        // Module pickups at contested points. The rail capacitor lies on the platform's
        // south-east corner: the perch that sees, and is seen from, the whole hall, reached by
        // the ramps, the jump pads or a grapple. The thrusters lie in the open south-west
        // quarter between the two ramps' feet, under the perch's sightline: whoever takes them
        // is exposed to it, and their 2.25 m jump climbs straight onto the platform.
        let module_pads = vec![
            ModulePad {
                center: Vec3i::new(300, 200, 300),
                module: ArenaModule::RailCapacitor,
            },
            ModulePad {
                center: Vec3i::new(-1_000, 0, 1_000),
                module: ArenaModule::Thrusters,
            },
        ];
        Self {
            half_size,
            pieces,
            pads,
            // Bots are the targets now; only the scripted shooting range has dummies.
            posts: Vec::new(),
            spawn: Vec3i::new(0, 0, 1_600),
            // Half a turn: facing -Z, toward the platform.
            spawn_yaw: 32_768,
            spawns,
            module_pads,
        }
    }

    /// Every spawn point: [`Arena::spawn`] first, then [`Arena::spawns`].
    #[must_use]
    pub fn spawn_points(&self) -> Vec<Spawn> {
        std::iter::once(Spawn {
            feet: self.spawn,
            yaw: self.spawn_yaw,
        })
        .chain(self.spawns.iter().copied())
        .collect()
    }

    /// Whether a fighter walking straight from `from` to `to` (feet, units) on the floor meets
    /// no piece taller than a step (ramps excepted): bots pick waypoints they can walk to.
    #[must_use]
    pub fn clear_walk(&self, from: Vec3i, to: Vec3i) -> bool {
        const SAMPLES: i32 = 16;
        const MARGIN: i32 = 40;
        (0..=SAMPLES).all(|step| {
            let x = from.x + (to.x - from.x) * step / SAMPLES;
            let z = from.z + (to.z - from.z) * step / SAMPLES;
            let feet = from.y + (to.y - from.y) * step / SAMPLES;
            self.pieces.iter().all(|piece| match *piece {
                Piece::Ramp { .. } => true,
                Piece::Block { center, half } => {
                    let top = center.y + half.y;
                    top <= feet + crate::player::STEP_UP.max_height
                        || (x - center.x).abs() > half.x + MARGIN
                        || (z - center.z).abs() > half.z + MARGIN
                }
            })
        })
    }

    /// The pad a player with feet at `feet` (units) stands on, if any.
    #[must_use]
    pub fn pad_under(&self, feet: Vec3i) -> Option<&JumpPad> {
        self.pads.iter().find(|pad| pad.under(feet))
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
