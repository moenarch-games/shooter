//! Posing a drawn robot for one frame from what the match says about its fighter: where it is
//! and looks, how it moves, and what it does. Presentation only: nothing here decides a rule.
//!
//! The match's fighters still move on the capsule controller (the mechanical robots of
//! shooter#73 do not drive them yet), so the robot stands on the capsule's feet, faces its look
//! and animates its legs from the fighter's motion:
//!
//! | Activity | Legs | Source |
//! | --- | --- | --- |
//! | idle | at rest | the kit's rest pose |
//! | run (forward or back) | the gait clip at a phase driven by the distance run | kit `walk-biped` / `walk-quadruped` |
//! | strafe | the same clip | **missing animation**: the kit has no sideways clip |
//! | jump or fall, grapple | tucked: hip, knee and ankle turned within the kit's ranges | procedural |
//! | slide | crouched, the body lowered so the feet stay down | procedural |
//! | death | the robot topples over backwards until it respawns | procedural (part loss is shooter#56) |
//!
//! Firing kicks the held weapon back and flashes it; a blade swing is **missing animation** (the
//! arm is rigid and the kit has no swing clip); the energy blade's sweep is still drawn by the
//! weapon effects. A missing animation is reported ([`Animation::missing`]) and marked over the
//! robot, never a silent dummy.
//!
//! **Joints.** Hinges and rotors turn at the angles the state gives ([`PartStates::joints`];
//! nothing in the match sets them yet). The kit turns a joint's moving piece about the cube's
//! centre, the simulation (`shooter_sim::mechanics`) joins the child at the centre of the top
//! face; both share the axis. The drawing follows the simulation: the moving piece and every part
//! on it turn about the simulation's anchor, so the plate stays flush under the child. For a
//! rotor both pivots lie on its axis, so they agree exactly.
//!
//! **Lost parts** ([`PartStates::detached`]): a detached part and everything mounted on it draw
//! where the authoritative state puts them, at rest relative to each other; the rest of the robot
//! keeps animating without them. Nothing in the match detaches parts yet (shooter#56).

use super::kit::{KitData, Rig};
use super::model::{Model, Visual};
use render_engine::glam::{Mat3, Mat4, Quat, Vec3};
use shooter_sim::robot::{Joint, Kind, Locomotion, PartId};
use shooter_sim::weapons::Weapon;

/// Horizontal speed (metres a second) above which a robot runs rather than idles.
pub const RUN_SPEED: f32 = 0.5;
/// The gait clips' stride: the body's travel in one cycle (metres): `walk-biped` 0.6 m/s over a
/// 1 s cycle, `walk-quadruped` 0.5 m/s over 0.8 s (asset-tooling `docs/combat-robot-kit.md`).
fn stride(clip: &str) -> f32 {
    if clip == "walk-quadruped" { 0.4 } else { 0.6 }
}
/// The fastest the legs cycle (cycles a second). The fighters run far faster than the clips'
/// walking pace, so the legs trot at this rate and slide over the ground until mechanical
/// locomotion moves the robots (shooter#58).
pub const MAX_CYCLES_PER_SECOND: f32 = 2.5;
/// A fall or rise faster than this (metres a second) is airborne.
pub const AIRBORNE_SPEED: f32 = 1.0;
/// How long a shot's recoil and flash last (seconds) and how far the weapon kicks (metres).
pub const RECOIL_SECONDS: f32 = 0.1;
const RECOIL: f32 = 0.05;
/// How long a death's topple takes (seconds).
pub const TOPPLE_SECONDS: f32 = 0.7;
/// The capsule's half height (metres): the robot's feet stand at its bottom.
#[allow(clippy::cast_precision_loss)]
pub const HALF_HEIGHT: f32 =
    shooter_sim::player::PLAYER_HALF_EXTENTS.y as f32 * crate::scene::METRES_PER_UNIT;
/// The leg's sole in its glTF frame: the descriptor's `sole` anchor on the `ankle` joint.
pub const SOLE: Vec3 = Vec3::new(0.0, 0.8, -0.01);
/// Where the simulation anchors a hinge or rotor's moving side, in the joint part's glTF frame:
/// the centre of its top face (the kit's pivot is the cube's centre, (0, 0.1, 0)).
pub const SIM_JOINT_ANCHOR: Vec3 = Vec3::new(0.0, 0.2, 0.0);

/// A leg's hip, knee and ankle (radians about the leg's own X, the kit's signs: a positive hip
/// swings the leg back, a positive knee folds the shin back) while airborne.
pub const TUCK: [f32; 3] = [-0.8, 1.4, -0.6];
/// The same while sliding: crouched, the foot level.
pub const CROUCH: [f32; 3] = [-1.1, 2.0, -0.9];

/// What the match says about one fighter this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RobotState {
    /// Its capsule's centre (metres).
    pub center: Vec3,
    /// Where it looks (its horizontal part turns the robot).
    pub forward: Vec3,
    /// Metres a second.
    pub velocity: Vec3,
    pub sliding: bool,
    /// Its grapple hook holds.
    pub hooked: bool,
    /// Seconds since its last shot.
    pub since_fired: f32,
    pub held: Weapon,
    /// How far through a blade swing it is (0..=1), while swinging.
    pub swing: Option<f32>,
    /// Seconds since it died, while dead.
    pub dead_for: Option<f32>,
}

/// What its legs and body do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Run,
    Strafe,
    Airborne,
    Grapple,
    Slide,
    Death,
}

/// What its weapons do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Fire,
    Blade,
}

/// A fighter's animation state this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Animation {
    pub activity: Activity,
    pub action: Action,
}

impl Animation {
    #[must_use]
    pub fn of(state: &RobotState) -> Self {
        let horizontal = Vec3::new(state.velocity.x, 0.0, state.velocity.z);
        let look = Vec3::new(state.forward.x, 0.0, state.forward.z).normalize_or(Vec3::Z);
        let activity = if state.dead_for.is_some() {
            Activity::Death
        } else if state.sliding {
            Activity::Slide
        } else if state.hooked {
            Activity::Grapple
        } else if state.velocity.y.abs() > AIRBORNE_SPEED {
            Activity::Airborne
        } else if horizontal.length() > RUN_SPEED {
            if horizontal.normalize().dot(look).abs() < 0.5 {
                Activity::Strafe
            } else {
                Activity::Run
            }
        } else {
            Activity::Idle
        };
        let action = if activity == Activity::Death {
            Action::None
        } else if state.swing.is_some() {
            Action::Blade
        } else if state.since_fired < RECOIL_SECONDS {
            Action::Fire
        } else {
            Action::None
        };
        Self { activity, action }
    }

    /// The animation it lacks, if any: drawn with a marker and named in the HUD.
    #[must_use]
    pub const fn missing(self) -> Option<&'static str> {
        match (self.activity, self.action) {
            (_, Action::Blade) => Some("blade swing"),
            (Activity::Strafe, _) => Some("strafe"),
            _ => None,
        }
    }

    /// "run (walk-biped) + fire", "strafe [missing animation: strafe]".
    #[must_use]
    pub fn label(self, model: &Model) -> String {
        let activity = match self.activity {
            Activity::Idle => "idle".to_owned(),
            Activity::Run | Activity::Strafe => {
                let name = if self.activity == Activity::Run {
                    "run"
                } else {
                    "strafe"
                };
                model
                    .gait
                    .map_or_else(|| name.to_owned(), |clip| format!("{name} ({clip})"))
            }
            Activity::Airborne => "jump/fall (procedural tuck)".to_owned(),
            Activity::Grapple => "grapple (procedural tuck)".to_owned(),
            Activity::Slide => "slide (procedural crouch)".to_owned(),
            Activity::Death => "death (procedural topple)".to_owned(),
        };
        let action = match self.action {
            Action::None => "",
            Action::Fire => " + fire",
            Action::Blade => " + blade",
        };
        let missing = self
            .missing()
            .map_or_else(String::new, |what| format!(" [missing animation: {what}]"));
        format!("{activity}{action}{missing}")
    }
}

/// The gait clip's phase after `seconds` more of `state`'s motion (cycles, wrapping at 1).
#[must_use]
pub fn advance_phase(phase: f32, model: &Model, state: &RobotState, seconds: f32) -> f32 {
    let Some(clip) = model.gait else {
        return phase;
    };
    let horizontal = Vec3::new(state.velocity.x, 0.0, state.velocity.z);
    let look = Vec3::new(state.forward.x, 0.0, state.forward.z).normalize_or(Vec3::Z);
    let animation = Animation::of(state);
    let speed = match animation.activity {
        // Backing up runs the clip backwards; strafing runs it forwards.
        Activity::Run => horizontal.dot(look),
        Activity::Strafe => horizontal.length(),
        _ => return phase,
    };
    let rate = (speed / stride(clip)).clamp(-MAX_CYCLES_PER_SECOND, MAX_CYCLES_PER_SECOND);
    (phase + rate * seconds).rem_euclid(1.0)
}

/// Per-part state from the authoritative match: lost parts, damaged armor and joint angles.
#[derive(Clone, Copy, Debug, Default)]
pub struct PartStates<'a> {
    /// Each detached part, with where its subtree's robot frame is in the world.
    pub detached: &'a [(PartId, Mat4)],
    /// Parts drawn in their damaged variant, where the kit has one.
    pub damaged: &'a [PartId],
    /// Hinge and rotor angles (radians).
    pub joints: &'a [(PartId, f32)],
}

/// One part as drawn this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct PosedPart {
    pub world_from_local: Mat4,
    /// A skinned part's joint palette.
    pub palette: Option<Vec<Mat4>>,
    /// Drawn in its damaged variant.
    pub damaged: bool,
    /// Detached from the robot.
    pub detached: bool,
    /// The held weapon (a placeholder drawn highlighted), flashing while it fires.
    pub held: bool,
    pub flash: bool,
}

/// A robot as drawn this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Posed {
    pub animation: Animation,
    pub world_from_robot: Mat4,
    pub parts: Vec<PosedPart>,
    /// Where a missing-animation marker floats: over its top.
    pub marker: Vec3,
    /// Where its grapple weapon is (the cable's start), if it has one attached.
    pub grapple: Option<Vec3>,
}

/// A rotation of `angle` about `axis` through `pivot`.
fn about(pivot: Vec3, axis: Vec3, angle: f32) -> Mat4 {
    Mat4::from_translation(pivot)
        * Mat4::from_axis_angle(axis, angle)
        * Mat4::from_translation(-pivot)
}

/// The leg rig's rotations for hip, knee and ankle angles about its own X.
fn leg_rotations(rig: &Rig, [hip, knee, ankle]: [f32; 3]) -> Vec<Quat> {
    let mut rotations = rig.rest();
    for (name, angle) in [("hip", hip), ("knee", knee), ("ankle", ankle)] {
        if let Some(joint) = rig.joint(name) {
            rotations[joint] = Quat::from_rotation_x(angle);
        }
    }
    rotations
}

/// The robot's turn for a look direction: +X along its horizontal part.
fn facing(forward: Vec3) -> Mat3 {
    let forward = Vec3::new(forward.x, 0.0, forward.z).normalize_or(Vec3::Z);
    Mat3::from_cols(forward, Vec3::Y, forward.cross(Vec3::Y))
}

/// `model` posed for `state`, its legs at gait `phase`, with `parts`' per-part state.
#[must_use]
pub fn pose(
    model: &Model,
    kit: &KitData,
    state: &RobotState,
    phase: f32,
    parts: &PartStates,
) -> Posed {
    let animation = Animation::of(state);
    let count = model.parts.len();
    // Lost subtrees.
    let mut lost: Vec<Option<Mat4>> = vec![None; count];
    for (part, world_from_robot) in parts.detached {
        if let Some(index) = model.parts.iter().position(|p| p.id == *part) {
            for at in model.subtree(index) {
                lost[at] = Some(*world_from_robot);
            }
        }
    }
    // Joint motions (robot frame) and every part's palette.
    let mut motion = vec![Mat4::IDENTITY; count];
    let mut palettes: Vec<Option<Vec<Mat4>>> = vec![None; count];
    for (index, part) in model.parts.iter().enumerate() {
        let Visual::Kit { entry, .. } = part.visual else {
            continue;
        };
        let Some(rig) = &kit.entries[entry].rig else {
            continue;
        };
        let attached = lost[index].is_none();
        palettes[index] = Some(match part.kind {
            Kind::Locomotion(Locomotion::Leg) if attached => {
                let rotations = match (animation.activity, model.gait.and_then(|c| rig.clip(c))) {
                    (Activity::Run | Activity::Strafe, Some(clip)) => {
                        let mut rotations = rig.rest();
                        clip.sample(phase + part.phase, &mut rotations);
                        rotations
                    }
                    (Activity::Airborne | Activity::Grapple, _) => leg_rotations(rig, TUCK),
                    (Activity::Slide, _) => leg_rotations(rig, CROUCH),
                    _ => rig.rest(),
                };
                rig.palette(&rotations)
            }
            Kind::Joint(joint) if attached => {
                let angle = parts
                    .joints
                    .iter()
                    .find(|(id, _)| *id == part.id)
                    .map_or(0.0, |(_, angle)| *angle);
                let axis = match joint {
                    Joint::Hinge => Vec3::X,
                    Joint::Rotor => Vec3::Y,
                };
                let turn = about(SIM_JOINT_ANCHOR, axis, angle);
                let moving = rig.joint(match joint {
                    Joint::Hinge => "hinge",
                    Joint::Rotor => "rotor",
                });
                motion[index] = part.robot_from_local * turn * part.robot_from_local.inverse();
                rig.palette(&rig.rest())
                    .into_iter()
                    .enumerate()
                    .map(|(j, m)| if Some(j) == moving { turn * m } else { m })
                    .collect()
            }
            _ => rig.palette(&rig.rest()),
        });
    }
    // What is mounted on a turned joint turns with it (parents come before children).
    let mut carried = vec![Mat4::IDENTITY; count];
    for index in 0..count {
        if let Some(parent) = model.parts[index].parent {
            carried[index] = carried[parent] * motion[parent];
        }
    }
    let robot_from_local: Vec<Mat4> = (0..count)
        .map(|index| carried[index] * model.parts[index].robot_from_local)
        .collect();

    // Stand on the capsule's feet; grounded, the lowest sole stays on the floor.
    let grounded = matches!(
        animation.activity,
        Activity::Idle | Activity::Run | Activity::Strafe | Activity::Slide
    );
    let mut lift = 0.0;
    if grounded {
        let soles = model.parts.iter().enumerate().filter_map(|(index, part)| {
            let Visual::Kit { entry, .. } = part.visual else {
                return None;
            };
            let rig = kit.entries[entry].rig.as_ref()?;
            let ankle = rig.joint("ankle")?;
            let palette = palettes[index].as_ref()?;
            (part.kind == Kind::Locomotion(Locomotion::Leg) && lost[index].is_none()).then(|| {
                (robot_from_local[index] * palette[ankle])
                    .transform_point3(SOLE)
                    .y
            })
        });
        if let Some(lowest) = soles.reduce(f32::min) {
            lift = model.floor - lowest;
        }
    }
    let feet = state.center - Vec3::Y * HALF_HEIGHT;
    let mut world_from_robot =
        Mat4::from_mat3_translation(facing(state.forward), feet + Vec3::Y * (lift - model.floor));
    if let (Activity::Death, Some(dead_for)) = (animation.activity, state.dead_for) {
        let t = (dead_for / TOPPLE_SECONDS).clamp(0.0, 1.0);
        let eased = t * t * (3.0 - 2.0 * t);
        // Backwards over its heels: +Y turns toward −X about +Z.
        world_from_robot *= about(
            Vec3::new(model.back, model.floor, 0.0),
            Vec3::Z,
            eased * std::f32::consts::FRAC_PI_2,
        );
    }

    let recoil = (RECOIL_SECONDS - state.since_fired).max(0.0) / RECOIL_SECONDS * RECOIL;
    let mut grapple = None;
    let posed = model
        .parts
        .iter()
        .enumerate()
        .map(|(index, part)| {
            let held = part.kind == Kind::Weapon(state.held);
            let firing = held && animation.action == Action::Fire;
            let world_from_local = match lost[index] {
                Some(world) => world * model.parts[index].robot_from_local,
                None if firing => {
                    world_from_robot
                        * Mat4::from_translation(-Vec3::X * recoil)
                        * robot_from_local[index]
                }
                None => world_from_robot * robot_from_local[index],
            };
            if part.kind == Kind::Weapon(Weapon::Grapple) && lost[index].is_none() {
                grapple = Some(world_from_local.transform_point3(Vec3::ZERO));
            }
            PosedPart {
                world_from_local,
                palette: palettes[index].take(),
                damaged: parts.damaged.contains(&part.id)
                    && matches!(
                        part.visual,
                        Visual::Kit {
                            damaged: Some(_),
                            ..
                        }
                    ),
                detached: lost[index].is_some(),
                held,
                flash: firing,
            }
        })
        .collect();
    Posed {
        animation,
        marker: world_from_robot.transform_point3(Vec3::Y * (model.top + 0.3)),
        world_from_robot,
        parts: posed,
        grapple,
    }
}
