//! physics-engine's compound bodies and revolute joints, in centimetres (`length_unit` 100):
//! warm starting and sleep off, so a rebuilt world continues exactly.

use std::collections::BTreeMap;
use std::fmt;

use physics_engine::BodyId as EngineId;
use physics_engine::approximate::{
    Body, CombineRule, CompoundChild, Config, JointId, Motor, PreparedCompound, Quaternion,
    QueryFilter, RayCast, RevoluteJoint, Shape as EngineShape, Vector,
};

use super::super::engine::{UNITS_PER_METRE, array, engine_body, vector};
use super::super::{BodyId, PhysicsError, RigidBody, error};
use super::{BodySpec, Child, Drive, Hinge, HingeId, HingeState, Motion, PartHit, Pose, Shape};
use crate::TICK_HZ;

/// N·m in the engine's torque unit (kg · cm² / s²).
const TORQUE: f32 = UNITS_PER_METRE * UNITS_PER_METRE;

const fn quaternion(value: [f32; 4]) -> Quaternion {
    Quaternion(value[0], value[1], value[2], value[3])
}

const fn quad(value: Quaternion) -> [f32; 4] {
    [value.0, value.1, value.2, value.3]
}

fn engine_error(what: &str, failure: impl fmt::Debug) -> PhysicsError {
    error(format!("physics-engine {what} failed: {failure:?}"))
}

fn child(child: &Child) -> CompoundChild {
    CompoundChild {
        shape: match child.shape {
            Shape::Box(half) => EngineShape::Box(vector(half)),
            Shape::Sphere(radius) => EngineShape::Sphere(radius),
            Shape::Capsule {
                half_segment,
                radius,
            } => EngineShape::capsule(half_segment, radius),
        },
        position: vector(child.position),
        orientation: quaternion(child.orientation),
        mass: child.mass,
    }
}

fn motor(drive: Drive) -> Motor {
    match drive {
        Drive::Off => Motor::Off,
        Drive::Rate { rate, max_torque } => Motor::Velocity {
            rate,
            max_torque: max_torque * TORQUE,
        },
        Drive::Angle {
            angle,
            max_rate,
            max_torque,
        } => Motor::Position {
            angle,
            max_rate,
            max_torque: max_torque * TORQUE,
        },
    }
}

struct Entry {
    handle: EngineId,
    prepared: PreparedCompound,
}

pub(super) struct World {
    engine: physics_engine::approximate::World,
    bodies: BTreeMap<BodyId, Entry>,
    hinges: Vec<JointId>,
}

impl fmt::Debug for World {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Articulated")
            .field("backend", &"physics-engine")
            .field("bodies", &self.bodies.len())
            .field("hinges", &self.hinges.len())
            .finish_non_exhaustive()
    }
}

impl World {
    pub fn new(statics: &[RigidBody], gravity: [f32; 3]) -> Result<Self, PhysicsError> {
        let mut engine = physics_engine::approximate::World::new(Config {
            gravity: vector(gravity),
            length_unit: UNITS_PER_METRE,
            // Rapier's four substeps with four passes each, as the engine's robot oracles run.
            substeps: 4,
            velocity_iterations: 4,
            // A world rebuilt from read-back state continues exactly only without warm starts.
            warm_start: false,
            ..Config::default()
        })
        .map_err(|failure| engine_error("World::new", failure))?;
        for body in statics {
            engine
                .add_body(engine_body(body))
                .map_err(|failure| engine_error("add_body", failure))?;
        }
        Ok(Self {
            engine,
            bodies: BTreeMap::new(),
            hinges: Vec::new(),
        })
    }

    fn insert(
        &mut self,
        spec: BodySpec<'_>,
        place: impl FnOnce(&PreparedCompound, &mut Body),
    ) -> Result<(), PhysicsError> {
        if self.bodies.contains_key(&spec.id) {
            return Err(error(format!("physics body {} already exists", spec.id.0)));
        }
        let children: Vec<_> = spec.children.iter().map(child).collect();
        let prepared = PreparedCompound::new(&children)
            .map_err(|failure| engine_error("PreparedCompound::new", failure))?;
        // Shape and mass are the compound's; `add_compound` replaces them.
        let mut body = Body::keyed(spec.id.0, EngineShape::Sphere(1.0), Vector::ZERO, 1.0);
        body.friction = spec.friction;
        if spec.friction == 0.0 {
            body.friction_combine = CombineRule::Min;
        }
        body.sleep_allowed = false;
        place(&prepared, &mut body);
        let handle = self
            .engine
            .add_compound(&prepared, body)
            .map_err(|failure| engine_error("add_compound", failure))?;
        self.bodies.insert(spec.id, Entry { handle, prepared });
        Ok(())
    }

    pub fn add_body(&mut self, spec: BodySpec<'_>, origin: Pose) -> Result<(), PhysicsError> {
        self.insert(spec, |prepared, body| {
            let (position, orientation) = prepared.body_pose(
                vector(origin.position),
                quaternion(origin.orientation).normalized(),
            );
            body.position = position;
            body.orientation = orientation;
        })
    }

    pub fn restore_body(&mut self, spec: BodySpec<'_>, motion: Motion) -> Result<(), PhysicsError> {
        self.insert(spec, |_, body| {
            body.position = vector(motion.pose.position);
            body.orientation = quaternion(motion.pose.orientation);
            body.velocity = vector(motion.velocity);
            body.angular_velocity = vector(motion.angular_velocity);
        })
    }

    fn entry(&self, id: BodyId) -> Result<&Entry, PhysicsError> {
        self.bodies
            .get(&id)
            .ok_or_else(|| error(format!("articulated body {} is missing", id.0)))
    }

    pub fn add_hinge(&mut self, hinge: Hinge) -> Result<HingeId, PhysicsError> {
        let (a, b) = (self.entry(hinge.a)?, self.entry(hinge.b)?);
        let (anchor, axis) = (vector(hinge.anchor), vector(hinge.axis));
        // The angle's zero is the authored pose: one reference in the shared authored frame,
        // turned into each body's principal frame.
        let reference = RevoluteJoint::perpendicular(axis);
        let mut joint = RevoluteJoint::new(a.handle, b.handle, axis)
            .anchors(a.prepared.body_point(anchor), b.prepared.body_point(anchor))
            .axes(
                a.prepared.body_direction(axis),
                b.prepared.body_direction(axis),
            );
        joint.reference_a = a.prepared.body_direction(reference);
        joint.reference_b = b.prepared.body_direction(reference);
        if let Some([min, max]) = hinge.limits {
            joint = joint.limits(min, max);
        }
        let id = self
            .engine
            .add_revolute_joint(joint)
            .map_err(|failure| engine_error("add_revolute_joint", failure))?;
        self.hinges.push(id);
        Ok(HingeId(self.hinges.len() - 1))
    }

    pub fn set_drive(&mut self, hinge: HingeId, drive: Drive) -> Result<(), PhysicsError> {
        let id = *self
            .hinges
            .get(hinge.0)
            .ok_or_else(|| error(format!("hinge {} is missing", hinge.0)))?;
        self.engine
            .set_revolute_motor(id, motor(drive))
            .map_err(|failure| engine_error("set_revolute_motor", failure))
    }

    pub fn step(&mut self) -> Result<(), PhysicsError> {
        self.engine
            .step(1.0 / f64::from(TICK_HZ))
            .map(|_| ())
            .map_err(|failure| engine_error("step", failure))
    }

    pub fn motion(&self, id: BodyId) -> Option<Motion> {
        let body = self.engine.body(self.bodies.get(&id)?.handle)?;
        Some(Motion {
            pose: Pose {
                position: array(body.position),
                orientation: quad(body.orientation),
            },
            velocity: array(body.velocity),
            angular_velocity: array(body.angular_velocity),
        })
    }

    pub fn origin(&self, id: BodyId) -> Option<Pose> {
        let entry = self.bodies.get(&id)?;
        let body = self.engine.body(entry.handle)?;
        let (position, orientation) = entry.prepared.origin_pose(body.position, body.orientation);
        Some(Pose {
            position: array(position),
            orientation: quad(orientation),
        })
    }

    pub fn child_pose(&self, id: BodyId, child: usize) -> Option<Pose> {
        let (position, orientation) = self
            .engine
            .compound_child_pose(self.bodies.get(&id)?.handle, child)
            .ok()?;
        Some(Pose {
            position: array(position),
            orientation: quad(orientation),
        })
    }

    /// The linear velocity is set outright; the angular one through a couple of impulses
    /// whose angular impulse is the world inertia (the compound's principal inertia in its
    /// principal frame) times the change, so the step turns it into exactly that change.
    pub fn set_velocity(
        &mut self,
        id: BodyId,
        velocity: [f32; 3],
        angular_velocity: [f32; 3],
    ) -> Result<(), PhysicsError> {
        let entry = self.entry(id)?;
        let handle = entry.handle;
        let inertia = entry.prepared.principal_inertia();
        let body = self
            .engine
            .body(handle)
            .ok_or_else(|| error(format!("articulated body {} is missing", id.0)))?;
        let (position, orientation, current) =
            (body.position, body.orientation, body.angular_velocity);
        self.engine
            .set_velocity(handle, vector(velocity))
            .map_err(|failure| engine_error("set_velocity", failure))?;
        let change = vector(angular_velocity) - current;
        let local = orientation.inverse_rotate(change);
        let momentum = orientation.rotate(Vector(
            local.0 * inertia.0,
            local.1 * inertia.1,
            local.2 * inertia.2,
        ));
        let size = momentum.length();
        if size == 0.0 {
            return Ok(());
        }
        // A unit lever across the momentum, and the impulse that, applied there against its
        // opposite at the centre, gives exactly that angular impulse.
        let across = if momentum.0.abs() < 0.5 * size {
            Vector(1.0, 0.0, 0.0)
        } else {
            Vector(0.0, 1.0, 0.0)
        };
        let lever = momentum.cross(across).unit();
        let impulse = momentum.cross(lever);
        for (impulse, at) in [
            (impulse, position + lever),
            (Vector::ZERO - impulse, position),
        ] {
            self.engine
                .apply_impulse(handle, impulse, at)
                .map_err(|failure| engine_error("apply_impulse", failure))?;
        }
        Ok(())
    }

    pub fn supported(&self, id: BodyId) -> bool {
        self.bodies
            .get(&id)
            .is_some_and(|entry| self.engine.has_support(entry.handle))
    }

    pub fn hinge(&self, hinge: HingeId) -> Option<HingeState> {
        let state = self.engine.revolute_state(*self.hinges.get(hinge.0)?)?;
        Some(HingeState {
            angle: state.angle,
            rate: state.rate,
            torque: state.motor_torque / TORQUE,
            saturated: state.saturated,
        })
    }

    pub fn cast_ray(&self, origin: [f32; 3], translation: [f32; 3]) -> Option<PartHit> {
        let hit = self
            .engine
            .cast_ray_first(
                RayCast::new(vector(origin), vector(translation)),
                QueryFilter::default(),
            )
            .expect("physics-engine ray query failed")?;
        let body = self.engine.body(hit.body)?.key.map(BodyId)?;
        Some(PartHit {
            body,
            child: hit.child.map(|child| child as usize),
            fraction: hit.fraction,
            normal: array(hit.normal),
        })
    }
}
