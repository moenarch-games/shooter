//! An arena: one floor, perimeter walls and blocks and ramps, as fixed physics bodies. The
//! greybox maps list their pieces directly and the clients draw them as boxes; a kit-built map
//! (the assembly works, [`crate::assembly_works`]) lists arena-kit module placements
//! ([`crate::arena_kit`]) its pieces are derived from, and the clients draw the modules. Either
//! way what a player sees is what they collide with.
//!
//! Lengths are centimetre units, as in physics-engine (`length_unit` 100). Y is up; the floor's top is at 0.

use crate::arena_kit::Placement;
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
    /// More points bots walk between (feet, units), after the spawn points
    /// ([`Arena::bot_points`]): corners, ramp ends and the walkway, where spawn points alone
    /// would leave a bot no straight walk on.
    pub waypoints: Vec<Vec3i>,
    /// The arena kit's modules this arena is built from ([`crate::arena_kit`]), which the clients
    /// draw; its [`Arena::pieces`] are derived from them. Empty on the greybox maps, whose pieces
    /// are drawn as plain boxes.
    pub placements: Vec<Placement>,
    /// Where field repair stations stand (feet, units): anchors for shooter#61, which owns what
    /// they do. Nothing uses them yet.
    pub repair_anchors: Vec<Vec3i>,
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

/// `piece` turned half a turn round the arena's centre.
const fn turned_half(piece: &Piece) -> Piece {
    match *piece {
        Piece::Block { center, half } => Piece::Block {
            center: Vec3i::new(-center.x, center.y, -center.z),
            half,
        },
        Piece::Ramp {
            center,
            half,
            rises,
        } => Piece::Ramp {
            center: Vec3i::new(-center.x, center.y, -center.z),
            half,
            rises: match rises {
                RampRise::PosX => RampRise::NegX,
                RampRise::NegX => RampRise::PosX,
                RampRise::PosZ => RampRise::NegZ,
                RampRise::NegZ => RampRise::PosZ,
            },
        },
    }
}

/// `spawn` turned half a turn round the arena's centre.
const fn turned_spawn(spawn: &Spawn) -> Spawn {
    Spawn {
        feet: Vec3i::new(-spawn.feet.x, spawn.feet.y, -spawn.feet.z),
        yaw: spawn.yaw.wrapping_add(32_768),
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
            waypoints: Vec::new(),
            placements: Vec::new(),
            repair_anchors: Vec::new(),
        }
    }

    /// The greybox yard, an interim playtest map (shooter#82): a 76 m square with 5 m walls,
    /// laid out point-symmetrically (every piece north of the middle has a twin turned half a
    /// turn round the centre in the south). Across the middle runs a raised walkway, a 3 m deck on
    /// two pillars that fighters walk under, reached by a 14° ramp at either end and thrown onto
    /// by two jump pads beside it. 5 m wall lines either side of it split the yard into a central
    /// band and two outer yards; in each outer yard a cross wall, wing and bunker walls, a 2 m
    /// platform with its own ramp, a 3 m shed, a 7 m tower only a grapple (or a rocket jump)
    /// reaches, a 2 m container, cover walls and crates break the sightlines at every height.
    /// Fourteen spawn points, none in sight of another, sheltered round the edges and in the
    /// wings; bot waypoints at the wall gaps, ramp ends, walkway and platforms; the rail
    /// capacitor lies on the walkway's middle and thrusters on each platform.
    #[must_use]
    pub fn greybox_yard() -> Self {
        let half_size = 3_800;
        let wall = 50;
        let outer = half_size + wall;
        let mut pieces = vec![
            // Perimeter walls, 5 m high.
            block([0, 250, outer], [outer + wall, 250, wall]),
            block([0, 250, -outer], [outer + wall, 250, wall]),
            block([outer, 250, 0], [wall, 250, outer - wall]),
            block([-outer, 250, 0], [wall, 250, outer - wall]),
            // The walkway: a 40 cm deck, top at 3 m, 32 m long and 4 m wide, on two pillars;
            // 2.6 m clear underneath.
            block([0, 280, 0], [1_600, 20, 200]),
            block([-800, 130, 0], [50, 130, 50]),
            block([800, 130, 0], [50, 130, 50]),
            // Its ramps, 12 m long (14°), from the west and the east.
            ramp([-2_200, 150, 0], [600, 150, 200], RampRise::PosX),
            ramp([2_200, 150, 0], [600, 150, 200], RampRise::NegX),
        ];
        // The north half; the south half is the same turned half a turn.
        let north = [
            // A 5 m wall line between the central band and the outer yard, open at its middle
            // and both ends.
            block([-1_600, 250, -1_200], [800, 250, 30]),
            block([1_000, 250, -1_200], [600, 250, 30]),
            // The outer yard's 4 m cross wall, clear of the perimeter and the wall line.
            block([-600, 200, -2_800], [30, 200, 800]),
            // Wing walls, 4 m, each with room to pass at both ends.
            block([-3_100, 200, -1_500], [400, 200, 30]),
            block([3_250, 200, -1_700], [350, 200, 30]),
            // A 4 m bunker wall in the east wing.
            block([2_900, 200, -800], [30, 200, 400]),
            // A 2 m platform and its ramp from the west (14°).
            block([2_600, 100, -2_700], [600, 100, 500]),
            ramp([1_600, 100, -2_700], [400, 100, 200], RampRise::PosX),
            // A 3 m shed.
            block([1_000, 150, -3_400], [200, 150, 200]),
            // A 7 m tower: a grapple perch over the whole yard.
            block([-2_600, 350, -2_600], [150, 350, 150]),
            // A 2 m container: a thruster jump climbs it.
            block([-1_700, 100, -3_100], [400, 100, 150]),
            // Low cover walls, 1.2 m.
            block([800, 60, -2_200], [250, 60, 30]),
            // A 2 m wall: cover a standing robot cannot see over.
            block([-2_000, 100, -1_800], [30, 100, 300]),
            // A 2 m barrier inside the middle gap.
            block([-200, 100, -800], [350, 100, 30]),
            // Crates, 80 cm, and a 1.6 m stack.
            block([0, 40, -1_800], [40, 40, 40]),
            block([90, 40, -1_800], [40, 40, 40]),
            block([1_800, 40, -700], [40, 40, 40]),
            block([1_400, 40, -2_000], [40, 40, 40]),
            block([1_400, 120, -2_000], [40, 40, 40]),
        ];
        pieces.extend(north);
        pieces.extend(north.iter().map(turned_half));
        // Two pads beside the walkway, each throwing 12 m/s up (a 4 m apex) and 6 m/s toward it:
        // over its edge and down on its deck.
        let pads = vec![
            JumpPad {
                center: Vec3i::new(-1_200, 0, -500),
                half: [80, 80],
                launch: Vec3i::new(0, 1_200, 600),
            },
            JumpPad {
                center: Vec3i::new(1_200, 0, 500),
                half: [80, 80],
                launch: Vec3i::new(0, 1_200, -600),
            },
        ];
        // Each spawn point sheltered by walls, the platforms, the container or the tower from
        // every other one (and its twin in the south).
        let north_spawns = [
            spawn_at([-3_400, 0, -1_750], 16_384),
            spawn_at([3_500, 0, -2_000], 49_152),
            spawn_at([3_400, 0, -800], 49_152),
            spawn_at([-1_800, 0, -3_550], 16_384),
            spawn_at([-300, 0, -3_400], 16_384),
            spawn_at([-1_100, 0, -2_200], 0),
            spawn_at([1_500, 0, -3_650], 49_152),
        ];
        let spawns = north_spawns[1..]
            .iter()
            .copied()
            .chain(north_spawns.iter().map(turned_spawn))
            .collect();
        // Where bots walk besides the spawn points: up the ramps onto the walkway and the
        // platforms, and through the gaps in the walls.
        let north_waypoints = [
            Vec3i::new(-1_450, 300, 0),
            Vec3i::new(-400, 300, 0),
            Vec3i::new(1_000, 0, -2_700),
            Vec3i::new(2_100, 200, -2_700),
            Vec3i::new(2_600, 200, -2_700),
            Vec3i::new(-200, 0, -1_500),
            Vec3i::new(-200, 0, -900),
            Vec3i::new(-600, 0, -1_650),
            Vec3i::new(-600, 0, -3_700),
            Vec3i::new(2_300, 0, -1_400),
            Vec3i::new(-2_550, 0, -1_350),
            Vec3i::new(3_650, 0, -1_700),
            Vec3i::new(-3_650, 0, -1_500),
            Vec3i::new(-3_200, 0, -3_400),
            Vec3i::new(-2_300, 0, -2_100),
        ];
        let waypoints = north_waypoints
            .iter()
            .copied()
            .chain(
                north_waypoints
                    .iter()
                    .map(|point| Vec3i::new(-point.x, point.y, -point.z)),
            )
            .collect();
        let module_pads = vec![
            ModulePad {
                center: Vec3i::new(0, 300, 0),
                module: ArenaModule::RailCapacitor,
            },
            ModulePad {
                center: Vec3i::new(2_600, 200, -2_700),
                module: ArenaModule::Thrusters,
            },
            ModulePad {
                center: Vec3i::new(-2_600, 200, 2_700),
                module: ArenaModule::Thrusters,
            },
        ];
        Self {
            half_size,
            pieces,
            pads,
            posts: Vec::new(),
            spawn: north_spawns[0].feet,
            spawn_yaw: north_spawns[0].yaw,
            spawns,
            module_pads,
            waypoints,
            placements: Vec::new(),
            repair_anchors: Vec::new(),
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

    /// Where bots walk: every spawn point's feet, then [`Arena::waypoints`].
    #[must_use]
    pub fn bot_points(&self) -> Vec<Vec3i> {
        self.spawn_points()
            .iter()
            .map(|spawn| spawn.feet)
            .chain(self.waypoints.iter().copied())
            .collect()
    }

    /// Whether a fighter walking straight from `from` to `to` (feet, units) on the floor meets
    /// no piece taller than a step (ramps excepted, and pieces it fits under): bots pick
    /// waypoints they can walk to.
    #[must_use]
    pub fn clear_walk(&self, from: Vec3i, to: Vec3i) -> bool {
        const MARGIN: i32 = 40;
        // At least every 50 cm, so no wall slips between two samples.
        let samples = ((to.x - from.x).abs().max((to.z - from.z).abs()) / 50).max(16);
        let step_up = crate::player::STEP_UP.max_height;
        let height = 2 * crate::player::PLAYER_HALF_EXTENTS.y;
        // Only pieces near the walk's footprint can stop it (a kit-built map has hundreds).
        let near: Vec<&Piece> = self
            .pieces
            .iter()
            .filter(|piece| {
                let (center, half) = (piece.center(), piece.half());
                center.x + half.x + MARGIN >= from.x.min(to.x)
                    && center.x - half.x - MARGIN <= from.x.max(to.x)
                    && center.z + half.z + MARGIN >= from.z.min(to.z)
                    && center.z - half.z - MARGIN <= from.z.max(to.z)
            })
            .collect();
        (0..=samples).all(|step| {
            let x = from.x + (to.x - from.x) * step / samples;
            let z = from.z + (to.z - from.z) * step / samples;
            let feet = from.y + (to.y - from.y) * step / samples;
            near.iter().all(|piece| {
                let (center, half) = (piece.center(), piece.half());
                if (x - center.x).abs() > half.x + MARGIN || (z - center.z).abs() > half.z + MARGIN
                {
                    return true;
                }
                let bottom = center.y - half.y;
                let top = match **piece {
                    Piece::Block { .. } => center.y + half.y,
                    // A ramp's top where the fighter is: walked up or down along its rise, it
                    // stays within a step of the feet; crossed sideways, it is a wall.
                    Piece::Ramp { rises, .. } => {
                        let (along, run) = match rises {
                            RampRise::PosX => (x - center.x + half.x, 2 * half.x),
                            RampRise::NegX => (center.x + half.x - x, 2 * half.x),
                            RampRise::PosZ => (z - center.z + half.z, 2 * half.z),
                            RampRise::NegZ => (center.z + half.z - z, 2 * half.z),
                        };
                        bottom + 2 * half.y * along.clamp(0, run) / run.max(1)
                    }
                };
                top <= feet + step_up || bottom >= feet + height
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
        for body in self.statics() {
            world.add_body(body)?;
        }
        Ok(world)
    }

    /// The floor and the pieces as static bodies, in body-id order.
    #[must_use]
    pub fn statics(&self) -> Vec<RigidBody> {
        std::iter::once(RigidBody::floor(FLOOR_BODY, 0))
            .chain(self.pieces.iter().enumerate().map(|(index, piece)| {
                let id = BodyId(FLOOR_BODY.0 + 1 + index as u64);
                match *piece {
                    Piece::Block { center, half } => RigidBody::fixed(id, center, half),
                    Piece::Ramp {
                        center,
                        half,
                        rises,
                    } => RigidBody::ramp(id, center, half, rises),
                }
            }))
            .collect()
    }
}
