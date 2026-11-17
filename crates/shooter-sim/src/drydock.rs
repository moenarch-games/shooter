//! The drydock (`drydock`, shooter#55): the beta's large long-sightline arena, a repurposed
//! spaceship shipyard where two docked spacecraft flank a central industrial corridor (owner
//! decision D2.i55.1), built from asset-tooling's arena kit ([`crate::arena_kit`]) like the
//! assembly works: the ships' exterior hulls are the kit's hull modules, the dock machinery its
//! walkways, ramps, platforms, containers and cover. No ship has an interior.
//!
//! An 80 m square dock inside 10 m walls, point-symmetric (every placement west of the centre
//! line has a twin turned half a turn round the centre in the east): the two ships lie head to
//! tail, so each one's raised stern faces the other's foredeck across the corridor.
//!
//! - **The ships:** a 12 m wide, 56 m long hull in a berth either side of the corridor (its
//!   centre line 22 m off the dock's): 2 m hull sides and a 45° shoulder up to a 4 m foredeck,
//!   8 m wide, with hull ribs (80 cm low cover) staggered across it and a sensor mast at the
//!   bow; and a raised 6 m stern deck, 16 m long, climbed from the foredeck by an 8 m ramp, with
//!   a rib, a mast and the thrusters on it.
//! - **The corridor:** 32 m wide between the hulls, its floor broken by containers, barriers,
//!   low cover and an 8 m crane pylon in the middle (the rail capacitor lies by its foot). Along
//!   each hull a 4 m dock gantry on its supports runs 28 m, a 16 m ramp down from either end
//!   (two kit ramps, the upper one on platform blocks), and two gangways cross from it over the
//!   hull's shoulder onto the foredeck.
//! - **The outer lanes:** a 12 m lane behind each hull along the perimeter, with containers,
//!   a barrier and a jump pad that throws a fighter over the shoulder onto the foredeck; and a
//!   12 m lane round each ship's bow and stern joining the outer lanes to the corridor.
//!
//! Every hull top is reached by more than one route: the foredeck by either of the gantry's
//! ramps and two gangways, by the outer lane's pad and by the grapple (the bow mast); the stern
//! deck by its ramp from the foredeck and the grapple (its mast, or the wall above it). Each
//! stern deck is seen from the other ship's foredeck across the corridor and from the gantries,
//! and the pylon cuts the diagonal between the two stern decks: no hull top is a perch nobody
//! can reach or answer. Wheeled and tracked builds climb by the ramps; the pads and the grapple
//! are the agile builds' shortcuts.
//!
//! Fourteen spawn points, none in sight of another, all on the ground: in the outer lanes and
//! the bow and stern lanes behind containers, under the gantries and in the corridor's lee.
//! Bot waypoints on every level. Repair anchors (shooter#61) lie under the gantries and in the
//! outer lanes; the gantries' and the floor's low cover, the floor's barriers and the crates are
//! destruction candidates (shooter#60).

use crate::arena::{Arena, JumpPad, Spawn};
use crate::arena_kit::{self, Module, Placement, Turn};
use crate::physics::Vec3i;
use crate::pickups::{ArenaModule, ModulePad};

/// The floor spans ±40 m.
pub const HALF_SIZE: i32 = 4_000;
/// The perimeter's 60 cm walls stand just outside the floor.
const WALL_LINE: i32 = HALF_SIZE + 30;
/// The perimeter walls' height (10 m).
pub const PERIMETER_HEIGHT: i32 = 1_000;
/// The west ship's centre line (the east ship's is its twin).
pub const SHIP_X: i32 = -2_200;
/// Its hull's half width (12 m across, shoulders included) and the foredeck's (8 m).
const HULL_HALF: i32 = 600;
const DECK_HALF: i32 = 400;
/// Its bow (north) and stern (south) ends.
pub const BOW_Z: i32 = -2_800;
pub const STERN_Z: i32 = 2_800;
/// Where the raised stern deck begins.
pub const STERN_DECK_Z: i32 = 1_200;
/// The hull sides' top: the shoulder rises from here.
const SIDE_TOP: i32 = 200;
/// The foredeck's top (4 m) and the stern deck's (6 m).
pub const FOREDECK_TOP: i32 = 400;
pub const STERN_TOP: i32 = 600;
/// The dock gantry's top (4 m, level with the foredeck): a walkway section on a support on a
/// 1 m plinth.
pub const GANTRY_TOP: i32 = 400;
/// The gantry's centre line, between the hull and the corridor floor.
pub const GANTRY_X: i32 = -1_400;
/// The gantry's deck runs between these.
pub const GANTRY_Z: i32 = 1_400;

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

/// The west ship's hull: sides, shoulders, the foredeck, the raised stern deck and their ends,
/// closed all round (no interior).
fn hull(out: &mut Vec<Placement>) {
    let (west, east) = (SHIP_X - HULL_HALF, SHIP_X + HULL_HALF);
    let (deck_west, deck_east) = (SHIP_X - DECK_HALF, SHIP_X + DECK_HALF);
    // Along the hull in 4 m sections.
    let sections = (BOW_Z + 200..STERN_Z).step_by(400);
    for z in sections.clone() {
        // The 2 m sides, their outer faces on the hull's edges.
        out.push(place(Module::HullSide, west + 25, 0, z, Turn::Quarter));
        out.push(place(Module::HullSide, east - 25, 0, z, Turn::Quarter));
        // The shoulders, rising inward from the sides' top to the deck's edge.
        out.push(place(
            Module::HullShoulder,
            west + 100,
            SIDE_TOP,
            z,
            Turn::ThreeQuarter,
        ));
        out.push(place(
            Module::HullShoulder,
            east - 100,
            SIDE_TOP,
            z,
            Turn::Quarter,
        ));
        // The foredeck (under the stern deck too, which stands on it).
        for x in [SHIP_X - 200, SHIP_X + 200] {
            out.push(place(Module::HullDeck, x, FOREDECK_TOP - 50, z, Turn::None));
        }
    }
    // The bow and stern ends: three plates across, two courses up to the deck.
    for z in [BOW_Z + 25, STERN_Z - 25] {
        for x in [SHIP_X - 400, SHIP_X, SHIP_X + 400] {
            for y in [0, SIDE_TOP] {
                out.push(place(Module::HullSide, x, y, z, Turn::None));
            }
        }
    }
    // The raised stern deck: a 2 m course of plates round it on the foredeck, a deck on top.
    for z in (STERN_DECK_Z + 200..STERN_Z).step_by(400) {
        out.push(place(
            Module::HullSide,
            deck_west + 25,
            FOREDECK_TOP,
            z,
            Turn::Quarter,
        ));
        out.push(place(
            Module::HullSide,
            deck_east - 25,
            FOREDECK_TOP,
            z,
            Turn::Quarter,
        ));
        for x in [SHIP_X - 200, SHIP_X + 200] {
            out.push(place(Module::HullDeck, x, STERN_TOP - 50, z, Turn::None));
        }
    }
    for x in [SHIP_X - 200, SHIP_X + 200] {
        out.push(place(
            Module::HullSide,
            x,
            FOREDECK_TOP,
            STERN_DECK_Z + 25,
            Turn::None,
        ));
        out.push(place(
            Module::HullSide,
            x,
            FOREDECK_TOP,
            STERN_Z - 25,
            Turn::None,
        ));
    }
    // Its 8 m ramp from the foredeck, 4 m wide, rising south onto the stern deck.
    for x in [SHIP_X - 100, SHIP_X + 100] {
        out.push(place(
            Module::Ramp,
            x,
            FOREDECK_TOP,
            STERN_DECK_Z - 400,
            Turn::Half,
        ));
    }
    // On the foredeck: ribs staggered across it (walked round, not over) and the bow mast.
    out.push(place(
        Module::HullRib,
        SHIP_X + 200,
        FOREDECK_TOP,
        -1_500,
        Turn::None,
    ));
    out.push(place(
        Module::HullRib,
        SHIP_X - 200,
        FOREDECK_TOP,
        -400,
        Turn::None,
    ));
    out.push(place(
        Module::HullMast,
        SHIP_X,
        FOREDECK_TOP,
        -2_200,
        Turn::None,
    ));
    // On the stern deck: a rib by its ramp and the stern mast.
    out.push(place(
        Module::HullRib,
        SHIP_X + 200,
        STERN_TOP,
        1_700,
        Turn::None,
    ));
    out.push(place(
        Module::HullMast,
        SHIP_X - 200,
        STERN_TOP,
        2_500,
        Turn::None,
    ));
}

/// The west dock gantry: a 4 m walkway along the hull, a 16 m ramp down from each end and two
/// gangways over the shoulder onto the foredeck.
fn gantry(out: &mut Vec<Placement>) {
    for z in (-GANTRY_Z + 100..GANTRY_Z).step_by(200) {
        out.push(place(
            Module::Walkway,
            GANTRY_X,
            GANTRY_TOP - 40,
            z,
            Turn::Quarter,
        ));
    }
    for z in [-1_000, 0, 1_000] {
        out.push(place(Module::PlatformHalf, GANTRY_X, 0, z, Turn::None));
        out.push(place(Module::Support, GANTRY_X, 100, z, Turn::None));
    }
    // The ramps, 4 m wide: the lower from the floor to 2 m, the upper on platform blocks to the
    // gantry's end.
    for (end, turn) in [(-GANTRY_Z, Turn::Half), (GANTRY_Z, Turn::None)] {
        let away = end.signum();
        for x in [GANTRY_X - 100, GANTRY_X + 100] {
            out.push(place(Module::Ramp, x, 200, end + away * 400, turn));
            out.push(place(Module::Ramp, x, 0, end + away * 1_200, turn));
            for along in [100, 300, 500, 700] {
                out.push(place(
                    Module::Platform,
                    x,
                    0,
                    end + away * along,
                    Turn::None,
                ));
            }
        }
    }
    // The gangways over the shoulder.
    for z in [-1_000, 600] {
        out.push(place(
            Module::Walkway,
            SHIP_X + HULL_HALF - 100,
            GANTRY_TOP - 40,
            z,
            Turn::None,
        ));
    }
    // Low cover along its corridor edge.
    out.push(
        place(
            Module::CoverLow,
            GANTRY_X + 130,
            GANTRY_TOP,
            -500,
            Turn::Quarter,
        )
        .destructible(),
    );
    out.push(
        place(
            Module::CoverLow,
            GANTRY_X + 130,
            GANTRY_TOP,
            500,
            Turn::Quarter,
        )
        .destructible(),
    );
}

/// The west half's dock floor: the corridor's west half, the outer lane and the west ends of
/// the bow and stern lanes.
fn floor(out: &mut Vec<Placement>) {
    // The crane pylon in the corridor's middle, 4 m square and 8 m tall (its east half is the
    // west half turned).
    for z in [-100, 100] {
        for y in [0, 200, 400, 600] {
            out.push(place(Module::Platform, -100, y, z, Turn::None));
        }
    }
    // The corridor: a container, barriers, low cover and crates.
    out.push(place(Module::Container, -600, 0, -1_900, Turn::Quarter));
    out.push(place(Module::Barrier, -800, 0, 900, Turn::Quarter).destructible());
    out.push(place(Module::CoverLow, -700, 0, -500, Turn::None).destructible());
    out.push(place(Module::CoverLow, -300, 0, 2_000, Turn::None).destructible());
    out.push(place(Module::Crate, -1_000, 0, 2_300, Turn::None).destructible());
    out.push(place(Module::Crate, -920, 0, 2_300, Turn::None).destructible());
    out.push(place(Module::Generator, -900, 0, -2_900, Turn::None));
    // The outer lane: two containers and a barrier between them, pipes on the wall.
    out.push(place(Module::Container, -3_500, 0, -1_700, Turn::Quarter));
    out.push(place(Module::Container, -3_500, 0, 1_700, Turn::Quarter));
    out.push(place(Module::Barrier, -3_700, 0, 0, Turn::None).destructible());
    for z in [-2_700, -2_500, 2_500, 2_700] {
        out.push(place(Module::Pipes, -3_975, 0, z, Turn::Quarter));
    }
    // The bow lane: a container across the bow and a generator in the corner.
    out.push(place(Module::Container, SHIP_X, 0, -3_400, Turn::None));
    out.push(place(Module::Generator, -3_600, 0, -3_700, Turn::None));
    out.push(place(Module::Barrier, -400, 0, -3_800, Turn::Quarter));
    // The stern lane: a container behind the stern and a barrier toward the corridor.
    out.push(place(Module::Container, SHIP_X, 0, 3_400, Turn::None));
    out.push(place(Module::Barrier, -600, 0, 3_500, Turn::Quarter).destructible());
    out.push(place(Module::Crate, -3_600, 0, 3_500, Turn::None).destructible());
}

/// The west half's placements (the east half is the same turned half a turn).
fn west() -> Vec<Placement> {
    let mut out = Vec::new();
    hull(&mut out);
    gantry(&mut out);
    floor(&mut out);
    out
}

impl Arena {
    /// The drydock ([`crate::drydock`]): the large arena, built from the arena kit.
    #[must_use]
    pub fn drydock() -> Self {
        let west = west();
        let placements: Vec<Placement> = perimeter()
            .into_iter()
            .chain(west.iter().copied())
            .chain(west.iter().map(|placement| placement.turned_half()))
            .collect();
        let pieces = arena_kit::pieces(&placements);
        // In the outer lane, 7 m from the hull: over the shoulder onto the foredeck.
        let west_pads = [JumpPad {
            center: Vec3i::new(-3_400, 0, -600),
            half: [80, 80],
            launch: Vec3i::new(900, 1_500, 0),
        }];
        let pads = west_pads
            .iter()
            .copied()
            .chain(west_pads.iter().map(|pad| JumpPad {
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
        let west_spawns = [
            // The outer lane, north of its north container.
            spawn_at([-3_550, 0, -2_350], 0),
            // The outer lane, south of its south container.
            spawn_at([-3_550, 0, 2_300], 32_768),
            // The bow lane, behind the container across the bow.
            spawn_at([SHIP_X, 0, -3_850], 16_384),
            // The stern lane, behind the container across the stern.
            spawn_at([-2_050, 0, 3_650], 16_384),
            // The corridor, north of its container.
            spawn_at([-700, 0, -2_500], 0),
            // The corridor, behind its barrier.
            spawn_at([-1_000, 0, 1_100], 16_384),
            // The corridor, in the pylon's lee.
            spawn_at([-250, 0, 500], 0),
        ];
        let spawns = west_spawns[1..]
            .iter()
            .copied()
            .chain(west_spawns.iter().map(|spawn| Spawn {
                feet: turned(spawn.feet),
                yaw: spawn.yaw.wrapping_add(32_768),
            }))
            .collect();
        let west_waypoints = [
            // The gantry: its north ramp's foot, its deck and its south ramp's foot.
            Vec3i::new(GANTRY_X, 0, -3_200),
            Vec3i::new(GANTRY_X, GANTRY_TOP, -1_000),
            Vec3i::new(GANTRY_X, GANTRY_TOP, 600),
            Vec3i::new(GANTRY_X, 0, 3_200),
            // The foredeck, across the gangways, and the stern deck up its ramp.
            Vec3i::new(SHIP_X, FOREDECK_TOP, -2_600),
            Vec3i::new(SHIP_X, FOREDECK_TOP, -1_000),
            Vec3i::new(SHIP_X + 300, FOREDECK_TOP, 600),
            Vec3i::new(SHIP_X, FOREDECK_TOP, 200),
            Vec3i::new(SHIP_X, STERN_TOP, 1_400),
            Vec3i::new(SHIP_X, STERN_TOP, 2_400),
            // The corridor floor.
            Vec3i::new(-600, 0, -2_750),
            Vec3i::new(-1_000, 0, -1_000),
            Vec3i::new(-500, 0, 400),
            Vec3i::new(-500, 0, 1_600),
            // The outer lane.
            Vec3i::new(-3_200, 0, -2_400),
            Vec3i::new(-3_200, 0, 0),
            Vec3i::new(-3_200, 0, 2_400),
            // The bow and stern lanes.
            Vec3i::new(-3_400, 0, -3_200),
            Vec3i::new(-1_000, 0, -3_400),
            Vec3i::new(-3_400, 0, 3_200),
            Vec3i::new(-1_000, 0, 3_200),
            Vec3i::new(-3_000, 0, 3_750),
            Vec3i::new(-1_300, 0, 3_750),
        ];
        let waypoints = west_waypoints
            .iter()
            .copied()
            .chain(west_waypoints.iter().copied().map(turned))
            .collect();
        let module_pads = vec![
            ModulePad {
                center: Vec3i::new(-500, 0, -300),
                module: ArenaModule::RailCapacitor,
            },
            ModulePad {
                center: Vec3i::new(SHIP_X + 200, STERN_TOP, 2_200),
                module: ArenaModule::Thrusters,
            },
            ModulePad {
                center: turned(Vec3i::new(SHIP_X + 200, STERN_TOP, 2_200)),
                module: ArenaModule::Thrusters,
            },
        ];
        let west_repairs = [Vec3i::new(GANTRY_X, 0, 500), Vec3i::new(-3_700, 0, -400)];
        let repair_anchors = west_repairs
            .iter()
            .copied()
            .chain(west_repairs.iter().copied().map(turned))
            .collect();
        Self {
            half_size: HALF_SIZE,
            pieces,
            pads,
            posts: Vec::new(),
            spawn: west_spawns[0].feet,
            spawn_yaw: west_spawns[0].yaw,
            spawns,
            module_pads,
            waypoints,
            placements,
            repair_anchors,
        }
    }
}
