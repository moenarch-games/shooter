//! The third-person follow camera and its reticle, the same on native and in the browser.
//!
//! The camera sits behind and over the right shoulder of the local fighter's eye and looks
//! along the aim, so the whole fighter stays in view. Its rig is the simulation's
//! ([`shooter_sim::aim::desired_camera`], [`shooter_sim::aim::clear_reach`]): it never passes
//! through the arena, a bundle of rays from the eye toward the camera pulls it in front of the
//! first wall, floor or ceiling at once, and here it eases back out when the way clears.
//! Fighters never push it.
//!
//! The camera picks the target, the weapon's muzzle fires ([`shooter_sim::aim`], Atlas
//! D2.i102.1): the reticle is drawn on what the camera's centre ray meets, and a shot leaves the
//! held weapon's muzzle toward it. When the muzzle's own line meets cover short of that point
//! (cover beside the robot the camera sees past), a second, blocked marker is drawn where the
//! shot will really land: the camera cannot shoot past cover the muzzle cannot.
//!
//! The rig's client-side tuning lives here, in one place.

use render_engine::glam::Vec3;
use shooter_sim::aim;
use shooter_sim::physics::World;

use crate::scene::METRES_PER_UNIT;

/// Vertical field of view (radians).
pub const FOV_Y: f32 = 1.2;
/// How far the camera stays from anything it is pulled in by (metres).
pub const CLEARANCE: f32 = aim::CAMERA_CLEARANCE_UNITS * METRES_PER_UNIT;
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
    let [forward, right, up] = aim::basis(forward.to_array());
    (Vec3::from(forward), Vec3::from(right), Vec3::from(up))
}

fn units(metres: Vec3) -> [f32; 3] {
    (metres / METRES_PER_UNIT).to_array()
}

/// Where the camera wants to be for an eye and view direction (metres), nothing in the way.
#[must_use]
pub fn desired(eye: Vec3, forward: Vec3) -> Vec3 {
    Vec3::from(aim::desired_camera(units(eye), forward.to_array())) * METRES_PER_UNIT
}

/// How far (metres) from `eye` toward `to` the camera can go and keep [`CLEARANCE`] from the
/// arena (fighters ignored).
#[must_use]
pub fn clear_reach(world: &World, eye: Vec3, to: Vec3) -> f32 {
    aim::clear_reach(world, units(eye), units(to)) * METRES_PER_UNIT
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

/// Where a marker is drawn (metres) for a camera and the point it marks: on the line between
/// them, so it covers that point on screen.
#[must_use]
pub fn reticle(camera: Vec3, at: Vec3, forward: Vec3) -> Vec3 {
    camera + (at - camera).normalize_or(forward) * RETICLE_DISTANCE
}

/// This frame's reticle (metres): on the camera's target, and the blocked marker where the
/// muzzle's shot stops short of it, if it does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reticle {
    pub at: Vec3,
    pub blocked: Option<Vec3>,
}

impl Reticle {
    /// The reticle for `shot` ([`aim::aimed`]) and its own ray from the muzzle (`actual`), seen
    /// from `camera` looking along `forward`.
    #[must_use]
    pub fn of(camera: Vec3, forward: Vec3, shot: &aim::Shot, actual: &aim::Sight) -> Self {
        let metres = |units: [f32; 3]| Vec3::from(units) * METRES_PER_UNIT;
        Self {
            at: reticle(camera, metres(shot.target.end), forward),
            blocked: shot
                .blocked(actual)
                .map(|at| reticle(camera, metres(at), forward)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shooter_sim::aim::{Muzzle, RETICLE_RANGE_UNITS, aimed, sight};
    use shooter_sim::arena::FIRST_PLAYER_BODY;
    use shooter_sim::physics::{BodyId, RigidBody, Vec3i};
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
        // Where the simulation has it.
        let sim = aim::camera(&world, units(eye), shooter_sim::weapons::aim(0, 0));
        assert!((camera - metres(sim)).length() < 1e-4);
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
    fn the_reticle_sits_on_the_cameras_target_at_the_screens_centre() {
        // A wall 4 m ahead of the eye, the aim straight at it.
        let world = world(&[([0, 200, 450], [500, 200, 50])]);
        let eye = [0.0, 165.0, 0.0];
        let shot = aimed(&world, eye, None, 0, 0, OWNER, |_| true, []);
        let actual = sight(
            &world,
            shot.from,
            shot.direction(),
            RETICLE_RANGE_UNITS,
            OWNER,
            |_| true,
            [],
        );
        let camera = Follow::default().place(Some(&world), metres(eye), Vec3::Z, 1.0 / 60.0);
        let reticle = Reticle::of(camera, Vec3::Z, &shot, &actual);
        assert_eq!(reticle.blocked, None);
        let (a, b) = (
            screen(camera, Vec3::Z, reticle.at),
            screen(camera, Vec3::Z, metres(shot.target.end)),
        );
        assert!((a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5);
        assert!(a.0.abs() < 1e-4 && a.1.abs() < 1e-4, "{a:?}");
        assert!((shot.target.end[2] - 400.0).abs() < 1.0);
    }

    #[test]
    fn cover_beside_the_muzzle_shows_a_blocked_marker_though_the_camera_sees_past_it() {
        // A pillar right in front of a muzzle on the robot's chest, left of and below the
        // camera's line of sight. A fighter 10 m ahead is in plain view of the camera, but the
        // shot meets the pillar.
        let world = world(&[([0, 110, 50], [10, 10, 10])]);
        let eye = [0.0, 165.0, 0.0];
        let muzzle = Muzzle {
            mount: [0.0, 110.0, 10.0],
            at: [0.0, 110.0, 25.0],
        };
        let enemy = [0.0, 90.0, 1_000.0];
        let build = shooter_sim::Assembly::default();
        let parts = shooter_sim::damage::Hitboxes::place(
            &build,
            &shooter_sim::damage::PartDamage::fresh(&build),
            enemy,
            32_768,
        );
        // The reticle on it.
        let (yaw, pitch) = aim::at(Some(&world), eye, enemy);
        let forward = Vec3::from(aim::unit(shooter_sim::weapons::aim(yaw, pitch)));
        let shot = aimed(
            &world,
            eye,
            Some(muzzle),
            yaw,
            pitch,
            OWNER,
            |_| true,
            [(1, &parts)],
        );
        assert!(shot.target.fighter.is_some(), "the camera sees the fighter");
        let actual = sight(
            &world,
            shot.from,
            shot.direction(),
            RETICLE_RANGE_UNITS,
            OWNER,
            |_| true,
            [(1, &parts)],
        );
        assert_eq!(actual.fighter, None, "the pillar stops the shot");
        assert!((actual.end[2] - 40.0).abs() < 1.0, "{:?}", actual.end);
        let camera = Follow::default().place(Some(&world), metres(eye), forward, 1.0 / 60.0);
        let reticle = Reticle::of(camera, forward, &shot, &actual);
        let blocked = reticle.blocked.expect("a blocked marker");
        let on_target = screen(camera, forward, reticle.at);
        let on_pillar = screen(camera, forward, blocked);
        let pillar = screen(camera, forward, metres(actual.end));
        assert!((on_pillar.0 - pillar.0).abs() < 1e-5 && (on_pillar.1 - pillar.1).abs() < 1e-5);
        assert!(
            (on_target.0 - on_pillar.0).abs() > 0.05,
            "{on_target:?} {on_pillar:?}"
        );
    }
}
