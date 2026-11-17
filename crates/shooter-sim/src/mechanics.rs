//! Robots that move mechanically (Atlas D2.i70.5): a build's component tree becomes compound
//! rigid bodies on motorised revolute joints in the articulated world
//! ([`crate::physics::articulated`], physics-engine), driven by per-actuator commands that draw
//! from the robot's shared energy pool ([`crate::energy`]).
//!
//! **Placement.** Every part is a cuboid of 20 cm cells placed in its robot's frame (+Y up, +X
//! forward, +Z the robot's right) from the tree alone ([`placements`]): the root chassis is
//! centred on the origin with its own +Z up and +X forward; a child's base cell `(0, 0)` sits
//! centred on its parent's socket cell, its own X and Y along the face's `u` and `v`, its +Z out
//! of the face, then turned on that socket cell by its exact orientation (yaw about the face
//! normal, pitch, roll: [`crate::robot::Orientation`]).
//!
//! **Bodies** ([`Layout`]). Rigidly joined parts form one compound body: each part a box child
//! with its own mass (a weapon's mod included) where it is mounted, so the centre of mass,
//! inertia and stability are the real ones. A joint part ([`Joint`]) belongs to its parent's
//! body and starts a new body for what is mounted on its top face, joined at the centre of that
//! face: a hinge about its own X axis within a quarter turn either way, a rotor about its own
//! Z axis (its mount normal) without limits. Locomotion parts are their own bodies joined to
//! their parent's body: a wheel is a sphere spinning about its mount normal; a track is two
//! road wheels along its length spinning about its mount normal; a leg ([`Locomotion::Leg`]) is
//! one self-contained module, a thigh on a hip and a shin with a flat foot on a knee, both
//! about the leg's own X axis. Wheel spheres stand in for discs: physics-engine has no
//! cylinders.
//!
//! **Orientation is authoritative.** Nothing about a part's direction is corrected: a wheel
//! turns about its own mount normal, so wheels on a robot's two sides turn opposite ways for the
//! same command (a binding inverts one side, shooter#74), a wheel mounted on the front drives
//! the robot sideways, a mismatched drive spins it and lopsided mass tips it.
//!
//! **Commands** ([`Controls`]): one [`Command`] an actuator (joint or locomotion part) a tick,
//! in integers so they travel and replay exactly: a rate (a velocity motor, at most the part's
//! torque and top rate), an angle (a servo, likewise), a leg's stride, or idle (the motor is
//! off and the joint turns freely, within its limits). Legs step through a gait: the preset is
//! suggested from the attached legs ([`Layout::suggested_gait`]: two legs walk as a biped, four
//! as a quadruped) and can be overridden ([`GaitChoice`]); a stride commands each leg's hip and
//! knee servos along the gait's cycle, so legs move the robot only through their contacts, can
//! push it over and never place it.
//!
//! **Energy** (D2.i70.3): every commanded actuator asks the pool for its part's draw each tick
//! (a flat cost a tick while commanded: simplest, provisional), actuators in part-ID order;
//! one that is refused is off that tick. Regeneration follows the requests.
//!
//! **Stalls.** After the step each actuator reports its joint's angle, rate and motor torque;
//! it stalls when its motor gives its most torque yet the rate stays far from the command (or
//! the servo far from its angle).
//!
//! **Exactness.** The articulated world runs without warm starting or sleep, so [`State`] (the
//! builds, the pools, the gait clocks and every body's exact [`Motion`]) restored with
//! [`Mechanics::restore`] steps on bit for bit like the original under the same controls: the
//! host, a client's rollback prediction and a replay agree.
//!
//! The legs, wheels and tracks drive no fighter yet: the shipped fighters keep their capsule
//! controller (shooter#73's first half; the transition is its follow-up).

pub mod fixtures;

use std::f32::consts::{FRAC_PI_2, PI};

use crate::TICK_HZ;
use crate::energy::{MILLI, Pool, spend};
use crate::physics::articulated::{
    Articulated, BodySpec, Child, Drive as MotorDrive, Hinge, HingeId, HingeState, Motion, Pose,
    Shape,
};
use crate::physics::{BodyId, PhysicsError, RigidBody};
use crate::robot::{
    Assembly, BETA, CELL_MM, Drive, Face, Joint, Kind, Locomotion, MAX_PARTS, Orientation, Part,
    PartId, PartType, SocketId,
};
use crate::trig::{TRIG_ONE, cos, sin};

/// A cell's edge in units (centimetres).
#[allow(clippy::cast_precision_loss)]
const CELL: f32 = CELL_MM as f32 / 10.0;
/// The first robot's first body; each robot has [`BODIES_PER_ROBOT`] ids.
pub const FIRST_ROBOT_BODY: u64 = 10_000;
/// Body ids a robot reserves: every part could be two bodies (a leg, a track), plus its root.
pub const BODIES_PER_ROBOT: u64 = 2 * MAX_PARTS as u64 + 1;
/// Friction of wheels, tracks and feet; other bodies keep the default.
const GRIP: f32 = 1.0;
const FRICTION: f32 = 0.5;
/// A hinge part turns within a quarter turn either way.
const HINGE_LIMITS: [f32; 2] = [-FRAC_PI_2, FRAC_PI_2];
/// A leg's hip and knee limits (radians, about its own X axis; positive swings the foot
/// toward the leg's −Y).
const HIP_LIMITS: [f32; 2] = [-1.2, 1.2];
const KNEE_LIMITS: [f32; 2] = [0.0, 2.2];
/// A leg's radius, and its foot's half extents along its X and Y and half thickness.
const LEG_RADIUS: f32 = 8.0;
const FOOT: [f32; 3] = [9.0, 14.0, 3.0];
/// A wheel's sphere is this much (units) inside its cell footprint, so neighbouring wheels do
/// not rub.
const WHEEL_INSET: f32 = 2.0;
/// Spawned robots start this far (units) above the ground.
const SPAWN_GAP: f32 = 0.5;

// ---------------------------------------------------------------------------------------------
// Geometry: exact rotations from binary angles, in `f32`.

type V3 = [f32; 3];

/// A rotation: the images of X, Y and Z.
#[derive(Clone, Copy, Debug, PartialEq)]
struct M3([V3; 3]);

const fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

const fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

const fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

impl M3 {
    fn apply(self, v: V3) -> V3 {
        add(
            add(scale(self.0[0], v[0]), scale(self.0[1], v[1])),
            scale(self.0[2], v[2]),
        )
    }

    fn then(self, inner: Self) -> Self {
        Self(inner.0.map(|column| self.apply(column)))
    }

    /// A unit quaternion `[x, y, z, w]` of the rotation.
    fn quaternion(self) -> [f32; 4] {
        let [[m00, m10, m20], [m01, m11, m21], [m02, m12, m22]] = self.0;
        let trace = m00 + m11 + m22;
        let q = if trace > 0.0 {
            let s = (trace + 1.0).sqrt() * 2.0;
            [(m21 - m12) / s, (m02 - m20) / s, (m10 - m01) / s, 0.25 * s]
        } else if m00 > m11 && m00 > m22 {
            let s = (1.0 + m00 - m11 - m22).sqrt() * 2.0;
            [0.25 * s, (m01 + m10) / s, (m02 + m20) / s, (m21 - m12) / s]
        } else if m11 > m22 {
            let s = (1.0 + m11 - m00 - m22).sqrt() * 2.0;
            [(m01 + m10) / s, 0.25 * s, (m12 + m21) / s, (m02 - m20) / s]
        } else {
            let s = (1.0 + m22 - m00 - m11).sqrt() * 2.0;
            [(m02 + m20) / s, (m12 + m21) / s, 0.25 * s, (m10 - m01) / s]
        };
        let length = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
        q.map(|value| value / length)
    }
}

/// `sin` and `cos` of a binary angle (65 536 a turn), exact from the integer table.
#[allow(clippy::cast_precision_loss)]
fn sin_cos(angle: u16) -> (f32, f32) {
    let one = TRIG_ONE as f32;
    (sin(angle) as f32 / one, cos(angle) as f32 / one)
}

fn about_x(angle: u16) -> M3 {
    let (s, c) = sin_cos(angle);
    M3([[1.0, 0.0, 0.0], [0.0, c, s], [0.0, -s, c]])
}

fn about_y(angle: u16) -> M3 {
    let (s, c) = sin_cos(angle);
    M3([[c, 0.0, -s], [0.0, 1.0, 0.0], [s, 0.0, c]])
}

fn about_z(angle: u16) -> M3 {
    let (s, c) = sin_cos(angle);
    M3([[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]])
}

/// A part's frame: its cuboid's minimum corner and axes, in the robot's frame.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Frame {
    rotation: M3,
    origin: V3,
}

impl Frame {
    fn point(self, local: V3) -> V3 {
        add(self.origin, self.rotation.apply(local))
    }

    fn direction(self, local: V3) -> V3 {
        self.rotation.apply(local)
    }

    fn then(self, inner: Self) -> Self {
        Self {
            rotation: self.rotation.then(inner.rotation),
            origin: self.point(inner.origin),
        }
    }
}

/// A cuboid's size in units.
#[allow(clippy::cast_precision_loss)]
fn size(cells: [u8; 3]) -> V3 {
    cells.map(|count| f32::from(count) * CELL)
}

/// The socket cell `(u, v)` of `face` on a cuboid of `cells`: its centre on the face and the
/// face's `u`, `v` and outward normal as part axes ([`Face::grid`]).
#[allow(clippy::cast_precision_loss)]
fn socket_frame(face: Face, u: u8, v: u8, cells: [u8; 3]) -> Frame {
    let [x, y, z] = size(cells);
    let (cu, cv) = ((f32::from(u) + 0.5) * CELL, (f32::from(v) + 0.5) * CELL);
    let (columns, origin) = match face {
        Face::PosX => ([[0., 1., 0.], [0., 0., 1.], [1., 0., 0.]], [x, cu, cv]),
        Face::NegX => ([[0., 0., 1.], [0., 1., 0.], [-1., 0., 0.]], [0.0, cv, cu]),
        Face::PosY => ([[0., 0., 1.], [1., 0., 0.], [0., 1., 0.]], [cv, y, cu]),
        Face::NegY => ([[1., 0., 0.], [0., 0., 1.], [0., -1., 0.]], [cu, 0.0, cv]),
        Face::PosZ => ([[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]], [cu, cv, z]),
        Face::NegZ => ([[0., 1., 0.], [1., 0., 0.], [0., 0., -1.]], [cv, cu, 0.0]),
    };
    Frame {
        rotation: M3(columns),
        origin,
    }
}

/// How `orientation` turns a part on its socket cell: yaw about the normal, then pitch, then
/// roll (intrinsic Z-Y-X).
fn turn(orientation: Orientation) -> M3 {
    let (yaw, pitch, roll) = orientation.raw();
    about_z(yaw).then(about_y(pitch)).then(about_x(roll))
}

fn part_type(part: &Part) -> &'static PartType {
    BETA.part_type(part.kind)
        .expect("validated parts are in the catalog")
}

/// A part's cuboid in cells: a heavy mod lengthens a weapon to at least two cells, as its
/// footprint.
fn cells_of(part: &Part) -> [u8; 3] {
    let mut cells = part_type(part).cells;
    if part
        .modification
        .and_then(|id| BETA.modification(id))
        .is_some_and(|modification| modification.heavy)
    {
        cells[1] = cells[1].max(2);
    }
    cells
}

/// A part's mass (kg) with its mod's.
fn mass_of(part: &Part) -> f32 {
    let modification = part
        .modification
        .and_then(|id| BETA.modification(id))
        .map_or(0, |modification| modification.mass);
    f32::from(part_type(part).mass + modification)
}

/// Each part's frame, in part-ID order.
fn frames(build: &Assembly) -> Vec<(PartId, Frame)> {
    let parts = build.parts();
    let mut frames: Vec<Option<Frame>> = vec![None; parts.len()];
    let index = |id: PartId| parts.iter().position(|part| part.id == id);
    fn resolve(
        at: usize,
        parts: &[Part],
        frames: &mut [Option<Frame>],
        index: &dyn Fn(PartId) -> Option<usize>,
    ) -> Frame {
        if let Some(frame) = frames[at] {
            return frame;
        }
        let part = &parts[at];
        let frame = match part.mount {
            None => {
                // The root's +Z is up and its +Y the robot's left; it is centred on the origin.
                let rotation = M3([[1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]]);
                let centre = scale(size(cells_of(part)), 0.5);
                Frame {
                    rotation,
                    origin: scale(rotation.apply(centre), -1.0),
                }
            }
            Some(mount) => {
                let parent_at = index(mount.parent).expect("validated parents exist");
                let parent = resolve(parent_at, parts, frames, index);
                let parent_cells = cells_of(&parts[parent_at]);
                let site = part_type(&parts[parent_at])
                    .site(mount.socket)
                    .expect("validated sockets exist");
                let socket = socket_frame(site.face, site.u, site.v, parent_cells);
                let rotation = socket.rotation.then(turn(mount.orientation));
                let pivot = [CELL / 2.0, CELL / 2.0, 0.0];
                parent.then(Frame {
                    rotation,
                    origin: sub(socket.origin, rotation.apply(pivot)),
                })
            }
        };
        frames[at] = Some(frame);
        frame
    }
    (0..parts.len())
        .map(|at| (parts[at].id, resolve(at, parts, &mut frames, &index)))
        .collect()
}

/// Where a part is in its robot's frame: its cuboid's centre, orientation and half extents
/// (units).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub part: PartId,
    pub centre: [f32; 3],
    pub orientation: [f32; 4],
    pub half_extents: [f32; 3],
}

/// Every part of `build` placed in its robot's frame, in part-ID order.
#[must_use]
pub fn placements(build: &Assembly) -> Vec<Placement> {
    frames(build)
        .into_iter()
        .zip(build.parts())
        .map(|((id, frame), part)| {
            let half = scale(size(cells_of(part)), 0.5);
            Placement {
                part: id,
                centre: frame.point(half),
                orientation: frame.rotation.quaternion(),
                half_extents: half,
            }
        })
        .collect()
}

/// Where a socket is in its robot's frame (units): the centre of its cell on its part's face and
/// the face's outward normal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SocketSpot {
    pub part: PartId,
    pub socket: SocketId,
    pub centre: [f32; 3],
    pub normal: [f32; 3],
}

/// Every socket of every part of `build` in its robot's frame (part-ID order, then socket
/// order), except those on a mounted part's own mount face, which rests on its parent (the
/// builder's targets, shooter#57). Whether a socket is free for a part is
/// [`crate::robot::Catalog::validate`]'s to say.
#[must_use]
pub fn sockets(build: &Assembly) -> Vec<SocketSpot> {
    let mut spots = Vec::new();
    for ((id, frame), part) in frames(build).into_iter().zip(build.parts()) {
        let kind = part_type(part);
        let cells = cells_of(part);
        for socket in 0..kind.socket_count() {
            let socket = SocketId(u8::try_from(socket).unwrap_or(u8::MAX));
            let Some(site) = kind.site(socket) else {
                continue;
            };
            if site.face == Face::NegZ && part.mount.is_some() {
                continue;
            }
            let at = frame.then(socket_frame(site.face, site.u, site.v, cells));
            spots.push(SocketSpot {
                part: id,
                socket,
                centre: at.origin,
                normal: at.direction([0.0, 0.0, 1.0]),
            });
        }
    }
    spots
}

// ---------------------------------------------------------------------------------------------
// Layout: the bodies, hinges and actuators of a build.

/// What an actuator is.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Actuator {
    Joint(Joint),
    Locomotion(Locomotion),
}

/// One motorised part: its kind, its drive, the hinges it turns and, for a leg, where its hip
/// is in the robot's frame (for the gait presets).
#[derive(Clone, Debug, PartialEq)]
pub struct ActuatorPlan {
    pub part: PartId,
    pub kind: Actuator,
    pub drive: Drive,
    /// Indices into [`Layout::hinges`]: a leg's hip then knee, a track's road wheels.
    pub hinges: Vec<usize>,
    pub hip: [f32; 3],
}

/// One body of a robot: its children (robot frame) and its friction.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyPlan {
    pub children: Vec<Child>,
    pub friction: f32,
}

/// One hinge between two of a robot's bodies (indices into [`Layout::bodies`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HingePlan {
    pub a: usize,
    pub b: usize,
    pub anchor: [f32; 3],
    pub axis: [f32; 3],
    pub limits: Option<[f32; 2]>,
}

/// Which body and child carry a piece of a part: a part's box, a wheel, a road wheel, a thigh,
/// a shin or a foot.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Segment {
    pub part: PartId,
    pub body: usize,
    pub child: usize,
}

/// A build as bodies, hinges and actuators, authored in the robot's frame. Body 0 holds the root.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub bodies: Vec<BodyPlan>,
    pub hinges: Vec<HingePlan>,
    pub actuators: Vec<ActuatorPlan>,
    /// Every piece of every part, in part-ID order.
    pub segments: Vec<Segment>,
}

/// The quaternion that turns a capsule's local Y onto a part's local Z.
fn capsule_along_z(frame: Frame) -> [f32; 4] {
    frame
        .rotation
        .then(about_x(Orientation::QUARTER))
        .quaternion()
}

impl Layout {
    /// The layout of a validated `build`.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn of(build: &Assembly) -> Self {
        let parts = build.parts();
        let frames = frames(build);
        let frame_of = |id: PartId| {
            frames
                .iter()
                .find(|(part, _)| *part == id)
                .map(|(_, frame)| *frame)
                .expect("every part has a frame")
        };
        let is_joint = |id: PartId| {
            build
                .part(id)
                .is_some_and(|part| matches!(part_type(part).kind, Kind::Joint(_)))
        };
        // The rigid group of a part: the root's, or the joint whose top face it is on.
        let group = |part: &Part| {
            let mut at = part;
            while let Some(mount) = at.mount {
                if is_joint(mount.parent) {
                    return Some(mount.parent);
                }
                at = build.part(mount.parent).expect("validated parents exist");
            }
            None
        };
        let mut groups: Vec<Option<PartId>> = vec![None];
        for part in parts {
            if let Some(joint) = group(part)
                && !groups.contains(&Some(joint))
                && !matches!(part_type(part).kind, Kind::Locomotion(_))
            {
                groups.push(Some(joint));
            }
        }
        groups[1..].sort();
        let mut layout = Self {
            bodies: groups
                .iter()
                .map(|_| BodyPlan {
                    children: Vec::new(),
                    friction: FRICTION,
                })
                .collect(),
            hinges: Vec::new(),
            actuators: Vec::new(),
            segments: Vec::new(),
        };
        let body_of = |part: &Part| {
            groups
                .iter()
                .position(|key| *key == group(part))
                .expect("every rigid part's group is a body")
        };
        for part in parts {
            let kind = part_type(part);
            let frame = frame_of(part.id);
            let [x, y, z] = size(cells_of(part));
            let mass = mass_of(part);
            let new_body = |layout: &mut Self, children: Vec<Child>, friction: f32| {
                layout.bodies.push(BodyPlan { children, friction });
                layout.bodies.len() - 1
            };
            match kind.kind {
                Kind::Locomotion(locomotion) => {
                    let parent = body_of(part);
                    let axis_x = frame.direction([1.0, 0.0, 0.0]);
                    let axis_z = frame.direction([0.0, 0.0, 1.0]);
                    let mut hinges = Vec::new();
                    let mut hip = [0.0; 3];
                    let mut joint = |layout: &mut Self,
                                     a: usize,
                                     b: usize,
                                     anchor: V3,
                                     axis: V3,
                                     limits: Option<[f32; 2]>| {
                        layout.hinges.push(HingePlan {
                            a,
                            b,
                            anchor,
                            axis,
                            limits,
                        });
                        hinges.push(layout.hinges.len() - 1);
                    };
                    match locomotion {
                        Locomotion::Wheel => {
                            let centre = frame.point([x / 2.0, y / 2.0, z / 2.0]);
                            let wheel = Child {
                                shape: Shape::Sphere(x.min(y) / 2.0 - WHEEL_INSET),
                                position: centre,
                                orientation: frame.rotation.quaternion(),
                                mass,
                            };
                            let body = new_body(&mut layout, vec![wheel], GRIP);
                            layout.segments.push(Segment {
                                part: part.id,
                                body,
                                child: 0,
                            });
                            joint(&mut layout, parent, body, centre, axis_z, None);
                        }
                        Locomotion::Track => {
                            let radius = x.min(z) / 2.0;
                            for along in [radius, y - radius] {
                                let centre = frame.point([x / 2.0, along, z / 2.0]);
                                let road = Child {
                                    shape: Shape::Sphere(radius),
                                    position: centre,
                                    orientation: frame.rotation.quaternion(),
                                    mass: mass / 2.0,
                                };
                                let body = new_body(&mut layout, vec![road], GRIP);
                                layout.segments.push(Segment {
                                    part: part.id,
                                    body,
                                    child: 0,
                                });
                                joint(&mut layout, parent, body, centre, axis_z, None);
                            }
                        }
                        Locomotion::Leg => {
                            // A thigh over the upper half and a shin over the lower half
                            // down to a flat foot: the hip on the mount face, the knee half
                            // way.
                            let (cx, cy) = (x / 2.0, y / 2.0);
                            let knee = z / 2.0;
                            let shin_end = z - 2.0 * FOOT[2];
                            let orientation = capsule_along_z(frame);
                            let thigh = Child {
                                shape: Shape::Capsule {
                                    half_segment: knee / 2.0 - LEG_RADIUS,
                                    radius: LEG_RADIUS,
                                },
                                position: frame.point([cx, cy, knee / 2.0]),
                                orientation,
                                mass: mass * 0.4,
                            };
                            let shin = Child {
                                shape: Shape::Capsule {
                                    half_segment: (shin_end - knee) / 2.0 - LEG_RADIUS,
                                    radius: LEG_RADIUS,
                                },
                                position: frame.point([cx, cy, (knee + shin_end) / 2.0]),
                                orientation,
                                mass: mass * 0.4,
                            };
                            let foot = Child {
                                shape: Shape::Box(FOOT),
                                position: frame.point([cx, cy, z - FOOT[2]]),
                                orientation: frame.rotation.quaternion(),
                                mass: mass * 0.2,
                            };
                            let upper = new_body(&mut layout, vec![thigh], FRICTION);
                            let lower = new_body(&mut layout, vec![shin, foot], GRIP);
                            for (body, child) in [(upper, 0), (lower, 0), (lower, 1)] {
                                layout.segments.push(Segment {
                                    part: part.id,
                                    body,
                                    child,
                                });
                            }
                            hip = frame.point([cx, cy, 0.0]);
                            joint(&mut layout, parent, upper, hip, axis_x, Some(HIP_LIMITS));
                            let knee_at = frame.point([cx, cy, knee]);
                            joint(
                                &mut layout,
                                upper,
                                lower,
                                knee_at,
                                axis_x,
                                Some(KNEE_LIMITS),
                            );
                        }
                    }
                    layout.actuators.push(ActuatorPlan {
                        part: part.id,
                        kind: Actuator::Locomotion(locomotion),
                        drive: kind.drive,
                        hinges,
                        hip,
                    });
                }
                _ => {
                    let body = body_of(part);
                    let half = [x / 2.0, y / 2.0, z / 2.0];
                    layout.bodies[body].children.push(Child {
                        shape: Shape::Box(half),
                        position: frame.point(half),
                        orientation: frame.rotation.quaternion(),
                        mass,
                    });
                    layout.segments.push(Segment {
                        part: part.id,
                        body,
                        child: layout.bodies[body].children.len() - 1,
                    });
                }
            }
        }
        // Joints that carry something turn it: one hinge from the joint's body to the body on
        // its top face, at that face's centre.
        for part in parts {
            let Kind::Joint(joint) = part_type(part).kind else {
                continue;
            };
            let Some(moving) = groups.iter().position(|key| *key == Some(part.id)) else {
                continue;
            };
            let frame = frame_of(part.id);
            let [x, y, z] = size(cells_of(part));
            let (axis, limits) = match joint {
                Joint::Hinge => (frame.direction([1.0, 0.0, 0.0]), Some(HINGE_LIMITS)),
                Joint::Rotor => (frame.direction([0.0, 0.0, 1.0]), None),
            };
            layout.hinges.push(HingePlan {
                a: body_of(part),
                b: moving,
                anchor: frame.point([x / 2.0, y / 2.0, z]),
                axis,
                limits,
            });
            layout.actuators.push(ActuatorPlan {
                part: part.id,
                kind: Actuator::Joint(joint),
                drive: part_type(part).drive,
                hinges: vec![layout.hinges.len() - 1],
                hip: [0.0; 3],
            });
        }
        layout.actuators.sort_by_key(|actuator| actuator.part);
        layout.segments.sort_by_key(|segment| segment.part);
        layout
    }

    /// The legs' gait preset the build suggests: two legs walk as a biped, four as a quadruped,
    /// any other count has none.
    #[must_use]
    pub fn suggested_gait(&self) -> Option<Gait> {
        match self.legs().count() {
            2 => Some(Gait::Biped),
            4 => Some(Gait::Quadruped),
            _ => None,
        }
    }

    fn legs(&self) -> impl Iterator<Item = &ActuatorPlan> {
        self.actuators
            .iter()
            .filter(|actuator| actuator.kind == Actuator::Locomotion(Locomotion::Leg))
    }

    /// The lowest point of any child in the robot's frame (its Y).
    fn lowest(&self) -> f32 {
        self.bodies
            .iter()
            .flat_map(|body| &body.children)
            .map(|child| {
                let rotation = rotation_of(child.orientation);
                let reach = match child.shape {
                    Shape::Box(half) => (0..3)
                        .map(|axis| rotation.0[axis][1].abs() * half[axis])
                        .sum(),
                    Shape::Sphere(radius) => radius,
                    Shape::Capsule {
                        half_segment,
                        radius,
                    } => rotation.0[1][1].abs() * half_segment + radius,
                };
                child.position[1] - reach
            })
            .fold(f32::INFINITY, f32::min)
    }
}

/// The rotation of a unit quaternion.
fn rotation_of([x, y, z, w]: [f32; 4]) -> M3 {
    M3([
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y + z * w),
            2.0 * (x * z - y * w),
        ],
        [
            2.0 * (x * y - z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z + x * w),
        ],
        [
            2.0 * (x * z + y * w),
            2.0 * (y * z - x * w),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ])
}

// ---------------------------------------------------------------------------------------------
// Controls and gaits.

/// The legs' gait preset.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Gait {
    /// Legs alternate in pairs, half a cycle apart, each stepping quickly (a sixth of the
    /// cycle) so both feet are down most of the time.
    Biped,
    /// A crawl: one leg steps at a time (left hind, left fore, right hind, right fore), three
    /// on the ground.
    Quadruped,
}

impl Gait {
    const fn cycle(self) -> Cycle {
        match self {
            Self::Biped => Cycle {
                period: 36,
                swing: 6,
                stride: 0.2,
                lift: 0.6,
            },
            Self::Quadruped => Cycle {
                period: 48,
                swing: 12,
                stride: 0.35,
                lift: 0.8,
            },
        }
    }
}

/// A gait's cycle: its ticks, the ticks of it a leg spends in the air, the hip's swing either
/// way at a full stride and the knee's bend at the top of a step (radians).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cycle {
    period: u32,
    swing: u32,
    stride: f32,
    lift: f32,
}

/// Which gait the legs use: the build's suggestion, an override, or none (every leg steps at
/// once).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum GaitChoice {
    #[default]
    Suggested,
    Preset(Gait),
    Unison,
}

/// What one actuator is told this tick.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum Command {
    /// Motor off; the joint turns freely within its limits. Draws nothing.
    #[default]
    Idle,
    /// Turn at this many milliradians a second (at most the part's top rate).
    Rate(i16),
    /// Turn to this binary angle (65 536 a turn, within half a turn either way) and hold it.
    /// A leg holds its hip there and its knee straight.
    Angle(i16),
    /// A leg's stride through its gait, in percent of the full stride (negative walks back);
    /// zero stands. Other actuators treat it as idle.
    Stride(i8),
}

/// Part IDs a robot's controls address: every `u8`.
pub const PART_IDS: usize = 256;

/// A robot's commands for one tick: one per part ID (non-actuators ignore theirs) and the
/// legs' gait.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Controls {
    pub commands: [Command; PART_IDS],
    pub gait: GaitChoice,
}

impl Default for Controls {
    fn default() -> Self {
        Self::IDLE
    }
}

impl Controls {
    pub const IDLE: Self = Self {
        commands: [Command::Idle; PART_IDS],
        gait: GaitChoice::Suggested,
    };

    /// These controls with `part` told `command`.
    #[must_use]
    pub fn with(mut self, part: PartId, command: Command) -> Self {
        if let Some(slot) = self.commands.get_mut(usize::from(part.0)) {
            *slot = command;
        }
        self
    }

    /// The fixed walking mapping of the fixtures (bindings are shooter#74): every leg strides
    /// `forward` percent, plus `turn` on the robot's left side and minus it on its right
    /// (positive turns right).
    #[must_use]
    pub fn walk(mut self, layout: &Layout, forward: i8, turn: i8) -> Self {
        for leg in layout.legs() {
            let side = if leg.hip[2] < 0.0 {
                turn
            } else {
                turn.saturating_neg()
            };
            self = self.with(
                leg.part,
                Command::Stride(forward.saturating_add(side).clamp(-100, 100)),
            );
        }
        self
    }
}

/// A binary angle in radians.
fn radians(angle: i16) -> f32 {
    f32::from(angle) * (PI / 32_768.0)
}

/// A leg's hip and knee targets at `tick` of a gait `cycle` for a stride of `stride` percent: the foot sweeps back on the ground, then lifts and swings forward.
#[allow(clippy::cast_precision_loss)]
fn leg_targets(stride: i8, tick: u32, cycle: Cycle) -> (f32, f32) {
    let Cycle {
        period,
        swing,
        stride: full,
        lift,
    } = cycle;
    if stride == 0 {
        return (0.0, 0.0);
    }
    let amplitude = full * f32::from(stride) / 100.0;
    let stance = period - swing;
    let phase = tick % period;
    if phase < stance {
        let t = phase as f32 / stance as f32;
        (amplitude * (2.0 * t - 1.0), 0.0)
    } else {
        let t = (phase - stance) as f32 / swing as f32;
        (
            amplitude * (1.0 - 2.0 * t),
            lift * (1.0 - (2.0 * t - 1.0).abs()),
        )
    }
}

/// Each leg's phase offset in ticks (in the layout's leg order) for `gait`.
fn offsets(layout: &Layout, gait: Option<Gait>) -> Vec<u32> {
    let legs: Vec<_> = layout.legs().collect();
    let Some(gait) = gait else {
        return vec![0; legs.len()];
    };
    let period = gait.cycle().period;
    match gait {
        Gait::Biped => (0..legs.len())
            .map(|index| if index % 2 == 0 { 0 } else { period / 2 })
            .collect(),
        Gait::Quadruped if legs.len() == 4 => {
            // Fore legs are the two furthest forward (+X); left is −Z. Crawl order: left hind,
            // left fore, right hind, right fore.
            let mut by_x: Vec<usize> = (0..4).collect();
            by_x.sort_by(|&a, &b| legs[b].hip[0].total_cmp(&legs[a].hip[0]).then(a.cmp(&b)));
            let fore = [by_x[0], by_x[1]];
            (0..4)
                .map(|index| {
                    let left = legs[index].hip[2] < 0.0;
                    let order = match (fore.contains(&index), left) {
                        (false, true) => 0,
                        (true, true) => 1,
                        (false, false) => 2,
                        (true, false) => 3,
                    };
                    order * period / 4
                })
                .collect()
        }
        Gait::Quadruped => {
            let count = u32::try_from(legs.len()).unwrap_or(1).max(1);
            (0..count).map(|index| index * period / count).collect()
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Robots in the world.

/// An actuator after the last tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ActuatorState {
    pub part: PartId,
    /// Whether it was commanded and its energy granted.
    pub powered: bool,
    /// Its first joint: angle (radians), rate (radians a second) and motor torque (N·m).
    pub angle: f32,
    pub rate: f32,
    pub torque: f32,
    /// Whether any of its motors gave its most torque.
    pub saturated: bool,
    /// Saturated yet far from its command.
    pub stalled: bool,
}

/// One robot: its build, its layout, its body ids, its energy and its gait clock.
#[derive(Clone, Debug)]
pub struct Robot {
    pub build: Assembly,
    pub layout: Layout,
    pub pool: Pool,
    /// Milli-units ([`crate::energy`]).
    pub charge: u32,
    /// Ticks since it spawned: the gait's clock.
    pub clock: u32,
    /// The first body id.
    base: u64,
    hinges: Vec<HingeId>,
    /// Each actuator's state after the last tick, in [`Layout::actuators`] order.
    pub actuators: Vec<ActuatorState>,
    /// What each actuator was told last tick (for stall detection).
    commanded: Vec<MotorDrive>,
}

impl Robot {
    /// The physics id of its body `index`.
    #[must_use]
    pub const fn body(&self, index: usize) -> BodyId {
        BodyId(self.base + index as u64)
    }
}

/// A robot's exact state.
#[derive(Clone, Debug, PartialEq)]
pub struct RobotState {
    pub build: Assembly,
    pub charge: u32,
    pub clock: u32,
    /// Every body's motion, in [`Layout::bodies`] order.
    pub bodies: Vec<Motion>,
}

/// Everything [`Mechanics::restore`] needs besides the static arena.
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub robots: Vec<RobotState>,
}

/// A ray's hit on a robot's part.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PartHit {
    pub robot: usize,
    pub part: PartId,
    pub fraction: f32,
    pub normal: [f32; 3],
}

/// The world pose of one piece of a part (its box, wheel, thigh, shin or foot).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PartPose {
    pub segment: Segment,
    pub pose: Pose,
}

/// The robots in the articulated world.
#[derive(Debug)]
pub struct Mechanics {
    world: Articulated,
    robots: Vec<Robot>,
}

fn spec<'a>(id: BodyId, body: &'a BodyPlan) -> BodySpec<'a> {
    BodySpec {
        id,
        children: &body.children,
        friction: body.friction,
    }
}

impl Mechanics {
    /// An empty world on `statics` (the arena's, [`crate::Arena::statics`]).
    ///
    /// # Errors
    ///
    /// If the physics world refuses a static body.
    pub fn new(statics: &[RigidBody]) -> Result<Self, PhysicsError> {
        Ok(Self {
            world: Articulated::new(statics)?,
            robots: Vec::new(),
        })
    }

    #[must_use]
    pub fn robots(&self) -> &[Robot] {
        &self.robots
    }

    #[must_use]
    pub const fn world(&self) -> &Articulated {
        &self.world
    }

    fn admit(
        &mut self,
        build: Assembly,
        charge: u32,
        clock: u32,
        add: impl Fn(&mut Articulated, BodyId, &BodyPlan, usize) -> Result<(), PhysicsError>,
    ) -> Result<usize, PhysicsError> {
        let layout = Layout::of(&build);
        let base = FIRST_ROBOT_BODY + self.robots.len() as u64 * BODIES_PER_ROBOT;
        for (index, body) in layout.bodies.iter().enumerate() {
            add(&mut self.world, BodyId(base + index as u64), body, index)?;
        }
        let mut hinges = Vec::with_capacity(layout.hinges.len());
        for hinge in &layout.hinges {
            hinges.push(self.world.add_hinge(Hinge {
                a: BodyId(base + hinge.a as u64),
                b: BodyId(base + hinge.b as u64),
                anchor: hinge.anchor,
                axis: hinge.axis,
                limits: hinge.limits,
            })?);
        }
        let actuators = layout
            .actuators
            .iter()
            .map(|actuator| ActuatorState {
                part: actuator.part,
                ..ActuatorState::default()
            })
            .collect();
        let commanded = vec![MotorDrive::Off; layout.actuators.len()];
        self.robots.push(Robot {
            pool: Pool::of(&build),
            build,
            layout,
            charge,
            clock,
            base,
            hinges,
            actuators,
            commanded,
        });
        Ok(self.robots.len() - 1)
    }

    /// Spawns a robot of `build` at rest, its lowest point just above `feet` (units), turned
    /// `yaw` (binary angle) about +Y, with a full pool. Its index comes back.
    ///
    /// # Errors
    ///
    /// If the physics world refuses a body or a hinge.
    pub fn spawn(
        &mut self,
        build: Assembly,
        feet: [f32; 3],
        yaw: u16,
    ) -> Result<usize, PhysicsError> {
        let lowest = Layout::of(&build).lowest();
        let origin = Pose {
            position: [feet[0], feet[1] - lowest + SPAWN_GAP, feet[2]],
            orientation: about_y(yaw).quaternion(),
        };
        let capacity = Pool::of(&build).capacity;
        self.admit(build, capacity, 0, |world, id, body, _| {
            world.add_body(spec(id, body), origin)
        })
    }

    /// A world on `statics` holding the robots of `state` exactly as they were.
    ///
    /// # Errors
    ///
    /// If a robot's bodies do not match its build, or the physics world refuses one.
    pub fn restore(statics: &[RigidBody], state: &State) -> Result<Self, PhysicsError> {
        let mut mechanics = Self::new(statics)?;
        for robot in &state.robots {
            if Layout::of(&robot.build).bodies.len() != robot.bodies.len() {
                return Err(crate::physics::error(
                    "a robot's bodies do not match its build",
                ));
            }
            mechanics.admit(
                robot.build,
                robot.charge,
                robot.clock,
                |world, id, body, index| world.restore_body(spec(id, body), robot.bodies[index]),
            )?;
        }
        Ok(mechanics)
    }

    /// Every robot's exact state.
    ///
    /// # Panics
    ///
    /// If a robot's body has left the world (never: bodies are not removed).
    #[must_use]
    pub fn state(&self) -> State {
        State {
            robots: self
                .robots
                .iter()
                .map(|robot| RobotState {
                    build: robot.build,
                    charge: robot.charge,
                    clock: robot.clock,
                    bodies: (0..robot.layout.bodies.len())
                        .map(|index| {
                            self.world
                                .motion(robot.body(index))
                                .expect("a robot's bodies stay in the world")
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    /// One tick: each robot's actuators ask its pool for energy and set their motors from
    /// `controls` (one per robot, in order; missing ones idle), the pools regenerate, the
    /// world steps, and every actuator's state is read back.
    ///
    /// # Errors
    ///
    /// If the physics world fails.
    pub fn step(&mut self, controls: &[Controls]) -> Result<(), PhysicsError> {
        for (index, robot) in self.robots.iter_mut().enumerate() {
            let controls = controls.get(index).copied().unwrap_or(Controls::IDLE);
            let gait = match controls.gait {
                GaitChoice::Suggested => robot.layout.suggested_gait(),
                GaitChoice::Preset(gait) => Some(gait),
                GaitChoice::Unison => None,
            };
            let cycle = gait.unwrap_or(Gait::Biped).cycle();
            let offsets = offsets(&robot.layout, gait);
            let mut leg = 0;
            for (at, actuator) in robot.layout.actuators.iter().enumerate() {
                let command = controls.commands[usize::from(actuator.part.0)];
                let is_leg = actuator.kind == Actuator::Locomotion(Locomotion::Leg);
                let offset = if is_leg {
                    leg += 1;
                    offsets[leg - 1]
                } else {
                    0
                };
                let active = match command {
                    Command::Idle => false,
                    Command::Stride(_) => is_leg,
                    Command::Rate(_) | Command::Angle(_) => true,
                };
                let draw = u32::from(actuator.drive.draw) * MILLI / u32::from(TICK_HZ);
                let powered = active && spend(&mut robot.charge, draw);
                let max_torque = f32::from(actuator.drive.torque);
                let max_rate = f32::from(actuator.drive.rate) / 10.0;
                let servo = |angle: f32| MotorDrive::Angle {
                    angle,
                    max_rate,
                    max_torque,
                };
                let drives: Vec<MotorDrive> = if powered {
                    match command {
                        // A leg turns its hip so, holding its knee straight; a track's road wheels turn together.
                        Command::Rate(rate) => {
                            let rate = (f32::from(rate) / 1000.0).clamp(-max_rate, max_rate);
                            let drive = MotorDrive::Rate { rate, max_torque };
                            if is_leg {
                                vec![drive, servo(0.0)]
                            } else {
                                vec![drive; actuator.hinges.len()]
                            }
                        }
                        Command::Angle(angle) => {
                            let drive = servo(radians(angle));
                            if is_leg {
                                vec![drive, servo(0.0)]
                            } else {
                                vec![drive; actuator.hinges.len()]
                            }
                        }
                        Command::Stride(stride) => {
                            let (hip, knee) =
                                leg_targets(stride, robot.clock.wrapping_add(offset), cycle);
                            vec![servo(hip), servo(knee)]
                        }
                        Command::Idle => unreachable!("idle actuators are not powered"),
                    }
                } else {
                    vec![MotorDrive::Off; actuator.hinges.len()]
                };
                for (&hinge, &drive) in actuator.hinges.iter().zip(&drives) {
                    self.world.set_drive(robot.hinges[hinge], drive)?;
                }
                robot.commanded[at] = drives.first().copied().unwrap_or_default();
                robot.actuators[at].powered = powered;
            }
            robot.pool.regenerate(&mut robot.charge);
            robot.clock = robot.clock.wrapping_add(1);
        }
        self.world.step()?;
        for robot in &mut self.robots {
            for (at, actuator) in robot.layout.actuators.iter().enumerate() {
                let states: Vec<HingeState> = actuator
                    .hinges
                    .iter()
                    .map(|&hinge| self.world.hinge(robot.hinges[hinge]).unwrap_or_default())
                    .collect();
                let first = states.first().copied().unwrap_or_default();
                let saturated = states.iter().any(|state| state.saturated);
                let stalled = saturated
                    && match robot.commanded[at] {
                        MotorDrive::Off => false,
                        MotorDrive::Rate { rate, .. } => {
                            (first.rate - rate).abs() > 0.5 * rate.abs() + 0.2
                        }
                        MotorDrive::Angle { angle, .. } => {
                            (first.angle - angle).abs() > 0.05 && first.rate.abs() < 0.2
                        }
                    };
                robot.actuators[at] = ActuatorState {
                    part: actuator.part,
                    powered: robot.actuators[at].powered,
                    angle: first.angle,
                    rate: first.rate,
                    torque: first.torque,
                    saturated,
                    stalled,
                };
            }
        }
        Ok(())
    }

    /// The pose of robot `robot`'s authored frame as its root body carries it.
    #[must_use]
    pub fn root(&self, robot: usize) -> Option<Pose> {
        self.world.origin(self.robots.get(robot)?.body(0))
    }

    /// The motion of robot `robot`'s root body (its centre of mass).
    #[must_use]
    pub fn root_motion(&self, robot: usize) -> Option<Motion> {
        self.world.motion(self.robots.get(robot)?.body(0))
    }

    /// The world pose of every piece of every part of robot `robot`, in part-ID order: what a
    /// snapshot shows and per-part damage aims at.
    #[must_use]
    pub fn part_poses(&self, robot: usize) -> Vec<PartPose> {
        let Some(robot) = self.robots.get(robot) else {
            return Vec::new();
        };
        robot
            .layout
            .segments
            .iter()
            .filter_map(|&segment| {
                Some(PartPose {
                    segment,
                    pose: self
                        .world
                        .child_pose(robot.body(segment.body), segment.child)?,
                })
            })
            .collect()
    }

    /// The first robot part on the segment from `origin` along `translation` (units), unless
    /// static geometry is nearer: per-part hit attribution (shooter#56).
    #[must_use]
    pub fn cast_ray(&self, origin: [f32; 3], translation: [f32; 3]) -> Option<PartHit> {
        let hit = self.world.cast_ray(origin, translation)?;
        let offset = hit.body.0.checked_sub(FIRST_ROBOT_BODY)?;
        let robot = usize::try_from(offset / BODIES_PER_ROBOT).ok()?;
        let body = usize::try_from(offset % BODIES_PER_ROBOT).ok()?;
        let child = hit.child?;
        let segment = self
            .robots
            .get(robot)?
            .layout
            .segments
            .iter()
            .find(|segment| segment.body == body && segment.child == child)?;
        Some(PartHit {
            robot,
            part: segment.part,
            fraction: hit.fraction,
            normal: hit.normal,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quaternions_round_trip_quarter_turns() {
        for yaw in [0, 8_192, 16_384, 32_768, 49_152] {
            let rotation = about_z(yaw).then(about_x(16_384));
            let back = rotation_of(rotation.quaternion());
            for (a, b) in rotation.0.iter().flatten().zip(back.0.iter().flatten()) {
                assert!((a - b).abs() < 1e-5, "{rotation:?} {back:?}");
            }
        }
        assert_eq!(about_z(0).quaternion(), [0.0, 0.0, 0.0, 1.0]);
    }
}
