//! How a player's body moves: walk, sprint, slide, jump, double jump, jump pads and air control.
//! The physics world (physics-engine, or the Rapier oracle under `physics-ab`) resolves contact,
//! slopes and support; this module only turns the player's intent and the body's realized
//! velocity into the velocity it asks for each tick.
//!
//! Every rule is integer: velocities are subunits per tick, directions Q16
//! (`sim_foundation::trig`), lengths integer square roots.

use crate::arena::Arena;
use crate::weapons::Weapon;
use sim_foundation::physics::{BodyId, PhysicsError, RigidBody, Vec3i, World};
use sim_foundation::{subunits_per_tick, subunits_per_tick_squared, trig};

/// The player's collision box, 0.6 m × 1.8 m × 0.6 m (an upright capsule in physics).
pub const PLAYER_HALF_EXTENTS: Vec3i = Vec3i::new(30, 90, 30);
/// Eye height above the feet.
pub const EYE_HEIGHT_UNITS: i32 = 165;
/// Eye height above the feet while sliding (presentation only: the capsule keeps its height).
pub const SLIDE_EYE_HEIGHT_UNITS: i32 = 100;
/// Walking speed in every direction: 7.2 m/s.
pub const WALK_SPEED_UNITS_PER_SECOND: i32 = 720;
pub const WALK_SPEED_SUBUNITS_PER_TICK: i32 = subunits_per_tick(WALK_SPEED_UNITS_PER_SECOND);
/// Sprinting speed, forward (or forward-diagonal) only: 10.5 m/s.
pub const SPRINT_SPEED_UNITS_PER_SECOND: i32 = 1_050;
pub const SPRINT_SPEED_SUBUNITS_PER_TICK: i32 = subunits_per_tick(SPRINT_SPEED_UNITS_PER_SECOND);
/// A slide starts at least this fast: 15 m/s.
pub const SLIDE_SPEED_UNITS_PER_SECOND: i32 = 1_500;
pub const SLIDE_SPEED_SUBUNITS_PER_TICK: i32 = subunits_per_tick(SLIDE_SPEED_UNITS_PER_SECOND);
/// Ground speed needed to start a slide: halfway between walking and sprinting.
pub const SLIDE_ENTRY_SUBUNITS_PER_TICK: i32 =
    (WALK_SPEED_SUBUNITS_PER_TICK + SPRINT_SPEED_SUBUNITS_PER_TICK) / 2;
/// A slide loses 9 m/s every second and ends at walking speed (about 0.9 s from a standing
/// sprint, 9.6 m).
pub const SLIDE_FRICTION_SUBUNITS_PER_TICK: i32 = subunits_per_tick_squared(900);
/// Air control: 18 m/s² toward the wished direction, never past the faster of the current
/// speed and the wished speed.
pub const AIR_ACCEL_SUBUNITS_PER_TICK: i32 = subunits_per_tick_squared(1_800);
/// Upward velocity a grounded jump sets: 6 m/s, a 1 m jump under [`GRAVITY`].
pub const JUMP_VELOCITY_UNITS_PER_SECOND: i32 = 600;
pub const JUMP_VELOCITY_SUBUNITS_PER_TICK: i32 = subunits_per_tick(JUMP_VELOCITY_UNITS_PER_SECOND);
/// Jumps a player may make in the air before touching ground again.
pub const AIR_JUMPS: u8 = 1;
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
    /// Jump held: jumps from ground while held, and once more in the air on a fresh press.
    pub jump: bool,
    /// Sprint held.
    pub sprint: bool,
    /// Crouch held: starts a slide when moving fast enough on ground.
    pub crouch: bool,
    /// Where the player aims up (positive) or down, in `trig` angle steps within
    /// ±[`crate::weapons::MAX_PITCH`].
    pub pitch: i16,
    /// Trigger held: fires whenever the held weapon is ready.
    pub fire: bool,
    /// Switch to this weapon (it stays held until another is selected).
    pub select: Option<Weapon>,
}

impl Intent {
    /// Standing still, facing yaw 0, looking level.
    pub const IDLE: Self = Self {
        forward: 0,
        strafe: 0,
        yaw: 0,
        jump: false,
        sprint: false,
        crouch: false,
        pitch: 0,
        fire: false,
        select: None,
    };
}

/// The movement state a player carries between ticks: part of the simulation state a server
/// owns and a client predicts.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Motion {
    pub sliding: bool,
    /// Crouch was released since the last slide began.
    slide_ready: bool,
    air_jumps: u8,
    jump_held: bool,
}

/// What [`Player::apply`] did this tick.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Moves {
    pub jumped: bool,
    pub air_jumped: bool,
    pub launched: bool,
}

/// One player's body in the arena's physics world and its movement state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Player {
    pub body: BodyId,
    pub motion: Motion,
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

/// The Q16 unit direction `intent` asks to move in, if any.
#[must_use]
pub fn wish_direction(intent: Intent) -> Option<(i32, i32)> {
    move_offset(intent.forward, intent.strafe)
        .map(|offset| trig::direction(intent.yaw.wrapping_add(offset)))
}

/// The speed (subunits per tick) `intent` asks for: sprinting needs forward input.
#[must_use]
pub const fn wish_speed(intent: Intent) -> i32 {
    if intent.sprint && intent.forward > 0 {
        SPRINT_SPEED_SUBUNITS_PER_TICK
    } else {
        WALK_SPEED_SUBUNITS_PER_TICK
    }
}

fn scale((x, z): (i32, i32), speed: i32) -> [i32; 2] {
    let scale = |unit| trig::checked_scale(speed, unit).unwrap_or(0);
    [scale(x), scale(z)]
}

/// The horizontal velocity (subunits per tick) `intent` asks for on ground.
#[must_use]
pub fn walk_velocity(intent: Intent) -> [i32; 2] {
    wish_direction(intent).map_or([0, 0], |direction| scale(direction, wish_speed(intent)))
}

fn length([x, z]: [i32; 2]) -> i32 {
    i32::try_from(trig::xz_length(x, z)).unwrap_or(i32::MAX)
}

/// `velocity` turned to `speed` along its own direction (zero stays zero).
fn with_speed(velocity: [i32; 2], speed: i32) -> [i32; 2] {
    let current = i64::from(length(velocity));
    if current == 0 {
        return velocity;
    }
    velocity.map(|axis| {
        let scaled = i64::from(axis) * i64::from(speed.max(0));
        // Round half away from zero so opposite velocities stay exactly negated.
        let rounded = (scaled.abs() + current / 2) / current * scaled.signum();
        i32::try_from(rounded).unwrap_or(0)
    })
}

/// Air control: accelerate toward the wished direction without exceeding the faster of the
/// current speed and the wished speed. No input keeps the momentum.
fn air_control(velocity: [i32; 2], intent: Intent) -> [i32; 2] {
    let Some(direction) = wish_direction(intent) else {
        return velocity;
    };
    let [ax, az] = scale(direction, AIR_ACCEL_SUBUNITS_PER_TICK);
    let steered = [velocity[0] + ax, velocity[1] + az];
    let cap = length(velocity).max(wish_speed(intent));
    if length(steered) > cap {
        with_speed(steered, cap)
    } else {
        steered
    }
}

impl Player {
    /// Adds a player standing at `feet` to `world`.
    pub fn spawn(world: &mut World, body: BodyId, feet: Vec3i) -> Result<Self, PhysicsError> {
        let center = Vec3i::new(feet.x, feet.y + PLAYER_HALF_EXTENTS.y, feet.z);
        world.add_body(RigidBody::dynamic(
            body,
            center,
            Vec3i::ZERO,
            PLAYER_HALF_EXTENTS,
        ))?;
        Ok(Self {
            body,
            motion: Motion::default(),
        })
    }

    /// The feet, in whole units.
    fn feet_units(&self, world: &World) -> Vec3i {
        let center = world
            .body(self.body)
            .map_or(Vec3i::ZERO, RigidBody::position);
        Vec3i::new(center.x, center.y - PLAYER_HALF_EXTENTS.y, center.z)
    }

    /// Sets the velocity `intent` asks for before the world steps, from the velocity the last
    /// step realized (walls and ceilings have already taken their share of it).
    ///
    /// - A jump pad under a grounded player throws it with the pad's velocity.
    /// - On ground: walking or sprinting sets the horizontal velocity outright; crouching while
    ///   faster than [`SLIDE_ENTRY_SUBUNITS_PER_TICK`] starts a slide, which keeps its direction
    ///   and decays to walking speed. A jump keeps the horizontal velocity (slide jumps carry).
    /// - In the air: air control steers, and a fresh jump press spends the air jump, turning the
    ///   momentum toward the wished direction.
    pub fn apply(
        &mut self,
        world: &mut World,
        arena: &Arena,
        intent: Intent,
    ) -> Result<Moves, PhysicsError> {
        let current = world
            .body(self.body)
            .map(RigidBody::velocity)
            .unwrap_or_default();
        let grounded = world.grounded(self.body);
        let feet = self.feet_units(world);
        let motion = &mut self.motion;
        let jump_pressed = intent.jump && !motion.jump_held;
        motion.jump_held = intent.jump;
        if !intent.crouch {
            motion.slide_ready = true;
        }

        let mut moves = Moves::default();
        let mut horizontal = [current.x, current.z];
        let mut vertical = current.y;
        let pad = arena.pad_under(feet).filter(|_| grounded && current.y <= 0);
        if let Some(pad) = pad {
            let launch = pad.launch_velocity();
            horizontal = [launch.x, launch.z];
            vertical = launch.y;
            motion.sliding = false;
            moves.launched = true;
        } else if grounded {
            motion.air_jumps = AIR_JUMPS;
            let speed = length(horizontal);
            if motion.sliding && (!intent.crouch || speed <= WALK_SPEED_SUBUNITS_PER_TICK) {
                motion.sliding = false;
            } else if !motion.sliding
                && intent.crouch
                && motion.slide_ready
                && speed >= SLIDE_ENTRY_SUBUNITS_PER_TICK
            {
                motion.sliding = true;
                motion.slide_ready = false;
                horizontal = with_speed(horizontal, speed.max(SLIDE_SPEED_SUBUNITS_PER_TICK));
            } else if motion.sliding {
                horizontal = with_speed(horizontal, speed - SLIDE_FRICTION_SUBUNITS_PER_TICK);
            }
            if !motion.sliding {
                horizontal = walk_velocity(intent);
            }
            if intent.jump {
                vertical = JUMP_VELOCITY_SUBUNITS_PER_TICK;
                motion.sliding = false;
                moves.jumped = true;
            }
        } else {
            // A slide survives leaving the ground (down a ramp, off a ledge) and goes on when
            // it lands; only a jump ends it.
            horizontal = air_control(horizontal, intent);
            if jump_pressed && motion.air_jumps > 0 {
                motion.air_jumps -= 1;
                motion.sliding = false;
                if let Some(direction) = wish_direction(intent) {
                    let speed = length(horizontal).max(wish_speed(intent));
                    horizontal = scale(direction, speed);
                }
                vertical = JUMP_VELOCITY_SUBUNITS_PER_TICK;
                moves.jumped = true;
                moves.air_jumped = true;
            }
        }
        world.set_velocity(
            self.body,
            Vec3i::new(horizontal[0], vertical, horizontal[1]),
        )?;
        Ok(moves)
    }

    /// The body's exact centre, in units.
    #[must_use]
    pub fn center(&self, world: &World) -> [f32; 3] {
        world
            .body(self.body)
            .map(RigidBody::exact_position)
            .unwrap_or_default()
    }

    /// The feet, in units.
    #[must_use]
    pub fn feet(&self, world: &World) -> [f32; 3] {
        let [x, y, z] = self.center(world);
        #[allow(clippy::cast_precision_loss)]
        [x, y - PLAYER_HALF_EXTENTS.y as f32, z]
    }

    /// Where shots leave from: the eye, lower while sliding.
    #[must_use]
    pub fn eye(&self, world: &World) -> [f32; 3] {
        let [x, y, z] = self.feet(world);
        let height = if self.motion.sliding {
            SLIDE_EYE_HEIGHT_UNITS
        } else {
            EYE_HEIGHT_UNITS
        };
        #[allow(clippy::cast_precision_loss)]
        [x, y + height as f32, z]
    }

    /// Adds `push` (subunits per tick) to the body's velocity: a blast's knockback.
    pub fn push(&self, world: &mut World, push: [i32; 3]) -> Result<(), PhysicsError> {
        let velocity = world
            .body(self.body)
            .map(RigidBody::velocity)
            .unwrap_or_default();
        world.set_velocity(
            self.body,
            Vec3i::new(
                velocity.x + push[0],
                velocity.y + push[1],
                velocity.z + push[2],
            ),
        )
    }

    /// The horizontal speed the last step realized, subunits per tick.
    #[must_use]
    pub fn horizontal_speed(&self, world: &World) -> i32 {
        let velocity = world
            .body(self.body)
            .map(RigidBody::velocity)
            .unwrap_or_default();
        length([velocity.x, velocity.z])
    }
}
