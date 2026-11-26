//! The mechanics fixtures (shooter#73): valid beta builds and the fixed controls that drive
//! them, shared by the tests, the Rapier A/B and the performance scene. A control layout
//! ([`crate::bindings`]) sends commands like these.
//!
//! Every wheeled fixture is the large chassis (40 × 60 × 80 cm, standing on end) with four
//! wheels at its bottom corners, each turned on its socket so it hangs below the chassis. A
//! wheel turns about its own mount normal, so the left wheels' axles point left and the right
//! ones' right: rolling forward turns the left wheels forward and the right ones backward.

use super::{Command, Controls};
use crate::physics::{BodyId, RigidBody, Vec3i};
use crate::robot::parts::{
    BATTERY, BEAM_LONG, CHASSIS_L, CHASSIS_S, ENERGY_RIFLE, HINGE, LEG, ROTOR, SENSOR, TRACK, WHEEL,
};
use crate::robot::{
    Assembly, BETA, Candidate, Face, MINI_TANK, Orientation, PartId, PartType, ROBO_DOG, ROVER,
    RawMount, RawPart,
};

const QUARTER: u16 = Orientation::QUARTER;

/// The wheels' part IDs: left rear, left front, right rear, right front (on the sides), or
/// front left, front right, rear left, rear right (on the front and back).
pub const WHEELS: [PartId; 4] = [PartId(2), PartId(3), PartId(4), PartId(5)];
/// The turret fixtures' rotor, hinge and barrel.
pub const ROTOR_PART: PartId = PartId(6);
pub const HINGE_PART: PartId = PartId(7);
pub const BARREL: PartId = PartId(8);

fn root(id: u8, kind: &PartType) -> RawPart {
    RawPart {
        id,
        kind: kind.id.0,
        mount: None,
        modification: None,
    }
}

fn on(id: u8, kind: &PartType, parent: u8, on: &PartType, face: Face, u: u8, v: u8) -> RawPart {
    RawPart {
        id,
        kind: kind.id.0,
        mount: Some(RawMount {
            parent,
            socket: on
                .socket(face, u, v)
                .expect("a fixture names a real socket")
                .0,
            ..RawMount::default()
        }),
        modification: None,
    }
}

fn yawed(mut part: RawPart, yaw: u16) -> RawPart {
    if let Some(mount) = part.mount.as_mut() {
        mount.yaw = yaw;
    }
    part
}

fn valid(parts: Vec<RawPart>) -> Assembly {
    BETA.validate(&Candidate { parts })
        .expect("a fixture is a valid build")
}

fn wheeled(sideways: bool) -> Vec<RawPart> {
    if !sideways {
        let mut parts = ROVER.candidate().parts;
        parts.retain(|part| part.id < 6);
        return parts;
    }
    // A sensor on the roof where the retired control core sat (shooter#89).
    let mut parts = vec![
        root(0, &CHASSIS_L),
        on(1, &SENSOR, 0, &CHASSIS_L, Face::PosZ, 0, 0),
    ];
    let (left, right) = (Face::PosX, Face::NegX);
    // Each pair: one wheel turned half a turn from the face's first cell, one turned a quarter
    // (either way) from its last, so both hang over the face's corners.
    let (second_left, second_right, quarter_left, quarter_right) =
        ((2, 0), (0, 2), 3 * QUARTER, QUARTER);
    parts.extend([
        yawed(on(2, &WHEEL, 0, &CHASSIS_L, left, 0, 0), 2 * QUARTER),
        yawed(
            on(3, &WHEEL, 0, &CHASSIS_L, left, second_left.0, second_left.1),
            quarter_left,
        ),
        yawed(on(4, &WHEEL, 0, &CHASSIS_L, right, 0, 0), 2 * QUARTER),
        yawed(
            on(
                5,
                &WHEEL,
                0,
                &CHASSIS_L,
                right,
                second_right.0,
                second_right.1,
            ),
            quarter_right,
        ),
    ]);
    parts
}

/// Four wheels on the chassis' sides: it rolls forward and back.
#[must_use]
pub fn car() -> Assembly {
    valid(wheeled(false))
}

/// The car armed for a match (shooter#58): the energy rifle on its front, and a fuel tank with a
/// generator on it on its roof beside the sensor, so it recharges as the named loadouts do.
#[must_use]
pub fn armed_car() -> Assembly {
    ROVER
}

/// The same four wheels on the chassis' front and back: the same commands drive it sideways.
#[must_use]
pub fn strafer() -> Assembly {
    valid(wheeled(true))
}

/// The car with a rotor on its roof carrying an upright barrel: a one-axis turret.
#[must_use]
pub fn turret() -> Assembly {
    let mut parts = wheeled(false);
    parts.extend([
        on(6, &ROTOR, 0, &CHASSIS_L, Face::PosZ, 1, 1),
        on(8, &ENERGY_RIFLE, 6, &ROTOR, Face::PosZ, 0, 0),
    ]);
    valid(parts)
}

/// The car with a rotor on its roof, a hinge on the rotor turned so it pitches about the
/// robot's sideways axis, and the barrel on the hinge: paired turret axes.
#[must_use]
pub fn paired_turret() -> Assembly {
    let mut parts = wheeled(false);
    parts.extend([
        on(6, &ROTOR, 0, &CHASSIS_L, Face::PosZ, 1, 1),
        yawed(on(7, &HINGE, 6, &ROTOR, Face::PosZ, 0, 0), QUARTER),
        on(8, &ENERGY_RIFLE, 7, &HINGE, Face::PosZ, 0, 0),
    ]);
    valid(parts)
}

/// A small chassis standing on end with a long beam out of its left side near the top and a
/// battery on the beam's tip: its centre of mass is past its footprint, so it tips left.
#[must_use]
pub fn lopsided() -> Assembly {
    valid(vec![
        root(0, &CHASSIS_S),
        on(2, &BEAM_LONG, 0, &CHASSIS_S, Face::PosY, 3, 0),
        on(3, &BATTERY, 2, &BEAM_LONG, Face::PosZ, 0, 0),
    ])
}

/// [`lopsided`] with the same beam and battery on its right side too: it stands.
#[must_use]
pub fn balanced() -> Assembly {
    valid(vec![
        root(0, &CHASSIS_S),
        on(2, &BEAM_LONG, 0, &CHASSIS_S, Face::PosY, 3, 0),
        on(3, &BATTERY, 2, &BEAM_LONG, Face::PosZ, 0, 0),
        on(4, &BEAM_LONG, 0, &CHASSIS_S, Face::NegY, 0, 3),
        on(5, &BATTERY, 4, &BEAM_LONG, Face::PosZ, 0, 0),
    ])
}

/// The large chassis on two legs under its rear half, a sensor on its roof (where the retired
/// control core sat, shooter#89).
#[must_use]
pub fn biped() -> Assembly {
    valid(vec![
        root(0, &CHASSIS_L),
        on(1, &SENSOR, 0, &CHASSIS_L, Face::PosZ, 0, 0),
        on(2, &LEG, 0, &CHASSIS_L, Face::NegZ, 0, 0),
        on(3, &LEG, 0, &CHASSIS_L, Face::NegZ, 2, 0),
    ])
}

/// A robo-dog: the large chassis with two long beams out of its front, hind legs under the
/// chassis and fore legs under the beams' tips. `turned` mounts every leg a quarter turn
/// round, so its hip swings it sideways: badly configured.
#[must_use]
pub fn dog(turned: bool) -> Assembly {
    let mut parts = ROBO_DOG.candidate().parts;
    parts.retain(|part| part.id < 8);
    if turned {
        for part in &mut parts {
            if part.kind == LEG.id.0
                && let Some(mount) = part.mount.as_mut()
            {
                mount.yaw = QUARTER;
            }
        }
    }
    valid(parts)
}

/// Two legs on the large chassis' roof, pointing up: they step in the air.
#[must_use]
pub fn upside_down_legs() -> Assembly {
    valid(vec![
        root(0, &CHASSIS_L),
        on(2, &LEG, 0, &CHASSIS_L, Face::PosZ, 0, 0),
        on(3, &LEG, 0, &CHASSIS_L, Face::PosZ, 1, 2),
    ])
}

/// Every wheel of a wheeled fixture told `left` (the first two) and `right` (the last two),
/// in milliradians a second about its own axle.
#[must_use]
pub fn wheels(left: i16, right: i16) -> Controls {
    Controls::IDLE
        .with(WHEELS[0], Command::Rate(left))
        .with(WHEELS[1], Command::Rate(left))
        .with(WHEELS[2], Command::Rate(right))
        .with(WHEELS[3], Command::Rate(right))
}

/// A binary angle (65 536 a turn) of `radians`, rounded.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn binary(radians: f64) -> i16 {
    (radians * 32_768.0 / std::f64::consts::PI).round() as i16
}

/// A flat floor at height 0 (body 1).
#[must_use]
pub fn floor() -> Vec<RigidBody> {
    vec![RigidBody::floor(BodyId(1), 0)]
}

/// The floor and a wall across +X whose near face is `distance` units ahead of the origin
/// (body 2): what the stall fixture drives into.
#[must_use]
pub fn walled(distance: i32) -> Vec<RigidBody> {
    let mut statics = floor();
    statics.push(RigidBody::fixed(
        BodyId(2),
        Vec3i::new(distance + 20, 100, 0),
        Vec3i::new(20, 100, 200),
    ));
    statics
}

/// A tracked robot: the large chassis on two tracks along its sides (the left one turned a
/// quarter so it lies along the side, as the robot tests' mini-tank has them) and a sensor on
/// its roof.
#[must_use]
pub fn tank() -> Assembly {
    valid(vec![
        root(0, &CHASSIS_L),
        on(1, &SENSOR, 0, &CHASSIS_L, Face::PosZ, 0, 0),
        yawed(on(2, &TRACK, 0, &CHASSIS_L, Face::PosY, 3, 0), QUARTER),
        on(3, &TRACK, 0, &CHASSIS_L, Face::NegY, 0, 0),
    ])
}

/// A tank whose tracks lie along its sides (shooter#58): the large chassis with a track flat on
/// each side, both reaching from its back to past its front, each road wheel on the ground, and a
/// sensor on its roof. ([`tank`] stands its tracks on end.)
#[must_use]
pub fn flat_tank() -> Assembly {
    let mut parts = MINI_TANK.candidate().parts;
    parts.retain(|part| part.id < 4);
    valid(parts)
}

/// The floor and a ramp rising toward +X from `start` units ahead of the origin, `length`
/// long and `height` high (body 2): a slope to drive up.
#[must_use]
pub fn slope(start: i32, length: i32, height: i32) -> Vec<RigidBody> {
    let mut statics = floor();
    statics.push(RigidBody::ramp(
        BodyId(2),
        Vec3i::new(start + length / 2, height / 2, 0),
        Vec3i::new(length / 2, height / 2, 400),
        crate::physics::RampRise::PosX,
    ));
    statics
}
