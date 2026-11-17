//! The assembly works (`assembly-works`, shooter#52): the beta's medium arena, a multilevel
//! robotic assembly factory repurposed for combat (owner decisions on shooter#52), built from
//! asset-tooling's arena kit ([`crate::arena_kit`]).
//!
//! A 56 m square hall inside 10 m walls, point-symmetric (every placement in the north half has a
//! twin turned half a turn round the centre in the south), on three levels:
//!
//! - **Ground:** the central assembly floor, broken up by assembly cells (2 m barriers, 1.2 m
//!   cover, a parts container, 4 m pillars and generators), and behind each mezzanine a work bay
//!   reached through a doorway or round the mezzanine's open end.
//! - **3 m:** a mezzanine along each bay's 4 m wall (the wall stands 1 m above its deck as a
//!   parapet), and the gantry bridge between them across the middle, with staggered low cover
//!   on it and the rail capacitor at its centre: the contested high ground. A 12 m ramp climbs
//!   each mezzanine; a jump pad beside the bridge throws onto it; the grapple reaches it from
//!   anywhere under it.
//! - **5 m:** a crane tower in the corner of each bay with thrusters on top, reached by a
//!   stack of 1 m work platforms (a jump each), by a jump pad at its foot or by the grapple.
//!
//! No long open lane runs the length of the hall: the mezzanines' parapets, the tower, the bay
//! walls and the floor's 4 m pillars cut every sightline under about 35 m, and each high place
//! is reached by at least two routes and seen from the other levels.
//!
//! Fourteen spawn points, none in sight of another; bot waypoints at the ramps, the doorways,
//! the decks and the bays. Repair anchors (shooter#61) lie under the mezzanines and in the
//! bays; the low cover, the barriers and the crates are destruction candidates (shooter#60).

use crate::arena::{Arena, JumpPad, Spawn};
use crate::arena_kit::{self, Module, Placement, Turn};
use crate::physics::Vec3i;
use crate::pickups::{ArenaModule, ModulePad};

/// The floor spans ±28 m.
pub const HALF_SIZE: i32 = 2_800;
/// The perimeter's 60 cm walls stand just outside the floor.
const WALL_LINE: i32 = HALF_SIZE + 30;
/// The decks' top (3 m): a walkway section sits on its supports at 2.6 m.
pub const DECK_TOP: i32 = 300;
const DECK_BASE: i32 = 260;
/// The crane towers' top (5 m).
pub const TOWER_TOP: i32 = 500;
/// The perimeter walls' height (10 m).
pub const PERIMETER_HEIGHT: i32 = 1_000;

const fn place(module: Module, x: i32, y: i32, z: i32, turn: Turn) -> Placement {
    Placement::new(module, [x, y, z], turn)
}

const fn spawn_at(feet: [i32; 3], yaw: u16) -> Spawn {
    Spawn {
        feet: Vec3i::new(feet[0], feet[1], feet[2]),
        yaw,
    }
}

/// The perimeter: two courses of tall wall modules (10 m, out of reach of a jump from the crane
/// towers) on every side and a post at each corner.
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

/// A stack of platform blocks whose top is `top` (whole metres), its base's centre at `x`, `z`.
fn stack(out: &mut Vec<Placement>, x: i32, z: i32, top: i32) {
    let mut y = 0;
    while y + 200 <= top {
        out.push(place(Module::Platform, x, y, z, Turn::None));
        y += 200;
    }
    if y < top {
        out.push(place(Module::PlatformHalf, x, y, z, Turn::None));
    }
}

/// The north half's placements (the south half is the same turned half a turn).
#[allow(clippy::too_many_lines)]
fn north() -> Vec<Placement> {
    let mut out = Vec::new();
    // The north mezzanine: a 24 m deck, 4 m deep, along the bay wall from x -18 to 6 m.
    for i in 0..12 {
        out.push(place(
            Module::Walkway,
            -1_700 + 200 * i,
            DECK_BASE,
            -1_600,
            Turn::None,
        ));
    }
    for x in [-1_500, -700, 100] {
        out.push(place(Module::Support, x, 0, -1_600, Turn::None));
    }
    // Its 12 m ramp from the floor, 4 m wide, rising north onto the deck's west end.
    for x in [-1_700, -1_500] {
        out.push(place(Module::RampTall, x, 0, -800, Turn::None));
    }
    // The gantry bridge's north half: 14 m of deck south from the mezzanine to the middle.
    for i in 0..7 {
        out.push(place(
            Module::Walkway,
            0,
            DECK_BASE,
            -1_300 + 200 * i,
            Turn::Quarter,
        ));
    }
    out.push(place(Module::Support, 0, 0, -800, Turn::None));
    // Low cover on the bridge, across its west half.
    out.push(place(Module::CoverLow, -100, DECK_TOP, -500, Turn::None).destructible());

    // The bay wall behind the mezzanine: 4 m, standing 1 m over the deck, from the west wall to
    // x 6 m, with a doorway into the bay at x -8 m and a post at its open east end.
    for i in 0..9 {
        out.push(place(Module::Wall, -2_700 + 200 * i, 0, -1_830, Turn::None));
    }
    out.push(place(Module::Doorway, -800, 0, -1_830, Turn::None));
    for i in 0..6 {
        out.push(place(Module::Wall, -500 + 200 * i, 0, -1_830, Turn::None));
    }
    out.push(place(Module::WallCorner, 630, 0, -1_830, Turn::None));

    // The crane tower (5 m, 4 x 4 m) in the bay's east corner, and the work platforms climbing
    // to it from the west in 1 m steps.
    for (x, z) in [
        (2_100, -2_300),
        (2_300, -2_300),
        (2_100, -2_100),
        (2_300, -2_100),
    ] {
        stack(&mut out, x, z, TOWER_TOP);
    }
    // The crane's 3 m mast in the tower's back corner: its grapple ring pulls a fighter up and
    // over the tower's edge.
    out.push(place(
        Module::HullMast,
        2_330,
        TOWER_TOP,
        -2_330,
        Turn::None,
    ));
    for (step, x) in [1_300, 1_500, 1_700, 1_900].into_iter().enumerate() {
        let top = 100 * (i32::try_from(step).unwrap_or(0) + 1);
        for z in [-2_300, -2_100] {
            stack(&mut out, x, z, top);
        }
    }

    // The bay: a parts container, crates and generators along its wall, pipes on the perimeter.
    out.push(place(Module::Container, -1_300, 0, -2_650, Turn::None));
    out.push(place(Module::Container, 900, 0, -2_650, Turn::None));
    out.push(place(Module::Crate, 1_000, 0, -1_950, Turn::None).destructible());
    out.push(place(Module::Crate, 1_080, 0, -1_950, Turn::None).destructible());
    out.push(place(Module::Crate, 1_040, 80, -1_950, Turn::None).destructible());
    out.push(place(Module::Generator, -2_650, 0, -2_400, Turn::Quarter));
    out.push(place(Module::Barrier, -2_200, 0, -2_100, Turn::Quarter).destructible());
    out.push(place(Module::Barrier, -300, 0, -2_300, Turn::Quarter).destructible());
    for x in [-1_900, -1_700, 700, 900] {
        out.push(place(Module::Pipes, x, 0, -2_775, Turn::None));
    }

    // The assembly floor's north half: a cell of barriers and low cover west of the bridge, a
    // pillar pair, a generator, crates and the west corridor's container.
    out.push(place(Module::Barrier, -1_000, 0, -600, Turn::Quarter).destructible());
    out.push(place(Module::CoverLow, -1_100, 0, -1_100, Turn::None).destructible());
    out.push(place(Module::CoverLow, 1_000, 0, -500, Turn::None).destructible());
    out.push(place(Module::Barrier, 1_400, 0, -1_200, Turn::Quarter).destructible());
    out.push(place(Module::Pillar, -600, 0, -1_100, Turn::None));
    out.push(place(Module::Pillar, 600, 0, -300, Turn::None));
    out.push(place(Module::Generator, 2_300, 0, -400, Turn::Quarter));
    out.push(place(Module::Container, -2_300, 0, -300, Turn::Quarter));
    // Charging bays: a 4 m deep, 2 m wide alcove in the east and the west wall.
    for side in [-1, 1] {
        for z in [-1_230, -970] {
            for x in [2_500, 2_700] {
                out.push(place(Module::Wall, side * x, 0, z, Turn::None));
            }
        }
    }
    // A blast screen inside the bay's doorway.
    out.push(place(Module::Barrier, -800, 0, -2_200, Turn::None).destructible());
    out.push(place(Module::Crate, 1_900, 0, -1_200, Turn::None).destructible());
    out
}

impl Arena {
    /// The assembly works ([`crate::assembly_works`]): the medium arena, built from the arena
    /// kit.
    #[must_use]
    pub fn assembly_works() -> Self {
        let north = north();
        // Under the bridge's middle, a 2 m assembly cradle.
        let middle = [place(Module::Platform, 0, 0, 0, Turn::None)];
        let placements: Vec<Placement> = perimeter()
            .into_iter()
            .chain(middle)
            .chain(north.iter().copied())
            .chain(north.iter().map(|placement| placement.turned_half()))
            .collect();
        let pieces = arena_kit::pieces(&placements);
        // Beside the bridge, 12 m/s up and 6 m/s toward it: over its edge onto its middle. At the
        // crane tower's foot, 15 m/s up (a 6.25 m apex) and 6 m/s toward it: onto its top.
        let north_pads = [
            JumpPad {
                center: Vec3i::new(-600, 0, -300),
                half: [80, 80],
                launch: Vec3i::new(600, 1_200, 0),
            },
            JumpPad {
                center: Vec3i::new(2_200, 0, -1_500),
                half: [80, 80],
                launch: Vec3i::new(0, 1_500, -600),
            },
        ];
        let pads = north_pads
            .iter()
            .copied()
            .chain(north_pads.iter().map(|pad| JumpPad {
                center: Vec3i::new(-pad.center.x, pad.center.y, -pad.center.z),
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
            spawn_at([-2_000, 0, -2_650], 16_384),
            spawn_at([-100, 0, -2_650], 49_152),
            spawn_at([2_600, 0, -2_650], 49_152),
            spawn_at([-2_600, 0, -1_650], 16_384),
            spawn_at([2_650, 0, -1_100], 49_152),
            spawn_at([-2_650, 0, -1_100], 16_384),
            spawn_at([-1_200, 0, -1_600], 0),
        ];
        let turned = |spawn: &Spawn| Spawn {
            feet: Vec3i::new(-spawn.feet.x, spawn.feet.y, -spawn.feet.z),
            yaw: spawn.yaw.wrapping_add(32_768),
        };
        let spawns = north_spawns[1..]
            .iter()
            .copied()
            .chain(north_spawns.iter().map(turned))
            .collect();
        let north_waypoints = [
            // The mezzanine ramp's foot and top, the deck and the bridge.
            Vec3i::new(-1_700, 0, -50),
            Vec3i::new(-1_700, DECK_TOP, -1_600),
            Vec3i::new(-600, DECK_TOP, -1_600),
            Vec3i::new(0, DECK_TOP, -1_000),
            Vec3i::new(0, DECK_TOP, -200),
            // Through the doorway into the bay and round the wall's open end.
            Vec3i::new(-800, 0, -1_500),
            Vec3i::new(-800, 0, -2_000),
            Vec3i::new(-100, 0, -2_100),
            Vec3i::new(900, 0, -1_600),
            Vec3i::new(900, 0, -2_050),
            // The bay and the floor.
            Vec3i::new(-2_000, 0, -2_550),
            Vec3i::new(1_000, 0, -2_350),
            Vec3i::new(-1_400, 0, -300),
            Vec3i::new(300, 0, -800),
            Vec3i::new(1_600, 0, -600),
            Vec3i::new(2_500, 0, -300),
            Vec3i::new(-2_550, 0, -2_000),
            Vec3i::new(-2_200, 0, -1_100),
            Vec3i::new(-1_950, 0, -1_100),
            Vec3i::new(2_600, 0, -1_900),
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
                center: Vec3i::new(0, DECK_TOP, 0),
                module: ArenaModule::RailCapacitor,
            },
            ModulePad {
                center: Vec3i::new(2_200, TOWER_TOP, -2_200),
                module: ArenaModule::Thrusters,
            },
            ModulePad {
                center: Vec3i::new(-2_200, TOWER_TOP, 2_200),
                module: ArenaModule::Thrusters,
            },
        ];
        let north_repairs = [Vec3i::new(-1_200, 0, -1_600), Vec3i::new(-1_800, 0, -2_500)];
        let repair_anchors = north_repairs
            .iter()
            .copied()
            .chain(
                north_repairs
                    .iter()
                    .map(|point| Vec3i::new(-point.x, point.y, -point.z)),
            )
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
