//! The Rapier oracle (`physics-ab` only): characters are kinematic position-based capsules moved
//! one after another by Rapier's `KinematicCharacterController`, each seeing the earlier movers
//! at their new positions. Rapier simulates in metres, the scale its tolerances assume.
//!
//! Rapier's broad phase depends on the history of insertions, removals and moves, and the
//! controller breaks exact ties by traversal order, so each step rebuilds the world from the
//! bodies in id order (from a cached copy of the static part) and is a function of game state
//! alone.

use std::collections::BTreeMap;
use std::fmt;

use rapier3d::control::{CharacterAutostep, CharacterLength, KinematicCharacterController};
use rapier3d::parry::query::{
    DefaultQueryDispatcher, QueryDispatcher, QueryDispatcherChain, ShapeCastOptions,
};
use rapier3d::prelude::{
    Capsule, Collider, ColliderBuilder, ColliderHandle, Cuboid, PhysicsWorld, Pose, QueryFilter,
    Ray, RigidBodyBuilder, RigidBodyHandle, SharedShape, Vector,
};

use super::queries::UprightShapes;
use super::{
    BodyId, CEILING_TOLERANCE_UNITS, CONTACT_OFFSET_UNITS, GROUND_PREDICTION_UNITS, Kind,
    MoveRequest, PhysicsError, RampRise, RayHit, RigidBody, StepUp, Vec3i, capsule_dimensions,
    error, exact, from_subunits, slide_around,
};

const UNITS_PER_METRE: f32 = 100.0;
const CONTACT_OFFSET: f32 = CONTACT_OFFSET_UNITS / UNITS_PER_METRE;
/// How far (metres) below a body the controller still finds ground: its contact offset plus
/// the ground prediction. Support queries and snapping use the same reach.
const SUPPORT_REACH: f32 = CONTACT_OFFSET + GROUND_PREDICTION_UNITS / UNITS_PER_METRE;
/// Ray hits this close (units) along the ray tie, broken by body id.
const TIE_UNITS: f32 = 1.0e-3;
/// A ground gap (units) below this counts as standing on it.
const GAP_TOLERANCE_UNITS: f32 = 1.0e-3;
const TICK_SECONDS: f32 = 1.0 / crate::TICK_HZ as f32;

fn metres(value: [f32; 3]) -> Vector {
    Vector::new(value[0], value[1], value[2]) / UNITS_PER_METRE
}

fn lifted(value: [f32; 3]) -> Vector {
    metres(value) + Vector::new(0.0, CONTACT_OFFSET, 0.0)
}

fn units(value: Vector) -> [f32; 3] {
    let value = value * UNITS_PER_METRE;
    [value.x, value.y, value.z]
}

fn empty_world() -> PhysicsWorld {
    let mut world = PhysicsWorld::new();
    world.gravity = Vector::ZERO;
    world.integration_parameters.dt = TICK_SECONDS;
    world
}

fn capsule(half_extents: Vec3i) -> Capsule {
    let (half_height, radius) = capsule_dimensions(half_extents);
    Capsule::new_y(half_height / UNITS_PER_METRE, radius / UNITS_PER_METRE)
}

/// A ramp's wedge in metres around its box centre: the box's bottom rectangle and its top edge
/// at the high end, as a convex hull.
fn wedge(half_extents: Vec3i, rises: RampRise) -> SharedShape {
    let [hx, hy, hz] = exact(half_extents).map(|value| value / UNITS_PER_METRE);
    let [dx, dz] = rises.direction();
    let mut points = Vec::with_capacity(6);
    for sx in [-1.0, 1.0] {
        for sz in [-1.0, 1.0] {
            points.push(Vector::new(sx * hx, -hy, sz * hz));
            #[allow(clippy::cast_precision_loss)]
            let high = (dx != 0 && sx == dx as f32) || (dz != 0 && sz == dz as f32);
            if high {
                points.push(Vector::new(sx * hx, hy, sz * hz));
            }
        }
    }
    SharedShape::convex_hull(&points).expect("a ramp's box spans a solid wedge")
}

/// Inserts `body`; a character's Rapier body handle comes back.
fn insert(world: &mut PhysicsWorld, body: &RigidBody) -> Option<RigidBodyHandle> {
    let user_data = u128::from(body.id.0);
    let fixed = |world: &mut PhysicsWorld, shape: SharedShape| {
        world.colliders.insert(
            ColliderBuilder::new(shape)
                .translation(metres(body.position))
                .user_data(user_data)
                .build(),
        );
        None
    };
    match body.kind {
        Kind::Moving => {
            let handle = world.bodies.insert(
                RigidBodyBuilder::kinematic_position_based()
                    .translation(lifted(body.position))
                    .build(),
            );
            world.colliders.insert_with_parent(
                ColliderBuilder::new(SharedShape::new(capsule(body.half_extents)))
                    .user_data(user_data)
                    .build(),
                handle,
                &mut world.bodies,
            );
            Some(handle)
        }
        Kind::Floor => fixed(world, SharedShape::halfspace(Vector::Y)),
        Kind::Ramp(rises) => fixed(world, wedge(body.half_extents, rises)),
        Kind::Fixed => fixed(
            world,
            SharedShape::new(Cuboid::new(metres(exact(body.half_extents)))),
        ),
    }
}

fn game(collider: &Collider) -> BodyId {
    BodyId(u64::try_from(collider.user_data).unwrap_or(u64::MAX))
}

pub(super) struct World {
    gravity: [f32; 3],
    world: PhysicsWorld,
    /// The static bodies' world, refreshed: rebuilt only when one of them changes.
    fixed: Option<PhysicsWorld>,
    /// Exact closed forms for upright capsules against boxes, capsules and the floor, ahead of
    /// parry's general GJK/EPA path ([`super::queries`]).
    queries: QueryDispatcherChain<UprightShapes, DefaultQueryDispatcher>,
    controller: KinematicCharacterController,
    bodies: BTreeMap<BodyId, RigidBody>,
    /// Each character's Rapier body in [`Self::world`].
    handles: BTreeMap<BodyId, RigidBodyHandle>,
    requests: Vec<MoveRequest>,
}

impl fmt::Debug for World {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("World")
            .field("backend", &"rapier")
            .field("bodies", &self.bodies.len())
            .finish_non_exhaustive()
    }
}

impl World {
    pub fn new(gravity: Vec3i) -> Self {
        Self {
            gravity: from_subunits(gravity),
            world: empty_world(),
            fixed: None,
            queries: UprightShapes.chain(DefaultQueryDispatcher),
            controller: KinematicCharacterController {
                up: Vector::Y,
                offset: CharacterLength::Absolute(CONTACT_OFFSET),
                slide: true,
                autostep: None,
                max_slope_climb_angle: std::f32::consts::FRAC_PI_4,
                min_slope_slide_angle: std::f32::consts::FRAC_PI_4,
                snap_to_ground: Some(CharacterLength::Absolute(SUPPORT_REACH)),
                normal_nudge_factor: KinematicCharacterController::default().normal_nudge_factor,
            },
            bodies: BTreeMap::new(),
            handles: BTreeMap::new(),
            requests: Vec::new(),
        }
    }

    pub fn add_body(&mut self, body: RigidBody) -> Result<(), PhysicsError> {
        if self.bodies.contains_key(&body.id) {
            return Err(error(format!("physics body {} already exists", body.id.0)));
        }
        if !body.is_moving() {
            self.fixed = None;
        }
        self.bodies.insert(body.id, body);
        self.rebuild();
        Ok(())
    }

    pub fn remove_body(&mut self, id: BodyId) -> Option<RigidBody> {
        let body = self.bodies.remove(&id)?;
        if !body.is_moving() {
            self.fixed = None;
        }
        self.rebuild();
        Some(body)
    }

    pub fn body(&self, id: BodyId) -> Option<&RigidBody> {
        self.bodies.get(&id)
    }

    pub fn set_velocity(&mut self, id: BodyId, velocity: [f32; 3]) -> Result<(), PhysicsError> {
        let body = self
            .bodies
            .get_mut(&id)
            .filter(|body| body.is_moving())
            .ok_or_else(|| error(format!("moving physics body {} is missing", id.0)))?;
        body.velocity = velocity;
        Ok(())
    }

    #[allow(clippy::cast_precision_loss)]
    pub fn set_step_up(&mut self, step_up: Option<StepUp>) {
        let metres = |units: i32| CharacterLength::Absolute(units as f32 / UNITS_PER_METRE);
        self.controller.autostep = step_up.map(|step| CharacterAutostep {
            max_height: metres(step.max_height),
            min_width: metres(step.min_width),
            include_dynamic_bodies: false,
        });
    }

    pub fn moves(&self) -> &[MoveRequest] {
        &self.requests
    }

    /// Rebuilds the world from the bodies: the static part (cached until one changes), then
    /// the characters in id order.
    fn rebuild(&mut self) {
        let bodies = &self.bodies;
        let fixed = self.fixed.get_or_insert_with(|| {
            let mut world = empty_world();
            for body in bodies.values().filter(|body| !body.is_moving()) {
                insert(&mut world, body);
            }
            world.detect_collisions(&(), &());
            world
        });
        let mut world = empty_world();
        world.bodies = fixed.bodies.clone();
        world.colliders = fixed.colliders.clone();
        world.broad_phase = fixed.broad_phase.clone();
        world.narrow_phase = fixed.narrow_phase.clone();
        world.islands = fixed.islands.clone();
        self.handles.clear();
        for body in self.bodies.values().filter(|body| body.is_moving()) {
            if let Some(handle) = insert(&mut world, body) {
                self.handles.insert(body.id, handle);
            }
        }
        self.world = world;
        self.world.detect_collisions(&(), &());
    }

    /// Places character `id` at `position` without resolving contact.
    fn place(&mut self, id: BodyId, position: [f32; 3]) -> Result<(), PhysicsError> {
        let body = self
            .bodies
            .get_mut(&id)
            .ok_or_else(|| error("moving body vanished"))?;
        if body.position == position {
            return Ok(());
        }
        body.position = position;
        if let Some(rapier) = self
            .handles
            .get(&id)
            .and_then(|&handle| self.world.bodies.get_mut(handle))
        {
            rapier.set_translation(lifted(position), false);
        }
        self.world.detect_collisions(&(), &());
        Ok(())
    }

    pub fn step(&mut self) -> Result<(), PhysicsError> {
        self.rebuild();
        self.requests.clear();
        let ids: Vec<_> = self
            .bodies
            .values()
            .filter(|body| body.is_moving())
            .map(|body| body.id)
            .collect();
        for id in ids {
            let body = self
                .bodies
                .get(&id)
                .ok_or_else(|| error("moving body vanished"))?;
            let mut velocity = body.velocity;
            // On the ground and not rising: no gravity (it would only press into the ground),
            // and a body just above the ground settles onto it.
            let mut settle = 0.0;
            match self.ground_gap(body, body.position) {
                Some(gap) if velocity[1] <= 0.0 => {
                    velocity[1] = 0.0;
                    if gap > GAP_TOLERANCE_UNITS {
                        settle = -gap;
                    }
                }
                _ => {
                    for (axis, gravity) in velocity.iter_mut().zip(self.gravity) {
                        *axis += gravity;
                    }
                }
            }
            let desired = [velocity[0], velocity[1] + settle, velocity[2]];
            // Asked to move nowhere, the controller still snaps and nudges by fractions of a
            // unit: a body with nowhere to go stays exactly where it is.
            if desired == [0.0; 3] {
                self.requests.push(MoveRequest { body: id, desired });
                self.bodies
                    .get_mut(&id)
                    .ok_or_else(|| error("moving body vanished"))?
                    .velocity = velocity;
                continue;
            }
            let desired = slide_around(self.bodies.values(), body, desired);
            self.requests.push(MoveRequest { body: id, desired });
            let (moved, grounded, contact) = self.move_shape(body, desired)?;
            let to = [0, 1, 2].map(|axis| body.position[axis] + moved[axis]);
            let supported = grounded && self.ground_gap(body, to).is_some();
            if (supported && velocity[1] <= 0.0)
                || (velocity[1] > 0.0
                    && contact
                    && moved[1] < velocity[1] - CEILING_TOLERANCE_UNITS)
            {
                velocity[1] = 0.0;
            }
            velocity[0] = moved[0];
            velocity[2] = moved[2];
            self.place(id, to)?;
            self.bodies
                .get_mut(&id)
                .ok_or_else(|| error("moving body vanished"))?
                .velocity = velocity;
        }
        Ok(())
    }

    /// The translation (units) the controller applies to `body` for `desired`, whether it ends
    /// grounded, and whether the move touched anything.
    fn move_shape(
        &self,
        body: &RigidBody,
        desired: [f32; 3],
    ) -> Result<([f32; 3], bool, bool), PhysicsError> {
        let handle = *self
            .handles
            .get(&body.id)
            .ok_or_else(|| error("moving body has no Rapier body"))?;
        let queries = self.world.broad_phase.as_query_pipeline(
            &self.queries,
            &self.world.bodies,
            &self.world.colliders,
            QueryFilter::default().exclude_rigid_body(handle),
        );
        let mut contact = false;
        let movement = self.controller.move_shape(
            TICK_SECONDS,
            &queries,
            &capsule(body.half_extents),
            &Pose::from_translation(lifted(body.position)),
            metres(desired),
            |_| contact = true,
        );
        Ok((units(movement.translation), movement.grounded, contact))
    }

    /// How far (units) walkable static ground within the support reach lies below `body` at
    /// `position`, beyond the contact offset. A face too steep to stand on met first (a ramp's
    /// chute end) is set aside and the cast repeated, a few times at most.
    fn ground_gap(&self, body: &RigidBody, position: [f32; 3]) -> Option<f32> {
        let mut steep: Vec<ColliderHandle> = Vec::new();
        for _ in 0..4 {
            let admit = |handle: ColliderHandle, _: &Collider| !steep.contains(&handle);
            let queries = self.world.broad_phase.as_query_pipeline(
                &self.queries,
                &self.world.bodies,
                &self.world.colliders,
                QueryFilter::only_fixed().predicate(&admit),
            );
            let (handle, hit) = queries.cast_shape(
                &Pose::from_translation(lifted(position)),
                -Vector::Y,
                &capsule(body.half_extents),
                ShapeCastOptions {
                    target_distance: CONTACT_OFFSET,
                    stop_at_penetration: false,
                    max_time_of_impact: SUPPORT_REACH,
                    compute_impact_geometry_on_penetration: true,
                },
            )?;
            if hit.normal1.dot(Vector::Y) >= self.controller.max_slope_climb_angle.cos() {
                return Some(hit.time_of_impact * UNITS_PER_METRE);
            }
            steep.push(handle);
        }
        None
    }

    pub fn grounded(&self, id: BodyId) -> bool {
        self.bodies
            .get(&id)
            .filter(|body| body.is_moving())
            .is_some_and(|body| self.ground_gap(body, body.position).is_some())
    }

    pub fn cast_ray(
        &self,
        origin: [f32; 3],
        translation: [f32; 3],
        accept: &impl Fn(BodyId) -> bool,
    ) -> Option<RayHit> {
        let ray = Ray::new(metres(origin), metres(translation));
        // The first accepted hit below `below` (an id), within `reach` of the ray.
        let first = |below: Option<BodyId>, reach: f32| {
            let predicate = |_, collider: &Collider| {
                let id = game(collider);
                below.is_none_or(|below| id < below) && accept(id)
            };
            let queries = self.world.broad_phase.as_query_pipeline(
                &self.queries,
                &self.world.bodies,
                &self.world.colliders,
                QueryFilter::default().predicate(&predicate),
            );
            let (handle, hit) = queries.cast_ray_and_get_normal(&ray, reach, true)?;
            Some((game(self.world.colliders.get(handle)?), hit))
        };
        // Rapier breaks equally near hits by its tree order, the game by body id: look for a
        // lower id at the same distance until none.
        let length = translation
            .iter()
            .map(|axis| axis * axis)
            .sum::<f32>()
            .sqrt();
        let tie = TIE_UNITS / length;
        let (mut id, mut hit) = first(None, 1.0)?;
        while let Some((lower, tied)) = first(Some(id), hit.time_of_impact + tie) {
            (id, hit) = (lower, tied);
        }
        Some(RayHit {
            body: id,
            fraction: hit.time_of_impact,
            normal: [hit.normal.x, hit.normal.y, hit.normal.z],
        })
    }

    pub fn overlap_box(
        &self,
        centre: [f32; 3],
        half_extents: [f32; 3],
    ) -> Result<Vec<BodyId>, PhysicsError> {
        let queries = self.world.broad_phase.as_query_pipeline(
            &self.queries,
            &self.world.bodies,
            &self.world.colliders,
            QueryFilter::default(),
        );
        let shape = Cuboid::new(metres(half_extents));
        let mut hits: Vec<_> = queries
            .intersect_shape(Pose::from_translation(metres(centre)), &shape)
            .map(|(_, collider)| game(collider))
            .collect();
        hits.sort_unstable();
        hits.dedup();
        Ok(hits)
    }
}
