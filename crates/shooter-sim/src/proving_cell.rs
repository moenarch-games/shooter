//! The proving cell (`proving-cell`, shooter#53): the beta's small close-quarters arena, a
//! purpose-built combat test facility (owner decisions on shooter#53), built from asset-tooling's
//! arena kit ([`crate::arena_kit`]) like the assembly works.
//!
//! A 32 m square cell inside 10 m walls, point-symmetric (every placement in the north half has a
//! twin turned half a turn round the centre in the south), on three levels:
//!
//! - **Ground:** the test floor round the stage, with a 2 m barrier, a 2 m container (its top
//!   meets the stage's corner) and crates; a control booth in the north-east (and the south-west)
//!   corner, a 7 m room with a doorway on each inner side and a blast screen behind each; a test
//!   bay, a 2 m wide and 4 m deep alcove, in the east (and the west) wall; and under each gallery
//!   a 2.6 m high passage from behind the gallery's ramp to the booth's west doorway.
//! - **2 m:** the test stage in the middle, a 6 m dais with the rail capacitor at its centre and
//!   staggered low cover on it: an 8 m ramp climbs it from the north and from the south, and a
//!   double jump reaches it (or the container) from the floor.
//! - **3 m:** an observation gallery along the north (and the south) wall, 4 m deep, with low
//!   cover at its edge and thrusters on it: a 12 m ramp climbs it along the side wall, a jump pad
//!   in front of it throws a fighter onto it, and the grapple reaches it by the wall above it.
//!
//! The cell is small and its lines are short: the stage, the container, the booths and the ramps
//! cut the sightlines over the floor, and nothing is more than a few seconds' sprint away. The
//! booths have two doorways, the passages under the galleries open along their whole length and
//! the stage has two ramps, so no fight is forced down a one-way corridor; the high ground is
//! reached by a ramp, a jump pad and the grapple each.
//!
//! Twelve spawn points (one per default slot), none in sight of another: in the booths behind
//! their screens, at the passages' closed ends, in the test bays, under the galleries, between a
//! gallery and a booth and in the container's lee. Bot waypoints at the ramps, the stage, the
//! galleries, the passages, the bays and the booths. Repair anchors (shooter#61) lie in the
//! booths; the floor and gallery low cover, the open-floor barrier and the crates are destruction
//! candidates (shooter#60). The blast screens stay intact: the booths' spawn points rely on them.

use crate::arena::{Arena, JumpPad, Spawn};
use crate::arena_kit::{self, Module, Placement, Turn};
use crate::physics::Vec3i;
use crate::pickups::{ArenaModule, ModulePad};

/// The floor spans ±16 m.
pub const HALF_SIZE: i32 = 1_600;
/// The perimeter's 60 cm walls stand just outside the floor.
const WALL_LINE: i32 = HALF_SIZE + 30;
/// The stage's top (2 m).
pub const STAGE_TOP: i32 = 200;
/// The galleries' top (3 m): a walkway section sits on its supports at 2.6 m.
pub const DECK_TOP: i32 = 300;
const DECK_BASE: i32 = 260;
/// The perimeter walls' height (10 m).
pub const PERIMETER_HEIGHT: i32 = 1_000;
/// The booth's corner post (its walls' inner line), on both axes.
const BOOTH: i32 = 870;

const fn place(module: Module, x: i32, y: i32, z: i32, turn: Turn) -> Placement {
    Placement::new(module, [x, y, z], turn)
}

const fn spawn_at(feet: [i32; 3], yaw: u16) -> Spawn {
    Spawn {
        feet: Vec3i::new(feet[0], feet[1], feet[2]),
        yaw,
    }
}

/// The point turned half a turn round the centre.
const fn turned(point: Vec3i) -> Vec3i {
    Vec3i::new(-point.x, point.y, -point.z)
}

/// The perimeter: two courses of tall wall modules (10 m) on every side and a post at each
/// corner.
fn perimeter() -> Vec<Placement> {
    let mut out = Vec::new();
    for y in [0, PERIMETER_HEIGHT / 2] {
        for i in 0..(HALF_SIZE / 100) {
            let along = -HALF_SIZE + 100 + 200 * i;
            out.push(place(Module::WallTall, along, y, -WALL_LINE, Turn::None));
            out.push(place(Module::WallTall, along, y, WALL_LINE, Turn::None));
            out.push(place(Module::WallTall, -WALL_LINE, y, along, Turn::Quarter));
            out.push(place(Module::WallTall, WALL_LINE, y, along, Turn::Quarter));
        }
        for (x, z) in [(-1, -1), (1, -1), (-1, 1), (1, 1)] {
            out.push(place(
                Module::WallCornerTall,
                x * WALL_LINE,
                y,
                z * WALL_LINE,
                Turn::None,
            ));
        }
    }
    out
}

/// The test stage: a 6 m square of 2 m platform blocks round the centre (its own twin).
fn stage() -> Vec<Placement> {
    let mut out = Vec::new();
    for x in [-200, 0, 200] {
        for z in [-200, 0, 200] {
            out.push(place(Module::Platform, x, 0, z, Turn::None));
        }
    }
    out
}

/// The north half's placements (the south half is the same turned half a turn).
fn north() -> Vec<Placement> {
    let mut out = Vec::new();
    // The north gallery: a 20 m deck, 4 m deep, along the north wall from the west wall to x 4 m.
    for i in 0..10 {
        out.push(place(
            Module::Walkway,
            -1_500 + 200 * i,
            DECK_BASE,
            -1_400,
            Turn::None,
        ));
    }
    for x in [-900, 100] {
        out.push(place(Module::Support, x, 0, -1_400, Turn::None));
    }
    // Its 12 m ramp from the floor, 4 m wide, along the west wall, rising north onto the deck.
    for x in [-1_500, -1_300] {
        out.push(place(Module::RampTall, x, 0, -600, Turn::None));
    }
    // Low cover at the gallery's edge.
    out.push(place(Module::CoverLow, -500, DECK_TOP, -1_230, Turn::None).destructible());
    out.push(place(Module::CoverLow, 200, DECK_TOP, -1_230, Turn::None).destructible());

    // The stage's north ramp, 4 m wide, rising south onto the stage from under the gallery's
    // edge, and low cover on the stage's east edge.
    for x in [-100, 100] {
        out.push(place(Module::Ramp, x, 0, -700, Turn::Half));
    }
    out.push(place(Module::CoverLow, 250, STAGE_TOP, -150, Turn::Quarter).destructible());

    // The north-east control booth: 4 m walls from a corner post, a doorway south onto the floor
    // and one west into the passage under the gallery.
    out.push(place(Module::WallCorner, BOOTH, 0, -BOOTH, Turn::None));
    out.push(place(Module::Wall, 1_000, 0, -BOOTH, Turn::None));
    out.push(place(Module::Doorway, 1_300, 0, -BOOTH, Turn::None));
    out.push(place(Module::WallHalf, 1_550, 0, -BOOTH, Turn::None));
    out.push(place(Module::WallHalf, BOOTH, 0, -950, Turn::Quarter));
    out.push(place(Module::Wall, BOOTH, 0, -1_100, Turn::Quarter));
    out.push(place(Module::Doorway, BOOTH, 0, -1_400, Turn::Quarter));
    // Inside: a blast screen 2 m behind each doorway (no robot sees over it), and the test rig's
    // generator in the corner.
    out.push(place(Module::Barrier, 1_300, 0, -1_100, Turn::None));
    out.push(place(Module::Barrier, 1_100, 0, -1_400, Turn::Quarter));
    out.push(place(Module::Generator, 1_450, 0, -1_500, Turn::None));

    // The test floor's north half: a barrier east of the stage, a test bay in the east wall (a
    // 2 m wide, 4 m deep alcove), a container west of the stage (its top meets the stage's
    // corner) and crates by the stage's ramp.
    out.push(place(Module::Barrier, 600, 0, -500, Turn::Quarter).destructible());
    for z in [-270, -530] {
        for x in [1_300, 1_500] {
            out.push(place(Module::Wall, x, 0, z, Turn::None));
        }
    }
    out.push(place(Module::Container, -700, 0, -450, Turn::None));
    out.push(place(Module::Crate, -450, 0, -1_000, Turn::None).destructible());
    out.push(place(Module::Crate, -450, 80, -1_000, Turn::None).destructible());
    // The passage's west end, under the gallery behind its ramp: a barrier closes it to a 2 m
    // way in along the wall; pipes on the wall.
    out.push(place(Module::Barrier, -1_150, 0, -1_300, Turn::Quarter));
    out.push(place(Module::Pipes, -1_500, 0, -1_575, Turn::None));
    out
}

impl Arena {
    /// The proving cell ([`crate::proving_cell`]): the small arena, built from the arena kit.
    #[must_use]
    pub fn proving_cell() -> Self {
        let north = north();
        let placements: Vec<Placement> = perimeter()
            .into_iter()
            .chain(stage())
            .chain(north.iter().copied())
            .chain(north.iter().map(|placement| placement.turned_half()))
            .collect();
        let pieces = arena_kit::pieces(&placements);
        // In front of the gallery, 12 m/s up and 6 m/s north: over its edge onto its deck.
        let north_pads = [JumpPad {
            center: Vec3i::new(-700, 0, -850),
            half: [80, 80],
            launch: Vec3i::new(0, 1_200, -600),
        }];
        let pads = north_pads
            .iter()
            .copied()
            .chain(north_pads.iter().map(|pad| JumpPad {
                center: turned(pad.center),
                launch: Vec3i::new(-pad.launch.x, pad.launch.y, -pad.launch.z),
                ..*pad
            }))
            .collect::<Vec<_>>();
        let placements = placements
            .into_iter()
            .chain(pads.iter().map(|pad| {
                place(
                    Module::JumpPad,
                    pad.center.x,
                    pad.center.y,
                    pad.center.z,
                    Turn::None,
                )
            }))
            .collect();
        let north_spawns = [
            // In the booth, behind both blast screens.
            spawn_at([1_400, 0, -1_300], 0),
            // At the passage's closed west end, behind the gallery's ramp.
            spawn_at([-1_450, 0, -1_450], 16_384),
            // In the test bay.
            spawn_at([1_450, 0, -400], 49_152),
            // Under the gallery, west of the stage's ramp.
            spawn_at([-150, 0, -1_450], 0),
            // Between the gallery's end and the booth.
            spawn_at([650, 0, -1_050], 0),
            // In the lee of the container, by the stage.
            spawn_at([-450, 0, -150], 49_152),
        ];
        let spawns = north_spawns[1..]
            .iter()
            .copied()
            .chain(north_spawns.iter().map(|spawn| Spawn {
                feet: turned(spawn.feet),
                yaw: spawn.yaw.wrapping_add(32_768),
            }))
            .collect();
        let north_waypoints = [
            // The gallery ramp's foot and top, and the deck.
            Vec3i::new(-1_400, 0, 60),
            Vec3i::new(-1_400, DECK_TOP, -1_400),
            Vec3i::new(-600, DECK_TOP, -1_400),
            Vec3i::new(200, DECK_TOP, -1_400),
            // The stage's north ramp foot and the stage.
            Vec3i::new(0, 0, -1_200),
            Vec3i::new(0, STAGE_TOP, -150),
            // The passage under the gallery.
            Vec3i::new(-900, 0, -1_500),
            Vec3i::new(-450, 0, -1_550),
            Vec3i::new(500, 0, -1_400),
            // The booth: inside its west doorway, its inner corner, its south-east corner,
            // inside and outside its south doorway.
            Vec3i::new(1_000, 0, -1_400),
            Vec3i::new(1_000, 0, -1_200),
            Vec3i::new(1_550, 0, -1_000),
            Vec3i::new(1_300, 0, -985),
            Vec3i::new(1_300, 0, -650),
            // The floor.
            Vec3i::new(-1_000, 0, -1_000),
            Vec3i::new(-300, 0, -800),
            Vec3i::new(300, 0, -700),
            Vec3i::new(900, 0, -150),
            // Out of the test bay.
            Vec3i::new(1_000, 0, -400),
            Vec3i::new(600, 0, -300),
        ];
        let waypoints = north_waypoints
            .iter()
            .copied()
            .chain(north_waypoints.iter().copied().map(turned))
            .collect();
        let module_pads = vec![
            ModulePad {
                center: Vec3i::new(0, STAGE_TOP, 0),
                module: ArenaModule::RailCapacitor,
            },
            ModulePad {
                center: Vec3i::new(-1_000, DECK_TOP, -1_400),
                module: ArenaModule::Thrusters,
            },
            ModulePad {
                center: Vec3i::new(1_000, DECK_TOP, 1_400),
                module: ArenaModule::Thrusters,
            },
        ];
        let north_repairs = [Vec3i::new(1_250, 0, -1_150)];
        let repair_anchors = north_repairs
            .iter()
            .copied()
            .chain(north_repairs.iter().copied().map(turned))
            .collect();
        Self {
            half_size: HALF_SIZE,
            pieces,
            pads,
            posts: Vec::new(),
            spawn: north_spawns[0].feet,
            spawn_yaw: north_spawns[0].yaw,
            spawns,
            module_pads,
            waypoints,
            placements,
            repair_anchors,
        }
    }
}
