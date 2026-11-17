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
//! face. A target not ahead of the muzzle (closer than the muzzle reaches) is shot along the aim.
//! The grapple and the blade keep leaving the eye along the aim (their own contracts).
//!
//! The reticle covers the intended target, and when the muzzle's own line meets something short
//! of it a second, blocked marker shows where the shot will really land ([`Shot::blocked`]).

use crate::arena::FIRST_PLAYER_BODY;
use crate::damage::Hitboxes;
use crate::physics::{BodyId, World};
use crate::robot::{Assembly, BETA, Kind, LONG_BARREL, ModId};
use crate::trig::{self, TRIG_ONE};
use crate::weapons::{self, MAX_PITCH, Trace, Weapon};

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
}

/// The muzzle of `build`'s first connected `weapon` part in `parts` (its connected parts where
/// they are, [`crate::Fighter::hitboxes`]), with its mod's muzzle when it has one; `None` when
/// it mounts no such weapon still connected, or the weapon has no muzzle (the blade).
#[must_use]
pub fn muzzle(build: &Assembly, parts: &Hitboxes, weapon: Weapon) -> Option<Muzzle> {
    #[allow(clippy::cast_precision_loss)]
    let cell_mm = crate::robot::CELL_MM as f32;
    parts.boxes.iter().find_map(|part| {
        let built = build.parts().get(part.index)?;
        let kind = BETA.part_type(built.kind)?.kind;
        if kind != Kind::Weapon(weapon) {
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
    let run = (unit[0].hypot(unit[2]) * SCALE).round() as i32;
    let pitch = trig::yaw_from_vector(y, run.max(1))?;
    let yaw = trig::yaw_from_vector(x, z).unwrap_or(0);
    #[allow(clippy::cast_possible_wrap)]
    Some((yaw, (pitch as i16).clamp(-MAX_PITCH, MAX_PITCH)))
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
}

/// A shot of a weapon at `muzzle` (`None`: from the eye along the aim, the grapple and the
/// blade) from a fighter's `eye` aiming at `yaw` and `pitch`: the camera picks the intended
/// target among the scenery and the `fighters` (where the shooter sees them), and the shot
/// leaves the muzzle toward it.
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
    let (yaw, pitch) = if dot(toward, forward) > 1.0 {
        angles(toward).unwrap_or((yaw, pitch))
    } else {
        (yaw, pitch)
    };
    Shot {
        target,
        from,
        yaw,
        pitch,
    }
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
    use crate::robot::{MARKSMAN, STANDARD};

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

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        length(sub(a, b)) < 1e-3
    }

    #[test]
    fn a_muzzle_is_the_kits_anchor_along_its_weapons_barrel() {
        let parts = placed(&STANDARD, &PartDamage::fresh(&STANDARD));
        let rifle = parts.get(index_of(&STANDARD, Weapon::EnergyRifle)).unwrap();
        let rifle_muzzle = muzzle(&STANDARD, &parts, Weapon::EnergyRifle).unwrap();
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
        let long = muzzle(&MARKSMAN, &parts, Weapon::EnergyRifle).unwrap();
        let along = add(scale(rifle.axes[1], -10.0), scale(rifle.axes[2], 20.0));
        assert!(
            close(long.at, add(rifle.center, along)),
            "{long:?} {rifle:?}"
        );
        // The blade has none.
        assert_eq!(muzzle(&STANDARD, &parts, Weapon::EnergyBlade), None);
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
        let muzzle = muzzle(&STANDARD, &parts, Weapon::Shotgun).unwrap();
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
        assert_eq!(muzzle(&STANDARD, &parts, Weapon::EnergyRifle), None);
        assert!(muzzle(&STANDARD, &parts, Weapon::Shotgun).is_some());
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
