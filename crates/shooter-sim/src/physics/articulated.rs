//! The robots' dynamic world (Atlas D2.i70.5, D4.i70.1): compound rigid bodies joined by
//! motorised revolute joints, under gravity, on the arena's static geometry. physics-engine
//! simulates it (its compound bodies, #567, and revolute joints with motors, #568); the Rapier
//! oracle mirrors it under `physics-ab` (one dynamic body with a collider a child, impulse
//! revolute joints with motors).
//!
//! This is a different world from [`super::World`], whose characters are kinematic capsules the
//! shooter moves itself: here the engine integrates every body.
//!
//! **Bodies.** A body is a list of [`Child`] shapes with their masses, authored in one frame
//! (the robot's): its centre of mass, inertia and stability come from where each mass is. The
//! engine keeps a compound at its centre of mass in its principal frame ([`Motion`]);
//! [`Articulated::origin`] gives the authored frame's pose back.
//!
//! **Hinges.** A [`Hinge`] joins two bodies authored in the same frame at an anchor about an
//! axis given in that frame; its angle is zero in the authored pose. Each tick the game sets
//! every hinge's [`Drive`] (a rate or an angle, with the most torque it may use) and reads its
//! [`HingeState`] back after the step: angle, rate, the torque the motor gave and whether it
//! was at its limit (with the rate far from the command, a stall).
//!
//! **Exact continuation.** The engine runs without warm starting and without sleep, so a world
//! rebuilt from the bodies' read-back [`Motion`]s ([`Articulated::restore_body`]), the same
//! hinges added in the same order and the same drives steps on bit for bit like the original:
//! the basis of rollback, prediction and replay. The oracle does not promise that.
//!
//! Units: positions in game units (centimetres), velocities in units a second, angles in
//! radians, rates in radians a second, masses in kilograms and torques in N·m.

mod engine;
#[cfg(feature = "physics-ab")]
mod rapier;

use super::{BodyId, PhysicsError, RigidBody};

/// A child shape in its body's authored frame (units).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// Half extents.
    Box([f32; 3]),
    Sphere(f32),
    /// Along its local Y: the segment's half length and the radius.
    Capsule {
        half_segment: f32,
        radius: f32,
    },
}

/// One shape of a body: where it is and how it is turned in the body's authored frame, and its
/// mass (kilograms).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Child {
    pub shape: Shape,
    pub position: [f32; 3],
    /// A unit quaternion `[x, y, z, w]`.
    pub orientation: [f32; 4],
    pub mass: f32,
}

/// A position and a unit quaternion `[x, y, z, w]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub position: [f32; 3],
    pub orientation: [f32; 4],
}

impl Pose {
    pub const IDENTITY: Self = Self {
        position: [0.0; 3],
        orientation: [0.0, 0.0, 0.0, 1.0],
    };
}

/// A body's exact dynamic state: its centre of mass and principal frame, its velocity (units a
/// second) and angular velocity (radians a second). Read back and restored bit for bit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub pose: Pose,
    pub velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
}

/// A body to add: its id, its children and its friction.
#[derive(Clone, Copy, Debug)]
pub struct BodySpec<'a> {
    pub id: BodyId,
    pub children: &'a [Child],
    pub friction: f32,
}

/// A revolute joint between bodies `a` and `b`, both authored in the frame `anchor` and `axis`
/// are given in, with optional angle limits (radians, within `[-π, π]`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hinge {
    pub a: BodyId,
    pub b: BodyId,
    pub anchor: [f32; 3],
    pub axis: [f32; 3],
    pub limits: Option<[f32; 2]>,
}

/// What drives a hinge this tick, `b` relative to `a` about the axis (right-handed).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Drive {
    /// Free (limits still hold).
    #[default]
    Off,
    /// Turn at `rate` with at most `max_torque`.
    Rate { rate: f32, max_torque: f32 },
    /// Turn toward `angle` at most `max_rate` with at most `max_torque`, and hold it.
    Angle {
        angle: f32,
        max_rate: f32,
        max_torque: f32,
    },
}

/// A hinge after the last step.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HingeState {
    pub angle: f32,
    pub rate: f32,
    /// The motor's torque about the axis (N·m), averaged over the step.
    pub torque: f32,
    /// Whether the motor gave its most torque.
    pub saturated: bool,
}

/// A hinge's handle: its index in the order hinges were added.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct HingeId(pub usize);

/// The first child of a body a ray met, the fraction of the ray travelled and the surface
/// normal there. `child` is `None` for static geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PartHit {
    pub body: BodyId,
    pub child: Option<usize>,
    pub fraction: f32,
    pub normal: [f32; 3],
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
enum Inner {
    Engine(engine::World),
    #[cfg(feature = "physics-ab")]
    Rapier(Box<rapier::World>),
}

/// The robots' dynamic world.
#[derive(Debug)]
pub struct Articulated(Inner);

macro_rules! dispatch {
    ($self:expr, $world:ident => $body:expr) => {
        match $self {
            Inner::Engine($world) => $body,
            #[cfg(feature = "physics-ab")]
            Inner::Rapier($world) => $body,
        }
    };
}

fn finite(values: &[f32]) -> Result<(), PhysicsError> {
    if values.iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err(super::error("articulated body state must be finite"))
    }
}

impl Articulated {
    /// A world of the static bodies `statics` (floor, boxes and ramps; moving bodies are
    /// refused) under gravity, on the backend [`super::using_backend`] chose.
    ///
    /// # Errors
    ///
    /// If a body is not static or the backend refuses one.
    pub fn new(statics: &[RigidBody]) -> Result<Self, PhysicsError> {
        if statics.iter().any(RigidBody::is_moving) {
            return Err(super::error("an articulated world's statics cannot move"));
        }
        #[cfg(feature = "physics-ab")]
        if super::BACKEND.with(std::cell::Cell::get) == super::Backend::Rapier {
            return Ok(Self(Inner::Rapier(Box::new(rapier::World::new(statics)))));
        }
        engine::World::new(statics).map(|world| Self(Inner::Engine(world)))
    }

    #[must_use]
    pub const fn backend(&self) -> super::Backend {
        match self.0 {
            Inner::Engine(_) => super::Backend::Engine,
            #[cfg(feature = "physics-ab")]
            Inner::Rapier(_) => super::Backend::Rapier,
        }
    }

    /// Adds a body at rest with its authored frame at `origin`.
    ///
    /// # Errors
    ///
    /// If its id is taken, it has no or invalid children, or the backend refuses it.
    pub fn add_body(&mut self, spec: BodySpec<'_>, origin: Pose) -> Result<(), PhysicsError> {
        finite(&origin.position)?;
        finite(&origin.orientation)?;
        dispatch!(&mut self.0, world => world.add_body(spec, origin))
    }

    /// Adds a body in the exact state `motion` (as [`Self::motion`] read it back).
    ///
    /// # Errors
    ///
    /// As [`Self::add_body`].
    pub fn restore_body(&mut self, spec: BodySpec<'_>, motion: Motion) -> Result<(), PhysicsError> {
        finite(&motion.pose.position)?;
        finite(&motion.pose.orientation)?;
        finite(&motion.velocity)?;
        finite(&motion.angular_velocity)?;
        dispatch!(&mut self.0, world => world.restore_body(spec, motion))
    }

    /// Joins two bodies.
    ///
    /// # Errors
    ///
    /// If a body is missing or the hinge is degenerate.
    pub fn add_hinge(&mut self, hinge: Hinge) -> Result<HingeId, PhysicsError> {
        dispatch!(&mut self.0, world => world.add_hinge(hinge))
    }

    /// Sets a hinge's drive for the next step.
    ///
    /// # Errors
    ///
    /// If the hinge is unknown or the drive is not finite or has a negative maximum.
    pub fn set_drive(&mut self, hinge: HingeId, drive: Drive) -> Result<(), PhysicsError> {
        dispatch!(&mut self.0, world => world.set_drive(hinge, drive))
    }

    /// One fixed tick.
    ///
    /// # Errors
    ///
    /// If the backend fails the step.
    pub fn step(&mut self) -> Result<(), PhysicsError> {
        dispatch!(&mut self.0, world => world.step())
    }

    /// Body `id`'s exact state.
    #[must_use]
    pub fn motion(&self, id: BodyId) -> Option<Motion> {
        dispatch!(&self.0, world => world.motion(id))
    }

    /// The pose of body `id`'s authored frame.
    #[must_use]
    pub fn origin(&self, id: BodyId) -> Option<Pose> {
        dispatch!(&self.0, world => world.origin(id))
    }

    /// The world pose of child `child` of body `id`.
    #[must_use]
    pub fn child_pose(&self, id: BodyId, child: usize) -> Option<Pose> {
        dispatch!(&self.0, world => world.child_pose(id, child))
    }

    #[must_use]
    pub fn hinge(&self, hinge: HingeId) -> Option<HingeState> {
        dispatch!(&self.0, world => world.hinge(hinge))
    }

    /// The first body on the segment from `origin` along `translation` (units).
    #[must_use]
    pub fn cast_ray(&self, origin: [f32; 3], translation: [f32; 3]) -> Option<PartHit> {
        if translation == [0.0; 3] {
            return None;
        }
        dispatch!(&self.0, world => world.cast_ray(origin, translation))
    }
}
