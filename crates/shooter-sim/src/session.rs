//! A single-player arena simulation: the world, one player with their weapons, the target
//! dummies and the rockets in flight, advanced one fixed tick at a time. game-server will own the
//! same state for a match; the clients draw it and play its [`Event`]s.

use crate::arena::{Arena, FIRST_PLAYER_BODY};
use crate::dummies::Dummy;
use crate::player::{GRAVITY, Intent, KILL_HEIGHT_UNITS, Motion, PLAYER_HALF_EXTENTS, Player};
use crate::weapons::{self, Arsenal, Delivery, Flight, Rocket, Weapon};
use sim_foundation::physics::{BodyId, PhysicsError, Vec3i, World};

/// What a tick did to the player, for presentation (footsteps, landing, camera) and checks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StepReport {
    pub grounded: bool,
    pub jumped: bool,
    /// The jump was the air jump.
    pub air_jumped: bool,
    /// A jump pad threw the player.
    pub launched: bool,
    pub sliding: bool,
    /// It touched down this tick after being airborne.
    pub landed: bool,
    /// The weapon that fired this tick.
    pub fired: Option<Weapon>,
    /// Damage dealt to dummies this tick.
    pub damage: i32,
    /// A rocket exploded this tick.
    pub exploded: bool,
}

/// Something a tick did that the clients show or sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// A shot left the eye (units) along `direction` (unit vector).
    Fired {
        weapon: Weapon,
        eye: [f32; 3],
        direction: [f32; 3],
    },
    /// A hitscan beam from the eye to `end`; `struck` when it met a body there.
    Beam {
        weapon: Weapon,
        from: [f32; 3],
        end: [f32; 3],
        normal: [f32; 3],
        struck: bool,
    },
    /// A rocket blew up at `at`.
    Exploded { at: [f32; 3] },
    /// A dummy took damage.
    Damaged {
        dummy: usize,
        at: [f32; 3],
        damage: i32,
        downed: bool,
    },
}

#[derive(Debug)]
pub struct Session {
    pub arena: Arena,
    pub world: World,
    pub player: Player,
    pub arsenal: Arsenal,
    pub dummies: Vec<Dummy>,
    pub rockets: Vec<Rocket>,
    /// What the last tick did, in order.
    pub events: Vec<Event>,
    tick: u32,
    grounded: bool,
}

impl Session {
    pub fn new(arena: Arena) -> Result<Self, PhysicsError> {
        let mut world = arena.build_world(GRAVITY)?;
        let player = Player::spawn(&mut world, BodyId(FIRST_PLAYER_BODY), arena.spawn)?;
        let dummies = Dummy::spawn_all(&mut world, &arena.posts)?;
        Ok(Self {
            arena,
            world,
            player,
            arsenal: Arsenal::default(),
            dummies,
            rockets: Vec::new(),
            events: Vec::new(),
            tick: 0,
            grounded: false,
        })
    }

    /// Ticks run so far.
    #[must_use]
    pub const fn ticks(&self) -> u32 {
        self.tick
    }

    /// One tick at [`sim_foundation::TICK_HZ`]: the player moves, the dummies walk, the held
    /// weapon fires, rockets fly and explode, then the world steps.
    pub fn tick(&mut self, intent: Intent) -> Result<StepReport, PhysicsError> {
        self.events.clear();
        let moves = self.player.apply(&mut self.world, &self.arena, intent)?;
        for dummy in &mut self.dummies {
            dummy.drive(&mut self.world, self.tick)?;
        }
        let mut damage = 0;
        let fired = self.arsenal.trigger(intent.select, intent.fire, self.tick);
        if let Some(weapon) = fired {
            damage += self.fire(weapon, intent)?;
        }
        let mut exploded = false;
        let mut index = 0;
        while index < self.rockets.len() {
            match self.rockets[index].fly(&self.world) {
                Flight::Flying => index += 1,
                Flight::Fizzled => {
                    self.rockets.remove(index);
                }
                Flight::Exploded { at, .. } => {
                    self.rockets.remove(index);
                    exploded = true;
                    damage += self.explode(at)?;
                }
            }
        }

        self.world.step()?;
        self.tick += 1;
        if self.player.feet(&self.world)[1] < KILL_HEIGHT_UNITS as f32 {
            self.respawn()?;
        }
        let grounded = self.world.grounded(self.player.body);
        let left_ground = moves.jumped || moves.launched;
        let landed = grounded && !self.grounded && !left_ground;
        self.grounded = grounded;
        Ok(StepReport {
            grounded,
            jumped: moves.jumped,
            air_jumped: moves.air_jumped,
            launched: moves.launched,
            sliding: self.player.motion.sliding,
            landed,
            fired,
            damage,
            exploded,
        })
    }

    fn dummy_of(&self, body: BodyId) -> Option<usize> {
        self.dummies.iter().position(|dummy| dummy.body == body)
    }

    fn hurt(&mut self, index: usize, damage: i32, at: [f32; 3]) -> i32 {
        let dummy = &mut self.dummies[index];
        if !dummy.alive() || damage <= 0 {
            return 0;
        }
        let downed = dummy.hurt(&mut self.world, damage, self.tick);
        self.events.push(Event::Damaged {
            dummy: index,
            at,
            damage,
            downed,
        });
        damage
    }

    /// Fires `weapon` from the eye: a hitscan beam resolves now, a rocket starts flying.
    fn fire(&mut self, weapon: Weapon, intent: Intent) -> Result<i32, PhysicsError> {
        let eye = self.player.eye(&self.world);
        let direction = weapons::aim(intent.yaw, intent.pitch);
        #[allow(clippy::cast_precision_loss)]
        let unit = direction.map(|axis| axis as f32 / sim_foundation::trig::TRIG_ONE as f32);
        self.events.push(Event::Fired {
            weapon,
            eye,
            direction: unit,
        });
        match weapon.delivery() {
            Delivery::Hitscan {
                damage,
                range_units,
            } => {
                let trace =
                    weapons::trace(&self.world, eye, direction, range_units, self.player.body);
                self.events.push(Event::Beam {
                    weapon,
                    from: eye,
                    end: trace.end,
                    normal: trace.hit.map_or([0.0; 3], |hit| hit.normal),
                    struck: trace.hit.is_some(),
                });
                Ok(trace
                    .hit
                    .and_then(|hit| self.dummy_of(hit.body))
                    .map_or(0, |index| self.hurt(index, damage, trace.end)))
            }
            Delivery::Rocket => {
                self.rockets
                    .push(Rocket::launch(self.player.body, eye, direction));
                Ok(0)
            }
        }
    }

    /// A blast at `at`: splash damage to every standing dummy in reach and knockback to the
    /// player (rocket jumping). Rockets do not hurt their owner.
    fn explode(&mut self, at: [f32; 3]) -> Result<i32, PhysicsError> {
        self.events.push(Event::Exploded { at });
        let mut dealt = 0;
        for index in 0..self.dummies.len() {
            let Some(center) = self.dummies[index].center(&self.world) else {
                continue;
            };
            if let Some((damage, _)) = weapons::splash(at, center) {
                dealt += self.hurt(index, damage, center);
            }
        }
        if let Some((_, push)) = weapons::splash(at, self.player.center(&self.world)) {
            self.player.push(&mut self.world, push)?;
        }
        Ok(dealt)
    }

    /// Back to the arena's spawn, at rest.
    pub fn respawn(&mut self) -> Result<(), PhysicsError> {
        let spawn = self.arena.spawn;
        self.player.motion = Motion::default();
        self.world.set_position(
            self.player.body,
            Vec3i::new(spawn.x, spawn.y + PLAYER_HALF_EXTENTS.y, spawn.z),
        )?;
        self.world.set_velocity(self.player.body, Vec3i::ZERO)
    }
}
