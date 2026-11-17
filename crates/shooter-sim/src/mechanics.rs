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
//! same command (a binding inverts one side, [`crate::bindings`]), a wheel mounted on the front drives
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
//! **Energy** (D2.i70.3, D2.i123.2; shooter#128): every commanded actuator asks the robot's
//! electricity for its part's full draw each tick ([`Drive::draw`], actuators in part-ID order);
//! one that is refused is off that tick, and only it. After the step it pays for what it did:
//! its share of the draw follows its motors' torque and speed ([`motor_share`]), and the rest is
//! refunded. An idle actuator asks nothing, so resting motors cost nothing; a robot driven faster
//! or carrying more load pays more. Generation follows every request ([`crate::energy`]).
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
//! **In the match** ([`crate::session`]): every living fighter is a robot here, in its slot
//! ([`Robot::slot`]; its bodies' ids follow from it). Every lifecycle change (a spawn, a death,
//! a part shot off, a client putting its robot in the host's state) edits the [`State`] and
//! rebuilds the world from it, so the host, a predicting client and a replay hold the same world.
//! A part shot off leaves the bodies ([`Layout::connected`], [`Mechanics::sever`]): it no longer
//! weighs, touches or drives anything, and the bodies that stay keep their pose and motion
//! ([`crate::physics::articulated::rebase`]). The match's movement drive ([`Mechanics::drive`])
//! sets the whole robot's velocity and turns it (an interim chassis drive: the bindings do not
//! carry a robot yet, see [`crate::bindings`]); its actuators take the commands its person's
//! control layout resolved ([`crate::bindings::Motors::controls`]), its bodies slide frictionless
//! over the arena (the drive is what carries it) and its legs bend only within their gaits'
//! range ([`Mechanics::for_match`]).
//!
//! **Debris** (shooter#75, Atlas D2.i70.10). In the match a part carried away whole (its parent
//! broke) and at least [`DEBRIS_MIN_MASS_KG`] heavy becomes a body of its own where its robot
//! carried it, moving as that point did ([`Mechanics::break_off`], [`DebrisState`]): one box with
//! the part's mass and friction, colliding with the arena, the robots and other debris. It
//! retires after [`DEBRIS_LIFETIME_TICKS`], or after lying still [`DEBRIS_REST_TICKS`], or when
//! it falls out of the arena ([`Mechanics::retire_debris`]), or when its robot respawns
//! ([`Mechanics::clear_debris`]); past [`MAX_DEBRIS`] the oldest piece retires. Debris is part of
//! the [`State`], restored and continued exactly like the robots.

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
/// A leg's hip and knee limits in the match ([`Mechanics::for_match`]): the gait presets' own
/// range (their widest stride and highest step), so the movement drive's bumps and landings
/// cannot fold a leg its servos could not straighten again under the robot's weight.
const CARRIED_HIP: [f32; 2] = [-0.4, 0.4];
const CARRIED_KNEE: [f32; 2] = [0.0, 0.8];
/// A leg's radius, and its foot's half extents along its X and Y and half thickness.
const LEG_RADIUS: f32 = 8.0;
const FOOT: [f32; 3] = [9.0, 14.0, 3.0];
/// A wheel's sphere is this much (units) inside its cell footprint, so neighbouring wheels do
/// not rub.
const WHEEL_INSET: f32 = 2.0;
/// Spawned robots start this far (units) above the ground.
const SPAWN_GAP: f32 = 0.5;

/// The first debris body's id: debris ids follow the robots' (every slot's) and run in
/// [`State::debris`] order.
pub const FIRST_DEBRIS_BODY: u64 = FIRST_ROBOT_BODY + 256 * BODIES_PER_ROBOT;
/// At most this many pieces of debris are physical at once in a world; a newer piece past the
/// budget retires the oldest (Atlas Q2.i70.6, provisional).
pub const MAX_DEBRIS: usize = 16;
/// A piece of debris retires this many ticks (5 s) after it came off...
pub const DEBRIS_LIFETIME_TICKS: u16 = 300;
/// ...or once it has lain still this many ticks in a row (1 s)...
pub const DEBRIS_REST_TICKS: u16 = 60;
/// ...still meaning slower than this (units a second) and turning slower than
/// [`DEBRIS_REST_SPIN`] (radians a second).
pub const DEBRIS_REST_SPEED: f32 = 5.0;
pub const DEBRIS_REST_SPIN: f32 = 0.5;
/// A lost part this heavy (kg, its mod included) or heavier stays physical debris for a while;
/// a lighter one is a cosmetic shard only, which clients draw falling on their own (Atlas
/// D2.i70.10, provisional).
pub const DEBRIS_MIN_MASS_KG: f32 = 12.0;

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

/// Which body of a build a body is, whatever else the build has lost: the root's rigid group,
/// a joint's rigid group (what its top face carries), or a locomotion part's `n`th body (a
/// wheel; a track's road wheels; a leg's thigh, then shin and foot).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub enum BodyKey {
    Rigid(Option<PartId>),
    Moving(PartId, u8),
}

/// One body of a robot: what it is, its children (robot frame) and its friction.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyPlan {
    pub key: BodyKey,
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
    /// The guns among its parts (not the blade or the grapple module), in part-ID order: what
    /// a fire binding may name (shooter#127).
    pub guns: Vec<PartId>,
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
    pub fn of(build: &Assembly) -> Self {
        Self::connected(build, 0)
    }

    /// The layout of what is left of a validated `build` once the parts in `lost` (bit `i`:
    /// the part at index `i` in part-ID order, [`crate::damage`]; a lost part's subtree is
    /// lost too) have come off: they no longer weigh, touch or drive anything. Every part keeps
    /// its place in the robot's frame.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn connected(build: &Assembly, lost: u32) -> Self {
        let all = build.parts();
        let kept: Vec<Part> = all
            .iter()
            .enumerate()
            .filter(|(index, _)| *index >= 32 || lost & (1 << index) == 0)
            .map(|(_, part)| *part)
            .collect();
        let parts = kept.as_slice();
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
                .map(|key| BodyPlan {
                    key: BodyKey::Rigid(*key),
                    children: Vec::new(),
                    friction: FRICTION,
                })
                .collect(),
            hinges: Vec::new(),
            actuators: Vec::new(),
            segments: Vec::new(),
            guns: Vec::new(),
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
            let mut moving = 0u8;
            let mut new_body = |layout: &mut Self, children: Vec<Child>, friction: f32| {
                layout.bodies.push(BodyPlan {
                    key: BodyKey::Moving(part.id, moving),
                    children,
                    friction,
                });
                moving += 1;
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
        // The guns a fire binding may name (shooter#127, [`crate::bindings::Output::Fire`]).
        layout.guns = parts
            .iter()
            .filter(|part| matches!(part_type(part).kind, Kind::Weapon(weapon) if weapon.gun()))
            .map(|part| part.id)
            .collect();
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

    /// The fixtures' fixed walking mapping (the presets bind the same, [`crate::bindings`]):
    /// every leg strides `forward` percent, plus `turn` on the robot's left side and minus it on
    /// its right (positive turns right).
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

/// The share (0 to 1) of its full draw a motor used over one step (D2.i123.2, shooter#128): with
/// `t` its joints' mean torque and `s` their mean speed, each as a share of the most it gives,
/// `(t + s + t·s) / 3`. Its torque under load (holding, accelerating, carrying weight) and its
/// speed (the work it turns out) each cost, the product being the mechanical work; a motor
/// that neither strains nor turns uses nothing, and one at full torque and speed all of it.
/// Provisional curve (Atlas Q2.i123.2), not a motor model.
#[must_use]
pub fn motor_share(states: &[HingeState], max_torque: f32, max_rate: f32) -> f32 {
    if states.is_empty() {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let count = states.len() as f32;
    let share = |value: f32, most: f32| {
        if most > 0.0 {
            (value.abs() / most).clamp(0.0, 1.0)
        } else {
            0.0
        }
    };
    let torque = states
        .iter()
        .map(|state| share(state.torque, max_torque))
        .sum::<f32>()
        / count;
    let speed = states
        .iter()
        .map(|state| share(state.rate, max_rate))
        .sum::<f32>()
        / count;
    ((torque + speed + torque * speed) / 3.0).clamp(0.0, 1.0)
}

/// What a motor granted `granted` milli-units pays for a step at `share` of its draw
/// ([`motor_share`]): rounded up, never more than it was granted.
#[must_use]
pub fn motor_use(granted: u32, share: f32) -> u32 {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let used = (granted as f32 * share).ceil() as u32;
    used.min(granted)
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

/// One robot: its slot, its build and the parts it has lost, its layout, its body ids, its
/// energy and its gait clock.
#[derive(Clone, Debug)]
pub struct Robot {
    /// Its stable identity: a match's fighter slot. Its bodies' ids follow from it.
    pub slot: u8,
    pub build: Assembly,
    /// The parts that came off ([`Layout::connected`]).
    pub lost: u32,
    pub layout: Layout,
    pub pool: Pool,
    /// Its electricity, milli-units ([`crate::energy`]).
    pub charge: u32,
    /// Its fuel, milli-units ([`crate::energy`]): only its own generation burns it here (the
    /// match generates for its fighters itself).
    pub fuel: u32,
    /// The electricity its motors used last step, after the refunds (milli-units).
    pub motors_spent: u32,
    /// Whether a commanded motor went unpowered last step (its electricity ran out).
    pub starved: bool,
    /// Ticks since it spawned: the gait's clock.
    pub clock: u32,
    /// The first body id.
    base: u64,
    hinges: Vec<HingeId>,
    /// Each actuator's state after the last tick, in [`Layout::actuators`] order.
    pub actuators: Vec<ActuatorState>,
    /// What each actuator was told last tick (for stall detection).
    commanded: Vec<MotorDrive>,
    /// What each actuator was granted before this step (milli-units), refunded after it as far as
    /// its load and speed did not use it.
    drawn: Vec<u32>,
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
    pub slot: u8,
    pub build: Assembly,
    pub lost: u32,
    pub charge: u32,
    pub fuel: u32,
    pub clock: u32,
    /// Every body's motion, in [`Layout::bodies`] order (of [`Layout::connected`]).
    pub bodies: Vec<Motion>,
}

impl RobotState {
    /// The layout its bodies follow.
    #[must_use]
    pub fn layout(&self) -> Layout {
        Layout::connected(&self.build, self.lost)
    }

    /// Where each connected part is: the pose of the authored (robot) frame of the body that
    /// carries it (a leg's thigh, a track's first road wheel), in part-ID order. With
    /// [`placements`] that is the part's cuboid in the world: what strikes meet and what a
    /// client draws.
    ///
    /// # Errors
    ///
    /// If its bodies do not match its layout.
    pub fn part_frames(&self) -> Result<Vec<(PartId, Pose)>, PhysicsError> {
        part_frames(&self.layout(), &self.bodies)
    }
}

/// Each part of `layout` with the authored-frame pose of its first body in `bodies`.
fn part_frames(layout: &Layout, bodies: &[Motion]) -> Result<Vec<(PartId, Pose)>, PhysicsError> {
    if bodies.len() != layout.bodies.len() {
        return Err(crate::physics::error(
            "a robot's bodies do not match its build",
        ));
    }
    let origins = layout
        .bodies
        .iter()
        .zip(bodies)
        .map(|(plan, motion)| crate::physics::articulated::origin_of(&plan.children, motion))
        .collect::<Result<Vec<_>, _>>()?;
    let mut frames: Vec<(PartId, Pose)> = Vec::new();
    for segment in &layout.segments {
        if frames.last().is_none_or(|(part, _)| *part != segment.part) {
            frames.push((segment.part, origins[segment.body]));
        }
    }
    Ok(frames)
}

/// A large part that came off a robot (Atlas D2.i70.10, shooter#75): for a while a body of its
/// own in the world, collidable and falling, until it retires ([`DEBRIS_LIFETIME_TICKS`],
/// [`DEBRIS_REST_TICKS`], [`MAX_DEBRIS`]). It weighs, touches and drives nothing of its robot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DebrisState {
    /// The slot of the robot it came off.
    pub slot: u8,
    pub part: PartId,
    /// The part's box with its mass, authored in its robot's frame as it was when it came off:
    /// the body's authored frame is that robot frame ([`DebrisState::frame`]).
    pub piece: Child,
    /// Ticks since it came off.
    pub age: u16,
    /// Ticks in a row it has lain still.
    pub rest: u16,
    pub motion: Motion,
}

impl DebrisState {
    /// Where the robot frame it was authored in is now: with [`placements`] of its robot's
    /// build, where its part is (what a client draws).
    ///
    /// # Errors
    ///
    /// If its piece has no valid mass properties.
    pub fn frame(&self) -> Result<Pose, PhysicsError> {
        crate::physics::articulated::origin_of(&[self.piece], &self.motion)
    }

    /// Whether it retires after this tick: past its lifetime, still long enough, or fallen out
    /// of the arena.
    #[must_use]
    pub fn retired(&self) -> bool {
        #[allow(clippy::cast_precision_loss)]
        let fallen = self.motion.pose.position[1] < crate::player::KILL_HEIGHT_UNITS as f32;
        self.age >= DEBRIS_LIFETIME_TICKS || self.rest >= DEBRIS_REST_TICKS || fallen
    }
}

/// Everything [`Mechanics::restore`] needs besides the static arena: the robots in slot order
/// and the debris in the order it came off.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub robots: Vec<RobotState>,
    pub debris: Vec<DebrisState>,
}

/// A unit quaternion turning the robot's frame (+X forward) to face `yaw` (the game's binary
/// yaw, [`crate::trig::direction`]), upright: exact from the integer table.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn facing(yaw: u16) -> [f32; 4] {
    let (x, z) = crate::trig::direction(yaw);
    let one = TRIG_ONE as f32;
    let (fx, fz) = (x as f32 / one, z as f32 / one);
    M3([[fx, 0.0, fz], [0.0, 1.0, 0.0], [-fz, 0.0, fx]]).quaternion()
}

/// How far (units) a robot of `build` standing whole at rest reaches below its frame's origin:
/// where its feet are.
#[must_use]
pub fn stance(build: &Assembly) -> f32 {
    -Layout::of(build).lowest()
}

/// A ray's hit on a robot's part.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PartHit {
    pub robot: usize,
    pub part: PartId,
    pub fraction: f32,
    pub normal: [f32; 3],
}

/// What a robot stood on after the last step ([`Mechanics::footing`]).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Footing {
    /// A connected leg, wheel or track touched ground.
    pub feet: bool,
    /// Any of its bodies did.
    pub any: bool,
}

/// The world pose of one piece of a part (its box, wheel, thigh, shin or foot).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PartPose {
    pub segment: Segment,
    pub pose: Pose,
}

/// The robots in the articulated world, in slot order.
///
/// **Lifecycle.** A robot is added ([`Mechanics::spawn_at`]), removed ([`Mechanics::despawn`]),
/// loses parts ([`Mechanics::sever`]) or is put in an exact state ([`Mechanics::place`]) by
/// editing the [`State`] and rebuilding the world from it ([`Mechanics::restore`]): the world
/// after any change is the one a client or a replay restores from the same state, so the host,
/// the prediction and replays continue alike.
#[derive(Debug)]
pub struct Mechanics {
    world: Articulated,
    robots: Vec<Robot>,
    statics: Vec<RigidBody>,
    gravity: [f32; 3],
    /// Whether [`Mechanics::step`] regenerates the robots' pools (the match regenerates them
    /// itself, after every request of the tick).
    pub regenerate: bool,
    /// The match's friction for every robot body, if not their own (and its legs' narrower
    /// limits): the movement drive ([`Mechanics::drive`]) is what carries a robot over the ground,
    /// so its feet and wheels slide where it drives them instead of dragging it over, and it
    /// slides along walls as the capsule did (Atlas D2.3's fast movement on mechanical robots,
    /// until the robots walk and roll on their actuators alone: Atlas D2.i70.5).
    traction: Option<f32>,
    /// The physical debris, in the order it came off; motions as of the last step or rebuild.
    debris: Vec<DebrisState>,
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
        Self::with_gravity(statics, crate::physics::articulated::EARTH_GRAVITY)
    }

    /// [`Self::new`] under `gravity` (m/s²): the arena's for the match.
    ///
    /// # Errors
    ///
    /// If the physics world refuses a static body.
    pub fn with_gravity(statics: &[RigidBody], gravity: [f32; 3]) -> Result<Self, PhysicsError> {
        Ok(Self {
            world: Articulated::with_gravity(statics, gravity)?,
            robots: Vec::new(),
            statics: statics.to_vec(),
            gravity,
            regenerate: true,
            traction: None,
            debris: Vec::new(),
        })
    }

    /// The robots, in slot order.
    #[must_use]
    pub fn robots(&self) -> &[Robot] {
        &self.robots
    }

    /// The robot in `slot`.
    #[must_use]
    pub fn robot(&self, slot: u8) -> Option<&Robot> {
        self.robots.iter().find(|robot| robot.slot == slot)
    }

    /// The robot in `slot`, to set its charge before a step.
    pub fn robot_mut(&mut self, slot: u8) -> Option<&mut Robot> {
        self.robots.iter_mut().find(|robot| robot.slot == slot)
    }

    /// The world rebuilt from `state` (the same statics and gravity): every lifecycle change.
    fn rebuild(&mut self, state: &State) -> Result<(), PhysicsError> {
        let mut fresh = Self {
            world: Articulated::with_gravity(&self.statics, self.gravity)?,
            robots: Vec::new(),
            statics: std::mem::take(&mut self.statics),
            gravity: self.gravity,
            regenerate: self.regenerate,
            traction: self.traction,
            debris: Vec::new(),
        };
        let admitted = fresh.admit_all(state);
        if let Err(error) = admitted {
            self.statics = fresh.statics;
            return Err(error);
        }
        *self = fresh;
        Ok(())
    }

    /// The same robots on new `statics` (an arena placeable broke or came back whole,
    /// [`crate::scenery`]): the world rebuilt from its exact state.
    ///
    /// # Errors
    ///
    /// If the physics world refuses a static body or a robot; the world is as it was then.
    pub fn reshape(&mut self, statics: &[RigidBody]) -> Result<(), PhysicsError> {
        let state = self.state();
        let old = std::mem::replace(&mut self.statics, statics.to_vec());
        let rebuilt = self.rebuild(&state);
        if rebuilt.is_err() {
            self.statics = old;
        }
        rebuilt
    }

    /// The match's robots ([`crate::session`]): under `gravity` (m/s²), their pools
    /// regenerated by the match, every body frictionless (the movement drive carries them) and
    /// their legs bending only within the gait presets' range.
    ///
    /// # Errors
    ///
    /// If the physics world refuses a static body.
    pub fn for_match(statics: &[RigidBody], gravity: [f32; 3]) -> Result<Self, PhysicsError> {
        let mut mechanics = Self::with_gravity(statics, gravity)?;
        mechanics.regenerate = false;
        mechanics.traction = Some(0.0);
        Ok(mechanics)
    }

    /// Adds a robot of `build` to `slot` at rest, its lowest point just above `feet` (units),
    /// turned to `orientation` (a unit quaternion of its frame, [`facing`]), with a full pool.
    ///
    /// # Errors
    ///
    /// If the slot is taken or the physics world refuses a body or a hinge.
    pub fn spawn_at(
        &mut self,
        slot: u8,
        build: Assembly,
        feet: [f32; 3],
        orientation: [f32; 4],
    ) -> Result<(), PhysicsError> {
        if self.robot(slot).is_some() {
            return Err(crate::physics::error(format!("robot slot {slot} is taken")));
        }
        let layout = Layout::of(&build);
        let origin = Pose {
            position: [feet[0], feet[1] - layout.lowest() + SPAWN_GAP, feet[2]],
            orientation,
        };
        let bodies = layout
            .bodies
            .iter()
            .map(|body| crate::physics::articulated::motion_at(&body.children, origin))
            .collect::<Result<Vec<_>, _>>()?;
        let mut state = self.state();
        state.robots.push(RobotState {
            slot,
            build,
            lost: 0,
            charge: Pool::of(&build).capacity,
            fuel: Pool::of(&build).fuel,
            clock: 0,
            bodies,
        });
        state.robots.sort_by_key(|robot| robot.slot);
        self.rebuild(&state)
    }

    /// Takes the robot in `slot` out of the world (a death, a slot changing hands). Whether
    /// there was one.
    ///
    /// # Errors
    ///
    /// If the rebuilt world refuses a body.
    pub fn despawn(&mut self, slot: u8) -> Result<bool, PhysicsError> {
        let mut state = self.state();
        let before = state.robots.len();
        state.robots.retain(|robot| robot.slot != slot);
        if state.robots.len() == before {
            return Ok(false);
        }
        self.rebuild(&state)?;
        Ok(true)
    }

    /// Puts the robot of `robot.slot` in the exact state `robot` (a client resuming its
    /// prediction from a snapshot), replacing whatever was in that slot.
    ///
    /// # Errors
    ///
    /// If its bodies do not match its build or the world refuses one.
    pub fn place(&mut self, robot: RobotState) -> Result<(), PhysicsError> {
        let mut state = self.state();
        state.robots.retain(|other| other.slot != robot.slot);
        state.robots.push(robot);
        state.robots.sort_by_key(|robot| robot.slot);
        self.rebuild(&state)
    }

    /// The robot in `slot` has lost the parts in `lost` (every part lost so far, by index,
    /// [`Layout::connected`]): they come off and stop weighing, touching and driving, while
    /// every body that stays keeps its authored pose, velocity field and spin
    /// ([`crate::physics::articulated::rebase`] when its children changed). Whether anything
    /// changed. Nothing it lost stays physical ([`Mechanics::break_off`] leaves debris).
    ///
    /// # Errors
    ///
    /// If the rebuilt world refuses a body.
    pub fn sever(&mut self, slot: u8, lost: u32) -> Result<bool, PhysicsError> {
        self.break_off(slot, lost, 0)
    }

    /// [`Mechanics::sever`], and each newly lost part in `intact` (by index: the parts carried
    /// away whole, not destroyed ones, which shatter) at least [`DEBRIS_MIN_MASS_KG`] heavy
    /// becomes physical debris where its robot carried it (Atlas D2.i70.10): the match's way.
    /// Past [`MAX_DEBRIS`] the oldest pieces retire.
    ///
    /// # Errors
    ///
    /// If the rebuilt world refuses a body.
    pub fn break_off(&mut self, slot: u8, lost: u32, intact: u32) -> Result<bool, PhysicsError> {
        let mut state = self.state();
        let Some(robot) = state.robots.iter_mut().find(|robot| robot.slot == slot) else {
            return Ok(false);
        };
        let lost = lost | robot.lost;
        if lost == robot.lost {
            return Ok(false);
        }
        let old = robot.layout();
        let fresh = Self::debris_of(robot, &old, lost & !robot.lost & intact)?;
        let new = Layout::connected(&robot.build, lost);
        let mut bodies = Vec::with_capacity(new.bodies.len());
        for plan in &new.bodies {
            let Some(at) = old.bodies.iter().position(|body| body.key == plan.key) else {
                return Err(crate::physics::error("a severed robot gained a body"));
            };
            let motion = &robot.bodies[at];
            bodies.push(if old.bodies[at].children == plan.children {
                *motion
            } else {
                crate::physics::articulated::rebase(
                    &old.bodies[at].children,
                    motion,
                    &plan.children,
                )?
            });
        }
        robot.lost = lost;
        robot.bodies = bodies;
        state.debris.extend(fresh);
        while state.debris.len() > MAX_DEBRIS {
            // Past the budget the oldest piece retires (ties: the one that came off first).
            let oldest = (0..state.debris.len())
                .rev()
                .max_by_key(|&at| state.debris[at].age)
                .unwrap_or(0);
            state.debris.remove(oldest);
        }
        self.rebuild(&state)?;
        Ok(true)
    }

    /// The debris the parts `newly` lost leave behind `robot` (laid out as `layout`): each
    /// newly lost part at least [`DEBRIS_MIN_MASS_KG`] heavy becomes a body of its own where
    /// its robot carried it, moving as that point of its body moved.
    fn debris_of(
        robot: &RobotState,
        layout: &Layout,
        newly: u32,
    ) -> Result<Vec<DebrisState>, PhysicsError> {
        if newly == 0 {
            return Ok(Vec::new());
        }
        let frames = part_frames(layout, &robot.bodies)?;
        let placed = placements(&robot.build);
        let mut debris = Vec::new();
        for (index, part) in robot.build.parts().iter().enumerate() {
            let mass = mass_of(part);
            if newly & (1 << index) == 0 || mass < DEBRIS_MIN_MASS_KG {
                continue;
            }
            let (Some((_, frame)), Some(segment)) = (
                frames.iter().find(|(id, _)| *id == part.id),
                layout
                    .segments
                    .iter()
                    .find(|segment| segment.part == part.id),
            ) else {
                continue;
            };
            let placement = placed[index];
            let piece = Child {
                shape: Shape::Box(placement.half_extents),
                position: placement.centre,
                orientation: placement.orientation,
                mass,
            };
            let mut motion = crate::physics::articulated::motion_at(&[piece], *frame)?;
            let carrier = robot.bodies[segment.body];
            let lever = sub(motion.pose.position, carrier.pose.position);
            let spin = carrier.angular_velocity;
            let swing = [
                spin[1] * lever[2] - spin[2] * lever[1],
                spin[2] * lever[0] - spin[0] * lever[2],
                spin[0] * lever[1] - spin[1] * lever[0],
            ];
            motion.velocity = add(carrier.velocity, swing);
            motion.angular_velocity = spin;
            debris.push(DebrisState {
                slot: robot.slot,
                part: part.id,
                piece,
                age: 0,
                rest: 0,
                motion,
            });
        }
        Ok(debris)
    }

    /// The physical debris, in the order it came off.
    #[must_use]
    pub fn debris(&self) -> &[DebrisState] {
        &self.debris
    }

    /// Retires the debris that came off the robot in `slot` (its next life starts). Whether any
    /// did.
    ///
    /// # Errors
    ///
    /// If the rebuilt world refuses a body.
    pub fn clear_debris(&mut self, slot: u8) -> Result<bool, PhysicsError> {
        if self.debris.iter().all(|piece| piece.slot != slot) {
            return Ok(false);
        }
        let mut state = self.state();
        state.debris.retain(|piece| piece.slot != slot);
        self.rebuild(&state)?;
        Ok(true)
    }

    /// Puts the world's debris in the exact state `debris` (a client resuming its prediction
    /// from a snapshot).
    ///
    /// # Errors
    ///
    /// If there is more than [`MAX_DEBRIS`] or the rebuilt world refuses a body.
    pub fn place_debris(&mut self, debris: &[DebrisState]) -> Result<(), PhysicsError> {
        if self.debris == debris {
            return Ok(());
        }
        let mut state = self.state();
        state.debris = debris.to_vec();
        self.rebuild(&state)
    }

    /// The robot in `slot`'s bodies' motions with their masses.
    fn masses(&self, slot: u8) -> Vec<(BodyId, Motion, f32)> {
        let Some(robot) = self.robot(slot) else {
            return Vec::new();
        };
        robot
            .layout
            .bodies
            .iter()
            .enumerate()
            .filter_map(|(index, body)| {
                let id = robot.body(index);
                let mass = body.children.iter().map(|child| child.mass).sum();
                Some((id, self.world.motion(id)?, mass))
            })
            .collect()
    }

    /// The whole robot in `slot`'s centre of mass and its velocity (units, units a second).
    fn centre(bodies: &[(BodyId, Motion, f32)]) -> ([f32; 3], [f32; 3]) {
        let total: f32 = bodies.iter().map(|(_, _, mass)| mass).sum();
        if total <= 0.0 {
            return ([0.0; 3], [0.0; 3]);
        }
        let weigh = |of: fn(&Motion) -> [f32; 3]| {
            std::array::from_fn(|axis| {
                bodies
                    .iter()
                    .map(|(_, motion, mass)| of(motion)[axis] * mass)
                    .sum::<f32>()
                    / total
            })
        };
        (
            weigh(|motion| motion.pose.position),
            weigh(|motion| motion.velocity),
        )
    }

    /// The velocity (units a second) of the whole robot in `slot`'s centre of mass: what the
    /// movement rules see it realize.
    #[must_use]
    pub fn velocity(&self, slot: u8) -> Option<[f32; 3]> {
        let bodies = self.masses(slot);
        (!bodies.is_empty()).then(|| Self::centre(&bodies).1)
    }

    /// Sets the robot in `slot` moving with its whole centre of mass at `velocity` (units a
    /// second) and its root body spinning at `angular_velocity` (radians a second), as one rigid
    /// whole would: the change from its motion now is added to every body (the spin's about the
    /// whole robot's centre of mass, so it moves none of it), so the joints' own motion stays
    /// (the match's movement drive).
    ///
    /// # Errors
    ///
    /// If the physics world refuses a velocity.
    pub fn drive(
        &mut self,
        slot: u8,
        velocity: [f32; 3],
        angular_velocity: [f32; 3],
    ) -> Result<(), PhysicsError> {
        let bodies = self.masses(slot);
        let Some(&(_, root, _)) = bodies.first() else {
            return Ok(());
        };
        let (centre, moving) = Self::centre(&bodies);
        let dv = sub(velocity, moving);
        let dw = sub(angular_velocity, root.angular_velocity);
        for (id, motion, _) in bodies {
            let lever = sub(motion.pose.position, centre);
            let swing = [
                dw[1] * lever[2] - dw[2] * lever[1],
                dw[2] * lever[0] - dw[0] * lever[2],
                dw[0] * lever[1] - dw[1] * lever[0],
            ];
            self.world.set_velocity(
                id,
                add(add(motion.velocity, dv), swing),
                add(motion.angular_velocity, dw),
            )?;
        }
        Ok(())
    }

    /// Whether the robot in `slot` stood on anything after the last step: `feet` when one of
    /// its connected locomotion parts' bodies did (it has footing to walk, roll and jump),
    /// `any` when any of its bodies did.
    #[must_use]
    pub fn footing(&self, slot: u8) -> Footing {
        let Some(robot) = self.robot(slot) else {
            return Footing::default();
        };
        let mut footing = Footing::default();
        for (index, body) in robot.layout.bodies.iter().enumerate() {
            if self.world.supported(robot.body(index)) {
                footing.any = true;
                footing.feet |= matches!(body.key, BodyKey::Moving(..));
            }
        }
        footing
    }

    /// Each connected part of the robot in `slot` with its body's authored-frame pose
    /// ([`RobotState::part_frames`]).
    ///
    /// # Errors
    ///
    /// If a body has left the world.
    pub fn part_frames(&self, slot: u8) -> Result<Vec<(PartId, Pose)>, PhysicsError> {
        let Some(robot) = self.robot(slot) else {
            return Ok(Vec::new());
        };
        let bodies = (0..robot.layout.bodies.len())
            .map(|index| {
                self.world
                    .motion(robot.body(index))
                    .ok_or_else(|| crate::physics::error("a robot's body left the world"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        part_frames(&robot.layout, &bodies)
    }

    /// The world's gravity (m/s²).
    #[must_use]
    pub const fn gravity(&self) -> [f32; 3] {
        self.gravity
    }

    #[must_use]
    pub const fn world(&self) -> &Articulated {
        &self.world
    }

    fn admit(
        &mut self,
        robot: &RobotState,
        add: impl Fn(&mut Articulated, BodyId, &BodyPlan, usize) -> Result<(), PhysicsError>,
    ) -> Result<usize, PhysicsError> {
        let RobotState {
            slot,
            build,
            lost,
            charge,
            fuel,
            clock,
            ..
        } = *robot;
        let layout = robot.layout();
        let base = FIRST_ROBOT_BODY + u64::from(slot) * BODIES_PER_ROBOT;
        for (index, body) in layout.bodies.iter().enumerate() {
            add(&mut self.world, BodyId(base + index as u64), body, index)?;
        }
        // In the match a leg's hip and knee stay within its gaits' range ([`CARRIED_HIP`],
        // [`CARRIED_KNEE`]): bumps and landings cannot fold it past what a step uses.
        let carried = self.traction.is_some();
        let leg_limits = |at: usize| {
            layout
                .legs()
                .find_map(|leg| leg.hinges.iter().position(|&hinge| hinge == at))
                .map(|joint| {
                    if joint == 0 {
                        CARRIED_HIP
                    } else {
                        CARRIED_KNEE
                    }
                })
        };
        let mut hinges = Vec::with_capacity(layout.hinges.len());
        for (at, hinge) in layout.hinges.iter().enumerate() {
            let limits = match leg_limits(at) {
                Some(limits) if carried => Some(limits),
                _ => hinge.limits,
            };
            hinges.push(self.world.add_hinge(Hinge {
                a: BodyId(base + hinge.a as u64),
                b: BodyId(base + hinge.b as u64),
                anchor: hinge.anchor,
                axis: hinge.axis,
                limits,
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
        let drawn = vec![0; layout.actuators.len()];
        self.robots.push(Robot {
            slot,
            pool: Pool::of(&build),
            build,
            lost,
            layout,
            charge,
            fuel,
            motors_spent: 0,
            starved: false,
            drawn,
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
        let slot = self
            .robots
            .last()
            .map_or(0, |robot| robot.slot.saturating_add(1));
        self.spawn_at(slot, build, feet, about_y(yaw).quaternion())?;
        Ok(self.robots.len() - 1)
    }

    /// A world on `statics` holding the robots of `state` exactly as they were.
    ///
    /// # Errors
    ///
    /// If a robot's bodies do not match its build, or the physics world refuses one.
    pub fn restore(statics: &[RigidBody], state: &State) -> Result<Self, PhysicsError> {
        Self::restore_with(statics, crate::physics::articulated::EARTH_GRAVITY, state)
    }

    /// [`Self::restore`] under `gravity` (m/s²).
    ///
    /// # Errors
    ///
    /// As [`Self::restore`], or if two robots share a slot or are out of slot order.
    pub fn restore_with(
        statics: &[RigidBody],
        gravity: [f32; 3],
        state: &State,
    ) -> Result<Self, PhysicsError> {
        let mut mechanics = Self::with_gravity(statics, gravity)?;
        mechanics.admit_all(state)?;
        Ok(mechanics)
    }

    /// [`Self::for_match`] holding the robots and debris of `state` exactly as they were: a
    /// match's world resumed (a client's prediction, a replay).
    ///
    /// # Errors
    ///
    /// As [`Self::restore_with`].
    pub fn restore_for_match(
        statics: &[RigidBody],
        gravity: [f32; 3],
        state: &State,
    ) -> Result<Self, PhysicsError> {
        let mut mechanics = Self::for_match(statics, gravity)?;
        mechanics.admit_all(state)?;
        Ok(mechanics)
    }

    /// The robots of `state` into this world, which has none.
    fn admit_all(&mut self, state: &State) -> Result<(), PhysicsError> {
        let traction = self.traction;
        for robot in &state.robots {
            if robot.layout().bodies.len() != robot.bodies.len() {
                return Err(crate::physics::error(
                    "a robot's bodies do not match its build",
                ));
            }
            if self
                .robots
                .last()
                .is_some_and(|last| last.slot >= robot.slot)
            {
                return Err(crate::physics::error("robots out of slot order"));
            }
            self.admit(robot, |world, id, body, index| {
                let mut spec = spec(id, body);
                if let Some(friction) = traction {
                    spec.friction = friction;
                }
                world.restore_body(spec, robot.bodies[index])
            })?;
        }
        if state.debris.len() > MAX_DEBRIS {
            return Err(crate::physics::error("more debris than its budget"));
        }
        for (at, piece) in state.debris.iter().enumerate() {
            self.world.restore_body(
                BodySpec {
                    id: BodyId(FIRST_DEBRIS_BODY + at as u64),
                    children: std::slice::from_ref(&piece.piece),
                    friction: FRICTION,
                },
                piece.motion,
            )?;
        }
        self.debris.clone_from(&state.debris);
        Ok(())
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
                    slot: robot.slot,
                    build: robot.build,
                    lost: robot.lost,
                    charge: robot.charge,
                    fuel: robot.fuel,
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
            debris: self.debris.clone(),
        }
    }

    /// One tick: each robot's commanded actuators ask its store for their full draw and set
    /// their motors from `controls` (one per robot, in order; missing ones idle), the world
    /// steps, every actuator's state is read back and refunded what its load and speed did not
    /// use, and (outside the match) the robots generate ([`Pool::generate`]).
    ///
    /// # Errors
    ///
    /// If the physics world fails.
    pub fn step(&mut self, controls: &[Controls]) -> Result<(), PhysicsError> {
        for (index, robot) in self.robots.iter_mut().enumerate() {
            robot.motors_spent = 0;
            robot.starved = false;
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
                robot.drawn[at] = if powered { draw } else { 0 };
                robot.starved |= active && !powered;
            }
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
                // What it used of its grant: its load and speed ([`motor_share`]); the rest goes
                // back to the store it came from, which it never overfills (it was in it).
                let granted = std::mem::take(&mut robot.drawn[at]);
                if granted > 0 {
                    let max_torque = f32::from(actuator.drive.torque);
                    let max_rate = f32::from(actuator.drive.rate) / 10.0;
                    let used = motor_use(granted, motor_share(&states, max_torque, max_rate));
                    robot.charge = robot.charge.saturating_add(granted - used);
                    robot.motors_spent += used;
                }
            }
            if self.regenerate {
                robot.pool.generate(&mut robot.charge, &mut robot.fuel);
            }
        }
        self.age_debris();
        Ok(())
    }

    /// After a step: every piece of debris reads its motion back and ages.
    fn age_debris(&mut self) {
        let length = |value: [f32; 3]| {
            (value[0] * value[0] + value[1] * value[1] + value[2] * value[2]).sqrt()
        };
        for (at, piece) in self.debris.iter_mut().enumerate() {
            if let Some(motion) = self.world.motion(BodyId(FIRST_DEBRIS_BODY + at as u64)) {
                piece.motion = motion;
            }
            piece.age = piece.age.saturating_add(1);
            let still = length(piece.motion.velocity) < DEBRIS_REST_SPEED
                && length(piece.motion.angular_velocity) < DEBRIS_REST_SPIN;
            piece.rest = if still {
                piece.rest.saturating_add(1)
            } else {
                0
            };
        }
    }

    /// The pieces of debris that have retired ([`DebrisState::retired`]) leave the world: the
    /// match does this first thing every tick, before anything moves a body, so the step that
    /// follows finds every robot's footing again. Whether any left.
    ///
    /// # Errors
    ///
    /// If the rebuilt world refuses a body.
    pub fn retire_debris(&mut self) -> Result<bool, PhysicsError> {
        if self.debris.iter().any(DebrisState::retired) {
            let mut state = self.state();
            state.debris.retain(|piece| !piece.retired());
            // The robots' actuators read back this step stay what they were.
            let actuators: Vec<_> = self
                .robots
                .iter()
                .map(|robot| (robot.actuators.clone(), robot.commanded.clone()))
                .collect();
            self.rebuild(&state)?;
            for (robot, (actuators, commanded)) in self.robots.iter_mut().zip(actuators) {
                robot.actuators = actuators;
                robot.commanded = commanded;
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn in_slot(&self, slot: usize) -> Option<&Robot> {
        self.robot(u8::try_from(slot).ok()?)
    }

    /// The pose of the authored frame of the robot in `slot` as its root body carries it.
    #[must_use]
    pub fn root(&self, slot: usize) -> Option<Pose> {
        self.world.origin(self.in_slot(slot)?.body(0))
    }

    /// The motion of the root body (its centre of mass) of the robot in `slot`.
    #[must_use]
    pub fn root_motion(&self, slot: usize) -> Option<Motion> {
        self.world.motion(self.in_slot(slot)?.body(0))
    }

    /// The world pose of every piece of every part of the robot in `slot`, in part-ID order.
    #[must_use]
    pub fn part_poses(&self, slot: usize) -> Vec<PartPose> {
        let Some(robot) = self.in_slot(slot) else {
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
            .in_slot(robot)?
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
