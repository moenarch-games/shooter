//! Scripted fast moves, each in a small arena of its own (or the greybox): sprint and slide,
//! double jump onto a ledge, a jump pad onto the platform, slides up and down a ramp, a slide
//! into a thin wall, a sprint up a step, a rocket jump, a grapple onto a ledge and a shooting
//! range. The movement and weapon tests check what each one does and the Rapier A/B test runs
//! each on both backends.

use crate::arena::{Arena, Piece, Post};
use crate::player::Intent;
use crate::tour::{Leg, NORTH, SOUTH, hold, stand, walk};
use crate::weapons::{MAX_PITCH, Weapon};
use sim_foundation::RampRise;
use sim_foundation::physics::Vec3i;

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

/// A 2.5 m ledge 3.3 m south: out of reach of a double jump (about 2 m), in reach of a rocket
/// jump. Jump, fire a rocket at the feet, then steer onto the ledge.
pub const ROCKET_JUMP: Scenario = Scenario {
    name: "rocket jump onto a ledge",
    arena: || open([0, 0, -330], vec![block([0, 125, 600], [300, 125, 600])]),
    legs: &[
        hold(
            "take the launcher",
            Intent {
                yaw: SOUTH,
                select: Some(Weapon::RocketLauncher),
                ..Intent::IDLE
            },
            20,
        ),
        hold(
            "jump and fire down",
            Intent {
                yaw: SOUTH,
                pitch: DOWN,
                jump: true,
                fire: true,
                ..Intent::IDLE
            },
            1,
        ),
        hold(
            "blast",
            Intent {
                yaw: SOUTH,
                pitch: DOWN,
                ..Intent::IDLE
            },
            8,
        ),
        walk("steer onto the ledge", SOUTH, 50),
        stand("on the ledge", 60),
    ],
};

/// Pitch steps from the grapple scenario's eye (0, 165, -600) up to the pillar's face at
/// (0, 500, 650): atan(335 / 1250) ≈ 15.0°.
const PILLAR_PITCH: i16 = 2_731;

/// A 3 m ledge 9 m south (beyond a double jump and a rocket jump) with an 8 m pillar on it.
/// Take the grapple, hook the pillar's face 5 m up over the ledge's edge, get reeled up and over
/// the edge, let go and step onto the ledge.
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
            "take the grapple",
            Intent {
                yaw: SOUTH,
                pitch: PILLAR_PITCH,
                select: Some(Weapon::Grapple),
                ..Intent::IDLE
            },
            15,
        ),
        hold(
            "hook and reel in",
            Intent {
                yaw: SOUTH,
                pitch: PILLAR_PITCH,
                fire: true,
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
    match sim_foundation::trig::yaw_from_vector(x, z + 600) {
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
            select: Some(weapon),
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
            select: Some(weapon),
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
        stand("settle", 10),
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

pub const ALL: &[Scenario] = &[
    SPRINT_AND_SLIDE,
    DOUBLE_JUMP,
    JUMP_PAD,
    RAMP_SLIDE,
    THIN_WALL,
    STEP,
    ROCKET_JUMP,
    SHOOTING_RANGE,
    GRAPPLE,
];

/// This tick's controls for scripted person `index`: run and strafe in turns, sweep the view
/// round, hop, slide, hold the trigger two thirds of the time, swap weapons every 5 s and swing
/// the blade now and then. A pure function of the tick, so a replay needs nothing else. The
/// match replays (`shooter-replay`) and the native client's offscreen profile run play it.
#[must_use]
pub fn scripted_intent(index: u8, tick: u32) -> Intent {
    let person = u32::from(index);
    let segment = (tick + 45 * person) / 90;
    let weapons = [
        Weapon::EnergyRifle,
        Weapon::RailGun,
        Weapon::RocketLauncher,
        Weapon::Grapple,
    ];
    #[allow(clippy::cast_possible_truncation)]
    Intent {
        forward: if segment % 4 == 3 { -1 } else { 1 },
        strafe: if segment.is_multiple_of(2) { 1 } else { -1 },
        yaw: (tick.wrapping_mul(40 + 13 * person) + person * 16_384) as u16,
        jump: (tick + 30 * person) % 120 < 2,
        sprint: segment.is_multiple_of(3),
        crouch: segment % 5 == 4 && tick % 90 < 20,
        pitch: 0,
        fire: (tick + 20 * person) % 60 < 40,
        melee: (tick + 50 * person).is_multiple_of(200),
        select: tick
            .is_multiple_of(300)
            .then(|| weapons[(tick / 300 + person) as usize % weapons.len()]),
    }
}
