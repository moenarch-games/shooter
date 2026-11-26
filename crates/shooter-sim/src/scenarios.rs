//! Scripted fast moves, each in a small arena of its own (or a greybox map): sprint and slide,
//! double jump onto a ledge, a thruster-module jump onto a higher one, a jump pad onto the platform, slides up and down a ramp, a slide
//! into a thin wall, a sprint up a step, a rocket jump, a grapple onto a ledge, a shooting range
//! and a close range for the shotgun, laser and flamethrower, and the greybox yard's walkway
//! (up its ramp, along and off it) and its jump pad onto it, and the assembly works' routes (its
//! ramp, mezzanine and bridge, its cover, and its pad, work platforms and crane mast onto the
//! crane tower), and the proving cell's (its ramp and gallery, its stage, its pad and a grapple
//! from the stage onto the gallery), and the drydock's (its gantry ramp and a gangway onto a
//! foredeck, the stern ramp, the outer lane's pad and a grapple on the bow mast). The movement
//! and weapon tests check
//! what each one does and the Rapier A/B test runs each on both backends.

use crate::arena::{Arena, Piece, Post};
use crate::bindings::Motors;
use crate::drydock::{FOREDECK_TOP, GANTRY_X, SHIP_X};
use crate::physics::RampRise;
use crate::physics::Vec3i;
use crate::pickups::{ArenaModule, ModulePad};
use crate::player::Intent;
use crate::proving_cell::STAGE_TOP;
use crate::robot::{Assembly, HEAVY, PartId, RAIL_RACK, ROCKET_POD, SKIRMISHER, TWIN_RAIL};
use crate::tour::{EAST, Leg, NORTH, SOUTH, WEST, hold, stand, walk};
use crate::weapons::{MAX_PITCH, Select, Weapon};

/// One scripted scenario.
#[derive(Clone, Copy, Debug)]
pub struct Scenario {
    pub name: &'static str,
    pub arena: fn() -> Arena,
    pub legs: &'static [Leg],
}

const fn sprint(name: &'static str, yaw: u16, ticks: u32) -> Leg {
    hold(
        name,
        Intent {
            forward: 1,
            yaw,
            sprint: true,
            ..Intent::IDLE
        },
        ticks,
    )
}

/// Sprinting forward with crouch held: a slide once fast enough.
const fn slide(name: &'static str, yaw: u16, ticks: u32) -> Leg {
    hold(
        name,
        Intent {
            forward: 1,
            yaw,
            sprint: true,
            crouch: true,
            ..Intent::IDLE
        },
        ticks,
    )
}

const fn jump_toward(name: &'static str, yaw: u16) -> Leg {
    hold(
        name,
        Intent {
            forward: 1,
            yaw,
            jump: true,
            ..Intent::IDLE
        },
        1,
    )
}

/// An open floor with `pieces` and a spawn at `spawn`, facing south (+Z).
fn open(spawn: [i32; 3], pieces: Vec<Piece>) -> Arena {
    Arena {
        half_size: 2_000,
        pieces,
        pads: Vec::new(),
        posts: Vec::new(),
        spawn: Vec3i::new(spawn[0], spawn[1], spawn[2]),
        spawn_yaw: SOUTH,
        spawns: Vec::new(),
        module_pads: Vec::new(),
        waypoints: Vec::new(),
        placements: Vec::new(),
        repair_anchors: Vec::new(),
    }
}

fn block(center: [i32; 3], half: [i32; 3]) -> Piece {
    Piece::Block {
        center: Vec3i::new(center[0], center[1], center[2]),
        half: Vec3i::new(half[0], half[1], half[2]),
    }
}

/// Sprint south, then slide on.
pub const SPRINT_AND_SLIDE: Scenario = Scenario {
    name: "sprint and slide",
    arena: || open([0, 0, -1_500], Vec::new()),
    legs: &[
        stand("settle", 10),
        sprint("sprint", SOUTH, 40),
        slide("slide", SOUTH, 70),
        stand("stop", 10),
    ],
};

/// A 1.6 m ledge 3.7 m south: one jump (1 m) cannot reach its top, a double jump can.
pub const DOUBLE_JUMP: Scenario = Scenario {
    name: "double jump onto a ledge",
    arena: || open([0, 0, -600], vec![block([0, 80, 0], [200, 80, 200])]),
    legs: &[
        stand("settle", 10),
        walk("run up", SOUTH, 6),
        jump_toward("jump", SOUTH),
        walk("rise", SOUTH, 18),
        jump_toward("air jump", SOUTH),
        walk("climb", SOUTH, 40),
        stand("on the ledge", 30),
    ],
};

/// The [`DOUBLE_JUMP`] run at a 2 m ledge, with thrusters ([`crate::pickups`]) lying where the
/// player spawns: it takes them, and one thruster jump (2.25 m) climbs the ledge that neither a
/// plain jump nor a plain double jump's first leap could.
pub const THRUSTER_JUMP: Scenario = Scenario {
    name: "thruster jump onto a ledge",
    arena: || Arena {
        module_pads: vec![ModulePad {
            center: Vec3i::new(0, 0, -600),
            module: ArenaModule::Thrusters,
        }],
        ..open([0, 0, -600], vec![block([0, 100, 0], [200, 100, 200])])
    },
    legs: &[
        stand("take the thrusters", 10),
        walk("run up", SOUTH, 6),
        jump_toward("jump", SOUTH),
        walk("climb", SOUTH, 50),
        stand("on the ledge", 45),
    ],
};

/// The greybox from beside its east pad: onto the pad, thrown west onto the platform.
pub const JUMP_PAD: Scenario = Scenario {
    name: "jump pad onto the platform",
    arena: || Arena {
        spawn: Vec3i::new(1_000, 0, 400),
        spawn_yaw: NORTH,
        ..Arena::greybox()
    },
    legs: &[
        stand("settle", 10),
        walk("onto the pad", NORTH, 30),
        stand("thrown", 90),
        stand("on the platform", 20),
    ],
};

/// A 14° ramp rising north to a 2 m deck: slide up it, turn and slide back down.
pub const RAMP_SLIDE: Scenario = Scenario {
    name: "slide up and down a ramp",
    arena: || {
        let mut arena = open(
            [0, 0, 1_000],
            vec![
                Piece::Ramp {
                    center: Vec3i::new(0, 100, 0),
                    half: Vec3i::new(200, 100, 400),
                    rises: RampRise::NegZ,
                },
                block([0, 100, -1_000], [200, 100, 600]),
            ],
        );
        arena.spawn_yaw = NORTH;
        arena
    },
    legs: &[
        stand("settle", 10),
        sprint("sprint at the ramp", NORTH, 20),
        slide("slide up", NORTH, 60),
        stand("on the deck", 20),
        sprint("sprint back", SOUTH, 20),
        slide("slide down", SOUTH, 60),
        stand("at the foot", 20),
    ],
};

/// A 4 cm wall: a slide at 15 m/s (25 units a tick) must not pass through it.
pub const THIN_WALL: Scenario = Scenario {
    name: "slide into a thin wall",
    arena: || open([0, 0, -800], vec![block([0, 100, 0], [200, 100, 2])]),
    legs: &[
        stand("settle", 10),
        sprint("sprint", SOUTH, 20),
        slide("slide into the wall", SOUTH, 50),
        stand("against the wall", 10),
    ],
};

/// A 20 cm step 3.7 m south, 4 m deep: a sprint climbs it without a jump
/// ([`crate::player::STEP_UP`]) and runs on across its top.
pub const STEP: Scenario = Scenario {
    name: "sprint up a step",
    arena: || open([0, 0, -600], vec![block([0, 10, 0], [200, 10, 200])]),
    legs: &[
        stand("settle", 10),
        sprint("sprint up the step", SOUTH, 30),
        stand("on the step", 10),
    ],
};

/// Straight down.
const DOWN: i16 = -MAX_PITCH;

/// The rocket jump's launcher hinge ([`crate::robot::ROCKET_JUMPER`]) tracking the aim: its
/// tracking binding held (shooter#132).
const TRACK_LAUNCHER: Motors = Motors::track(PartId(7));

/// A 2.5 m ledge 3.3 m south, 12 m wide: out of reach of a double jump (about 2 m), in reach of
/// a rocket jump. A fixed launcher keeps its mount's direction (shooter#132), so the scenario is
/// the [`crate::robot::ROCKET_JUMPER`]'s: look down with its hinge tracking the aim until the
/// launcher points at the floor, jump and fire (past the hinge's quarter turn the rocket flies
/// along the launcher, straight down beside the feet), then steer onto the ledge.
pub const ROCKET_JUMP: Scenario = Scenario {
    name: "rocket jump onto a ledge",
    arena: || open([0, 0, -330], vec![block([0, 125, 600], [600, 125, 600])]),
    legs: &[
        hold(
            "take the launcher",
            Intent {
                yaw: SOUTH,
                pitch: DOWN,
                select: Some(Select::Weapon(Weapon::RocketLauncher)),
                motors: TRACK_LAUNCHER,
                ..Intent::IDLE
            },
            40,
        ),
        hold(
            "jump and fire down",
            Intent {
                yaw: SOUTH,
                pitch: DOWN,
                jump: true,
                fire: true,
                motors: TRACK_LAUNCHER,
                ..Intent::IDLE
            },
            1,
        ),
        hold(
            "blast",
            Intent {
                yaw: SOUTH,
                pitch: DOWN,
                motors: TRACK_LAUNCHER,
                ..Intent::IDLE
            },
            8,
        ),
        walk("steer onto the ledge", SOUTH, 50),
        stand("on the ledge", 60),
    ],
};

/// Pitch steps from the grapple scenario's eye (0, 165, -600) that put the camera's target, and
/// so the line from the grapple module's muzzle (a hand left of, below and ahead of the eye),
/// on the pillar's face at (0, 500, 650): about 16.1°.
const PILLAR_PITCH: i16 = 2_931;

/// A 3 m ledge 9 m south (beyond a double jump and a rocket jump) with an 8 m pillar on it.
/// Aim the grapple module (its own key; the standard build mounts it), hook the pillar's face
/// 5 m up over the ledge's edge, get reeled up and over the edge, let go and step onto the ledge.
pub const GRAPPLE: Scenario = Scenario {
    name: "grapple onto a ledge",
    arena: || {
        open(
            [0, 0, -600],
            vec![
                block([0, 150, 600], [300, 150, 300]),
                block([0, 400, 700], [50, 400, 50]),
            ],
        )
    },
    legs: &[
        hold(
            "aim the grapple",
            Intent {
                yaw: SOUTH,
                pitch: PILLAR_PITCH,
                ..Intent::IDLE
            },
            15,
        ),
        hold(
            "hook and reel in",
            Intent {
                yaw: SOUTH,
                pitch: PILLAR_PITCH,
                grapple: true,
                ..Intent::IDLE
            },
            90,
        ),
        walk("let go onto the ledge", SOUTH, 40),
        stand("on the ledge", 30),
    ],
};

/// The yaw from the shooting range's spawn (0, 0, -600) toward a point on the floor.
const fn yaw_to(x: i32, z: i32) -> u16 {
    match crate::trig::yaw_from_vector(x, z + 600) {
        Some(yaw) => yaw,
        None => 0,
    }
}

const fn shoot(name: &'static str, yaw: u16, weapon: Weapon, ticks: u32) -> Leg {
    hold(
        name,
        Intent {
            yaw,
            fire: true,
            select: Some(Select::Weapon(weapon)),
            ..Intent::IDLE
        },
        ticks,
    )
}

const fn aim_with(name: &'static str, yaw: u16, weapon: Weapon, ticks: u32) -> Leg {
    hold(
        name,
        Intent {
            yaw,
            select: Some(Select::Weapon(weapon)),
            ..Intent::IDLE
        },
        ticks,
    )
}

/// A shooting range: three dummies 6 m south, the west one behind a wall. The rifle and the rail
/// gun take down the middle one, a rocket hits the east one, rifle bolts at the west one stop at
/// the wall.
pub const SHOOTING_RANGE: Scenario = Scenario {
    name: "shooting range",
    arena: || {
        let mut arena = open([0, 0, -600], vec![block([-200, 150, -300], [50, 150, 50])]);
        arena.posts = vec![
            Post {
                feet: Vec3i::new(0, 0, 0),
                patrol_units: 0,
            },
            Post {
                feet: Vec3i::new(400, 0, 0),
                patrol_units: 0,
            },
            Post {
                feet: Vec3i::new(-400, 0, 0),
                patrol_units: 0,
            },
        ];
        arena
    },
    legs: &[
        hold(
            "settle",
            Intent {
                yaw: SOUTH,
                ..Intent::IDLE
            },
            30,
        ),
        shoot("rifle the middle", SOUTH, Weapon::EnergyRifle, 60),
        aim_with("take the rail gun", SOUTH, Weapon::RailGun, 12),
        shoot("rail the middle", SOUTH, Weapon::RailGun, 1),
        aim_with(
            "take the launcher",
            yaw_to(400, 0),
            Weapon::RocketLauncher,
            72,
        ),
        shoot("rocket the east", yaw_to(400, 0), Weapon::RocketLauncher, 1),
        aim_with("rocket flies", yaw_to(400, 0), Weapon::RocketLauncher, 40),
        aim_with("take the rifle", yaw_to(-400, 0), Weapon::EnergyRifle, 12),
        shoot("rifle the wall", yaw_to(-400, 0), Weapon::EnergyRifle, 30),
    ],
};

/// A close range: a dummy 4 m south, one off to the east and one to the west behind a pillar.
/// The flamethrower ignites, then burns the south one and stops at the pillar; let go and
/// pressed again, it ignites anew. A shotgun blast (its pellets reach the south one within their
/// first tick, at full damage) and the laser hit the south and east ones.
pub const CLOSE_RANGE: Scenario = Scenario {
    name: "close range",
    arena: || {
        let mut arena = open([0, 0, -600], vec![block([-150, 150, -400], [40, 150, 40])]);
        arena.posts = [[0, 0, -200], [300, 0, -200], [-300, 0, -200]]
            .map(|[x, y, z]| Post {
                feet: Vec3i::new(x, y, z),
                patrol_units: 0,
            })
            .to_vec();
        arena
    },
    legs: &[
        hold(
            "settle",
            Intent {
                yaw: SOUTH,
                ..Intent::IDLE
            },
            30,
        ),
        aim_with("take the flamethrower", SOUTH, Weapon::Flamethrower, 12),
        shoot("flame the south", SOUTH, Weapon::Flamethrower, 45),
        // Its flamethrower is fixed on its front: the robot turns to the pillar before it
        // burns there (shooter#132).
        aim_with(
            "turn to the pillar",
            yaw_to(-300, -200),
            Weapon::Flamethrower,
            15,
        ),
        shoot(
            "flame the pillar",
            yaw_to(-300, -200),
            Weapon::Flamethrower,
            30,
        ),
        aim_with("let go", SOUTH, Weapon::Flamethrower, 1),
        shoot("ignite again", SOUTH, Weapon::Flamethrower, 15),
        aim_with("take the shotgun", SOUTH, Weapon::Shotgun, 12),
        shoot("blast the south", SOUTH, Weapon::Shotgun, 1),
        aim_with("take the laser", yaw_to(300, -200), Weapon::Laser, 54),
        shoot("laser the east", yaw_to(300, -200), Weapon::Laser, 30),
    ],
};

/// The greybox yard's walkway from its west wing: up the west ramp, east along the deck past the
/// rail capacitor, then south off its edge to the ground 3 m below.
pub const YARD_WALKWAY: Scenario = Scenario {
    name: "the yard's walkway",
    arena: || Arena {
        spawn: Vec3i::new(-3_200, 0, 0),
        spawn_yaw: EAST,
        ..Arena::greybox_yard()
    },
    legs: &[
        stand("settle", 10),
        walk("up the west ramp", EAST, 140),
        walk("along the walkway", EAST, 160),
        stand("on the walkway", 10),
        walk("off the edge", SOUTH, 40),
        stand("below", 40),
    ],
};

/// The greybox yard from south of its south-east pad: onto the pad, thrown north onto the
/// walkway.
pub const YARD_JUMP_PAD: Scenario = Scenario {
    name: "the yard's pad onto the walkway",
    arena: || Arena {
        spawn: Vec3i::new(1_200, 0, 900),
        spawn_yaw: NORTH,
        ..Arena::greybox_yard()
    },
    legs: &[
        stand("settle", 10),
        walk("onto the pad", NORTH, 34),
        stand("thrown", 90),
        stand("on the walkway", 20),
    ],
};

/// The assembly works from the foot of its west mezzanine ramp: north up the 12 m ramp onto the
/// mezzanine, east along its deck to the gantry bridge, south along the bridge past its low
/// cover to the rail capacitor in the middle, then east off the bridge's edge to the floor 3 m
/// below.
pub const WORKS_RAMP_AND_BRIDGE: Scenario = Scenario {
    name: "the works' ramp, mezzanine and bridge",
    arena: || Arena {
        spawn: Vec3i::new(-1_700, 0, 100),
        spawn_yaw: NORTH,
        ..Arena::assembly_works()
    },
    legs: &[
        stand("settle", 10),
        walk("up the ramp", NORTH, 145),
        walk("along the mezzanine", EAST, 150),
        walk("along the bridge", SOUTH, 130),
        stand("on the bridge", 10),
        walk("off the edge", EAST, 40),
        stand("below", 40),
    ],
};

/// The assembly works' floor north-west of the bridge: south into a 1.2 m low cover module, a
/// double jump over it, and east into the 2 m barrier no jump clears.
pub const WORKS_COVER: Scenario = Scenario {
    name: "the works' cover",
    arena: || Arena {
        spawn: Vec3i::new(-1_100, 0, -1_500),
        spawn_yaw: SOUTH,
        ..Arena::assembly_works()
    },
    legs: &[
        stand("settle", 10),
        walk("into the cover", SOUTH, 40),
        jump_toward("jump", SOUTH),
        walk("rise", SOUTH, 16),
        jump_toward("air jump", SOUTH),
        walk("over", SOUTH, 40),
        stand("land", 10),
        walk("into the barrier", EAST, 30),
        stand("against the barrier", 10),
    ],
};

/// The assembly works' crane tower from the south: onto the jump pad at its foot, thrown up onto
/// its 5 m top.
pub const WORKS_TOWER_PAD: Scenario = Scenario {
    name: "the works' pad onto the crane tower",
    arena: || Arena {
        spawn: Vec3i::new(2_200, 0, -1_250),
        spawn_yaw: NORTH,
        ..Arena::assembly_works()
    },
    legs: &[
        stand("settle", 10),
        walk("onto the pad", NORTH, 24),
        stand("thrown", 90),
        stand("on the tower", 20),
    ],
};

/// The assembly works' work platforms: east from the bay floor, a double jump onto the 2 m
/// step, another onto the 4 m one and a plain jump onto the crane tower's 5 m top.
pub const WORKS_STEPS: Scenario = Scenario {
    name: "the works' steps to the crane tower",
    arena: || Arena {
        spawn: Vec3i::new(1_000, 0, -2_200),
        spawn_yaw: EAST,
        ..Arena::assembly_works()
    },
    legs: &[
        stand("settle", 10),
        jump_toward("jump", EAST),
        walk("rise", EAST, 14),
        jump_toward("air jump", EAST),
        walk("onto the 2 m step", EAST, 30),
        jump_toward("jump again", EAST),
        walk("rise again", EAST, 14),
        jump_toward("air jump again", EAST),
        walk("onto the 4 m step", EAST, 30),
        jump_toward("jump onto the tower", EAST),
        walk("climb", EAST, 16),
        walk("onto the tower", EAST, 12),
        stand("on the tower", 20),
    ],
};

/// Pitch steps from the tower grapple's eye (2330, 165, -1450) that put the line from the grapple
/// module's muzzle on the crane mast's ring, over the tower's lip.
const MAST_PITCH: i16 = 6_400;

/// The assembly works' crane mast from the floor south of the tower: hook its ring, reel up the
/// tower's face and over its edge, and let go onto the top.
pub const WORKS_GRAPPLE: Scenario = Scenario {
    name: "the works' grapple onto the crane tower",
    arena: || Arena {
        spawn: Vec3i::new(2_330, 0, -1_450),
        spawn_yaw: NORTH,
        ..Arena::assembly_works()
    },
    legs: &[
        hold(
            "aim the grapple",
            Intent {
                yaw: NORTH,
                pitch: MAST_PITCH,
                ..Intent::IDLE
            },
            15,
        ),
        hold(
            "hook and reel in",
            Intent {
                yaw: NORTH,
                pitch: MAST_PITCH,
                grapple: true,
                ..Intent::IDLE
            },
            90,
        ),
        walk("let go onto the tower", NORTH, 30),
        stand("on the tower", 30),
    ],
};

/// The proving cell from the foot of its north gallery's ramp: north up the 12 m ramp onto the
/// gallery, east along its deck past the thrusters, then south off its edge between its low
/// covers to the floor 3 m below.
pub const CELL_GALLERY: Scenario = Scenario {
    name: "the cell's ramp and gallery",
    arena: || Arena {
        spawn: Vec3i::new(-1_400, 0, 150),
        spawn_yaw: NORTH,
        ..Arena::proving_cell()
    },
    legs: &[
        stand("settle", 10),
        walk("up the ramp", NORTH, 130),
        walk("along the gallery", EAST, 92),
        stand("on the gallery", 10),
        walk("off the edge", SOUTH, 40),
        stand("below", 40),
    ],
};

/// The proving cell's stage: south up its north ramp onto the 2 m stage, across it past the rail
/// capacitor and down its south ramp.
pub const CELL_STAGE: Scenario = Scenario {
    name: "the cell's stage",
    arena: || Arena {
        spawn: Vec3i::new(0, 0, -1_250),
        spawn_yaw: SOUTH,
        ..Arena::proving_cell()
    },
    legs: &[
        stand("settle", 10),
        walk("up the ramp", SOUTH, 85),
        stand("on the stage", 10),
        walk("down the south ramp", SOUTH, 125),
        stand("at its foot", 10),
    ],
};

/// The proving cell's jump pad in front of the north gallery: onto it, thrown north onto the
/// gallery's deck.
pub const CELL_PAD: Scenario = Scenario {
    name: "the cell's pad onto the gallery",
    arena: || Arena {
        spawn: Vec3i::new(-700, 0, -650),
        spawn_yaw: NORTH,
        ..Arena::proving_cell()
    },
    legs: &[
        stand("settle", 10),
        walk("onto the pad", NORTH, 20),
        stand("thrown", 90),
        stand("on the gallery", 20),
    ],
};

/// Pitch steps from the stage grapple's eye (-100, 365, -250) up to the north wall 7 m up behind
/// the gallery, (-100, 700, -1600): atan(335 / 1350) ≈ 13.9°.
const GALLERY_PITCH: i16 = 2_537;

/// The proving cell from its stage's north edge: hook the north wall over the gallery, get reeled
/// up and over the gallery's edge, let go and step onto its deck.
pub const CELL_GRAPPLE: Scenario = Scenario {
    name: "the cell's grapple onto the gallery",
    arena: || Arena {
        spawn: Vec3i::new(-100, STAGE_TOP, -250),
        spawn_yaw: NORTH,
        ..Arena::proving_cell()
    },
    legs: &[
        hold(
            "aim the grapple",
            Intent {
                yaw: NORTH,
                pitch: GALLERY_PITCH,
                ..Intent::IDLE
            },
            15,
        ),
        hold(
            "hook and reel in",
            Intent {
                yaw: NORTH,
                pitch: GALLERY_PITCH,
                grapple: true,
                ..Intent::IDLE
            },
            90,
        ),
        walk("let go onto the gallery", NORTH, 30),
        stand("on the gallery", 60),
    ],
};

/// The drydock from the foot of its west gantry's north ramp: south up the 16 m ramp onto the
/// 4 m gantry, along it to the north gangway, then west over the gangway and the hull's
/// shoulder onto the foredeck.
pub const DOCK_GANTRY: Scenario = Scenario {
    name: "the dock's gantry and gangway",
    arena: || Arena {
        spawn: Vec3i::new(GANTRY_X, 0, -3_200),
        spawn_yaw: SOUTH,
        ..Arena::drydock()
    },
    legs: &[
        stand("settle", 10),
        walk("up the ramp", SOUTH, 160),
        walk("along the gantry", SOUTH, 30),
        stand("on the gantry", 10),
        walk("over the gangway", WEST, 70),
        stand("on the foredeck", 20),
    ],
};

/// The drydock's west foredeck: south up the 8 m ramp onto the 6 m stern deck, and along it.
pub const DOCK_STERN: Scenario = Scenario {
    name: "the dock's stern ramp",
    arena: || Arena {
        spawn: Vec3i::new(SHIP_X, FOREDECK_TOP, 0),
        spawn_yaw: SOUTH,
        ..Arena::drydock()
    },
    legs: &[
        stand("settle", 10),
        walk("up the ramp", SOUTH, 110),
        stand("on the stern deck", 20),
    ],
};

/// The drydock's west outer lane: onto its jump pad, thrown east over the hull's shoulder onto
/// the foredeck.
pub const DOCK_PAD: Scenario = Scenario {
    name: "the dock's pad onto the foredeck",
    arena: || Arena {
        spawn: Vec3i::new(-3_650, 0, -600),
        spawn_yaw: EAST,
        ..Arena::drydock()
    },
    legs: &[
        stand("settle", 10),
        walk("onto the pad", EAST, 25),
        stand("thrown", 100),
        stand("on the foredeck", 20),
    ],
};

/// Pitch steps from the outer lane's eye (-3300, 165, -2200) up to the bow mast's top on the
/// foredeck, (-2200, 670, -2200): atan(505 / 1100) ≈ 24.7°, raised about 1.6° so the line from
/// the grapple module's muzzle (below the eye) still meets the mast's top.
const BOW_MAST_PITCH: i16 = 4_797;

/// The drydock's west outer lane: hook the bow mast over the hull, get reeled up over the
/// shoulder and flung toward the mast, let go and step back to land on the foredeck short of
/// the mast's foot.
pub const DOCK_GRAPPLE: Scenario = Scenario {
    name: "the dock's grapple onto the foredeck",
    arena: || Arena {
        spawn: Vec3i::new(-3_300, 0, -2_200),
        spawn_yaw: EAST,
        ..Arena::drydock()
    },
    legs: &[
        hold(
            "aim the grapple",
            Intent {
                yaw: EAST,
                pitch: BOW_MAST_PITCH,
                ..Intent::IDLE
            },
            15,
        ),
        hold(
            "hook and reel in",
            Intent {
                yaw: EAST,
                pitch: BOW_MAST_PITCH,
                grapple: true,
                ..Intent::IDLE
            },
            95,
        ),
        walk("let go onto the foredeck", WEST, 10),
        stand("on the foredeck", 90),
    ],
};

pub const ALL: &[Scenario] = &[
    SPRINT_AND_SLIDE,
    DOUBLE_JUMP,
    THRUSTER_JUMP,
    JUMP_PAD,
    RAMP_SLIDE,
    THIN_WALL,
    STEP,
    ROCKET_JUMP,
    SHOOTING_RANGE,
    GRAPPLE,
    CLOSE_RANGE,
    YARD_WALKWAY,
    YARD_JUMP_PAD,
    WORKS_RAMP_AND_BRIDGE,
    WORKS_COVER,
    WORKS_TOWER_PAD,
    WORKS_STEPS,
    WORKS_GRAPPLE,
    CELL_GALLERY,
    CELL_STAGE,
    CELL_PAD,
    CELL_GRAPPLE,
    DOCK_GANTRY,
    DOCK_STERN,
    DOCK_PAD,
    DOCK_GRAPPLE,
];

/// This tick's controls for scripted person `index`: run and strafe in turns, sweep the view
/// round, hop, slide, hold the trigger two thirds of the time, swap weapons every 5 s and swing
/// the blade now and then, its legs on the named loadouts' preset layout
/// ([`crate::bindings::Bindings::preset`]; every named loadout has the same legs). A pure
/// function of the tick, so a replay needs nothing else. The match replays (`shooter-replay`)
/// and the native client's offscreen profile run play it.
#[must_use]
pub fn scripted_intent(index: u8, tick: u32) -> Intent {
    let person = u32::from(index);
    let segment = (tick + 45 * person) / 90;
    let weapons = Weapon::ROTATION;
    let forward = if segment % 4 == 3 { -1 } else { 1 };
    let strafe = if segment.is_multiple_of(2) { 1 } else { -1 };
    #[allow(clippy::cast_possible_truncation)]
    Intent {
        forward,
        strafe,
        yaw: (tick.wrapping_mul(40 + 13 * person) + person * 16_384) as u16,
        jump: (tick + 30 * person) % 120 < 2,
        sprint: segment.is_multiple_of(3),
        crouch: segment % 5 == 4 && tick % 90 < 20,
        pitch: 0,
        fire: (tick + 20 * person) % 60 < 40,
        melee: (tick + 50 * person).is_multiple_of(200),
        // The grapple module's own key, a second held every four (shooter#104).
        grapple: (tick + 70 * person) % 240 < 60,
        // A weapon every 5 s and, halfway between, the copy on part 9 (the twin rail's second
        // rail gun, the rocket pod's third launcher; refused where it is no gun, shooter#125).
        select: if tick.is_multiple_of(300) {
            Some(Select::Weapon(
                weapons[(tick / 300 + person) as usize % weapons.len()],
            ))
        } else if tick % 300 == 150 {
            Some(Select::Part(crate::robot::PartId(9)))
        } else {
            None
        },
        edit: false,
        repair: None,
        // A weapon group: the parts 7 to 12 asked to fire together for 0.75 s every 4 s
        // (shooter#127; the twin rail's two rail guns at once, the rocket pod staggered by its
        // salvo limit, refused where a part is no gun).
        triggers: if (tick + 35 * person) % 240 < 45 {
            (7..=12).map(crate::robot::PartId).collect()
        } else {
            crate::weapons::Triggers::NONE
        },
        motors: {
            let layout = crate::mechanics::Layout::of(&crate::robot::STANDARD);
            crate::bindings::Bindings::preset(&layout).resolve(
                &layout,
                &mut crate::bindings::BindingState::default(),
                crate::bindings::Pressed::wasd(forward, strafe),
            )
        },
    }
}

/// The robot build scripted person `index` asks for when it joins: the first person comes back
/// as the [`HEAVY`] build after its first death, the second as the modded [`SKIRMISHER`]
/// (rapid-cycler rifle, efficiency-coiled shotgun), the third as the [`TWIN_RAIL`] and the fourth
/// as the [`ROCKET_POD`] (one weapon mounted several times, shooter#125) and the fifth as the
/// [`RAIL_RACK`] (six weapons on two mount rails, shooter#126); the others keep the
/// standard build.
#[must_use]
pub const fn scripted_build(index: u8) -> Option<Assembly> {
    match index {
        0 => Some(HEAVY),
        1 => Some(SKIRMISHER),
        2 => Some(TWIN_RAIL),
        3 => Some(ROCKET_POD),
        4 => Some(RAIL_RACK),
        _ => None,
    }
}

/// A staged shot at a map's arena cover (shooter#102's visual captures, the profile run's
/// `cover` scene): the standard build standing at `feet` (units) behind a piece of the arena's
/// static cover, aiming at `yaw` and `pitch` past it, so the camera over its right shoulder
/// sees a target beyond while its rifle's muzzle, low on its left, is behind the cover: the
/// rifle's shots stop on the cover, short of the reticle ([`crate::aim::Lands::Blocked`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CoverShot {
    pub map: crate::maps::MapId,
    pub feet: [f32; 3],
    pub yaw: u16,
    pub pitch: i16,
}

/// The staged cover shots, one per map that has one: the assembly works' and the proving
/// cell's (found by searching each map's floor for a spot where the rifle's shot is blocked
/// about 3 m ahead by the arena's static cover, where the camera sees it, while the camera's
/// target lies 17 m or more beyond).
pub const COVER_SHOTS: [CoverShot; 2] = [
    CoverShot {
        map: crate::maps::MapId::AssemblyWorks,
        feet: [-1_900.0, 1.0, -1_500.0],
        yaw: 18_432,
        pitch: 0,
    },
    CoverShot {
        map: crate::maps::MapId::ProvingCell,
        feet: [-300.0, 1.0, -1_300.0],
        yaw: 18_432,
        pitch: 0,
    },
];

/// When the staged fighter starts firing (ticks): the captures at 60 and 90 ticks show beams.
pub const COVER_FIRE_TICK: u32 = 45;

impl CoverShot {
    /// The staged shot on `map`, if it has one.
    #[must_use]
    pub fn of(map: crate::maps::MapId) -> Option<Self> {
        COVER_SHOTS.into_iter().find(|shot| shot.map == map)
    }

    /// Its intent at `tick`: holding the rifle, aimed past the cover, standing still, firing
    /// from [`COVER_FIRE_TICK`] on.
    #[must_use]
    pub fn intent(self, tick: u32) -> Intent {
        Intent {
            yaw: self.yaw,
            pitch: self.pitch,
            select: Some(Select::Weapon(Weapon::EnergyRifle)),
            fire: tick >= COVER_FIRE_TICK,
            ..Intent::IDLE
        }
    }
}
