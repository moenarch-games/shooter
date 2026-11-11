//! How a player's body moves: walk, look and jump. The physics world (physics-engine, or the
//! Rapier oracle under `physics-ab`) resolves contact, slopes and support; this module only turns
//! the player's intent into the velocity it asks for each tick.

use sim_foundation::physics::{BodyId, PhysicsError, Vec3i, World};
use sim_foundation::{subunits_per_tick, subunits_per_tick_squared, trig};

/// The player's collision box, 0.6 m × 1.8 m × 0.6 m (an upright capsule in physics).
pub const PLAYER_HALF_EXTENTS: Vec3i = Vec3i::new(30, 90, 30);
/// Eye height above the feet.
pub const EYE_HEIGHT_UNITS: i32 = 165;
/// Walking speed in every direction: 7.2 m/s.
pub const WALK_SPEED_UNITS_PER_SECOND: i32 = 720;
pub const WALK_SPEED_SUBUNITS_PER_TICK: i32 = subunits_per_tick(WALK_SPEED_UNITS_PER_SECOND);
/// Upward velocity a grounded jump sets: 6 m/s, a 1 m jump under [`GRAVITY`].
pub const JUMP_VELOCITY_UNITS_PER_SECOND: i32 = 600;
pub const JUMP_VELOCITY_SUBUNITS_PER_TICK: i32 = subunits_per_tick(JUMP_VELOCITY_UNITS_PER_SECOND);
/// Arena gravity, 18 m/s²: heavier than Earth's for short, snappy jumps.
pub const GRAVITY: Vec3i = Vec3i::new(0, subunits_per_tick_squared(-1_800), 0);
/// Below this height a player has fallen out of the arena and respawns.
pub const KILL_HEIGHT_UNITS: i32 = -2_000;

/// What a player wants this tick.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Intent {
    /// +1 forward, -1 backward.
    pub forward: i8,
    /// +1 right, -1 left.
    pub strafe: i8,
    /// The direction the player looks (`sim_foundation::trig` yaw).
    pub yaw: u16,
    /// Jump if standing on ground.
    pub jump: bool,
}

/// One player's body in the arena's physics world.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Player {
    pub body: BodyId,
}

/// What a tick did to a player, for presentation (footsteps, landing).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StepReport {
    pub grounded: bool,
    pub jumped: bool,
    /// It touched down this tick after being airborne.
    pub landed: bool,
}

/// The yaw offset of a forward/strafe combination: strafing right turns away from +X, the way
/// yaw grows (right of yaw 0, which faces +Z, is -X).
const fn move_offset(forward: i8, strafe: i8) -> Option<u16> {
    const EIGHTH: u16 = trig::YAW_EIGHTH_TURN;
    Some(match (forward.signum(), strafe.signum()) {
        (0, 0) => return None,
        (1, 0) => 0,
        (1, 1) => 0u16.wrapping_sub(EIGHTH),
        (0, 1) => 0u16.wrapping_sub(2 * EIGHTH),
        (-1, 1) => 0u16.wrapping_sub(3 * EIGHTH),
        (-1, 0) => 4 * EIGHTH,
        (-1, -1) => 3 * EIGHTH,
        (0, -1) => 2 * EIGHTH,
        _ => EIGHTH,
    })
}

/// The horizontal velocity (subunits per tick) `intent` asks for.
#[must_use]
pub fn walk_velocity(intent: Intent) -> [i32; 2] {
    let Some(offset) = move_offset(intent.forward, intent.strafe) else {
        return [0, 0];
    };
    let (x, z) = trig::direction(intent.yaw.wrapping_add(offset));
    let scale = |unit| trig::checked_scale(WALK_SPEED_SUBUNITS_PER_TICK, unit).unwrap_or(0);
    [scale(x), scale(z)]
}

impl Player {
    /// Adds a player standing at `feet` to `world`.
    pub fn spawn(world: &mut World, body: BodyId, feet: Vec3i) -> Result<Self, PhysicsError> {
        let center = Vec3i::new(feet.x, feet.y + PLAYER_HALF_EXTENTS.y, feet.z);
        world.add_body(sim_foundation::physics::RigidBody::dynamic(
            body,
            center,
            Vec3i::ZERO,
            PLAYER_HALF_EXTENTS,
        ))?;
        Ok(Self { body })
    }

    /// Sets the velocity `intent` asks for before the world steps: walking replaces the
    /// horizontal velocity (full air control for now), a grounded jump sets the vertical one.
    pub fn apply(&self, world: &mut World, intent: Intent) -> Result<bool, PhysicsError> {
        let current = world
            .body(self.body)
            .map(sim_foundation::physics::RigidBody::velocity)
            .unwrap_or_default();
        let jumped = intent.jump && world.grounded(self.body);
        let [x, z] = walk_velocity(intent);
        let y = if jumped {
            JUMP_VELOCITY_SUBUNITS_PER_TICK
        } else {
            current.y
        };
        world.set_velocity(self.body, Vec3i::new(x, y, z))?;
        Ok(jumped)
    }

    /// The body's exact centre, in units.
    #[must_use]
    pub fn center(&self, world: &World) -> [f32; 3] {
        world
            .body(self.body)
            .map(sim_foundation::physics::RigidBody::exact_position)
            .unwrap_or_default()
    }

    /// The feet, in units.
    #[must_use]
    pub fn feet(&self, world: &World) -> [f32; 3] {
        let [x, y, z] = self.center(world);
        #[allow(clippy::cast_precision_loss)]
        [x, y - PLAYER_HALF_EXTENTS.y as f32, z]
    }
}

/// A single-player arena simulation: the world and one player, advanced one tick at a time.
#[derive(Debug)]
pub struct Session {
    pub arena: crate::Arena,
    pub world: World,
    pub player: Player,
    grounded: bool,
}

impl Session {
    pub fn new(arena: crate::Arena) -> Result<Self, PhysicsError> {
        let mut world = arena.build_world(GRAVITY)?;
        let player = Player::spawn(
            &mut world,
            BodyId(crate::arena::FIRST_PLAYER_BODY),
            arena.spawn,
        )?;
        Ok(Self {
            arena,
            world,
            player,
            grounded: false,
        })
    }

    /// One tick at [`sim_foundation::TICK_HZ`].
    pub fn tick(&mut self, intent: Intent) -> Result<StepReport, PhysicsError> {
        let jumped = self.player.apply(&mut self.world, intent)?;
        self.world.step()?;
        if self.player.feet(&self.world)[1] < KILL_HEIGHT_UNITS as f32 {
            self.respawn()?;
        }
        let grounded = self.world.grounded(self.player.body);
        let landed = grounded && !self.grounded && !jumped;
        self.grounded = grounded;
        Ok(StepReport {
            grounded,
            jumped,
            landed,
        })
    }

    /// Back to the arena's spawn, at rest.
    pub fn respawn(&mut self) -> Result<(), PhysicsError> {
        let spawn = self.arena.spawn;
        self.world.set_position(
            self.player.body,
            Vec3i::new(spawn.x, spawn.y + PLAYER_HALF_EXTENTS.y, spawn.z),
        )?;
        self.world.set_velocity(self.player.body, Vec3i::ZERO)
    }
}
