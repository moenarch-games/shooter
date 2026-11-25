//! Where a shot goes: the one answer the host's shots and the clients' reticle, effects and
//! prediction share (Atlas D2.i102.1, shooter#102).
//!
//! **Camera-targeted.** The third-person camera ([`camera`]) sits behind and over the right
//! shoulder of the fighter's eye ([`crate::Fighter::eye`]) and looks along the Q16 aim of its yaw
//! and pitch ([`crate::weapons::aim`]); a bundle of rays from the eye pulls it in front of the
//! arena's walls ([`clear_reach`]). Its centre ray, from level with the eye on, is the intended
//! target ([`target`]): the first fighter's part ([`crate::damage::Hitboxes`]) or scenery it
//! meets, where the shooter sees them. The client eases its drawn camera back out after a wall;
//! the simulation's is where the camera wants to be now, so host and clients agree.
//!
//! **Muzzle-authoritative.** A mounted ranged weapon's shot (the rifle, the rail gun, rockets,
//! the shotgun, the laser and the flamethrower) leaves its muzzle ([`muzzle`]): the weapon part's
//! box where the robot's bodies carry it now, and on it the muzzle anchor of the pinned weapons
//! kit ([`muzzle_anchor_mm`], a checked-in table the client's robot tests hold against the kit),
//! the long barrel's when it has one. It flies toward the intended target ([`aimed`]), so cover
//! between the muzzle and the target stops it though the camera sees past: peeking with the
//! camera never shoots through cover. A muzzle pushed into a wall fires from the wall's near
//! face. The grapple module's hook leaves its own muzzle toward a target ahead of it the same way
//! (shooter#104; its rope is still tied at the eye, [`crate::grapple`]), turning freely; the
//! blade keeps leaving the eye along the aim (its own contract).
//!
//! **Mount-bound** (Atlas D2.i123.8, shooter#132). A gun keeps its mount's direction: its shot
//! flies toward the target only when the target lies within [`CONVERGENCE_COS`] (10°,
//! provisional) of its barrel as mounted ([`Muzzle::axis`]: the part's own +Z, turned with
//! every joint that carries it); otherwise it flies along the barrel ([`Reach::Mount`]). A fixed
//! gun therefore fires only where its mount points, and a gun on a turret reaches further only
//! as its joints really turn it: a joint bound to track the aim ([`track`]) is told the servo
//! angle, within its limits, that turns the gun it carries toward the target, and gets there at
//! its servo's speed on the pool's energy.
//!
//! The reticle covers the intended target, and for each gun marks whether its shot lands on it,
//! is blocked short of it, or cannot reach it and where it lands instead ([`Shot::lands`]).

use crate::arena::FIRST_PLAYER_BODY;
use crate::damage::Hitboxes;
use crate::physics::{BodyId, World};
use crate::robot::{Assembly, BETA, Joint, Kind, LONG_BARREL, ModId, PartId};
use crate::trig::{self, TRIG_ONE};
use crate::weapons::{self, Instance, MAX_PITCH, Trace, Weapon};

/// How far (units) a client looks along its aim for the reticle: further than any arena.
pub const RETICLE_RANGE_UNITS: i32 = 20_000;
/// How far behind the eye the camera sits (units, along the aim).
pub const CAMERA_DISTANCE_UNITS: f32 = 350.0;
/// How far right of the eye (units).
pub const CAMERA_SHOULDER_UNITS: f32 = 60.0;
/// How far above the eye (units).
pub const CAMERA_LIFT_UNITS: f32 = 30.0;
/// How far the camera stays from anything it is pulled in by (units), and how far apart the
/// rays that find it run.
pub const CAMERA_CLEARANCE_UNITS: f32 = 20.0;
/// A shot that stops more than this (units) short of the intended target is blocked.
pub const BLOCKED_UNITS: f32 = 10.0;
/// A gun's convergence cone (Atlas D2.i123.8, shooter#132): its shot leaves the muzzle toward
/// the camera's target only when that lies within this cosine of its barrel's mount direction
/// (10°, provisional: Q2.i123.2); beyond, along the mount. A fixed gun keeps its mount's
/// direction; a turret's joints turn the mount itself ([`track`]).
pub const CONVERGENCE_COS: f32 = 0.984_807_7;
/// [`CONVERGENCE_COS`] as an angle (binary, 65 536 a turn): 10°.
pub const CONVERGENCE_ANGLE: u16 = 1_820;

/// Each weapon's muzzle in its glTF part frame (millimetres; the weapons kit's descriptor
/// `anchors[]`: `muzzle`, the rocket pod's `muzzle-a`), and with a mod that has its own muzzle
/// (the long barrel, on the weapon's `mod-muzzle` anchor) the mod's. From the pinned
/// `shooter-weapons` kit (`assets/weapon-kit.pin.json`); `cargo test -p shooter-client --test
/// robots` holds it against the kit. The blade has none.
pub const MUZZLE_ANCHORS_MM: [(Weapon, Option<ModId>, [i32; 3]); 10] = [
    (Weapon::EnergyRifle, None, [0, 310, 0]),
    (Weapon::EnergyRifle, Some(LONG_BARREL), [0, 400, 0]),
    (Weapon::RailGun, None, [0, 310, -100]),
    (Weapon::RailGun, Some(LONG_BARREL), [0, 400, -100]),
    (Weapon::RocketLauncher, None, [8, 353, 0]),
    (Weapon::Grapple, None, [0, 283, 0]),
    (Weapon::Shotgun, None, [0, 362, 0]),
    (Weapon::Laser, None, [0, 295, 0]),
    (Weapon::Laser, Some(LONG_BARREL), [0, 385, 0]),
    (Weapon::Flamethrower, None, [-12, 366, 0]),
];

/// The muzzle (millimetres, glTF part frame) of `weapon` carrying `modification`: the mod's
/// when it has one, else the weapon's own.
#[must_use]
pub fn muzzle_anchor_mm(weapon: Weapon, modification: Option<ModId>) -> Option<[i32; 3]> {
    let find = |modification: Option<ModId>| {
        MUZZLE_ANCHORS_MM
            .iter()
            .find(|(held, carried, _)| *held == weapon && *carried == modification)
            .map(|(_, _, at)| *at)
    };
    modification
        .and_then(|modification| find(Some(modification)))
        .or_else(|| find(None))
}

/// What a ray met first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sight {
    /// Where the ray ended: on the first fighter's part, else on the scenery, else at its range.
    pub end: [f32; 3],
    /// The fighter it met first (its index in the caller's list) and the fraction of the range
    /// it met it at, if it met one of its parts before the scenery.
    pub fighter: Option<(usize, f32)>,
    /// What the ray met among the scenery, fighters ignored.
    pub scenery: Trace,
}

impl Sight {
    /// It met a fighter's part or the scenery.
    #[must_use]
    pub const fn met(&self) -> bool {
        self.fighter.is_some() || self.scenery.hit.is_some()
    }
}

/// A ray from `from` (units) along the Q16 `direction` for `range_units`: the first of the
/// scenery (the bodies of `world` other than `owner` that `scenery` accepts) and the `fighters`
/// (an index and its parts' hitboxes).
#[must_use]
pub fn sight<'a>(
    world: &World,
    from: [f32; 3],
    direction: [i32; 3],
    range_units: i32,
    owner: BodyId,
    scenery: impl Fn(BodyId) -> bool,
    fighters: impl IntoIterator<Item = (usize, &'a Hitboxes)>,
) -> Sight {
    let unit = unit(direction);
    let scenery = weapons::trace(world, from, direction, range_units, owner, scenery);
    #[allow(clippy::cast_precision_loss)]
    let translation = unit.map(|axis| axis * range_units as f32);
    let reach = scenery.hit.map_or(1.0, |hit| hit.fraction);
    let mut target: Option<(usize, f32)> = None;
    for (index, parts) in fighters {
        if let Some(fraction) = parts.first(from, translation)
            && fraction < reach
            && target.is_none_or(|(_, best)| fraction < best)
        {
            target = Some((index, fraction));
        }
    }
    let end = target.map_or(scenery.end, |(_, fraction)| {
        [0, 1, 2].map(|axis| from[axis] + translation[axis] * fraction)
    });
    Sight {
        end,
        fighter: target,
        scenery,
    }
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f32; 3], by: f32) -> [f32; 3] {
    a.map(|axis| axis * by)
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

fn normalize_or(a: [f32; 3], fallback: [f32; 3]) -> [f32; 3] {
    let length = length(a);
    if length > f32::EPSILON {
        scale(a, 1.0 / length)
    } else {
        fallback
    }
}

/// A Q16 direction as a unit vector.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn unit(direction: [i32; 3]) -> [f32; 3] {
    direction.map(|axis| axis as f32 / TRIG_ONE as f32)
}

/// The camera's basis for a view direction: forward, right (yaw 0 faces +Z, its right is -X)
/// and up.
#[must_use]
pub fn basis(forward: [f32; 3]) -> [[f32; 3]; 3] {
    let forward = normalize_or(forward, [0.0, 0.0, 1.0]);
    let right = normalize_or(cross(forward, [0.0, 1.0, 0.0]), [-1.0, 0.0, 0.0]);
    [forward, right, cross(right, forward)]
}

/// Where the camera wants to be for an eye and view direction (units), nothing in the way.
#[must_use]
pub fn desired_camera(eye: [f32; 3], forward: [f32; 3]) -> [f32; 3] {
    let [forward, right, _] = basis(forward);
    add(
        add(eye, scale(forward, -CAMERA_DISTANCE_UNITS)),
        add(
            scale(right, CAMERA_SHOULDER_UNITS),
            [0.0, CAMERA_LIFT_UNITS, 0.0],
        ),
    )
}

/// How far (units) from `eye` toward `to` the camera can go and keep
/// [`CAMERA_CLEARANCE_UNITS`] from the arena (fighters ignored).
#[must_use]
pub fn clear_reach(world: &World, eye: [f32; 3], to: [f32; 3]) -> f32 {
    let span = sub(to, eye);
    let length = length(span);
    if length <= f32::EPSILON {
        return 0.0;
    }
    let [_, right, up] = basis(span);
    let fraction = [[0.0; 3], right, scale(right, -1.0), up, scale(up, -1.0)]
        .into_iter()
        .filter_map(|offset| {
            let translation = add(span, scale(offset, CAMERA_CLEARANCE_UNITS));
            world
                .cast_ray(eye, translation, |body| body.0 < FIRST_PLAYER_BODY)
                .map(|hit| hit.fraction)
        })
        .fold(1.0_f32, f32::min);
    if fraction >= 1.0 {
        length
    } else {
        (fraction * length - CAMERA_CLEARANCE_UNITS).max(0.0)
    }
}

fn camera_along(world: Option<&World>, eye: [f32; 3], forward: [f32; 3]) -> [f32; 3] {
    let wanted = desired_camera(eye, forward);
    let Some(world) = world else {
        return wanted;
    };
    let boom = sub(wanted, eye);
    let length = length(boom);
    if length <= f32::EPSILON {
        return eye;
    }
    let reach = clear_reach(world, eye, wanted).min(length);
    add(eye, scale(boom, reach / length))
}

/// The simulation's camera for an eye and Q16 aim (units): where it wants to be, pulled in
/// front of the arena at once.
#[must_use]
pub fn camera(world: &World, eye: [f32; 3], direction: [i32; 3]) -> [f32; 3] {
    camera_along(Some(world), eye, unit(direction))
}

/// The intended target: the camera's centre ray along the Q16 `direction`, from level with
/// the `eye` on (nothing behind the fighter is targeted), for `range_units` ([`sight`]).
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn target<'a>(
    world: &World,
    camera: [f32; 3],
    eye: [f32; 3],
    direction: [i32; 3],
    range_units: i32,
    owner: BodyId,
    scenery: impl Fn(BodyId) -> bool,
    fighters: impl IntoIterator<Item = (usize, &'a Hitboxes)>,
) -> Sight {
    let forward = unit(direction);
    let depth = dot(sub(eye, camera), forward).max(0.0);
    let start = add(camera, scale(forward, depth));
    sight(
        world,
        start,
        direction,
        range_units,
        owner,
        scenery,
        fighters,
    )
}

/// A mounted weapon's muzzle where its robot carries it (units).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Muzzle {
    /// The weapon part's centre: inside the robot, where the barrel starts.
    pub mount: [f32; 3],
    /// The muzzle.
    pub at: [f32; 3],
    /// The way its barrel points as mounted (a unit vector: the part's own +Z, out of the face
    /// it is mounted on, turned with every joint that carries it), for a gun: its shots converge
    /// on the target only within [`CONVERGENCE_COS`] of it ([`aimed`]). `None` turns freely
    /// toward the target: the grapple module's hook (its own contract, shooter#104).
    pub axis: Option<[f32; 3]>,
}

/// The muzzle of the weapon copy `instance` of `build` in `parts` (its connected parts where
/// they are, [`crate::Fighter::hitboxes`]), with its mod's muzzle when it has one; `None` when
/// that copy is no longer connected, or the weapon has no muzzle (the blade). Every copy fires
/// from its own muzzle (shooter#125).
#[must_use]
pub fn muzzle(build: &Assembly, parts: &Hitboxes, instance: Instance) -> Option<Muzzle> {
    #[allow(clippy::cast_precision_loss)]
    let cell_mm = crate::robot::CELL_MM as f32;
    let weapon = instance.weapon;
    parts.boxes.iter().find_map(|part| {
        let built = build.parts().get(part.index)?;
        let kind = BETA.part_type(built.kind)?.kind;
        if built.id != instance.part || kind != Kind::Weapon(weapon) {
            return None;
        }
        let [x, y, z] = muzzle_anchor_mm(weapon, built.modification)?;
        // glTF +X, +Y, +Z are the part's x, z and -y; the mount anchor is half a cell in from
        // the cuboid's minimum corner on x and y (the kit's `frame` rule); units are cm.
        #[allow(clippy::cast_precision_loss)]
        let local = [
            (cell_mm / 2.0 + x as f32) / 10.0 - part.half[0],
            (cell_mm / 2.0 - z as f32) / 10.0 - part.half[1],
            y as f32 / 10.0 - part.half[2],
        ];
        let at = (0..3).fold(part.center, |at, axis| {
            add(at, scale(part.axes[axis], local[axis]))
        });
        Some(Muzzle {
            mount: part.center,
            at,
            axis: weapon.gun().then_some(part.axes[2]),
        })
    })
}

/// The yaw and pitch (up positive) nearest a direction, or `None` for the zero vector.
#[must_use]
pub fn angles(direction: [f32; 3]) -> Option<(u16, i16)> {
    const SCALE: f32 = 1_000_000.0;
    let unit = normalize_or(direction, [0.0; 3]);
    if unit == [0.0; 3] {
        return None;
    }
    #[allow(clippy::cast_possible_truncation)]
    let [x, y, z] = unit.map(|axis| (axis * SCALE).round() as i32);
    #[allow(clippy::cast_possible_truncation)]
    let run = (libm::hypotf(unit[0], unit[2]) * SCALE).round() as i32;
    let pitch = trig::yaw_from_vector(y, run.max(1))?;
    let yaw = trig::yaw_from_vector(x, z).unwrap_or(0);
    #[allow(clippy::cast_possible_wrap)]
    Some((yaw, (pitch as i16).clamp(-MAX_PITCH, MAX_PITCH)))
}

/// Whether a shot can fly toward the intended target ([`aimed`]).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum Reach {
    /// Toward the target (or along the aim, from the eye).
    #[default]
    Target,
    /// The target is outside its gun's convergence cone ([`CONVERGENCE_COS`]): the shot flies
    /// along the gun's mount instead.
    Mount,
}

/// Where a shot really lands against the intended target ([`Shot::lands`]): what the reticle
/// marks for each gun.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lands {
    /// On the target.
    Target,
    /// Stopped short of the target by something else, there.
    Blocked([f32; 3]),
    /// The gun cannot point at the target: its shot flies along its mount and lands there.
    Unreachable([f32; 3]),
}

/// One shot as fired: the intended target, where it leaves and the aim it flies along.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shot {
    /// The camera's centre ray ([`target`]): what the reticle covers.
    pub target: Sight,
    /// Where the shot leaves (units): the muzzle, else the eye.
    pub from: [f32; 3],
    /// The yaw and pitch it flies along from there.
    pub yaw: u16,
    pub pitch: i16,
    /// Toward the target, or along its gun's mount.
    pub reach: Reach,
}

impl Shot {
    /// Its Q16 direction.
    #[must_use]
    pub fn direction(&self) -> [i32; 3] {
        weapons::aim(self.yaw, self.pitch)
    }

    /// Where `actual` (the shot's own ray from [`Shot::from`]) stops short of the intended
    /// target, if it does: on something else, more than [`BLOCKED_UNITS`] nearer.
    #[must_use]
    pub fn blocked(&self, actual: &Sight) -> Option<[f32; 3]> {
        if !actual.met() {
            return None;
        }
        if let (Some((hit, _)), Some((wanted, _))) = (actual.fighter, self.target.fighter)
            && hit == wanted
        {
            return None;
        }
        let reach = length(sub(actual.end, self.from));
        let wanted = length(sub(self.target.end, self.from));
        (reach + BLOCKED_UNITS < wanted).then_some(actual.end)
    }

    /// Where the shot lands (`actual`: its own ray from [`Shot::from`]): on the target, blocked
    /// short of it, or, its gun unable to point at the target, wherever its mount sends it.
    #[must_use]
    pub fn lands(&self, actual: &Sight) -> Lands {
        if self.reach == Reach::Mount {
            let on_target = matches!(
                (actual.fighter, self.target.fighter),
                (Some((hit, _)), Some((wanted, _))) if hit == wanted
            );
            if !on_target {
                return Lands::Unreachable(actual.end);
            }
        }
        self.blocked(actual).map_or(Lands::Target, Lands::Blocked)
    }
}

/// A shot of a weapon at `muzzle` (`None`: from the eye along the aim, the blade) from a
/// fighter's `eye` aiming at `yaw` and `pitch`: the camera picks the intended target among the
/// scenery and the `fighters` (where the shooter sees them), and the shot leaves the muzzle
/// toward it.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn aimed<'a>(
    world: &World,
    eye: [f32; 3],
    muzzle: Option<Muzzle>,
    yaw: u16,
    pitch: i16,
    owner: BodyId,
    scenery: impl Fn(BodyId) -> bool + Copy,
    fighters: impl IntoIterator<Item = (usize, &'a Hitboxes)>,
) -> Shot {
    let direction = weapons::aim(yaw, pitch);
    let forward = unit(direction);
    let camera = camera(world, eye, direction);
    let target = target(
        world,
        camera,
        eye,
        direction,
        RETICLE_RANGE_UNITS,
        owner,
        scenery,
        fighters,
    );
    let Some(muzzle) = muzzle else {
        return Shot {
            target,
            from: eye,
            yaw,
            pitch,
            reach: Reach::Target,
        };
    };
    // A muzzle in a wall fires from the wall's near face.
    let barrel = sub(muzzle.at, muzzle.mount);
    let from = world
        .cast_ray(muzzle.mount, barrel, |body| body != owner && scenery(body))
        .map_or(muzzle.at, |hit| {
            let back = 1.0 / length(barrel).max(1.0);
            add(muzzle.mount, scale(barrel, (hit.fraction - back).max(0.0)))
        });
    let toward = sub(target.end, from);
    let ((yaw, pitch), reach) = match muzzle.axis {
        // A gun converges on the target only within its cone about its mount; past it the shot
        // flies along the mount (Atlas D2.i123.8).
        Some(axis) => {
            if converges(axis, toward) {
                (angles(toward).unwrap_or((yaw, pitch)), Reach::Target)
            } else {
                (angles(axis).unwrap_or((yaw, pitch)), Reach::Mount)
            }
        }
        // The grapple's hook turns freely toward a target ahead, else along the aim.
        None if dot(toward, forward) > 1.0 => {
            (angles(toward).unwrap_or((yaw, pitch)), Reach::Target)
        }
        None => ((yaw, pitch), Reach::Target),
    };
    Shot {
        target,
        from,
        yaw,
        pitch,
        reach,
    }
}

/// Whether a gun whose barrel points along the unit `axis` can send a shot along `toward`:
/// within [`CONVERGENCE_COS`] of it (and not nearer than a unit). Arithmetic and a square root
/// only, so every platform agrees.
#[must_use]
pub fn converges(axis: [f32; 3], toward: [f32; 3]) -> bool {
    let distance = length(toward);
    distance > 1.0 && dot(toward, axis) >= CONVERGENCE_COS * distance
}

/// The binary angle (65 536 a turn, right-handed about `axis`) that turns `from` to point as
/// nearly as a turn about `axis` can along `to`: both seen square to the axis. `None` when
/// either lies along the axis. Arithmetic, square roots and the integer angle search
/// ([`trig::yaw_from_vector`]) only, so every platform agrees.
#[must_use]
pub fn turn_about(axis: [f32; 3], from: [f32; 3], to: [f32; 3]) -> Option<i32> {
    const SCALE: f32 = 1_000_000.0;
    let axis = normalize_or(axis, [0.0; 3]);
    let square = |vector: [f32; 3]| sub(vector, scale(axis, dot(vector, axis)));
    let along = square(from);
    if length(along) < 1e-3 {
        return None;
    }
    let u = scale(along, 1.0 / length(along));
    let v = cross(axis, u);
    let to = square(to);
    let length = length(to);
    if length < 1e-3 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation)]
    let [x, y] = [dot(to, u), dot(to, v)].map(|axis| (axis / length * SCALE).round() as i32);
    // atan2(y, x): the yaw search's angle has its sine along its first argument.
    let angle = trig::yaw_from_vector(y, x)?;
    #[allow(clippy::cast_possible_wrap)]
    Some(i32::from(angle as i16))
}

/// The servo angle (binary, 65 536 a turn) a tracking joint is told this tick (shooter#132,
/// [`crate::mechanics::Command::Track`]): the angle that turns the first connected gun it
/// carries (in part-ID order) to point from its muzzle at `point` (the camera's target), as
/// far as a turn about its own axis can, from its `angle` now (radians, from its bodies) and
/// within its `limits` (radians; a hinge's, none for a rotor). A target beyond its limits holds
/// it at the nearer limit: the joint refuses the rest, and the gun's shots then converge only
/// within their cone ([`aimed`]). A joint that carries no connected gun, or whose gun or target
/// lies along its axis, holds where it is. `None` when `joint` is not a connected joint in
/// `parts` (its robot's connected parts where they are, [`crate::Fighter::hitboxes`]).
///
/// The joint turns toward the angle at its servo's speed and torque and only while the pool
/// pays its draw ([`crate::mechanics`]): the angle is a target, never a pose.
#[must_use]
pub fn track(
    build: &Assembly,
    parts: &Hitboxes,
    joint: PartId,
    angle: f32,
    limits: Option<[f32; 2]>,
    point: [f32; 3],
) -> Option<i16> {
    let built = build.part(joint)?;
    let Kind::Joint(kind) = BETA.part_type(built.kind)?.kind else {
        return None;
    };
    let joint_box = parts.boxes.iter().find(|part| part.part == joint)?;
    let axis = match kind {
        Joint::Hinge => joint_box.axes[0],
        Joint::Rotor => joint_box.axes[2],
    };
    let binary = |radians: f32| -> i32 {
        #[allow(clippy::cast_possible_truncation)]
        let turns = (radians * (32_768.0 / std::f32::consts::PI)).round() as i32;
        turns
    };
    let now = binary(angle);
    let carried = |part: PartId| {
        let mut at = build.part(part);
        while let Some(mount) = at.and_then(|part| part.mount) {
            if mount.parent == joint {
                return true;
            }
            at = build.part(mount.parent);
        }
        false
    };
    let turn = BETA
        .weapons(build)
        .filter(|instance| instance.weapon.gun() && carried(instance.part))
        .find_map(|instance| muzzle(build, parts, instance))
        .and_then(|gun| turn_about(axis, gun.axis?, sub(point, gun.at)))
        .unwrap_or(0);
    // A joint's angle grows turning what it carries right-handed about its part's axis.
    let wanted = now + turn;
    let wanted = match limits {
        Some([low, high]) => wanted.clamp(binary(low), binary(high)),
        None => wanted,
    };
    // A rotor turns without end: the angle wraps like the servo's.
    #[allow(clippy::cast_possible_truncation)]
    Some(wanted as i16)
}

/// The yaw and pitch that put the camera's centre ray (the reticle) on `point` from a fighter
/// whose eye is at `eye` (units): what a bot or a test aims with. With `world`, the camera is
/// pulled in front of its walls as [`camera`] has it.
#[must_use]
pub fn at(world: Option<&World>, eye: [f32; 3], point: [f32; 3]) -> (u16, i16) {
    let mut forward = normalize_or(sub(point, eye), [0.0, 0.0, 1.0]);
    for _ in 0..8 {
        let camera = camera_along(world, eye, forward);
        forward = normalize_or(sub(point, camera), forward);
    }
    angles(forward).unwrap_or((0, 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::PartDamage;
    use crate::robot::{MARKSMAN, ROCKET_POD, STANDARD, TWIN_RAIL};

    fn placed(build: &Assembly, damage: &PartDamage) -> Hitboxes {
        Hitboxes::place(build, damage, [0.0, 100.0, 0.0], 0)
    }

    fn index_of(build: &Assembly, weapon: Weapon) -> usize {
        build
            .parts()
            .iter()
            .position(|part| {
                BETA.part_type(part.kind)
                    .is_some_and(|kind| kind.kind == Kind::Weapon(weapon))
            })
            .unwrap()
    }

    fn first(build: &Assembly, weapon: Weapon) -> Instance {
        BETA.first(build, weapon).unwrap()
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        length(sub(a, b)) < 1e-3
    }

    #[test]
    fn every_copy_of_a_weapon_fires_from_its_own_muzzle() {
        for (build, weapon, copies) in [
            (TWIN_RAIL, Weapon::RailGun, 2),
            (ROCKET_POD, Weapon::RocketLauncher, 4),
        ] {
            let mut damage = PartDamage::fresh(&build);
            let parts = placed(&build, &damage);
            let instances: Vec<Instance> = BETA
                .weapons(&build)
                .filter(|instance| instance.weapon == weapon)
                .collect();
            assert_eq!(instances.len(), copies);
            let muzzles: Vec<Muzzle> = instances
                .iter()
                .map(|instance| muzzle(&build, &parts, *instance).unwrap())
                .collect();
            for (instance, muzzle) in instances.iter().zip(&muzzles) {
                let index = build
                    .parts()
                    .iter()
                    .position(|part| part.id == instance.part)
                    .unwrap();
                assert_eq!(muzzle.mount, parts.get(index).unwrap().center);
            }
            for (a, first) in muzzles.iter().enumerate() {
                for second in &muzzles[a + 1..] {
                    assert!(length(sub(first.at, second.at)) > 10.0, "{muzzles:?}");
                }
            }
            // One copy lost: it has no muzzle, the others keep theirs.
            let index = build
                .parts()
                .iter()
                .position(|part| part.id == instances[0].part)
                .unwrap();
            damage.hurt(&build, index, i32::from(u16::MAX)).unwrap();
            let parts = placed(&build, &damage);
            assert_eq!(muzzle(&build, &parts, instances[0]), None);
            for (instance, before) in instances.iter().zip(&muzzles).skip(1) {
                assert_eq!(muzzle(&build, &parts, *instance).as_ref(), Some(before));
            }
            // A part that is not that weapon has no muzzle as it.
            let wrong = Instance {
                part: PartId(0),
                weapon,
            };
            assert_eq!(muzzle(&build, &parts, wrong), None);
        }
    }

    #[test]
    fn a_muzzle_is_the_kits_anchor_along_its_weapons_barrel() {
        let parts = placed(&STANDARD, &PartDamage::fresh(&STANDARD));
        let rifle = parts.get(index_of(&STANDARD, Weapon::EnergyRifle)).unwrap();
        let rifle_muzzle =
            muzzle(&STANDARD, &parts, first(&STANDARD, Weapon::EnergyRifle)).unwrap();
        assert_eq!(rifle_muzzle.mount, rifle.center);
        // 31 cm out of its socket, the rifle's box 40 cm long: 11 units beyond its centre on its
        // own +z (out of the chassis' front).
        assert!(
            close(
                rifle_muzzle.at,
                add(rifle.center, scale(rifle.axes[2], 11.0))
            ),
            "{rifle_muzzle:?} {rifle:?}"
        );
        // The long barrel's muzzle 9 cm further on (its two-cell module's rifle in the cell nearer
        // the part's -y).
        let parts = placed(&MARKSMAN, &PartDamage::fresh(&MARKSMAN));
        let rifle = parts.get(index_of(&MARKSMAN, Weapon::EnergyRifle)).unwrap();
        let long = muzzle(&MARKSMAN, &parts, first(&MARKSMAN, Weapon::EnergyRifle)).unwrap();
        let along = add(scale(rifle.axes[1], -10.0), scale(rifle.axes[2], 20.0));
        assert!(
            close(long.at, add(rifle.center, along)),
            "{long:?} {rifle:?}"
        );
        // The blade has none.
        assert_eq!(
            muzzle(&STANDARD, &parts, first(&STANDARD, Weapon::EnergyBlade)),
            None
        );
    }

    #[test]
    fn a_turned_mount_turns_its_muzzle() {
        let mut parts = placed(&STANDARD, &PartDamage::fresh(&STANDARD));
        let index = index_of(&STANDARD, Weapon::Shotgun);
        let shotgun = parts
            .boxes
            .iter_mut()
            .find(|part| part.index == index)
            .unwrap();
        // Mounted pointing up, its up toward -X.
        shotgun.axes = [[0.0, 0.0, 1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let centre = shotgun.center;
        let muzzle = muzzle(&STANDARD, &parts, first(&STANDARD, Weapon::Shotgun)).unwrap();
        // 36.2 cm out of the socket on a 40 cm box: 16.2 units above its centre.
        assert!(
            close(muzzle.at, add(centre, [0.0, 16.2, 0.0])),
            "{muzzle:?} from {centre:?}"
        );
    }

    #[test]
    fn a_lost_weapon_has_no_muzzle() {
        let mut damage = PartDamage::fresh(&STANDARD);
        let index = index_of(&STANDARD, Weapon::EnergyRifle);
        damage.hurt(&STANDARD, index, i32::from(u16::MAX)).unwrap();
        let parts = placed(&STANDARD, &damage);
        assert_eq!(
            muzzle(&STANDARD, &parts, first(&STANDARD, Weapon::EnergyRifle)),
            None
        );
        assert!(muzzle(&STANDARD, &parts, first(&STANDARD, Weapon::Shotgun)).is_some());
    }

    #[test]
    fn a_gun_converges_within_its_cone_and_a_turn_about_an_axis_is_signed() {
        let ahead = [0.0, 0.0, 1.0];
        // 5° off: in; 15° off: out; behind and at the muzzle: out.
        let at = |degrees: f32| {
            let radians = degrees.to_radians();
            [
                0.0,
                libm::sinf(radians) * 100.0,
                libm::cosf(radians) * 100.0,
            ]
        };
        assert!(converges(ahead, at(5.0)));
        assert!(!converges(ahead, at(15.0)));
        assert!(!converges(ahead, [0.0, 0.0, -100.0]));
        assert!(!converges(ahead, [0.0, 0.0, 0.5]));
        // A quarter turn about +Y takes +Z to +X (right-handed).
        let quarter = turn_about([0.0, 1.0, 0.0], ahead, [1.0, 0.0, 0.0]).unwrap();
        assert!((quarter - 16_384).abs() <= 1, "{quarter}");
        let back = turn_about([0.0, 1.0, 0.0], ahead, [-1.0, 0.0, 0.0]).unwrap();
        assert!((back + 16_384).abs() <= 1, "{back}");
        // Only the part square to the axis counts; along the axis there is no turn.
        let slanted = turn_about([0.0, 1.0, 0.0], ahead, [1.0, 5.0, 0.0]).unwrap();
        assert!((slanted - 16_384).abs() <= 1, "{slanted}");
        assert_eq!(turn_about([0.0, 1.0, 0.0], ahead, [0.0, 3.0, 0.0]), None);
        assert_eq!(turn_about([0.0, 1.0, 0.0], [0.0, 1.0, 0.0], ahead), None);
    }

    #[test]
    fn angles_give_back_the_direction() {
        for (yaw, pitch) in [(0, 0), (12_345, 2_000), (50_000, -9_000), (32_768, 15_000)] {
            let direction = unit(weapons::aim(yaw, pitch));
            let (y, p) = angles(direction).unwrap();
            assert!(
                i32::from(y.wrapping_sub(yaw) as i16).abs() <= 1,
                "{yaw} {y}"
            );
            assert!((i32::from(p) - i32::from(pitch)).abs() <= 1, "{pitch} {p}");
        }
        assert_eq!(angles([0.0; 3]), None);
    }

    #[test]
    fn the_reticle_ray_from_the_camera_lands_where_at_aims_it() {
        let eye = [0.0, 165.0, 0.0];
        for point in [
            [0.0, 120.0, 1_200.0],
            [300.0, 400.0, 900.0],
            [-50.0, 0.0, 300.0],
        ] {
            let (yaw, pitch) = at(None, eye, point);
            let forward = unit(weapons::aim(yaw, pitch));
            let camera = desired_camera(eye, forward);
            // The camera's centre ray passes within a centimetre of the point.
            let to = sub(point, camera);
            let off = sub(to, scale(forward, dot(to, forward)));
            assert!(length(off) < 1.0, "{point:?}: {off:?}");
        }
    }
}
