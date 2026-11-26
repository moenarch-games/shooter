//! How a fighter moves: walk, sprint, slide, jump, double jump, jump pads, air control and the
//! grapple's rope. Since shooter#73 the body these rules move is the fighter's mechanical robot
//! ([`crate::mechanics`]): [`Motion::drive`] turns the intent and the velocity its root body
//! realized into the velocity the movement drive sets ([`crate::mechanics::Mechanics::drive`]);
//! the articulated world (physics-engine, or the Rapier oracle under `physics-ab`) resolves
//! contact, slopes and support.
//!
//! Every rule is integer: velocities are subunits per tick, directions Q16
//! (`crate::trig`), lengths integer square roots.

use crate::arena::Arena;
use crate::fixed::{subunits_per_tick, subunits_per_tick_squared};
use crate::physics::{StepUp, Vec3i};
use crate::trig;
use crate::weapons::{Select, Triggers};

/// The reference fighter box, 0.6 m × 1.8 m × 0.6 m: the dummies' bodies, and how high above its feet
/// a robot's centre is.
pub const PLAYER_HALF_EXTENTS: Vec3i = Vec3i::new(30, 90, 30);
/// Eye height above the feet.
pub const EYE_HEIGHT_UNITS: i32 = 165;
/// Eye height above the feet while sliding.
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
/// Step-up: a grounded player running into a ledge up to 35 cm high, with at least 20 cm of
/// room on top, climbs it without jumping (stairs, kerbs, low lips). Crates (80 cm) and cover
/// (1.2 m) stay jumps.
pub const STEP_UP: StepUp = StepUp {
    max_height: 35,
    min_width: 20,
};
/// Below this height a player has fallen out of the arena and respawns.
pub const KILL_HEIGHT_UNITS: i32 = -2_000;

/// What a player wants this tick.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Intent {
    /// +1 forward, -1 backward.
    pub forward: i8,
    /// +1 right, -1 left.
    pub strafe: i8,
    /// The direction the player looks (`crate::trig` yaw).
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
    /// The primary trigger held: fires the held weapon copy whenever it is ready (a convenience
    /// beside the direct weapon bindings, shooter#127).
    pub fire: bool,
    /// Melee held: swings the energy blade whenever it is ready ([`crate::melee`]).
    pub melee: bool,
    /// Grapple held (Atlas D2.i104.2): the robot's mounted grapple module hooks whenever it is
    /// ready, and a set hook holds (reeling and swinging) until this is let go
    /// ([`crate::grapple`]). Its own key, whatever gun is held.
    pub grapple: bool,
    /// Switch to this weapon copy, or to a copy of this weapon ([`crate::weapons::resolve`]); it
    /// stays held until another is selected.
    pub select: Option<Select>,
    /// Edit Robot held (Atlas D2.i70.11): while its robot is dead, the person is in the builder
    /// and it does not come back until this is released (Ready). It does nothing while the
    /// robot is alive: a living robot is never rebuilt (D2.i46.7).
    pub edit: bool,
    /// Start or stop a field repair ([`crate::repair`]): sent once, like a weapon switch; the
    /// host checks it and refuses a part that is not eligible.
    pub repair: Option<crate::repair::Order>,
    /// What the person's control layout tells the robot's actuators this tick (Atlas D2.i70.7,
    /// [`crate::bindings`]): its joints, legs, wheels and tracks. The host keeps only commands
    /// its connected actuators take ([`crate::bindings::Motors::controls`]); the rest stand at
    /// rest.
    pub motors: crate::bindings::Motors,
    /// The weapon copies its direct weapon bindings and weapon groups ask to fire this tick
    /// (shooter#127, [`crate::bindings`]), beside the primary trigger: each fires if it is ready,
    /// its family's salvo limit allows and the pool grants its energy
    /// ([`crate::weapons::Arsenal::trigger_tuned`]).
    pub triggers: Triggers,
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
        melee: false,
        grapple: false,
        select: None,
        edit: false,
        repair: None,
        motors: crate::bindings::Motors::REST,
        triggers: Triggers::NONE,
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
    /// It was off its feet last tick: on the tick it lands, its legs do not throw it back up.
    airborne: bool,
}

impl Motion {
    /// The state packed in a byte, for snapshots: a client predicting the player resumes from it.
    #[must_use]
    pub const fn to_bits(self) -> u8 {
        (self.sliding as u8)
            | (self.slide_ready as u8) << 1
            | (self.jump_held as u8) << 2
            | (self.air_jumps & 0x0f) << 3
            | (self.airborne as u8) << 7
    }

    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self {
            sliding: bits & 1 != 0,
            slide_ready: bits & 2 != 0,
            jump_held: bits & 4 != 0,
            air_jumps: (bits >> 3) & 0x0f,
            airborne: bits & 0x80 != 0,
        }
    }
}

/// What [`Motion::drive`] did this tick.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Moves {
    pub jumped: bool,
    pub air_jumped: bool,
    pub launched: bool,
}

/// How much of its locomotion a robot still has: the connected legs, wheels and tracks out of
/// those its build mounts ([`crate::damage`]). The wished walking and sprinting speeds scale by
/// it (a robot on one of its two legs walks at half speed).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Traction {
    pub connected: u8,
    pub mounted: u8,
}

impl Traction {
    pub const FULL: Self = Self {
        connected: 1,
        mounted: 1,
    };

    const fn scale(self, speed: i32) -> i32 {
        if self.mounted == 0 {
            return 0;
        }
        speed * self.connected as i32 / self.mounted as i32
    }
}

/// What the movement rules see of a body this tick.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Footing {
    /// The velocity the last step realized, subunits per tick.
    pub velocity: Vec3i,
    /// It stands on its feet (legs, wheels or tracks on ground).
    pub grounded: bool,
    /// Its feet, in whole units.
    pub feet: Vec3i,
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

impl Motion {
    /// The velocity (subunits per tick) a body standing as `footing` says asks for this tick,
    /// with `intent`: the movement rules, whatever body carries them (the match's mechanical
    /// robots, [`crate::mechanics::Mechanics::drive`]).
    ///
    /// - A jump pad under a grounded body throws it with the pad's velocity.
    /// - On ground: walking or sprinting sets the horizontal velocity outright (at the share of
    ///   its locomotion it still has, [`Traction`]); crouching while faster than
    ///   [`SLIDE_ENTRY_SUBUNITS_PER_TICK`] starts a slide, which keeps its direction and decays
    ///   to walking speed. A jump keeps the horizontal velocity (slide jumps carry).
    /// - In the air: air control steers, and a fresh jump press spends the air jump, turning the
    ///   momentum toward the wished direction.
    /// - On a grapple's rope (`hooked`), it moves as in the air even on ground: the rope's pull
    ///   drags it along instead of the feet stopping it.
    /// - Every jump, the air jump too, leaves upward at `jump_velocity` (subunits per tick:
    ///   [`JUMP_VELOCITY_SUBUNITS_PER_TICK`], or more with thrusters, [`crate::pickups`]).
    pub fn drive(
        &mut self,
        arena: &Arena,
        intent: Intent,
        footing: Footing,
        hooked: bool,
        jump_velocity: i32,
        traction: Traction,
    ) -> (Vec3i, Moves) {
        let current = footing.velocity;
        let grounded = footing.grounded && !hooked;
        let landed = grounded && self.airborne;
        self.airborne = !grounded;
        let jump_pressed = intent.jump && !self.jump_held;
        self.jump_held = intent.jump;
        if !intent.crouch {
            self.slide_ready = true;
        }

        let mut moves = Moves::default();
        let mut horizontal = [current.x, current.z];
        let mut vertical = current.y;
        let pad = arena
            .pad_under(footing.feet)
            .filter(|_| grounded && current.y <= 0);
        if let Some(pad) = pad {
            let launch = pad.launch_velocity();
            horizontal = [launch.x, launch.z];
            vertical = launch.y;
            self.sliding = false;
            moves.launched = true;
        } else if grounded {
            self.air_jumps = AIR_JUMPS;
            let speed = length(horizontal);
            if self.sliding && (!intent.crouch || speed <= WALK_SPEED_SUBUNITS_PER_TICK) {
                self.sliding = false;
            } else if !self.sliding
                && intent.crouch
                && self.slide_ready
                && speed >= SLIDE_ENTRY_SUBUNITS_PER_TICK
            {
                self.sliding = true;
                self.slide_ready = false;
                horizontal = with_speed(horizontal, speed.max(SLIDE_SPEED_SUBUNITS_PER_TICK));
            } else if self.sliding {
                horizontal = with_speed(horizontal, speed - SLIDE_FRICTION_SUBUNITS_PER_TICK);
            }
            if !self.sliding {
                horizontal = wish_direction(intent).map_or([0, 0], |direction| {
                    scale(direction, traction.scale(wish_speed(intent)))
                });
            }
            if landed {
                // Touching down: the legs' give does not bounce it back up.
                vertical = vertical.min(0);
            }
            if intent.jump {
                vertical = jump_velocity;
                self.sliding = false;
                moves.jumped = true;
            }
        } else {
            // A slide survives leaving the ground (down a ramp, off a ledge) and goes on when
            // it lands; only a jump ends it.
            horizontal = air_control(horizontal, intent);
            if jump_pressed && self.air_jumps > 0 {
                self.air_jumps -= 1;
                self.sliding = false;
                if let Some(direction) = wish_direction(intent) {
                    let speed = length(horizontal).max(wish_speed(intent));
                    horizontal = scale(direction, speed);
                }
                vertical = jump_velocity;
                moves.jumped = true;
                moves.air_jumped = true;
            }
        }
        (Vec3i::new(horizontal[0], vertical, horizontal[1]), moves)
    }
}

/// Eye height above the feet: lower while sliding.
#[must_use]
pub const fn eye_height(motion: Motion) -> i32 {
    if motion.sliding {
        SLIDE_EYE_HEIGHT_UNITS
    } else {
        EYE_HEIGHT_UNITS
    }
}

/// A pad's throw or a thruster jump: the velocity (subunits per tick) it sets on the whole
/// robot, and what happened.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Launch {
    pub velocity: Option<Vec3i>,
    pub moves: Moves,
}

impl Motion {
    /// One tick of the world's devices and a jump mechanism for a robot that moves only on its
    /// own wheels and tracks (shooter#58, Atlas D2.i74.1): no drive walks, turns or slides it,
    /// and wheels and tracks do not jump. A jump pad (a world device) under it while it stands
    /// on them throws it with the pad's velocity; a jump press while it carries thrusters
    /// (`thrust`, subunits per tick up: a dedicated jump mechanism, [`crate::pickups`]) leaves
    /// upward at that speed, on the ground or not. Without thrusters a jump press does nothing.
    /// On a grapple's rope (`hooked`) it stands on nothing: the rope carries it.
    pub fn act(
        &mut self,
        arena: &Arena,
        intent: Intent,
        footing: Footing,
        hooked: bool,
        thrust: Option<i32>,
    ) -> Launch {
        let grounded = footing.grounded && !hooked;
        self.airborne = !grounded;
        self.sliding = false;
        self.air_jumps = 0;
        let pressed = intent.jump && !self.jump_held;
        self.jump_held = intent.jump;
        let current = footing.velocity;
        let mut launch = Launch::default();
        if let Some(pad) = arena
            .pad_under(footing.feet)
            .filter(|_| grounded && current.y <= 0)
        {
            launch.velocity = Some(pad.launch_velocity());
            launch.moves.launched = true;
        } else if let Some(thrust) = thrust.filter(|_| pressed) {
            launch.velocity = Some(Vec3i::new(current.x, current.y.max(thrust), current.z));
            launch.moves.jumped = true;
            launch.moves.air_jumped = !grounded;
        }
        launch
    }
}
