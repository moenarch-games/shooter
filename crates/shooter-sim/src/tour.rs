//! A scripted walk through the greybox arena: up the south ramp onto the platform, a jump there,
//! down the west ramp, into the west wall, and a jump onto a crate. The movement tests and the
//! Rapier A/B scenario both run it.

use crate::player::{Intent, Session};
use sim_foundation::physics::PhysicsError;
use sim_foundation::trig::YAW_QUARTER_TURN;

const NORTH: u16 = 2 * YAW_QUARTER_TURN; // -Z
const WEST: u16 = 3 * YAW_QUARTER_TURN; // -X
const SOUTH: u16 = 0; // +Z
const EAST: u16 = YAW_QUARTER_TURN; // +X

/// One leg of the tour: hold `intent` for `ticks`.
#[derive(Clone, Copy, Debug)]
pub struct Leg {
    pub name: &'static str,
    pub intent: Intent,
    pub ticks: u32,
}

const fn walk(name: &'static str, yaw: u16, ticks: u32) -> Leg {
    Leg {
        name,
        intent: Intent {
            forward: 1,
            strafe: 0,
            yaw,
            jump: false,
        },
        ticks,
    }
}

const fn stand(name: &'static str, ticks: u32) -> Leg {
    Leg {
        name,
        intent: Intent {
            forward: 0,
            strafe: 0,
            yaw: NORTH,
            jump: false,
        },
        ticks,
    }
}

const fn jump(name: &'static str, yaw: u16, forward: i8) -> Leg {
    Leg {
        name,
        intent: Intent {
            forward,
            strafe: 0,
            yaw,
            jump: true,
        },
        ticks: 1,
    }
}

/// The legs, in order, from the arena's spawn at (0, 0, 16 m) facing north.
pub const LEGS: &[Leg] = &[
    stand("settle", 10),
    // 16 m north at 12 units a tick: up the south ramp (z 12 m to 4 m) onto the platform.
    walk("up the south ramp", NORTH, 130),
    stand("on the platform", 10),
    jump("jump on the platform", NORTH, 0),
    stand("airborne and landing", 50),
    // West down the west ramp (x -4 m to -12 m), then into the west wall's face at x = -20 m.
    walk("down the west ramp", WEST, 110),
    walk("into the west wall", WEST, 60),
    // Back east to the crate's line (x = -12 m) and south against its north face (z = 11.6 m).
    walk("back east", EAST, 64),
    walk("against the crate", SOUTH, 100),
    // A running jump onto it, then stop on top.
    jump("jump at the crate", SOUTH, 1),
    walk("over the edge", SOUTH, 14),
    stand("on the crate", 40),
];

/// One sample of the tour: the tick, the player's exact centre and whether it stood on ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub tick: u32,
    pub leg: &'static str,
    pub center: [f32; 3],
    pub grounded: bool,
}

/// Runs every leg on `session`, sampling after each tick.
pub fn run(session: &mut Session) -> Result<Vec<Sample>, PhysicsError> {
    let mut samples = Vec::new();
    let mut tick = 0;
    for leg in LEGS {
        for _ in 0..leg.ticks {
            let report = session.tick(leg.intent)?;
            tick += 1;
            samples.push(Sample {
                tick,
                leg: leg.name,
                center: session.player.center(&session.world),
                grounded: report.grounded,
            });
        }
    }
    Ok(samples)
}

/// The last sample of the leg named `name`.
#[must_use]
pub fn end_of<'a>(samples: &'a [Sample], name: &str) -> Option<&'a Sample> {
    samples.iter().rev().find(|sample| sample.leg == name)
}
