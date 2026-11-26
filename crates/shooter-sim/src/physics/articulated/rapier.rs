//! The Rapier oracle of the articulated world (`physics-ab` only), in metres: a dynamic body
//! per game body with one collider a child (Rapier derives the mass properties from the
//! colliders' masses), impulse revolute joints with motors and contacts between the joined
//! bodies off, as physics-engine's own `revolute-robots` oracle builds them. The body's frame
//! is the authored frame, so each joint's local anchors and axes are the authored ones.

use std::collections::BTreeMap;
use std::fmt;

use physics_engine::approximate::{
    CompoundChild, PreparedCompound, Quaternion, Shape as EngineShape, Vector as EngineVector,
};
use rapier3d::parry::query::DefaultQueryDispatcher;
use rapier3d::prelude::{
    CoefficientCombineRule, ColliderBuilder, ImpulseJointHandle, JointAxis, PhysicsWorld, Pose,
    QueryFilter, Ray, RevoluteJoint, RevoluteJointBuilder, RigidBodyBuilder, RigidBodyHandle,
    Rotation, SharedShape, Vector,
};

use super::super::rapier::{TICK_SECONDS, UNITS_PER_METRE, metres, units, wedge};
use super::super::{BodyId, Kind, PhysicsError, RigidBody, error, exact};
use super::{
    BodySpec, Drive, Hinge, HingeId, HingeState, Motion, PartHit, Pose as GamePose, Shape,
};

/// Rapier's servo spring (acceleration-based) and velocity-motor factor, as physics-engine's
/// `revolute-robots` oracle sets them.
const SERVO_STIFFNESS: f32 = 1.0e5;
const SERVO_DAMPING: f32 = 630.0;
const VELOCITY_FACTOR: f32 = 1000.0;
/// How far apart (metres) a contact's points may be at the poses after the step for it to still
/// support a body: Rapier finds its contacts before it moves the bodies, so a body thrown off the
/// ground that tick still has them (physics-engine's own read their state after the step). 5 cm,
/// the kinematic world's ground prediction ([`super::super::World`]): a thrown robot leaves it in
/// its first tick, a robot's legs giving on Rapier's servo spring do not.
const SUPPORT_GAP_METRES: f32 = 0.05;

fn rotation(value: [f32; 4]) -> Rotation {
    Rotation::from_xyzw(value[0], value[1], value[2], value[3]).normalize()
}

fn quad(value: Rotation) -> [f32; 4] {
    [value.x, value.y, value.z, value.w]
}

/// A collider's user data: the game body in the high bits, its child plus one in the low 32
/// (zero for static geometry).
fn tag(body: BodyId, child: Option<usize>) -> u128 {
    (u128::from(body.0) << 32) | child.map_or(0, |child| child as u128 + 1)
}

fn untag(data: u128) -> (BodyId, Option<usize>) {
    let child = (data & 0xffff_ffff) as usize;
    (
        BodyId(u64::try_from(data >> 32).unwrap_or(u64::MAX)),
        child.checked_sub(1),
    )
}

/// The engine's mass properties of the same children: only to express Rapier's pose as the
/// centre-of-mass state [`Motion`] is.
fn prepared(spec: &BodySpec<'_>) -> Result<PreparedCompound, PhysicsError> {
    let children: Vec<_> = spec
        .children
        .iter()
        .map(|child| CompoundChild {
            shape: match child.shape {
                Shape::Box(half) => EngineShape::Box(EngineVector(half[0], half[1], half[2])),
                Shape::Sphere(radius) => EngineShape::Sphere(radius),
                Shape::Capsule {
                    half_segment,
                    radius,
                } => EngineShape::capsule(half_segment, radius),
            },
            position: EngineVector(child.position[0], child.position[1], child.position[2]),
            orientation: Quaternion(
                child.orientation[0],
                child.orientation[1],
                child.orientation[2],
                child.orientation[3],
            ),
            mass: child.mass,
        })
        .collect();
    PreparedCompound::new(&children)
        .map_err(|failure| error(format!("compound mass properties failed: {failure:?}")))
}

struct Entry {
    handle: RigidBodyHandle,
    prepared: PreparedCompound,
}

struct Joint {
    handle: ImpulseJointHandle,
    a: RigidBodyHandle,
    b: RigidBodyHandle,
    axis: Vector,
}

pub(super) struct World {
    world: PhysicsWorld,
    bodies: BTreeMap<BodyId, Entry>,
    hinges: Vec<Joint>,
}

impl fmt::Debug for World {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Articulated")
            .field("backend", &"rapier")
            .field("bodies", &self.bodies.len())
            .finish_non_exhaustive()
    }
}

impl World {
    pub fn new(statics: &[RigidBody], gravity: [f32; 3]) -> Self {
        let mut world = PhysicsWorld::new();
        world.gravity = Vector::from_array(gravity);
        world.integration_parameters.dt = TICK_SECONDS;
        for body in statics {
            let shape = match body.kind {
                Kind::Floor => SharedShape::halfspace(Vector::Y),
                Kind::Ramp(rises) => wedge(body.half_extents, rises),
                Kind::Fixed | Kind::Moving => SharedShape::cuboid(
                    body.half_extents.x as f32 / UNITS_PER_METRE,
                    body.half_extents.y as f32 / UNITS_PER_METRE,
                    body.half_extents.z as f32 / UNITS_PER_METRE,
                ),
            };
            world.colliders.insert(
                ColliderBuilder::new(shape)
                    .translation(metres(exact(body.position())))
                    .user_data(tag(body.id, None))
                    .build(),
            );
        }
        Self {
            world,
            bodies: BTreeMap::new(),
            hinges: Vec::new(),
        }
    }

    fn insert(
        &mut self,
        spec: BodySpec<'_>,
        origin: GamePose,
        velocity: [f32; 3],
        angular_velocity: [f32; 3],
        prepared: PreparedCompound,
    ) -> Result<(), PhysicsError> {
        if self.bodies.contains_key(&spec.id) {
            return Err(error(format!("physics body {} already exists", spec.id.0)));
        }
        let handle = self.world.bodies.insert(
            RigidBodyBuilder::dynamic()
                .pose(Pose::from_parts(
                    metres(origin.position),
                    rotation(origin.orientation),
                ))
                .linvel(metres(velocity))
                .angvel(Vector::from_array(angular_velocity))
                .can_sleep(false),
        );
        for (index, child) in spec.children.iter().enumerate() {
            let shape = match child.shape {
                Shape::Box(half) => {
                    let half = metres(half);
                    SharedShape::cuboid(half.x, half.y, half.z)
                }
                Shape::Sphere(radius) => SharedShape::ball(radius / UNITS_PER_METRE),
                Shape::Capsule {
                    half_segment,
                    radius,
                } => {
                    SharedShape::capsule_y(half_segment / UNITS_PER_METRE, radius / UNITS_PER_METRE)
                }
            };
            self.world.colliders.insert_with_parent(
                ColliderBuilder::new(shape)
                    .position(Pose::from_parts(
                        metres(child.position),
                        rotation(child.orientation),
                    ))
                    .mass(child.mass)
                    .friction(spec.friction)
                    .friction_combine_rule(if spec.friction == 0.0 {
                        CoefficientCombineRule::Min
                    } else {
                        CoefficientCombineRule::Average
                    })
                    .user_data(tag(spec.id, Some(index)))
                    .build(),
                handle,
                &mut self.world.bodies,
            );
        }
        self.bodies.insert(spec.id, Entry { handle, prepared });
        Ok(())
    }

    pub fn add_body(&mut self, spec: BodySpec<'_>, origin: GamePose) -> Result<(), PhysicsError> {
        let prepared = prepared(&spec)?;
        self.insert(spec, origin, [0.0; 3], [0.0; 3], prepared)
    }

    pub fn restore_body(&mut self, spec: BodySpec<'_>, motion: Motion) -> Result<(), PhysicsError> {
        let prepared = prepared(&spec)?;
        let p = motion.pose.position;
        let q = motion.pose.orientation;
        let (position, orientation) = prepared.origin_pose(
            EngineVector(p[0], p[1], p[2]),
            Quaternion(q[0], q[1], q[2], q[3]),
        );
        let origin = GamePose {
            position: [position.0, position.1, position.2],
            orientation: [orientation.0, orientation.1, orientation.2, orientation.3],
        };
        self.insert(
            spec,
            origin,
            motion.velocity,
            motion.angular_velocity,
            prepared,
        )
    }

    fn handle(&self, id: BodyId) -> Result<RigidBodyHandle, PhysicsError> {
        self.bodies
            .get(&id)
            .map(|entry| entry.handle)
            .ok_or_else(|| error(format!("articulated body {} is missing", id.0)))
    }

    pub fn add_hinge(&mut self, hinge: Hinge) -> Result<HingeId, PhysicsError> {
        let (a, b) = (self.handle(hinge.a)?, self.handle(hinge.b)?);
        let axis = Vector::from_array(hinge.axis).normalize();
        let anchor = metres(hinge.anchor);
        let mut builder = RevoluteJointBuilder::new(axis)
            .local_anchor1(anchor)
            .local_anchor2(anchor)
            .contacts_enabled(false);
        if let Some(limits) = hinge.limits {
            builder = builder.limits(limits);
        }
        let handle = self.world.insert_impulse_joint(a, b, builder);
        self.hinges.push(Joint { handle, a, b, axis });
        Ok(HingeId(self.hinges.len() - 1))
    }

    pub fn set_drive(&mut self, hinge: HingeId, drive: Drive) -> Result<(), PhysicsError> {
        let handle = self
            .hinges
            .get(hinge.0)
            .ok_or_else(|| error(format!("hinge {} is missing", hinge.0)))?
            .handle;
        let joint = &mut self
            .world
            .impulse_joints
            .get_mut(handle, true)
            .ok_or_else(|| error("rapier joint is missing"))?
            .data;
        match drive {
            Drive::Off => {
                joint.set_motor_velocity(JointAxis::AngX, 0.0, 0.0);
                joint.set_motor_max_force(JointAxis::AngX, 0.0);
            }
            Drive::Rate { rate, max_torque } => {
                joint.set_motor_velocity(JointAxis::AngX, rate, VELOCITY_FACTOR);
                joint.set_motor_max_force(JointAxis::AngX, max_torque);
            }
            Drive::Angle {
                angle, max_torque, ..
            } => {
                joint.set_motor_position(JointAxis::AngX, angle, SERVO_STIFFNESS, SERVO_DAMPING);
                joint.set_motor_max_force(JointAxis::AngX, max_torque);
            }
        }
        Ok(())
    }

    pub fn step(&mut self) -> Result<(), PhysicsError> {
        self.world.step();
        Ok(())
    }

    pub fn motion(&self, id: BodyId) -> Option<Motion> {
        let entry = self.bodies.get(&id)?;
        let body = self.world.bodies.get(entry.handle)?;
        let origin = units(body.translation());
        let q = quad(*body.rotation());
        let (position, orientation) = entry.prepared.body_pose(
            EngineVector(origin[0], origin[1], origin[2]),
            Quaternion(q[0], q[1], q[2], q[3]),
        );
        Some(Motion {
            pose: GamePose {
                position: [position.0, position.1, position.2],
                orientation: [orientation.0, orientation.1, orientation.2, orientation.3],
            },
            velocity: units(body.linvel()),
            angular_velocity: body.angvel().to_array(),
        })
    }

    pub fn origin(&self, id: BodyId) -> Option<GamePose> {
        let body = self.world.bodies.get(self.bodies.get(&id)?.handle)?;
        Some(GamePose {
            position: units(body.translation()),
            orientation: quad(*body.rotation()),
        })
    }

    pub fn child_pose(&self, id: BodyId, child: usize) -> Option<GamePose> {
        let body = self.world.bodies.get(self.bodies.get(&id)?.handle)?;
        let collider = body
            .colliders()
            .iter()
            .filter_map(|&handle| self.world.colliders.get(handle))
            .find(|collider| untag(collider.user_data) == (id, Some(child)))?;
        Some(GamePose {
            position: units(collider.translation()),
            orientation: quad(collider.rotation()),
        })
    }

    pub fn set_velocity(
        &mut self,
        id: BodyId,
        velocity: [f32; 3],
        angular_velocity: [f32; 3],
    ) -> Result<(), PhysicsError> {
        let handle = self.handle(id)?;
        let body = self
            .world
            .bodies
            .get_mut(handle)
            .ok_or_else(|| error("rapier body is missing"))?;
        body.set_linvel(metres(velocity), true);
        body.set_angvel(Vector::from_array(angular_velocity), true);
        Ok(())
    }

    /// A contact of any of the body's colliders whose normal (from it to the other) points
    /// mostly down.
    pub fn supported(&self, id: BodyId) -> bool {
        let Some(body) = self
            .bodies
            .get(&id)
            .and_then(|entry| self.world.bodies.get(entry.handle))
        else {
            return false;
        };
        body.colliders().iter().any(|&collider| {
            self.world
                .narrow_phase
                .contact_pairs_with(collider)
                .filter(|pair| pair.has_any_active_contact())
                .any(|pair| {
                    let sign = if pair.collider1 == collider {
                        1.0
                    } else {
                        -1.0
                    };
                    let (Some(one), Some(two)) = (
                        self.world.colliders.get(pair.collider1),
                        self.world.colliders.get(pair.collider2),
                    ) else {
                        return false;
                    };
                    let (one, two) = (one.position(), two.position());
                    pair.manifolds().iter().any(|manifold| {
                        let normal = one.transform_vector(manifold.local_n1);
                        sign * normal.y < -0.5
                            && manifold.points.iter().any(|point| {
                                let gap = two.transform_point(point.local_p2)
                                    - one.transform_point(point.local_p1);
                                gap.dot(normal) <= SUPPORT_GAP_METRES
                            })
                    })
                })
        })
    }

    pub fn hinge(&self, hinge: HingeId) -> Option<HingeState> {
        let joint = self.hinges.get(hinge.0)?;
        let data = self.world.impulse_joints.get(joint.handle)?.data;
        let (a, b) = (
            self.world.bodies.get(joint.a)?,
            self.world.bodies.get(joint.b)?,
        );
        let substep = TICK_SECONDS / self.world.integration_parameters.num_solver_iterations as f32;
        let motor = data.motor(JointAxis::AngX);
        // Rapier reports the motor's impulse with the opposite sign.
        let torque = motor.map_or(0.0, |motor| -motor.impulse / substep);
        let max = motor.map_or(0.0, |motor| motor.max_force);
        Some(HingeState {
            angle: RevoluteJoint { data }.angle(a.rotation(), b.rotation()),
            rate: (b.angvel() - a.angvel()).dot(*a.rotation() * joint.axis),
            torque,
            saturated: max > 0.0 && torque.abs() >= 0.999 * max,
        })
    }

    pub fn cast_ray(&self, origin: [f32; 3], translation: [f32; 3]) -> Option<PartHit> {
        let ray = Ray::new(metres(origin), metres(translation));
        let queries = self.world.broad_phase.as_query_pipeline(
            &DefaultQueryDispatcher,
            &self.world.bodies,
            &self.world.colliders,
            QueryFilter::default(),
        );
        let (handle, hit) = queries.cast_ray_and_get_normal(&ray, 1.0, true)?;
        let (body, child) = untag(self.world.colliders.get(handle)?.user_data);
        Some(PartHit {
            body,
            child,
            fraction: hit.time_of_impact,
            normal: hit.normal.to_array(),
        })
    }
}
