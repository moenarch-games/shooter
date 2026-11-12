//! The shooter's physics world over `physics_engine::approximate::World`, with a Rapier oracle
//! behind the same API under the `physics-ab` feature (never in a client).
//!
//! Every moving body is an upright capsule character; everything else is static: the floor
//! half-space, boxes and ramps. A step moves every character once: gravity unless it stands on
//! ground and is not rising, a settle onto ground just below it, and a turn around touching
//! characters ([`slide_around`]); then landing or a ceiling ends its vertical velocity and its
//! horizontal velocity becomes what the move realized. Units are the game's centimetres (the
//! engine's `length_unit` is 100), velocities subunits per tick on the way in.
//!
//! Each step keeps the move it handed every character ([`World::moves`]), so a recorded match
//! can replay the same requests on another engine.

mod engine;
#[cfg(feature = "physics-ab")]
mod queries;
#[cfg(feature = "physics-ab")]
mod rapier;

use std::fmt;

pub use crate::fixed::SUBUNITS_PER_UNIT;

/// The gap a character keeps to everything it touches, in units. A moving body's backend
/// position is its game centre raised by this, so a body resting on the ground has its feet
/// exactly on the ground in game coordinates.
pub const CONTACT_OFFSET_UNITS: f32 = 0.25;
/// Ground within the contact offset plus this (5 cm) counts as support, as in Rapier.
const GROUND_PREDICTION_UNITS: f32 = 5.0;
/// A ceiling stops a rising body when it realizes less than this much (units) of its rise.
const CEILING_TOLERANCE_UNITS: f32 = 1.0e-3;
/// Approaches within 60 degrees of head-on step fully aside in [`slide_around`].
const NEAR_HEAD_ON_COS: f32 = 0.5;
/// The outward motion (units per tick: 30 units/s) [`slide_around`] adds against a touching
/// body.
const SEPARATION_UNITS: f32 = 30.0 / crate::TICK_HZ as f32;

/// An integer vector in units (or subunits, where a name says so).
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Vec3i {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl Vec3i {
    pub const ZERO: Self = Self::new(0, 0, 0);

    #[must_use]
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }
}

/// A game-chosen body identity.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BodyId(pub u64);

/// The first body a ray met: its identity, the fraction of the ray travelled and the surface
/// normal there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    pub body: BodyId,
    pub fraction: f32,
    pub normal: [f32; 3],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhysicsError(String);

impl fmt::Display for PhysicsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for PhysicsError {}

fn error(message: impl Into<String>) -> PhysicsError {
    PhysicsError(message.into())
}

/// Automatic step-up for every character, in units: a grounded body walking into a ledge at
/// most `max_height` above its feet, with at least `min_width` of free depth on top, climbs it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StepUp {
    pub max_height: i32,
    pub min_width: i32,
}

/// The horizontal direction a ramp's top rises toward.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RampRise {
    PosX,
    NegX,
    PosZ,
    NegZ,
}

impl RampRise {
    /// The unit direction `(x, z)` the top rises toward.
    #[must_use]
    pub const fn direction(self) -> [i32; 2] {
        match self {
            Self::PosX => [1, 0],
            Self::NegX => [-1, 0],
            Self::PosZ => [0, 1],
            Self::NegZ => [0, -1],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Floor,
    Fixed,
    Ramp(RampRise),
    Moving,
}

/// One body: static geometry, or a character's capsule with its `f32` state.
#[derive(Clone, Debug, PartialEq)]
pub struct RigidBody {
    id: BodyId,
    kind: Kind,
    /// Centre, in units.
    position: [f32; 3],
    /// Units per tick; zero for static bodies.
    velocity: [f32; 3],
    half_extents: Vec3i,
}

#[allow(clippy::cast_precision_loss)]
const fn exact(value: Vec3i) -> [f32; 3] {
    [value.x as f32, value.y as f32, value.z as f32]
}

#[allow(clippy::cast_precision_loss)]
const fn from_subunits(value: Vec3i) -> [f32; 3] {
    let scale = SUBUNITS_PER_UNIT as f32;
    [
        value.x as f32 / scale,
        value.y as f32 / scale,
        value.z as f32 / scale,
    ]
}

/// Rounded to whole values; positions and velocities stay far inside `i32`.
#[allow(clippy::cast_possible_truncation)]
fn round(value: [f32; 3]) -> Vec3i {
    Vec3i::new(
        value[0].round() as i32,
        value[1].round() as i32,
        value[2].round() as i32,
    )
}

/// A character's upright capsule as `(half segment, radius)` in units: as wide and tall as its
/// box, never reaching out of it.
#[allow(clippy::cast_precision_loss)]
fn capsule_dimensions(half_extents: Vec3i) -> (f32, f32) {
    let half_height = half_extents.y as f32;
    let radius = (half_extents.x.max(half_extents.z) as f32).min(half_height);
    (half_height - radius, radius)
}

impl RigidBody {
    /// A fixed box centred on `position`.
    #[must_use]
    pub const fn fixed(id: BodyId, position: Vec3i, half_extents: Vec3i) -> Self {
        Self::fixed_kind(id, Kind::Fixed, position, half_extents)
    }

    /// A flat floor: everything below height `y` is solid.
    #[must_use]
    pub const fn floor(id: BodyId, y: i32) -> Self {
        Self::fixed_kind(id, Kind::Floor, Vec3i::new(0, y, 0), Vec3i::ZERO)
    }

    /// A fixed ramp: a wedge filling the box centred on `position`, its top rising from the
    /// bottom at the low end to the full height toward `rises`.
    #[must_use]
    pub const fn ramp(id: BodyId, position: Vec3i, half_extents: Vec3i, rises: RampRise) -> Self {
        Self::fixed_kind(id, Kind::Ramp(rises), position, half_extents)
    }

    const fn fixed_kind(id: BodyId, kind: Kind, position: Vec3i, half_extents: Vec3i) -> Self {
        Self {
            id,
            kind,
            position: exact(position),
            velocity: [0.0; 3],
            half_extents,
        }
    }

    /// A character at a whole-unit position and a velocity in subunits per tick.
    #[must_use]
    pub const fn dynamic(
        id: BodyId,
        position: Vec3i,
        velocity: Vec3i,
        half_extents: Vec3i,
    ) -> Self {
        Self::moving(id, exact(position), from_subunits(velocity), half_extents)
    }

    /// A character at an exact position (units) and velocity (units per tick).
    #[must_use]
    pub const fn moving(
        id: BodyId,
        position: [f32; 3],
        velocity: [f32; 3],
        half_extents: Vec3i,
    ) -> Self {
        Self {
            id,
            kind: Kind::Moving,
            position,
            velocity,
            half_extents,
        }
    }

    #[must_use]
    pub const fn id(&self) -> BodyId {
        self.id
    }

    /// The centre rounded to whole units, as gameplay rules see it.
    #[must_use]
    pub fn position(&self) -> Vec3i {
        round(self.position)
    }

    /// The velocity in subunits per tick.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn velocity(&self) -> Vec3i {
        round(self.velocity.map(|axis| axis * SUBUNITS_PER_UNIT as f32))
    }

    /// The centre, in units.
    #[must_use]
    pub const fn exact_position(&self) -> [f32; 3] {
        self.position
    }

    /// The velocity, in units per tick.
    #[must_use]
    pub const fn exact_velocity(&self) -> [f32; 3] {
        self.velocity
    }

    #[must_use]
    pub const fn half_extents(&self) -> Vec3i {
        self.half_extents
    }

    #[must_use]
    pub const fn is_moving(&self) -> bool {
        matches!(self.kind, Kind::Moving)
    }

    fn check_finite(&self) -> Result<(), PhysicsError> {
        if self
            .position
            .iter()
            .chain(&self.velocity)
            .all(|value| value.is_finite())
        {
            Ok(())
        } else {
            Err(error("physics body state must be finite"))
        }
    }
}

/// The move one step handed a character: the displacement asked of the controller (gravity,
/// the ground settle and [`slide_around`] included), in units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveRequest {
    pub body: BodyId,
    pub desired: [f32; 3],
}

/// `desired` with the horizontal part that presses into a touching character turned along it,
/// so characters slide around each other instead of locking head-on (Rapier's controller only
/// stops a kinematic body at another). Within 60 degrees of head-on, both step aside to
/// opposite sides at full speed. Both backends turn their moves the same way.
fn slide_around<'a>(
    bodies: impl IntoIterator<Item = &'a RigidBody>,
    body: &RigidBody,
    desired: [f32; 3],
) -> [f32; 3] {
    let [mut vx, vy, mut vz] = desired;
    let speed = vx.hypot(vz);
    if speed == 0.0 {
        return desired;
    }
    let radius = |half: Vec3i| capsule_dimensions(half).1;
    #[allow(clippy::cast_precision_loss)]
    let (low, high) = (
        body.position[1] - body.half_extents.y as f32,
        body.position[1] + body.half_extents.y as f32,
    );
    for other in bodies {
        if other.id == body.id || !other.is_moving() {
            continue;
        }
        #[allow(clippy::cast_precision_loss)]
        let other_half_y = other.half_extents.y as f32;
        if other.position[1] + other_half_y < low || other.position[1] - other_half_y > high {
            continue;
        }
        let (dx, dz) = (
            body.position[0] - other.position[0],
            body.position[2] - other.position[2],
        );
        let distance = dx.hypot(dz);
        let reach =
            radius(body.half_extents) + radius(other.half_extents) + CONTACT_OFFSET_UNITS + 1.0;
        if distance == 0.0 || distance > reach {
            continue;
        }
        // `n` points from the other body to this one; only motion against it is turned.
        let (nx, nz) = (dx / distance, dz / distance);
        let into = vx * nx + vz * nz;
        if into >= 0.0 {
            continue;
        }
        // The tangent (-nz, nx) is mirrored for the other body, whose `n` is opposite.
        let (tx, tz) = (-nz, nx);
        let along = vx * tx + vz * tz;
        let along = if into < -NEAR_HEAD_ON_COS * speed {
            // Near head-on, step aside at full speed, to the side the motion already leans
            // or, exactly head-on, the tangent's side (opposite for the two bodies).
            let side = if along.abs() > 1.0e-3 * speed {
                along.signum()
            } else {
                1.0
            };
            side * speed
        } else {
            along
        };
        vx = along * tx + SEPARATION_UNITS * nx;
        vz = along * tz + SEPARATION_UNITS * nz;
    }
    [vx, vy, vz]
}

/// The physics backend of a [`World`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Backend {
    Engine,
    #[cfg(feature = "physics-ab")]
    Rapier,
}

#[cfg(feature = "physics-ab")]
thread_local! {
    static BACKEND: std::cell::Cell<Backend> = const { std::cell::Cell::new(Backend::Engine) };
}

/// Runs `run` with every [`World::new`] on this thread built on `backend` (the A/B tests).
#[cfg(feature = "physics-ab")]
pub fn using_backend<R>(backend: Backend, run: impl FnOnce() -> R) -> R {
    let previous = BACKEND.with(|cell| cell.replace(backend));
    let result = run();
    BACKEND.with(|cell| cell.set(previous));
    result
}

// The engine variant is the production path and stays inline; only the oracle is boxed.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
enum Inner {
    Engine(engine::World),
    #[cfg(feature = "physics-ab")]
    Rapier(Box<rapier::World>),
}

/// The arena's physics world.
#[derive(Debug)]
pub struct World(Inner);

macro_rules! dispatch {
    ($self:expr, $world:ident => $body:expr) => {
        match $self {
            Inner::Engine($world) => $body,
            #[cfg(feature = "physics-ab")]
            Inner::Rapier($world) => $body,
        }
    };
}

impl World {
    /// An empty world; `gravity` is in subunits per tick squared.
    #[must_use]
    pub fn new(gravity: Vec3i) -> Self {
        #[cfg(feature = "physics-ab")]
        if BACKEND.with(std::cell::Cell::get) == Backend::Rapier {
            return Self(Inner::Rapier(Box::new(rapier::World::new(gravity))));
        }
        Self(Inner::Engine(engine::World::new(gravity)))
    }

    #[must_use]
    pub const fn backend(&self) -> Backend {
        match self.0 {
            Inner::Engine(_) => Backend::Engine,
            #[cfg(feature = "physics-ab")]
            Inner::Rapier(_) => Backend::Rapier,
        }
    }

    /// Adds `body`.
    ///
    /// # Errors
    ///
    /// If its id is taken, its state is not finite, or the backend refuses it.
    pub fn add_body(&mut self, body: RigidBody) -> Result<(), PhysicsError> {
        body.check_finite()?;
        dispatch!(&mut self.0, world => world.add_body(body))
    }

    pub fn remove_body(&mut self, id: BodyId) -> Option<RigidBody> {
        dispatch!(&mut self.0, world => world.remove_body(id))
    }

    #[must_use]
    pub fn body(&self, id: BodyId) -> Option<&RigidBody> {
        dispatch!(&self.0, world => world.body(id))
    }

    /// Sets a character's velocity, in subunits per tick.
    ///
    /// # Errors
    ///
    /// If `id` is not a character.
    pub fn set_velocity(&mut self, id: BodyId, velocity: Vec3i) -> Result<(), PhysicsError> {
        dispatch!(&mut self.0, world => world.set_velocity(id, from_subunits(velocity)))
    }

    pub fn set_step_up(&mut self, step_up: Option<StepUp>) {
        dispatch!(&mut self.0, world => world.set_step_up(step_up));
    }

    /// Moves every character once (see the module docs).
    ///
    /// # Errors
    ///
    /// If the backend fails a move.
    pub fn step(&mut self) -> Result<(), PhysicsError> {
        dispatch!(&mut self.0, world => world.step())
    }

    /// The move each character was handed in the last step, in body order.
    #[must_use]
    pub fn moves(&self) -> &[MoveRequest] {
        dispatch!(&self.0, world => world.moves())
    }

    /// Whether character `id` stands on walkable static ground.
    #[must_use]
    pub fn grounded(&self, id: BodyId) -> bool {
        dispatch!(&self.0, world => world.grounded(id))
    }

    /// The first body accepted by `hits` on the segment from `origin` along `translation`
    /// (units); ties go to the lower body id.
    pub fn cast_ray(
        &self,
        origin: [f32; 3],
        translation: [f32; 3],
        hits: impl Fn(BodyId) -> bool,
    ) -> Option<RayHit> {
        if translation == [0.0; 3] {
            return None;
        }
        dispatch!(&self.0, world => world.cast_ray(origin, translation, &hits))
    }

    /// Every body overlapping the box, in id order.
    ///
    /// # Errors
    ///
    /// If the backend fails the query.
    pub fn overlap_box(
        &self,
        centre: [f32; 3],
        half_extents: [f32; 3],
    ) -> Result<Vec<BodyId>, PhysicsError> {
        dispatch!(&self.0, world => world.overlap_box(centre, half_extents))
    }
}
