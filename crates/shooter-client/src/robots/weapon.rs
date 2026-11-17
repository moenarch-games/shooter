//! The weapons' animation states and glow channels, from what the match says about the fighter
//! (presentation only: the simulation decides every shot, swing and hook). Each weapon of the
//! weapons kit is one skinned mesh whose nodes its clips move; this picks the clip and the time
//! into it each frame:
//!
//! | Weapon | When | Kit state |
//! | --- | --- | --- |
//! | rifle, shotgun | just fired | `fire` from the shot |
//! | laser, flamethrower | firing (every tick the trigger is held) | `beam` / `fire`, looping |
//! | rail gun | just fired, then until it may fire again | `fire`, then `charge` over the rest of its refire |
//! | rocket launcher | just fired, then until it may fire again | `fire-a` / `fire-b` (the tubes alternate shot by shot), then `reload` over the rest of its refire |
//! | grapple | its hook set, then released | `launch` (the hook flies to where the host set it), then `retract` (it flies back) |
//! | blade | held or swinging, else put away | `extend` / `retract`; a quick-melee swing plays `swing` through the swing |
//!
//! **Glow channels.** A channel is a node whose scale is its level (1 lit as modelled, 0 hidden,
//! above 1 a flash). Between clips the `charge` channel shows the robot's energy pool (its share
//! of the pool's capacity) and the `fire` channel rests lit; the clips flash and drain them.
//! Refire times are the build's, its mods' tuning included ([`shooter_sim::robot::Tuning`]).

use super::kit::{Gear, Local, Rig};
use super::pose::RobotState;
use render_engine::glam::{Quat, Vec3};
use shooter_sim::weapons::{Instance, Weapon};

/// How long after its last tick's shot a continuous weapon still shows firing (seconds).
pub const CONTINUOUS_SECONDS: f32 = 0.1;

/// What a robot's weapons remember between frames: how long its hook has been set (or released)
/// and its blade out (or away).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Memory {
    /// Seconds drawn (the looping clips' clock).
    pub clock: f32,
    hooked: bool,
    /// Seconds since its hook was set or released.
    pub hook_for: f32,
    /// Where its hook last held (metres).
    pub last_hook: Option<Vec3>,
    blade_out: bool,
    /// Seconds since its blade came out or went away.
    pub blade_for: f32,
}

impl Memory {
    /// A robot first seen in `state`, every transition long done.
    #[must_use]
    pub fn settled(state: &RobotState) -> Self {
        Self {
            clock: 0.0,
            hooked: state.hook.is_some(),
            hook_for: f32::INFINITY,
            last_hook: state.hook,
            blade_out: blade_out(state),
            blade_for: f32::INFINITY,
        }
    }

    /// `seconds` later, in `state`.
    pub fn advance(&mut self, state: &RobotState, seconds: f32) {
        self.clock += seconds;
        self.hook_for += seconds;
        self.blade_for += seconds;
        if state.hook.is_some() != self.hooked {
            self.hooked = state.hook.is_some();
            self.hook_for = 0.0;
        }
        if state.hook.is_some() {
            self.last_hook = state.hook;
        }
        if blade_out(state) != self.blade_out {
            self.blade_out = !self.blade_out;
            self.blade_for = 0.0;
        }
    }
}

/// The blade is out while held or swinging.
fn blade_out(state: &RobotState) -> bool {
    state.held.weapon == Weapon::EnergyBlade || state.swing.is_some()
}

/// Where a grapple's hook is drawn this frame: toward `target` (metres) by `reach` (0 at its
/// clip's pose, 1 with its cable end on the target).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reach {
    pub target: Vec3,
    pub reach: f32,
}

/// `clip` of `rig` at `time` seconds into it, onto `locals`, if the rig has it.
fn play(rig: &Rig, clip: &str, time: f32, locals: &mut [Local]) {
    if let Some(clip) = rig.clip(clip) {
        clip.pose(time, locals);
    }
}

fn duration(rig: &Rig, clip: &str) -> f32 {
    rig.clip(clip).map_or(0.0, |clip| clip.duration)
}

/// Plays `first` for its length from `since`, then `then` stretched over the rest of `refire`.
fn then_recover(
    rig: &Rig,
    [first, then]: [&str; 2],
    since: f32,
    refire: f32,
    locals: &mut [Local],
) {
    let shot = duration(rig, first);
    if since < shot {
        play(rig, first, since, locals);
    } else if since < refire {
        let through = (since - shot) / (refire - shot).max(1e-3);
        play(rig, then, through * duration(rig, then), locals);
    }
}

/// The weapon copy `instance`'s joint transforms this frame (its rig `rig`, its descriptor `gear`,
/// `refire` seconds between its shots in this build), and where its hook reaches if it is the
/// grapple and its hook is out (in the world; the caller turns it into the weapon's frame).
#[must_use]
pub fn locals(
    rig: &Rig,
    gear: &Gear,
    instance: Instance,
    refire: f32,
    state: &RobotState,
    memory: &Memory,
) -> (Vec<Local>, Option<Reach>) {
    let mut locals = rig.rest_locals();
    if let Some(joint) = gear.charge.as_deref().and_then(|node| rig.joint(node)) {
        locals[joint].scale.y = state.energy.clamp(0.0, 1.0);
    }
    let weapon = instance.weapon;
    // Only the copy that fired shows the shot (shooter#125); copies that fire together each
    // show their own (shooter#127).
    let since = state
        .recent
        .since(instance.part)
        .unwrap_or(if state.fired == instance {
            state.since_fired
        } else {
            f32::INFINITY
        });
    let mut reach = None;
    match weapon {
        Weapon::EnergyRifle | Weapon::Shotgun => play(rig, "fire", since, &mut locals),
        Weapon::Laser | Weapon::Flamethrower if since < CONTINUOUS_SECONDS => {
            let clip = if weapon == Weapon::Laser {
                "beam"
            } else {
                "fire"
            };
            let length = duration(rig, clip).max(1e-3);
            play(rig, clip, memory.clock.rem_euclid(length), &mut locals);
        }
        Weapon::Laser | Weapon::Flamethrower => {}
        Weapon::RailGun => then_recover(rig, ["fire", "charge"], since, refire, &mut locals),
        Weapon::RocketLauncher => {
            let tube = if state.shots % 2 == 1 {
                "fire-a"
            } else {
                "fire-b"
            };
            then_recover(rig, [tube, "reload"], since, refire, &mut locals);
        }
        Weapon::Grapple => {
            let launch = duration(rig, "launch").max(1e-3);
            let retract = duration(rig, "retract").max(1e-3);
            if let Some(target) = state.hook {
                play(rig, "launch", memory.hook_for, &mut locals);
                reach = Some(Reach {
                    target,
                    reach: (memory.hook_for / launch).min(1.0),
                });
            } else if memory.hook_for < retract {
                play(rig, "retract", memory.hook_for, &mut locals);
                reach = memory.last_hook.map(|target| Reach {
                    target,
                    reach: 1.0 - memory.hook_for / retract,
                });
            }
        }
        Weapon::EnergyBlade => {
            let clip = if memory.blade_out {
                "extend"
            } else {
                "retract"
            };
            play(rig, clip, memory.blade_for, &mut locals);
            if let Some(progress) = state.swing {
                play(
                    rig,
                    "swing",
                    progress.clamp(0.0, 1.0) * duration(rig, "swing"),
                    &mut locals,
                );
            }
        }
    }
    (locals, reach)
}

/// Moves the grapple's `hook` node so that its `cable` anchor lies `reach` of the way from its
/// clip pose to `target` (the weapon's glTF frame), pointing along its flight.
pub fn reach_hook(rig: &Rig, gear: &Gear, locals: &mut [Local], target: Vec3, reach: f32) {
    let (Some(hook), Some(cable)) = (rig.joint("hook"), gear.anchor("cable")) else {
        return;
    };
    let parent = rig.joints[hook]
        .parent
        .map_or(rig.joints[hook].above, |parent| {
            rig.globals_of(locals)[parent]
        });
    let aim = parent.inverse().transform_point3(target);
    // The cable end in the hook's own frame.
    let offset = rig.joints[hook]
        .inverse_bind
        .transform_point3(cable.position);
    let from = locals[hook].translation;
    let rotation = Quat::from_rotation_arc(Vec3::Y, (aim - from).normalize_or(Vec3::Y));
    let translation = aim - rotation * (locals[hook].scale * offset);
    let reach = reach.clamp(0.0, 1.0);
    locals[hook].translation = from.lerp(translation, reach);
    locals[hook].rotation = locals[hook].rotation.slerp(rotation, reach).normalize();
}
