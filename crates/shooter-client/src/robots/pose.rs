//! Posing a drawn robot for one frame from what the match says about its fighter: where its
//! mechanical robot's bodies are and what it does. Presentation only: nothing here decides a rule.
//!
//! **The simulation's pose.** A living fighter is its mechanical robot (`shooter_sim::mechanics`,
//! shooter#73) and every frame comes with where that robot's bodies are ([`PartStates::frames`]:
//! the predicted local robot, the others between their snapshots). Each part is drawn on the body
//! that carries it, so the legs, wheels, tracks and joints are where the simulation has them:
//!
//! | What | Drawn from |
//! | --- | --- |
//! | the robot as a whole (standing, tilting, lying on its chassis without legs) | its root body |
//! | a leg's stride | the hip (thigh against the body it hangs from) and knee (shin against thigh) angles about the leg's own X, on the kit's hip and knee; the ankle stays at rest (the simulation's foot is fixed to its shin) |
//! | a wheel's or track's roll | the kit's `roll` clip at the angle its body (a track's first road wheel) has turned about its axle (the descriptor's rule: `(angle mod 2π) / 2π` of its second) |
//! | a hinge's or rotor's turn | its moving piece (and everything on it, which rides the carried body) turned as the carried body is against the joint's |
//! | weapons | the weapons kit's states ([`super::weapon`]): no mechanics pose |
//! | death | the robot topples over backwards until it respawns: procedural (a dead fighter has no mechanical robot) |
//! | lost parts | where [`PartStates::detached`] puts them ([`super::track`] lets them fall to the floor) |
//!
//! Without frames (the builder's bench, a gallery's still) every part is drawn at rest, the
//! robot's lowest point on the fighter's feet.
//!
//! The kit turns a joint's moving piece about the cube's centre, the simulation joins the child at
//! the centre of the top face; both share the axis, and the drawing follows the simulation, so
//! the plate stays flush under the child. The kit's leg clips (`walk-biped`, `walk-quadruped`)
//! are not played: the simulation's gait strides the legs.
//!
//! The weapons play the weapons kit's states: a shot's recoil and flash, the continuous weapons'
//! loops, the rail gun's recharge and the rocket pod's reload, the grapple's launch and retract
//! (its hook flying to where the host set it), the blade's extend, retract and quick-melee swing
//! (the kit's `swing` about the blade's wrist), and their glow channels; each mod sits on its
//! weapon's anchor and moves with it. A missing animation (strafe: neither the kit nor the
//! simulation's gait has a sideways step, the legs stride while the robot moves sideways) is
//! reported ([`Animation::missing`]) and marked over the robot, never a silent dummy.

use super::frames::Frames;
use super::kit::{KitData, Rig};
use super::model::{Model, ModelPart};
use super::weapon::{self, Memory};
use render_engine::glam::{Mat3, Mat4, Quat, Vec3};
use shooter_sim::robot::{Joint, Kind, Locomotion, PartId};
use shooter_sim::weapons::{Instance, Weapon};

/// Horizontal speed (metres a second) above which a robot runs rather than idles.
pub const RUN_SPEED: f32 = 0.5;
/// A fall or rise faster than this (metres a second) is airborne.
pub const AIRBORNE_SPEED: f32 = 1.0;
/// How long after a shot the robot is labelled firing (seconds).
pub const RECOIL_SECONDS: f32 = 0.1;
/// How long a death's topple takes (seconds).
pub const TOPPLE_SECONDS: f32 = 0.7;
/// The fighter's reference half height (metres): its feet are this far below its centre.
#[allow(clippy::cast_precision_loss)]
pub const HALF_HEIGHT: f32 =
    shooter_sim::player::PLAYER_HALF_EXTENTS.y as f32 * crate::scene::METRES_PER_UNIT;
/// The leg's sole in its glTF frame: the descriptor's `sole` anchor on the `ankle` joint.
pub const SOLE: Vec3 = Vec3::new(0.0, 0.8, -0.01);

/// What the match says about one fighter this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RobotState {
    /// Its centre (metres): its feet are [`HALF_HEIGHT`] below.
    pub center: Vec3,
    /// Where it looks (its horizontal part turns the robot).
    pub forward: Vec3,
    /// Metres a second.
    pub velocity: Vec3,
    pub sliding: bool,
    /// Where its grapple hook holds (metres), while it does.
    pub hook: Option<Vec3>,
    /// Seconds since its last shot (or swing, or hook), and the weapon copy that made it
    /// (shooter#125: only that copy shows the shot).
    pub since_fired: f32,
    pub fired: Instance,
    /// How many shots it has fired this life (the rocket pod's tubes alternate).
    pub shots: u32,
    /// Seconds since each of its copies that fired lately last did (shooter#127: copies fire
    /// together, each showing its own shot); a copy not in it shows `fired`'s.
    pub recent: Recent,
    /// The weapon copy it holds.
    pub held: Instance,
    /// Its energy pool's share of its capacity (0..=1): the weapons' charge glow.
    pub energy: f32,
    /// How far through a blade swing it is (0..=1), while swinging.
    pub swing: Option<f32>,
    /// Seconds since it died, while dead.
    pub dead_for: Option<f32>,
}

/// Seconds since each weapon copy last fired, by part, for the most recent few.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Recent {
    len: u8,
    entries: [(PartId, f32); RECENT],
}

/// How many copies [`Recent`] remembers.
const RECENT: usize = 8;

impl Default for Recent {
    fn default() -> Self {
        Self::NONE
    }
}

impl Recent {
    /// No copy has fired.
    pub const NONE: Self = Self {
        len: 0,
        entries: [(PartId(0), f32::INFINITY); RECENT],
    };

    /// Seconds since the copy on `part` last fired, if it is remembered.
    #[must_use]
    pub fn since(&self, part: PartId) -> Option<f32> {
        self.entries[..usize::from(self.len)]
            .iter()
            .find(|(at, _)| *at == part)
            .map(|(_, since)| *since)
    }

    /// The copy on `part` fired now: it replaces its own entry, else the oldest.
    pub fn fired(&mut self, part: PartId) {
        let len = usize::from(self.len);
        let at = self.entries[..len]
            .iter()
            .position(|(at, _)| *at == part)
            .unwrap_or_else(|| {
                if len < RECENT {
                    self.len += 1;
                    len
                } else {
                    (0..RECENT)
                        .max_by(|a, b| self.entries[*a].1.total_cmp(&self.entries[*b].1))
                        .unwrap_or(0)
                }
            });
        self.entries[at] = (part, 0.0);
    }

    /// `seconds` later.
    pub fn advance(&mut self, seconds: f32) {
        for (_, since) in &mut self.entries[..usize::from(self.len)] {
            *since += seconds;
        }
    }
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

/// A fighter's animation state this frame: what it does, for the HUD and the weapons (its legs
/// and body are where the simulation has them).
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
        } else if state.hook.is_some() {
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
        match self.activity {
            Activity::Strafe => Some("strafe"),
            _ => None,
        }
    }

    /// "run + fire", "strafe [missing animation: strafe]", "death (procedural topple)".
    #[must_use]
    pub fn label(self) -> String {
        let activity = match self.activity {
            Activity::Idle => "idle",
            Activity::Run => "run",
            Activity::Strafe => "strafe",
            Activity::Airborne => "jump/fall",
            Activity::Grapple => "grapple",
            Activity::Slide => "slide",
            Activity::Death => "death (procedural topple)",
        };
        let action = match self.action {
            Action::None => "",
            Action::Fire => " + fire",
            Action::Blade => " + blade (kit swing)",
        };
        let missing = self
            .missing()
            .map_or_else(String::new, |what| format!(" [missing animation: {what}]"));
        format!("{activity}{action}{missing}")
    }
}

/// Per-part state from the authoritative match: where its bodies are, lost parts and damaged
/// armor.
#[derive(Clone, Copy, Debug, Default)]
pub struct PartStates<'a> {
    /// Where the simulation has its mechanical robot's bodies, while it has one: the pose every
    /// connected part is drawn in.
    pub frames: Option<&'a Frames>,
    /// Each lost part (every one, not only the root of a lost branch), with where its robot
    /// frame is in the world.
    pub detached: &'a [(PartId, Mat4)],
    /// Parts drawn in their damaged variant, where the kit has one.
    pub damaged: &'a [PartId],
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
}

/// Where a weapon's shots are drawn from: its muzzle (or emitter, launcher; the long barrel's
/// when it has one), metres, and the way it points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Muzzle {
    pub weapon: Weapon,
    /// The weapon part it is the muzzle of: every mounted copy has its own (shooter#125).
    pub part: PartId,
    pub position: Vec3,
    pub direction: Vec3,
}

/// A robot as drawn this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Posed {
    pub animation: Animation,
    /// Its root body's frame (at rest without the simulation's frames).
    pub world_from_robot: Mat4,
    pub parts: Vec<PosedPart>,
    /// Where a missing-animation marker floats: over its top.
    pub marker: Vec3,
    /// Where its grapple's cable starts (the launcher's muzzle), if it has one attached.
    pub grapple: Option<Vec3>,
    /// Each mod's mesh frame ([`Model::mods`] order).
    pub mods: Vec<Mat4>,
    /// Its attached weapons' muzzles.
    pub muzzles: Vec<Muzzle>,
}

/// A rotation of `angle` about `axis` through `pivot`.
fn about(pivot: Vec3, axis: Vec3, angle: f32) -> Mat4 {
    Mat4::from_translation(pivot)
        * Mat4::from_axis_angle(axis, angle)
        * Mat4::from_translation(-pivot)
}

/// `motion` (a turn in the robot's frame at rest) in `part`'s glTF frame.
fn local(part: &ModelPart, motion: Mat4) -> Mat4 {
    part.robot_from_local.inverse() * motion * part.robot_from_local
}

/// The angle (radians, right-handed, within ±π) `motion`'s rotation turns about `axis`.
fn angle_about(motion: Mat4, axis: Vec3) -> f32 {
    let (_, rotation, _) = motion.to_scale_rotation_translation();
    let rotation = if rotation.w < 0.0 {
        -rotation
    } else {
        rotation
    };
    2.0 * Vec3::new(rotation.x, rotation.y, rotation.z)
        .dot(axis)
        .atan2(rotation.w)
}

/// A leg's hip and knee angles (radians about the leg's own X, in its glTF frame) where
/// `frames` have its thigh and shin: the thigh against the body it hangs from, the shin against
/// the thigh.
#[must_use]
pub fn leg_angles(part: &ModelPart, frames: &Frames) -> Option<[f32; 2]> {
    let bodies = frames.part(part.id)?;
    let (thigh, shin) = (*bodies.moving.first()?, *bodies.moving.get(1)?);
    Some([
        angle_about(local(part, frames.relative(bodies.base, thigh)), Vec3::X),
        angle_about(local(part, frames.relative(thigh, shin)), Vec3::X),
    ])
}

/// A wheel's or track's angle (radians about its axle, [`super::model::Roll::axis`]) where
/// `frames` have its rolling body (a track's first road wheel).
#[must_use]
pub fn roll_angle(part: &ModelPart, frames: &Frames) -> Option<f32> {
    let roll = part.roll.as_ref()?;
    let bodies = frames.part(part.id)?;
    let wheel = *bodies.moving.first()?;
    Some(angle_about(frames.relative(bodies.base, wheel), roll.axis))
}

/// How a joint's moving piece is turned (in its glTF frame) where `frames` have the body its top
/// face carries.
#[must_use]
pub fn joint_turn(part: &ModelPart, frames: &Frames) -> Option<Mat4> {
    let bodies = frames.part(part.id)?;
    let carried = *bodies.moving.first()?;
    Some(local(part, frames.relative(bodies.base, carried)))
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

/// Where a robot without the simulation's frames stands: at rest on the fighter's feet, facing
/// its look; toppling over backwards once dead.
fn rest_frame(model: &Model, state: &RobotState, animation: Animation) -> Mat4 {
    let feet = state.center - Vec3::Y * HALF_HEIGHT;
    let mut world_from_robot =
        Mat4::from_mat3_translation(facing(state.forward), feet - Vec3::Y * model.floor);
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
    world_from_robot
}

/// `model` posed for `state`, with `parts`' per-part state (the simulation's frames, lost and
/// damaged parts) and its weapons' `memory`.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn pose(
    model: &Model,
    kit: &KitData,
    state: &RobotState,
    parts: &PartStates,
    memory: &Memory,
) -> Posed {
    let animation = Animation::of(state);
    let count = model.parts.len();
    // Lost parts.
    let mut lost: Vec<Option<Mat4>> = vec![None; count];
    for (part, world_from_robot) in parts.detached {
        if let Some(index) = model.parts.iter().position(|p| p.id == *part) {
            lost[index] = Some(*world_from_robot);
        }
    }
    // The root body, and each part on the body it hangs from (all at rest without frames).
    let frames = parts
        .frames
        .filter(|_| animation.activity != Activity::Death);
    let world_from_robot = frames.map_or_else(|| rest_frame(model, state, animation), Frames::root);
    let bases: Vec<Mat4> = model
        .parts
        .iter()
        .map(|part| {
            frames
                .and_then(|frames| Some(frames.body(frames.part(part.id)?.base)))
                .unwrap_or(world_from_robot)
        })
        .collect();
    let mut palettes: Vec<Option<Vec<Mat4>>> = vec![None; count];
    // Attached weapons' joints, posed once their place in the world is known (the hook).
    let mut weapons = vec![None; count];
    for (index, part) in model.parts.iter().enumerate() {
        let entry = part.entry;
        let Some(rig) = &kit.entries[entry].rig else {
            continue;
        };
        let attached = lost[index].is_none();
        palettes[index] = Some(match part.kind {
            Kind::Locomotion(Locomotion::Leg) if attached => {
                let [hip, knee] = frames
                    .and_then(|frames| leg_angles(part, frames))
                    .unwrap_or_default();
                rig.palette(&leg_rotations(rig, [hip, knee, 0.0]))
            }
            Kind::Weapon(held) if attached && kit.entries[entry].gear.is_some() => {
                let gear = kit.entries[entry]
                    .gear
                    .as_ref()
                    .unwrap_or_else(|| unreachable!());
                let instance = Instance {
                    part: part.id,
                    weapon: held,
                };
                weapons[index] = Some(weapon::locals(
                    rig,
                    gear,
                    instance,
                    part.refire,
                    state,
                    memory,
                ));
                continue;
            }
            Kind::Locomotion(Locomotion::Wheel | Locomotion::Track) if attached => {
                // The kit's roll clip at the axle's angle: (angle mod 2π) / 2π through it.
                let angle = frames
                    .and_then(|frames| roll_angle(part, frames))
                    .unwrap_or_default();
                let rolling = kit.entries[entry].rolling.as_ref();
                match rolling.and_then(|rolling| Some((rolling, rig.clip(&rolling.clip)?))) {
                    Some((rolling, clip)) => {
                        let mut locals = rig.rest_locals();
                        clip.pose(
                            rolling.time(angle.rem_euclid(std::f32::consts::TAU)),
                            &mut locals,
                        );
                        rig.palette_of(&locals)
                    }
                    None => rig.palette(&rig.rest()),
                }
            }
            Kind::Joint(joint) if attached => {
                let turn = frames
                    .and_then(|frames| joint_turn(part, frames))
                    .unwrap_or(Mat4::IDENTITY);
                let moving = rig.joint(match joint {
                    Joint::Hinge => "hinge",
                    Joint::Rotor => "rotor",
                });
                rig.palette(&rig.rest())
                    .into_iter()
                    .enumerate()
                    .map(|(j, m)| if Some(j) == moving { turn * m } else { m })
                    .collect()
            }
            _ => rig.palette(&rig.rest()),
        });
    }

    let mut grapple = None;
    let mut muzzles = Vec::new();
    let mut posed = Vec::with_capacity(count);
    for (index, part) in model.parts.iter().enumerate() {
        let world_from_local = match lost[index] {
            Some(world) => world * part.robot_from_local,
            None => bases[index] * part.robot_from_local,
        };
        if let Some((mut locals, reach)) = weapons[index].take() {
            let entry = part.entry;
            let kit_entry = &kit.entries[entry];
            if let (Some(rig), Some(gear), Kind::Weapon(held)) =
                (&kit_entry.rig, &kit_entry.gear, part.kind)
            {
                if let Some(reach) = reach {
                    let target = world_from_local.inverse().transform_point3(reach.target);
                    weapon::reach_hook(rig, gear, &mut locals, target, reach.reach);
                }
                let palette = rig.palette_of(&locals);
                // A mod's muzzle replaces the weapon's; the rocket pod shows the tube that fired.
                let mod_muzzle = model.mods.iter().find_map(|modification| {
                    let muzzle = modification.muzzle.as_ref()?;
                    (modification.part == index).then(|| {
                        let frame = world_from_local
                            * anchored(rig, &palette, &modification.anchor)
                            * modification.anchor.frame();
                        (frame, muzzle)
                    })
                });
                let name = match held {
                    Weapon::RocketLauncher if state.shots.is_multiple_of(2) => "muzzle-b",
                    Weapon::RocketLauncher => "muzzle-a",
                    _ => "muzzle",
                };
                let found = mod_muzzle.or_else(|| {
                    let anchor = gear.anchor(name)?;
                    Some((world_from_local * anchored(rig, &palette, anchor), anchor))
                });
                if let Some((frame, anchor)) = found {
                    let muzzle = Muzzle {
                        weapon: held,
                        part: part.id,
                        position: frame.transform_point3(anchor.position),
                        direction: frame.transform_vector3(anchor.normal).normalize_or(Vec3::X),
                    };
                    if held == Weapon::Grapple {
                        grapple = Some(muzzle.position);
                    }
                    muzzles.push(muzzle);
                }
                palettes[index] = Some(palette);
            }
        }
        posed.push(PosedPart {
            world_from_local,
            palette: palettes[index].take(),
            damaged: parts.damaged.contains(&part.id) && part.damaged.is_some(),
            detached: lost[index].is_some(),
        });
    }
    // Each mod on its weapon's anchor, moving with the node it rides (at rest on a lost weapon).
    let mods = model
        .mods
        .iter()
        .map(|modification| {
            let weapon = &posed[modification.part];
            let entry = model.parts[modification.part].entry;
            let at = kit.entries[entry]
                .rig
                .as_ref()
                .map_or(Mat4::IDENTITY, |rig| {
                    let palette = weapon
                        .palette
                        .clone()
                        .unwrap_or_else(|| rig.palette(&rig.rest()));
                    anchored(rig, &palette, &modification.anchor)
                });
            weapon.world_from_local * at * modification.anchor.frame()
        })
        .collect();
    Posed {
        animation,
        marker: world_from_robot.transform_point3(Vec3::Y * (model.top + 0.3)),
        world_from_robot,
        parts: posed,
        grapple,
        mods,
        muzzles,
    }
}

/// The skinning transform of the node `anchor` rides (its weapon's glTF frame, posed).
fn anchored(rig: &Rig, palette: &[Mat4], anchor: &super::kit::Anchor) -> Mat4 {
    anchor
        .node
        .as_deref()
        .and_then(|node| rig.joint(node))
        .and_then(|joint| palette.get(joint).copied())
        .unwrap_or(Mat4::IDENTITY)
}
