//! Target dummies: player-sized bodies that stand at their posts (or walk to and fro along X),
//! take damage and come back a few seconds after they go down. They make the weapons testable
//! before game-server bots exist.

use crate::arena::{FIRST_DUMMY_BODY, Post};
use crate::fixed::subunits_per_tick;
use crate::physics::{BodyId, PhysicsError, RigidBody, Vec3i, World};
use crate::player::PLAYER_HALF_EXTENTS;

/// A dummy's health: as large as a player's pool will be (Atlas D2.3).
pub const DUMMY_HEALTH: i32 = 200;
/// Ticks a downed dummy stays away: 3 s.
pub const DUMMY_RESPAWN_TICKS: u32 = 180;
/// A patrolling dummy walks at 4.5 m/s.
pub const PATROL_SPEED_SUBUNITS_PER_TICK: i32 = subunits_per_tick(450);

/// One target dummy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Dummy {
    pub body: BodyId,
    pub post: Post,
    pub health: i32,
    /// While down: the tick it comes back.
    pub down_until: Option<u32>,
    /// Patrol direction along X: -1, 0 or 1.
    heading: i32,
}

fn center_of(feet: Vec3i) -> Vec3i {
    Vec3i::new(feet.x, feet.y + PLAYER_HALF_EXTENTS.y, feet.z)
}

impl Dummy {
    /// Places a dummy on every post, in post order.
    pub fn spawn_all(world: &mut World, posts: &[Post]) -> Result<Vec<Self>, PhysicsError> {
        posts
            .iter()
            .enumerate()
            .map(|(index, post)| {
                let dummy = Self {
                    body: BodyId(FIRST_DUMMY_BODY + index as u64),
                    post: *post,
                    health: DUMMY_HEALTH,
                    down_until: None,
                    heading: i32::from(post.patrol_units > 0),
                };
                dummy.add_body(world)?;
                Ok(dummy)
            })
            .collect()
    }

    fn add_body(&self, world: &mut World) -> Result<(), PhysicsError> {
        world.add_body(RigidBody::dynamic(
            self.body,
            center_of(self.post.feet),
            Vec3i::ZERO,
            PLAYER_HALF_EXTENTS,
        ))
    }

    #[must_use]
    pub const fn alive(&self) -> bool {
        self.down_until.is_none()
    }

    /// Before the world steps: come back if it is time, then stand or walk.
    pub fn drive(&mut self, world: &mut World, tick: u32) -> Result<(), PhysicsError> {
        if let Some(until) = self.down_until {
            if tick < until {
                return Ok(());
            }
            self.down_until = None;
            self.health = DUMMY_HEALTH;
            self.heading = i32::from(self.post.patrol_units > 0);
            self.add_body(world)?;
        }
        let Some(body) = world.body(self.body) else {
            return Ok(());
        };
        let (x, fall) = (body.position().x, body.velocity().y);
        let offset = x - self.post.feet.x;
        if self.post.patrol_units > 0 {
            if offset >= self.post.patrol_units {
                self.heading = -1;
            } else if offset <= -self.post.patrol_units {
                self.heading = 1;
            }
        }
        world.set_velocity(
            self.body,
            Vec3i::new(self.heading * PATROL_SPEED_SUBUNITS_PER_TICK, fall, 0),
        )
    }

    /// Takes `damage`; returns whether it went down (its body leaves the world until it returns).
    pub fn hurt(&mut self, world: &mut World, damage: i32, tick: u32) -> bool {
        if !self.alive() {
            return false;
        }
        self.health -= damage;
        if self.health > 0 {
            return false;
        }
        self.health = 0;
        self.down_until = Some(tick + DUMMY_RESPAWN_TICKS);
        world.remove_body(self.body);
        true
    }

    /// The body's exact centre (units), while it stands.
    #[must_use]
    pub fn center(&self, world: &World) -> Option<[f32; 3]> {
        world.body(self.body).map(RigidBody::exact_position)
    }
}
