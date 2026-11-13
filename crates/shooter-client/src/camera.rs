//! The third-person follow camera and its reticle, the same on native and in the browser.
//!
//! The camera sits behind and over the right shoulder of the local fighter's eye and looks
//! along the aim, so the whole fighter stays in view. It never passes through the arena: a
//! bundle of rays from the eye toward the camera (its centre and four points around it) pulls
//! it in front of the first wall, floor or ceiling, at once, and it eases back out when the
//! way clears. Fighters never push it.
//!
//! Shots still leave the fighter's eye along its aim, never the camera
//! ([`shooter_sim::aim`]), so the camera cannot shoot past cover the fighter cannot. The
//! reticle is drawn where [`shooter_sim::aim::sight`] ends for the eye and the aim the
//! simulation gets: on the first fighter or wall the shot meets. Seen from over the shoulder it
//! sits a little off the screen's centre, further the closer that point is, and lands on any
//! cover in front of the fighter that the camera sees past.
//!
//! The rig's tuning lives here, in one place.

use render_engine::glam::Vec3;
use shooter_sim::arena::FIRST_PLAYER_BODY;
use shooter_sim::physics::World;

use crate::scene::METRES_PER_UNIT;

/// Vertical field of view (radians).
pub const FOV_Y: f32 = 1.2;
/// How far behind the eye the camera sits (metres, along the aim).
pub const DISTANCE: f32 = 3.2;
/// How far right of the eye (metres).
pub const SHOULDER: f32 = 0.6;
/// How far above the eye (metres).
pub const LIFT: f32 = 0.25;
/// How far the camera stays from anything it is pulled in by (metres), and how far apart the
/// rays that find it run.
pub const CLEARANCE: f32 = 0.2;
/// How fast the camera eases back out once the way clears (metres per second).
pub const RECOVER: f32 = 6.0;
/// Closer than this (metres) to the eye, the local fighter is not drawn: the camera would be
/// inside it.
pub const HIDE_WITHIN: f32 = 0.6;
/// How far ahead of the camera the reticle is drawn (metres).
pub const RETICLE_DISTANCE: f32 = 0.3;

/// The camera's basis for a view direction: forward, right (yaw 0 faces +Z, its right is -X)
/// and up.
#[must_use]
pub fn basis(forward: Vec3) -> (Vec3, Vec3, Vec3) {
    let forward = forward.normalize_or(Vec3::Z);
    let right = forward.cross(Vec3::Y).normalize_or(Vec3::NEG_X);
    (forward, right, right.cross(forward))
}

/// Where the camera wants to be for an eye and view direction (metres), nothing in the way.
#[must_use]
pub fn desired(eye: Vec3, forward: Vec3) -> Vec3 {
    let (forward, right, _) = basis(forward);
    eye - forward * DISTANCE + right * SHOULDER + Vec3::Y * LIFT
}

fn units(metres: Vec3) -> [f32; 3] {
    (metres / METRES_PER_UNIT).to_array()
}

/// How far (metres) from `eye` toward `to` the camera can go and keep [`CLEARANCE`] from the
/// arena (fighters ignored).
#[must_use]
pub fn clear_reach(world: &World, eye: Vec3, to: Vec3) -> f32 {
    let span = to - eye;
    let length = span.length();
    if length <= f32::EPSILON {
        return 0.0;
    }
    let (_, right, up) = basis(span);
    let fraction = [Vec3::ZERO, right, -right, up, -up]
        .into_iter()
        .filter_map(|offset| {
            let translation = span + offset * CLEARANCE;
            world
                .cast_ray(units(eye), units(translation), |body| {
                    body.0 < FIRST_PLAYER_BODY
                })
                .map(|hit| hit.fraction)
        })
        .fold(1.0_f32, f32::min);
    if fraction >= 1.0 {
        length
    } else {
        (fraction * length - CLEARANCE).max(0.0)
    }
}

/// The follow camera: how far along its boom it is, easing out after being pulled in.
#[derive(Clone, Copy, Debug)]
pub struct Follow {
    reach: f32,
}

impl Default for Follow {
    fn default() -> Self {
        Self {
            reach: f32::INFINITY,
        }
    }
}

impl Follow {
    /// This frame's camera for the eye and view direction (metres), `seconds` after the last,
    /// held clear of `world`'s arena (without a world yet, where it wants to be).
    pub fn place(&mut self, world: Option<&World>, eye: Vec3, forward: Vec3, seconds: f32) -> Vec3 {
        let wanted = desired(eye, forward);
        let boom = wanted - eye;
        let length = boom.length();
        let clear = world.map_or(length, |world| clear_reach(world, eye, wanted));
        self.reach = clear.min(self.reach + RECOVER * seconds).min(length);
        if length <= f32::EPSILON {
            return eye;
        }
        eye + boom * (self.reach / length)
    }

    /// How far the camera is from the eye (metres).
    #[must_use]
    pub const fn reach(&self) -> f32 {
        self.reach
    }

    /// Snaps back to full length (a respawn: the old way's walls are elsewhere).
    pub fn reset(&mut self) {
        self.reach = f32::INFINITY;
    }
}

/// Where the reticle is drawn (metres) for a camera and the point its aim meets: on the line
/// between them, so it covers that point on screen.
#[must_use]
pub fn reticle(camera: Vec3, aim_end: Vec3, forward: Vec3) -> Vec3 {
    camera + (aim_end - camera).normalize_or(forward) * RETICLE_DISTANCE
}

#[cfg(test)]
mod tests {
    use super::*;
    use shooter_sim::aim::{RETICLE_RANGE_UNITS, sight};
    use shooter_sim::physics::{BodyId, RigidBody, Vec3i};
    use shooter_sim::weapons::aim;
    use shooter_sim::{Arena, Piece};

    const OWNER: BodyId = BodyId(FIRST_PLAYER_BODY);

    /// The floor and the given blocks (centre and half extents in units).
    fn world(blocks: &[([i32; 3], [i32; 3])]) -> World {
        let arena = Arena {
            pieces: blocks
                .iter()
                .map(|&(center, half)| Piece::Block {
                    center: Vec3i::new(center[0], center[1], center[2]),
                    half: Vec3i::new(half[0], half[1], half[2]),
                })
                .collect(),
            ..Arena::greybox()
        };
        let mut world = arena.build_world(Vec3i::new(0, 0, 0)).unwrap();
        // A fighter right behind the eye: the camera ignores it.
        world
            .add_body(RigidBody::fixed(
                BodyId(FIRST_PLAYER_BODY + 1),
                Vec3i::new(0, 165, -150),
                Vec3i::new(30, 90, 30),
            ))
            .unwrap();
        world
    }

    fn metres(units: [f32; 3]) -> Vec3 {
        Vec3::from(units) * METRES_PER_UNIT
    }

    /// Projects `point` onto the screen of a camera at `camera` looking along `forward`:
    /// (right, up) over depth.
    fn screen(camera: Vec3, forward: Vec3, point: Vec3) -> (f32, f32) {
        let (forward, right, up) = basis(forward);
        let offset = point - camera;
        let depth = offset.dot(forward);
        (offset.dot(right) / depth, offset.dot(up) / depth)
    }

    #[test]
    fn in_the_open_the_camera_is_behind_and_over_the_shoulder() {
        let world = world(&[]);
        let eye = Vec3::new(0.0, 1.65, 0.0);
        let mut follow = Follow::default();
        let camera = follow.place(Some(&world), eye, Vec3::Z, 1.0 / 60.0);
        assert!((camera - desired(eye, Vec3::Z)).length() < 1e-4);
        assert!(
            camera.z < -3.0 && camera.x < 0.0 && camera.y > eye.y,
            "{camera}"
        );
    }

    #[test]
    fn a_wall_behind_pulls_the_camera_in_front_of_it_and_it_eases_back_out() {
        // A wall 1.5 m behind the eye (its face at z = -1.5 m).
        let world = world(&[([0, 200, -175], [500, 200, 25])]);
        let eye = Vec3::new(0.0, 1.65, 0.0);
        let mut follow = Follow::default();
        let camera = follow.place(Some(&world), eye, Vec3::Z, 1.0 / 60.0);
        assert!(camera.z > -1.5 + CLEARANCE * 0.5, "{camera}");
        assert!(camera.z < 0.0, "{camera}");
        // Turned away from the wall, the camera does not jump back: it eases out.
        let turned = follow.place(Some(&world), eye, Vec3::NEG_Z, 1.0 / 60.0);
        assert!((turned - eye).length() < follow.reach() + 1e-4);
        assert!(follow.reach() < (desired(eye, Vec3::NEG_Z) - eye).length());
        for _ in 0..60 {
            follow.place(Some(&world), eye, Vec3::NEG_Z, 1.0 / 60.0);
        }
        assert!((follow.reach() - (desired(eye, Vec3::NEG_Z) - eye).length()).abs() < 1e-4);
    }

    #[test]
    fn in_a_narrow_corridor_the_camera_stays_between_the_walls() {
        // Walls 0.5 m either side of the eye along +Z.
        let world = world(&[
            ([-75, 200, 0], [25, 200, 2_000]),
            ([75, 200, 0], [25, 200, 2_000]),
        ]);
        let eye = Vec3::new(0.0, 1.65, 0.0);
        let camera = Follow::default().place(Some(&world), eye, Vec3::Z, 1.0 / 60.0);
        assert!(camera.x.abs() < 0.5, "{camera}");
    }

    #[test]
    fn looking_straight_up_keeps_the_camera_above_the_floor() {
        let world = world(&[]);
        let eye = Vec3::new(0.0, 0.5, 0.0);
        let up = Vec3::new(0.0, 1.0, 0.02).normalize();
        let camera = Follow::default().place(Some(&world), eye, up, 1.0 / 60.0);
        assert!(camera.y >= CLEARANCE * 0.5, "{camera}");
    }

    #[test]
    fn the_reticle_covers_where_the_authoritative_ray_ends() {
        // A wall 4 m ahead of the eye, the aim straight at it.
        let world = world(&[([0, 200, 450], [500, 200, 50])]);
        let eye = [0.0, 165.0, 0.0];
        let direction = aim(0, 0);
        let end = sight(
            &world,
            eye,
            direction,
            RETICLE_RANGE_UNITS,
            OWNER,
            |_| true,
            [],
        )
        .end;
        let (eye, end) = (metres(eye), metres(end));
        let camera = Follow::default().place(Some(&world), eye, Vec3::Z, 1.0 / 60.0);
        let reticle = reticle(camera, end, Vec3::Z);
        let (a, b) = (
            screen(camera, Vec3::Z, reticle),
            screen(camera, Vec3::Z, end),
        );
        assert!((a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5);
    }

    #[test]
    fn cover_beside_the_fighter_takes_the_reticle_though_the_camera_sees_past_it() {
        // A pillar right in front of the eye's aim, left of the camera's line of sight. A
        // fighter 10 m ahead is in plain view of the camera, but the shot meets the pillar.
        let world = world(&[([0, 200, 120], [20, 200, 20])]);
        let eye = [0.0, 165.0, 0.0];
        let enemy = [0.0, 165.0 - 75.0, 1_000.0];
        let shot = sight(
            &world,
            eye,
            aim(0, 0),
            RETICLE_RANGE_UNITS,
            OWNER,
            |_| true,
            [(1, enemy)],
        );
        assert_eq!(shot.fighter, None, "the pillar stops the shot");
        assert!((shot.end[2] - 100.0).abs() < 1.0, "{:?}", shot.end);
        let camera = Follow::default().place(Some(&world), metres(eye), Vec3::Z, 1.0 / 60.0);
        let seen = world.cast_ray(units(camera), units(metres(enemy) - camera), |body| {
            body.0 < FIRST_PLAYER_BODY
        });
        assert_eq!(seen, None, "the camera sees the fighter past the pillar");
        let reticle = reticle(camera, metres(shot.end), Vec3::Z);
        let on_screen = screen(camera, Vec3::Z, reticle);
        let pillar = screen(camera, Vec3::Z, metres(shot.end));
        let fighter = screen(camera, Vec3::Z, metres(enemy));
        assert!((on_screen.0 - pillar.0).abs() < 1e-5);
        assert!(
            (on_screen.0 - fighter.0).abs() > 0.05,
            "{on_screen:?} {fighter:?}"
        );
    }
}
