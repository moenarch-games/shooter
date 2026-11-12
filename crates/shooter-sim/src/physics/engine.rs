//! The physics-engine backend: every game body is an engine body keyed by its [`BodyId`], so
//! queries come back in game-id order; characters are external capsules moved in one
//! `move_characters` batch per step. The engine never integrates (gravity is zero there):
//! gravity, settling and the velocity rules are the step's own.

use std::collections::BTreeMap;
use std::fmt;

use physics_engine::BodyId as EngineId;
use physics_engine::approximate::{
    Autostep, Body, CharacterMove, CharacterMoveResult, CharacterSettings, Config, Quaternion,
    QueryFilter, QueryHit, QueryPose, RayCast, Shape, ShapeCast, Vector,
};

use super::{
    BodyId, CEILING_TOLERANCE_UNITS, CONTACT_OFFSET_UNITS, GROUND_PREDICTION_UNITS, Kind,
    MoveRequest, PhysicsError, RampRise, RayHit, RigidBody, StepUp, Vec3i, capsule_dimensions,
    error, exact, from_subunits, slide_around,
};

/// The game's centimetres per metre: the engine's length unit.
const UNITS_PER_METRE: f32 = 100.0;

const fn vector(value: [f32; 3]) -> Vector {
    Vector(value[0], value[1], value[2])
}

const fn array(value: Vector) -> [f32; 3] {
    [value.0, value.1, value.2]
}

/// A character's engine position: its game centre raised by the contact offset.
const fn lifted(position: [f32; 3]) -> Vector {
    Vector(position[0], position[1] + CONTACT_OFFSET_UNITS, position[2])
}

const fn lowered(position: Vector) -> [f32; 3] {
    [position.0, position.1 - CONTACT_OFFSET_UNITS, position.2]
}

fn capsule(half_extents: Vec3i) -> Shape {
    let (half_height, radius) = capsule_dimensions(half_extents);
    Shape::capsule(half_height, radius)
}

fn engine_error(what: &str, failure: impl fmt::Debug) -> PhysicsError {
    error(format!("physics-engine {what} failed: {failure:?}"))
}

/// `body` as an engine body keyed by its game id: a character is an external capsule,
/// everything else static.
fn engine_body(body: &RigidBody) -> Body {
    let key = body.id.0;
    match body.kind {
        Kind::Moving => {
            let mut engine_body =
                Body::keyed(key, capsule(body.half_extents), lifted(body.position), 1.0);
            engine_body.external = true;
            engine_body
        }
        Kind::Floor => Body::keyed(key, Shape::HalfSpace, vector(body.position), 0.0),
        // The engine's wedge rises toward local -X; it is turned about Y toward the ramp's
        // rise, and along Z its local X extent is the box's Z extent.
        Kind::Ramp(rises) => {
            let [hx, hy, hz] = exact(body.half_extents);
            let (half, turn) = match rises {
                RampRise::NegX => (Vector(hx, hy, hz), 0.0),
                RampRise::PosX => (Vector(hx, hy, hz), std::f32::consts::PI),
                RampRise::PosZ => (Vector(hz, hy, hx), std::f32::consts::FRAC_PI_2),
                RampRise::NegZ => (Vector(hz, hy, hx), -std::f32::consts::FRAC_PI_2),
            };
            let mut wedge = Body::keyed(key, Shape::Wedge(half), vector(body.position), 0.0);
            let (sin, cos) = (turn / 2.0).sin_cos();
            wedge.orientation = Quaternion(0.0, sin, 0.0, cos);
            wedge
        }
        Kind::Fixed => Body::keyed(
            key,
            Shape::Box(vector(exact(body.half_extents))),
            vector(body.position),
            0.0,
        ),
    }
}

#[derive(Debug)]
struct Entry {
    body: RigidBody,
    handle: EngineId,
    /// A character's grounded result from its last move; `None` once placed or added, until a
    /// step moves it.
    grounded: Option<bool>,
}

pub(super) struct World {
    gravity: [f32; 3],
    engine: physics_engine::approximate::World,
    settings: CharacterSettings,
    autostep: Option<Autostep>,
    bodies: BTreeMap<BodyId, Entry>,
    /// The last step's batch, its requests in game terms, its results and the velocities the
    /// step carried into them (reused buffers).
    batch: Vec<CharacterMove>,
    requests: Vec<MoveRequest>,
    results: Vec<CharacterMoveResult>,
    velocities: Vec<[f32; 3]>,
}

impl fmt::Debug for World {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("World")
            .field("backend", &"physics-engine")
            .field("bodies", &self.bodies.len())
            .finish_non_exhaustive()
    }
}

impl World {
    pub fn new(gravity: Vec3i) -> Self {
        let engine = physics_engine::approximate::World::new(Config {
            gravity: Vector::ZERO,
            length_unit: UNITS_PER_METRE,
            ..Config::default()
        })
        .expect("the adapter's engine configuration is valid");
        let settings = CharacterSettings {
            offset: CONTACT_OFFSET_UNITS,
            ..engine.character_settings()
        };
        Self {
            gravity: from_subunits(gravity),
            engine,
            settings,
            autostep: None,
            bodies: BTreeMap::new(),
            batch: Vec::new(),
            requests: Vec::new(),
            results: Vec::new(),
            velocities: Vec::new(),
        }
    }

    pub fn add_body(&mut self, body: RigidBody) -> Result<(), PhysicsError> {
        if self.bodies.contains_key(&body.id) {
            return Err(error(format!("physics body {} already exists", body.id.0)));
        }
        let handle = self
            .engine
            .add_body(engine_body(&body))
            .map_err(|failure| engine_error("add_body", failure))?;
        if body.is_moving() {
            self.engine
                .add_character(handle, self.settings)
                .map_err(|failure| engine_error("add_character", failure))?;
        }
        self.bodies.insert(
            body.id,
            Entry {
                body,
                handle,
                grounded: None,
            },
        );
        Ok(())
    }

    pub fn remove_body(&mut self, id: BodyId) -> Option<RigidBody> {
        let entry = self.bodies.remove(&id)?;
        self.engine.remove_body(entry.handle);
        Some(entry.body)
    }

    pub fn body(&self, id: BodyId) -> Option<&RigidBody> {
        self.bodies.get(&id).map(|entry| &entry.body)
    }

    pub fn set_velocity(&mut self, id: BodyId, velocity: [f32; 3]) -> Result<(), PhysicsError> {
        let entry = self
            .bodies
            .get_mut(&id)
            .filter(|entry| entry.body.is_moving())
            .ok_or_else(|| error(format!("moving physics body {} is missing", id.0)))?;
        entry.body.velocity = velocity;
        Ok(())
    }

    #[allow(clippy::cast_precision_loss)]
    pub fn set_step_up(&mut self, step_up: Option<StepUp>) {
        self.autostep = step_up.map(|step| Autostep {
            max_height: step.max_height as f32,
            min_width: step.min_width as f32,
        });
    }

    pub fn moves(&self) -> &[MoveRequest] {
        &self.requests
    }

    /// The game id of engine body `id`.
    fn game(&self, id: EngineId) -> Option<BodyId> {
        self.engine.body(id).and_then(|body| body.key).map(BodyId)
    }

    /// Whether a support-cast hit is walkable static ground (a too-steep face met first, a
    /// ramp's chute end, is passed over as the engine's own support test does).
    fn supports(&self, hit: &QueryHit) -> bool {
        let min_up = self.settings.max_slope.cos();
        (hit.starts_overlapping || hit.normal.1 > 0.0)
            && hit.normal.1 >= min_up
            && self
                .engine
                .body(hit.body)
                .is_some_and(|body| !body.external)
    }

    /// How far below `body` walkable static ground lies within the ground prediction, beyond
    /// the contact offset: Rapier's support gap, for the settle and the gravity rule.
    fn ground_gap(
        &self,
        body: &RigidBody,
        handle: EngineId,
        hits: &mut Vec<QueryHit>,
    ) -> Option<f32> {
        let offset = self.settings.offset;
        let reach = offset + GROUND_PREDICTION_UNITS;
        let (half_segment, radius) = capsule_dimensions(body.half_extents);
        let cast = ShapeCast::new(
            QueryPose::new(
                Shape::capsule(half_segment, radius + offset),
                lifted(body.position),
            ),
            Vector(0.0, -reach, 0.0),
        );
        let filter = QueryFilter {
            exclude: Some(handle),
            ..QueryFilter::default()
        };
        self.engine
            .cast_shape(cast, filter, hits)
            .expect("physics-engine ground cast failed");
        hits.iter()
            .find(|hit| self.supports(hit))
            .map(|hit| hit.fraction * reach)
    }

    pub fn step(&mut self) -> Result<(), PhysicsError> {
        let mut batch = std::mem::take(&mut self.batch);
        let mut requests = std::mem::take(&mut self.requests);
        let mut velocities = std::mem::take(&mut self.velocities);
        batch.clear();
        requests.clear();
        velocities.clear();
        let mut hits = Vec::new();
        for entry in self.bodies.values().filter(|entry| entry.body.is_moving()) {
            let body = &entry.body;
            let mut velocity = body.velocity;
            let mut settle = 0.0;
            match self
                .ground_gap(body, entry.handle, &mut hits)
                .filter(|_| velocity[1] <= 0.0)
            {
                Some(gap) => {
                    velocity[1] = 0.0;
                    if gap > 1.0e-3 {
                        settle = -gap;
                    }
                }
                None => {
                    for (axis, gravity) in velocity.iter_mut().zip(self.gravity) {
                        *axis += gravity;
                    }
                }
            }
            velocities.push(velocity);
            let desired = slide_around(
                self.bodies.values().map(|entry| &entry.body),
                body,
                [velocity[0], velocity[1] + settle, velocity[2]],
            );
            requests.push(MoveRequest {
                body: body.id,
                desired,
            });
            batch.push(CharacterMove {
                body: entry.handle,
                desired: vector(desired),
                exclude: None,
                autostep: self.autostep,
            });
        }
        let mut results = std::mem::take(&mut self.results);
        let outcome = self
            .engine
            .move_characters(&batch, &mut results)
            .map_err(|failure| engine_error("move_characters", failure));
        self.batch = batch;
        self.velocities = velocities;
        if let Err(failure) = outcome {
            self.results = results;
            self.requests = requests;
            return Err(failure);
        }
        for ((request, &velocity), result) in requests.iter().zip(&self.velocities).zip(&results) {
            let moved = array(result.translation);
            let position = if moved == [0.0; 3] && result.depenetration == Vector::ZERO {
                None
            } else {
                Some(
                    self.engine
                        .body(result.body)
                        .map(|body| lowered(body.position))
                        .ok_or_else(|| error("moved engine body vanished"))?,
                )
            };
            let entry = self
                .bodies
                .get_mut(&request.body)
                .ok_or_else(|| error("moved body vanished"))?;
            let mut velocity = velocity;
            if (result.grounded && velocity[1] <= 0.0)
                || (velocity[1] > 0.0 && moved[1] < velocity[1] - CEILING_TOLERANCE_UNITS)
            {
                velocity[1] = 0.0;
            }
            // The horizontal velocity is what the move realized.
            velocity[0] = moved[0];
            velocity[2] = moved[2];
            entry.body.velocity = velocity;
            if let Some(position) = position {
                entry.body.position = position;
            }
            entry.grounded = Some(result.grounded);
        }
        self.results = results;
        self.requests = requests;
        Ok(())
    }

    /// A character's last move result or, for one placed or added since, Rapier's support
    /// cast: walkable static ground within the ground prediction beyond the contact offset.
    pub fn grounded(&self, id: BodyId) -> bool {
        let Some(entry) = self.bodies.get(&id).filter(|entry| entry.body.is_moving()) else {
            return false;
        };
        if let Some(grounded) = entry.grounded {
            return grounded;
        }
        let offset = self.settings.offset;
        let reach = offset + GROUND_PREDICTION_UNITS + offset;
        let cast = ShapeCast::new(
            QueryPose::new(
                capsule(entry.body.half_extents),
                lifted(entry.body.position),
            ),
            Vector(0.0, -reach, 0.0),
        );
        let filter = QueryFilter {
            exclude: Some(entry.handle),
            ..QueryFilter::default()
        };
        let mut hits = Vec::new();
        self.engine
            .cast_shape(cast, filter, &mut hits)
            .expect("physics-engine grounded cast failed");
        hits.iter().any(|hit| self.supports(hit))
    }

    /// The first ray hit on a body `accept`s, in the engine's order (fraction, then body id).
    /// The common case, an accepted first hit, needs no hit list.
    pub fn cast_ray(
        &self,
        origin: [f32; 3],
        translation: [f32; 3],
        accept: &impl Fn(BodyId) -> bool,
    ) -> Option<RayHit> {
        let ray = RayCast::new(vector(origin), vector(translation));
        let accepted = |hit: &physics_engine::approximate::RayHit| {
            self.game(hit.body)
                .filter(|&id| accept(id))
                .map(|body| RayHit {
                    body,
                    fraction: hit.fraction,
                    normal: array(hit.normal),
                })
        };
        let first = self
            .engine
            .cast_ray_first(ray, QueryFilter::default())
            .expect("physics-engine ray query failed")?;
        if let Some(hit) = accepted(&first) {
            return Some(hit);
        }
        let mut hits = Vec::new();
        self.engine
            .cast_ray(ray, QueryFilter::default(), &mut hits)
            .expect("physics-engine ray query failed");
        hits.iter().find_map(accepted)
    }

    pub fn overlap_box(
        &self,
        centre: [f32; 3],
        half_extents: [f32; 3],
    ) -> Result<Vec<BodyId>, PhysicsError> {
        let pose = QueryPose::new(Shape::Box(vector(half_extents)), vector(centre));
        let mut hits = Vec::new();
        self.engine
            .overlap_shape(pose, QueryFilter::default(), &mut hits)
            .map_err(|failure| engine_error("overlap_shape", failure))?;
        let mut bodies: Vec<_> = hits.iter().filter_map(|hit| self.game(hit.body)).collect();
        bodies.sort_unstable();
        bodies.dedup();
        Ok(bodies)
    }
}
