//! The drydock (`drydock`, shooter#55, #153): the beta's large long-sightline arena, a
//! repurposed spaceship shipyard where two docked spacecraft flank a central industrial corridor
//! (owner decision D2.i55.1), built from asset-tooling's arena kit ([`crate::arena_kit`]) like the
//! assembly works: the ships' exterior hulls are the kit's hull modules, the dock machinery its
//! walkways, ramps, platforms, containers and cover. No ship has an interior.
//!
//! An 80 m square dock inside 10 m walls with hybrid symmetry (D2.i55.2): the dock floor, its
//! lanes, spawn points, pickups and repair anchors are point-symmetric (every placement west of
//! the centre line has a twin turned half a turn round the centre in the east), while the two
//! ships lie head to tail on the same 12 m by 56 m footprint with distinct silhouettes, details,
//! gantry cover and local routes onto their hulls. Every place below is the west half's; the
//! east ship is described as it would lie in the west berth (bow north) and turned.
//!
//! - **The freighter (west):** 2 m hull sides and a 45° shoulder up to a 4 m foredeck, 8 m wide,
//!   with hull ribs (80 cm low cover) staggered across it and a sensor mast at the bow; and a
//!   raised 6 m stern deck, 16 m long, climbed from the foredeck by an 8 m ramp, with a rib, a
//!   mast and the thrusters on it: a long low hull with a high blunt stern castle.
//! - **The cutter (east):** a 2 m prow step across the whole hull's bow, 16 m long, with an 8 m
//!   ramp up onto the 4 m foredeck and a mast at its tip; a narrow 6 m bridge midships, 8 m long
//!   and 4 m wide, an 8 m ramp up onto it from the foredeck at either end, the thrusters and a
//!   mast on it; a rib, a generator and a mast on the after deck: a stepped hull with a hump
//!   in the middle. The prow step meets its gantry's bow ramp at 2 m: a way from the corridor
//!   onto the hull that is not the gantry's top.
//! - **The corridor:** 32 m wide between the hulls, its floor broken by containers, barriers,
//!   low cover and an 8 m crane pylon in the middle (the rail capacitor lies by its foot). Along
//!   each hull a 4 m dock gantry on its supports runs 28 m, a 16 m ramp down from either end
//!   (two kit ramps, the upper one on platform blocks); two gangways cross from it over the
//!   hull's shoulder onto the foredeck (the freighter's by its ribs, the cutter's in front of
//!   its bridge and beside it), the freighter's gantry with low cover along its corridor edge,
//!   the cutter's with stacked crates and two barriers.
//! - **The outer lanes:** a 12 m lane behind each hull along the perimeter, with containers,
//!   a barrier and jump pads over the shoulder (the freighter's onto its foredeck and onto its
//!   stern deck, the cutter's onto its foredeck before the bridge); and a 12 m lane round each
//!   ship's bow and stern joining the outer lanes to the corridor. The hulls screen the outer
//!   lanes from the corridor: the protected flank routes past its long sightlines.
//!
//! Every hull top is reached by more than one route, without the grapple too: the foredecks by
//! either gantry ramp and two gangways, by an outer lane's pad, and the cutter's also up its
//! prow; the freighter's stern deck by its ramp from the foredeck and its outer lane's pad, the
//! cutter's bridge by its two ramps; and every one by the grapple (the masts). Each 6 m deck is
//! seen from the other ship's hull across the corridor and from the gantries, and sees about as
//! much of the dock floor as the other: no hull top is a perch nobody can reach or answer.
//! Wheeled and tracked builds climb by the ramps (wheels not yet: shooter#174); the pads and the
//! grapple are the agile builds' shortcuts.
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
/// The cutter's prow step (its top 2 m up) runs from its bow to here (in the west berth).
pub const PROW_Z: i32 = -1_200;
pub const PROW_TOP: i32 = SIDE_TOP;
/// The cutter's bridge (its top at [`STERN_TOP`]) runs between these on its foredeck (in the
/// west berth), 4 m wide.
pub const BRIDGE_Z: [i32; 2] = [400, 1_200];
const BRIDGE_HALF: i32 = 200;

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

/// The freighter's hull in the west berth: sides, shoulders, the foredeck, the raised stern
/// deck and their ends, closed all round (no interior).
fn freighter(out: &mut Vec<Placement>) {
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

/// The cutter's hull as it would lie in the west berth (bow north): its prow step, the
/// foredeck behind it with the bridge on it, and their ends, closed all round (no interior).
fn cutter(out: &mut Vec<Placement>) {
    let (west, east) = (SHIP_X - HULL_HALF, SHIP_X + HULL_HALF);
    for z in (BOW_Z + 200..STERN_Z).step_by(400) {
        out.push(place(Module::HullSide, west + 25, 0, z, Turn::Quarter));
        out.push(place(Module::HullSide, east - 25, 0, z, Turn::Quarter));
        if z < PROW_Z {
            // The prow step: a deck across the whole hull on the 2 m sides.
            for x in [SHIP_X - 400, SHIP_X, SHIP_X + 400] {
                out.push(place(Module::HullDeck, x, PROW_TOP - 50, z, Turn::None));
            }
            continue;
        }
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
        for x in [SHIP_X - 200, SHIP_X + 200] {
            out.push(place(Module::HullDeck, x, FOREDECK_TOP - 50, z, Turn::None));
        }
    }
    // The bow end one course up to the prow step, the stern end two up to the foredeck, and the
    // foredeck's front either side of the prow's ramp.
    for x in [SHIP_X - 400, SHIP_X, SHIP_X + 400] {
        out.push(place(Module::HullSide, x, 0, BOW_Z + 25, Turn::None));
        for y in [0, SIDE_TOP] {
            out.push(place(Module::HullSide, x, y, STERN_Z - 25, Turn::None));
        }
    }
    for x in [SHIP_X - 400, SHIP_X + 400] {
        out.push(place(
            Module::HullSide,
            x,
            SIDE_TOP,
            PROW_Z + 25,
            Turn::None,
        ));
    }
    // The prow's 8 m ramp, 4 m wide, rising south onto the foredeck (8 m of the step before it).
    for x in [SHIP_X - 100, SHIP_X + 100] {
        out.push(place(Module::Ramp, x, PROW_TOP, PROW_Z - 400, Turn::Half));
    }
    // The bridge: a 2 m course of plates along its sides on the foredeck, a deck on top and a
    // second course along its outer side (a bulkhead: the bridge looks over the corridor, not
    // down into the outer lane, which stays a covered flank), an 8 m ramp up onto it at either
    // end (as wide as it).
    for z in (BRIDGE_Z[0] + 200..BRIDGE_Z[1]).step_by(400) {
        let (outer, inner) = (SHIP_X - BRIDGE_HALF + 25, SHIP_X + BRIDGE_HALF - 25);
        for (x, y) in [
            (outer, FOREDECK_TOP),
            (inner, FOREDECK_TOP),
            (outer, STERN_TOP),
        ] {
            out.push(place(Module::HullSide, x, y, z, Turn::Quarter));
        }
        out.push(place(
            Module::HullDeck,
            SHIP_X,
            STERN_TOP - 50,
            z,
            Turn::None,
        ));
    }
    for x in [SHIP_X - 100, SHIP_X + 100] {
        out.push(place(
            Module::Ramp,
            x,
            FOREDECK_TOP,
            BRIDGE_Z[0] - 400,
            Turn::Half,
        ));
        out.push(place(
            Module::Ramp,
            x,
            FOREDECK_TOP,
            BRIDGE_Z[1] + 400,
            Turn::None,
        ));
    }
    // Masts at the prow's tip and midway along the bridge against its bulkhead; on the after deck a rib, a
    // generator and the stern mast.
    for (x, y, z) in [
        (SHIP_X - 400, PROW_TOP, BOW_Z + 200),
        (SHIP_X - 100, STERN_TOP, 800),
        (SHIP_X - 300, FOREDECK_TOP, 2_650),
    ] {
        out.push(place(Module::HullMast, x, y, z, Turn::None));
    }
    out.push(place(
        Module::HullRib,
        SHIP_X - 200,
        FOREDECK_TOP,
        2_300,
        Turn::None,
    ));
    out.push(place(
        Module::Generator,
        SHIP_X + 200,
        FOREDECK_TOP,
        2_550,
        Turn::None,
    ));
}

/// A dock gantry in the west berth: a 4 m walkway along the hull, a 16 m ramp down from each
/// end and gangways over the shoulder onto the foredeck at `gangways`.
fn gantry(out: &mut Vec<Placement>, gangways: [i32; 2]) {
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
    for z in gangways {
        out.push(place(
            Module::Walkway,
            SHIP_X + HULL_HALF - 100,
            GANTRY_TOP - 40,
            z,
            Turn::None,
        ));
    }
}

/// The freighter's gantry: gangways by its foredeck's ribs, low cover along its corridor edge.
fn freighter_gantry(out: &mut Vec<Placement>) {
    gantry(out, [-1_000, 600]);
    for z in [-500, 500] {
        out.push(
            place(
                Module::CoverLow,
                GANTRY_X + 130,
                GANTRY_TOP,
                z,
                Turn::Quarter,
            )
            .destructible(),
        );
    }
}

/// The cutter's gantry (in the west berth): gangways onto the foredeck before the bridge and
/// beside it, stacked crates and a barrier along its corridor edge.
fn cutter_gantry(out: &mut Vec<Placement>) {
    gantry(out, [-800, 800]);
    for (z, y) in [(-340, 0), (-260, 0), (-300, 80)] {
        out.push(
            place(Module::Crate, GANTRY_X + 140, GANTRY_TOP + y, z, Turn::None).destructible(),
        );
    }
    for z in [300, 900] {
        out.push(
            place(
                Module::Barrier,
                GANTRY_X + 130,
                GANTRY_TOP,
                z,
                Turn::Quarter,
            )
            .destructible(),
        );
    }
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

/// The west half's placements: the freighter, its gantry and the dock floor.
fn west() -> Vec<Placement> {
    let mut out = Vec::new();
    freighter(&mut out);
    freighter_gantry(&mut out);
    floor(&mut out);
    out
}

/// The east half's placements as they would lie in the west (they are turned half a turn into
/// the east): the cutter, its gantry and the dock floor.
fn east() -> Vec<Placement> {
    let mut out = Vec::new();
    cutter(&mut out);
    cutter_gantry(&mut out);
    floor(&mut out);
    out
}

/// The outer lane's pads, thrown over the hull's shoulder: the freighter's onto its foredeck, 12 m
/// from its centre line, and (shooter#153) onto its stern deck, 15.5 m from it by the wall, off the
/// lane bots walk (they would be thrown up by chance); the cutter's (in the west berth) onto its
/// foredeck before the bridge, 12.5 m from it.
const FREIGHTER_PADS: [JumpPad; 2] = [
    JumpPad {
        center: Vec3i::new(-3_400, 0, -600),
        half: [80, 80],
        launch: Vec3i::new(900, 1_500, 0),
    },
    JumpPad {
        center: Vec3i::new(-3_750, 0, 1_000),
        half: [80, 80],
        launch: Vec3i::new(975, 1_800, 510),
    },
];
const CUTTER_PADS: [JumpPad; 1] = [JumpPad {
    center: Vec3i::new(-3_450, 0, -800),
    half: [80, 80],
    launch: Vec3i::new(900, 1_500, 0),
}];

impl Arena {
    /// The drydock ([`crate::drydock`]): the large arena, built from the arena kit.
    #[must_use]
    pub fn drydock() -> Self {
        let placements: Vec<Placement> = perimeter()
            .into_iter()
            .chain(west())
            .chain(east().into_iter().map(Placement::turned_half))
            .collect();
        let pieces = arena_kit::pieces(&placements);
        let pads = FREIGHTER_PADS
            .iter()
            .copied()
            .chain(CUTTER_PADS.iter().map(|pad| JumpPad {
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
            spawn_at([-3_550, 0, -2_350], 24_576),
            // The outer lane, south of its south container.
            spawn_at([-3_550, 0, 2_300], 8_192),
            // The bow lane, behind the container across the bow.
            spawn_at([SHIP_X, 0, -3_850], 16_384),
            // The stern lane, behind the container across the stern, midway to the wall (shooter#153:
            // 1 m from the container, a chassis turning in place caught on it).
            spawn_at([-2_300, 0, 3_720], 16_384),
            // The corridor, north of its container, facing the open corridor (shooter#153: it faced
            // the container 2 m off, where a wheeled robot turned in place for 20 s, shooter#170).
            spawn_at([-700, 0, -2_500], 16_384),
            // The corridor, behind its barrier, facing south along it (its way out; it faced east).
            spawn_at([-1_000, 0, 1_100], 0),
            // The corridor, in the pylon's lee, facing north-west past it (a car facing south backed
            // into it).
            spawn_at([-300, 0, 500], 40_960),
        ];
        let spawns = west_spawns[1..]
            .iter()
            .copied()
            .chain(west_spawns.iter().map(|spawn| Spawn {
                feet: turned(spawn.feet),
                yaw: spawn.yaw.wrapping_add(32_768),
            }))
            .collect();
        let freighter_waypoints = [
            // The gantry's deck by the gangways.
            Vec3i::new(GANTRY_X, GANTRY_TOP, -1_000),
            Vec3i::new(GANTRY_X, GANTRY_TOP, 600),
            // The foredeck, across the gangways, and the stern deck up its ramp.
            Vec3i::new(SHIP_X, FOREDECK_TOP, -2_600),
            Vec3i::new(SHIP_X, FOREDECK_TOP, -1_000),
            Vec3i::new(SHIP_X + 300, FOREDECK_TOP, 600),
            Vec3i::new(SHIP_X, FOREDECK_TOP, 200),
            Vec3i::new(SHIP_X, STERN_TOP, 1_400),
            Vec3i::new(SHIP_X, STERN_TOP, 2_400),
        ];
        // The cutter's in the west berth.
        let cutter_waypoints = [
            // The gantry's deck by the gangways, and its bow ramp where the prow step meets it.
            Vec3i::new(GANTRY_X, GANTRY_TOP, -800),
            Vec3i::new(GANTRY_X, GANTRY_TOP, 800),
            Vec3i::new(GANTRY_X, PROW_TOP, -2_200),
            // The prow step by the gantry and before its ramp, the foredeck at the ramp's top,
            // across the gangways and behind the bridge, and the bridge up its ramps.
            Vec3i::new(SHIP_X + 400, PROW_TOP, -2_200),
            Vec3i::new(SHIP_X, PROW_TOP, -2_400),
            Vec3i::new(SHIP_X, FOREDECK_TOP, -1_100),
            Vec3i::new(SHIP_X + 300, FOREDECK_TOP, -800),
            Vec3i::new(SHIP_X, FOREDECK_TOP, -800),
            Vec3i::new(SHIP_X + 300, FOREDECK_TOP, 800),
            Vec3i::new(SHIP_X, FOREDECK_TOP, 2_100),
            Vec3i::new(SHIP_X + 50, STERN_TOP, 500),
            Vec3i::new(SHIP_X + 50, STERN_TOP, 1_100),
        ];
        let floor_waypoints = [
            // The gantry's ramps' feet.
            Vec3i::new(GANTRY_X, 0, -3_200),
            Vec3i::new(GANTRY_X, 0, 3_200),
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
        let waypoints = floor_waypoints
            .iter()
            .chain(&freighter_waypoints)
            .copied()
            .chain(
                floor_waypoints
                    .iter()
                    .chain(&cutter_waypoints)
                    .copied()
                    .map(turned),
            )
            .collect();
        // The rail capacitor by the pylon's foot, the thrusters on each ship's 6 m deck.
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
                center: turned(Vec3i::new(SHIP_X + 50, STERN_TOP, 700)),
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
