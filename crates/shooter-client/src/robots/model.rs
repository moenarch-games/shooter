//! A validated build as drawn parts: each part of the component tree mapped to its kit entry and
//! placed at rest where the simulation places it ([`shooter_sim::mechanics::placements`]), in the
//! robot's frame (+X forward, +Y up, +Z right, metres; the root chassis centred on the origin),
//! each weapon's mod on the anchor of the weapon it sits on, and each wheel's and track's axle
//! where the simulation turns it ([`Roll`]).
//!
//! The kits' glTF frame ([asset-tooling `docs/combat-robot-kit.md`, `docs/shooter-weapons.md`,
//! `docs/robot-locomotion.md`], socket-grid convention): a part's own x, y and z are glTF +X, −Z
//! and +Y, and the glTF origin is the mount anchor, the centre of the mount face's first cell.
//! The simulation places a part by its cuboid's minimum corner, so a kit mesh is drawn at
//! `corner · rotation · anchor · axes`. A weapon points out of its socket, along its part's +z;
//! a wheel or track turns about it.

use super::kit::{Anchor, KitData};
use crate::scene::METRES_PER_UNIT;
use render_engine::glam::{Mat3, Mat4, Quat, Vec3, Vec4};
use shooter_sim::mechanics::placements;
use shooter_sim::robot::{Assembly, BETA, CELL_MM, Kind, Locomotion, PartId};
use shooter_sim::{TICK_HZ, Weapon};

/// glTF +X, +Y and +Z as the part's x, z and −y.
pub const PART_FROM_GLTF: Mat4 = Mat4::from_cols(Vec4::X, Vec4::Z, Vec4::NEG_Y, Vec4::W);

/// A socket cell's edge (metres).
#[allow(clippy::cast_precision_loss)]
pub const CELL: f32 = CELL_MM as f32 / 1_000.0;

/// The mount anchor in the part's frame (metres from its cuboid's minimum corner).
pub const MOUNT_ANCHOR: Vec3 = Vec3::new(CELL / 2.0, CELL / 2.0, 0.0);

/// A wheel or track as it rolls, in the robot's frame at rest: the axle the simulation turns it
/// about (its mount normal) and where its rolling bodies are.
#[derive(Clone, Debug, PartialEq)]
pub struct Roll {
    /// Its axle (a unit vector): a positive angle turns right-handed about it, as the
    /// simulation's joint does.
    pub axis: Vec3,
    /// Its rolling bodies' centres (metres): the wheel's sphere, the track's two road wheels.
    pub centres: Vec<Vec3>,
    /// Metres of ground a radian of turn rolls.
    pub radius: f32,
}

impl Roll {
    /// How fast (radians a second) it turns rolling on the floor while the robot moves at
    /// `velocity` (metres a second, robot frame) and turns at `yaw_rate` (radians a second about
    /// +Y): its centre's ground speed across its axle over its radius. An upright axle does not
    /// roll.
    #[must_use]
    pub fn rate(&self, velocity: Vec3, yaw_rate: f32) -> f32 {
        #[allow(clippy::cast_precision_loss)]
        let centre = self.centres.iter().sum::<Vec3>() / self.centres.len().max(1) as f32;
        let moving = velocity + (Vec3::Y * yaw_rate).cross(centre);
        let ahead = self.axis.cross(Vec3::Y);
        Vec3::new(moving.x, 0.0, moving.z).dot(ahead) / self.radius
    }

    /// Its lowest point (metres, robot frame): the bottom of its lowest rolling disc.
    #[must_use]
    pub fn bottom(&self) -> f32 {
        let reach = self.radius * (1.0 - self.axis.y * self.axis.y).max(0.0).sqrt();
        self.centres
            .iter()
            .map(|centre| centre.y - reach)
            .fold(f32::MAX, f32::min)
    }
}

/// One drawn part.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelPart {
    pub id: PartId,
    pub key: &'static str,
    pub kind: Kind,
    /// Its parent (an index into [`Model::parts`]).
    pub parent: Option<usize>,
    /// Its kit entry ([`KitData::entries`]).
    pub entry: usize,
    /// Its damaged variant's entry, if its kit has one.
    pub damaged: Option<usize>,
    /// Its mesh's frame (the kit's glTF frame) in the robot's frame at rest.
    pub robot_from_local: Mat4,
    /// A leg's offset in its gait's cycle (0..1).
    pub phase: f32,
    /// A weapon's seconds between shots in this build (its mod's tuning included).
    pub refire: f32,
    /// A wheel's or track's axle.
    pub roll: Option<Roll>,
}

/// A weapon's mod as drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelMod {
    /// The weapon it is on (an index into [`Model::parts`]).
    pub part: usize,
    pub key: &'static str,
    /// Its kit entry ([`KitData::entries`]).
    pub entry: usize,
    /// The weapon's anchor it sits on (in the weapon's glTF frame, riding its node).
    pub anchor: Anchor,
    /// Its own muzzle (in its frame), which replaces the weapon's while it is on: the long
    /// barrel's.
    pub muzzle: Option<Anchor>,
}

/// A build as drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub parts: Vec<ModelPart>,
    /// The weapons' mods.
    pub mods: Vec<ModelMod>,
    /// The lowest and highest point at rest (metres, robot frame): where its feet stand and its
    /// top.
    pub floor: f32,
    pub top: f32,
    /// Its rearmost point (metres along +X), the pivot it topples over when it dies.
    pub back: f32,
    /// The leg clip its legs walk with: `walk-quadruped` with four legs, `walk-biped` with any
    /// other number, none without legs.
    pub gait: Option<&'static str>,
}

impl Model {
    /// `build` laid out for drawing with `kit`.
    ///
    /// # Errors
    ///
    /// If a part is not in the catalog or in no kit, or a
    /// mod is not in the kit or its weapon lacks the anchor it sits on.
    pub fn of(build: &Assembly, kit: &KitData) -> Result<Self, String> {
        let mut parts = Vec::with_capacity(build.parts().len());
        let mut mods = Vec::new();
        let (mut floor, mut top, mut back) = (f32::MAX, f32::MIN, f32::MAX);
        let mut legs = Vec::new();
        for (part, placement) in build.parts().iter().zip(placements(build)) {
            let part_type = BETA
                .part_type(part.kind)
                .ok_or_else(|| format!("robot: part type {} is not in the catalog", part.kind.0))?;
            let rotation = Quat::from_array(placement.orientation).normalize();
            let centre = Vec3::from(placement.centre) * METRES_PER_UNIT;
            let half = Vec3::from(placement.half_extents) * METRES_PER_UNIT;
            // The cuboid's half extent along each robot axis.
            let axes = Mat3::from_quat(rotation);
            let extent = axes.x_axis.abs() * half.x
                + axes.y_axis.abs() * half.y
                + axes.z_axis.abs() * half.z;
            top = top.max(centre.y + extent.y);
            back = back.min(centre.x - extent.x);
            let entry = kit
                .entry(part_type.key, "default")
                .ok_or_else(|| format!("robot: part {} has no kit entry", part_type.key))?;
            let corner = centre - rotation * half;
            let robot_from_local = Mat4::from_rotation_translation(rotation, corner)
                * Mat4::from_translation(MOUNT_ANCHOR)
                * PART_FROM_GLTF;
            let roll = kit.entries[entry].rolling.as_ref().map(|rolling| Roll {
                axis: robot_from_local
                    .transform_vector3(rolling.axis)
                    .normalize_or(Vec3::Y),
                centres: rolling
                    .centres
                    .iter()
                    .map(|centre| robot_from_local.transform_point3(*centre))
                    .collect(),
                radius: rolling.radius,
            });
            // A wheel or track stands on its rolling discs (the wheel's tyre is inset from its
            // cuboid), anything else on its cuboid.
            floor = floor.min(roll.as_ref().map_or(centre.y - extent.y, Roll::bottom));
            if part_type.kind == Kind::Locomotion(Locomotion::Leg) {
                legs.push((parts.len(), centre));
            }
            let weapon = match part_type.kind {
                Kind::Weapon(weapon) => Some(weapon),
                _ => None,
            };
            if let (Some(id), Some(_)) = (part.modification, weapon) {
                mods.push(Self::modification(
                    kit,
                    id,
                    entry,
                    parts.len(),
                    part_type.key,
                )?);
            }
            parts.push(ModelPart {
                id: part.id,
                key: part_type.key,
                kind: part_type.kind,
                parent: part
                    .mount
                    .and_then(|mount| build.parts().iter().position(|p| p.id == mount.parent)),
                entry,
                damaged: kit.entry(part_type.key, "damaged"),
                robot_from_local,
                phase: 0.0,
                refire: weapon.map_or(0.0, |weapon| refire(build, weapon)),
                roll,
            });
        }
        let gait = match legs.len() {
            0 => None,
            4 => Some("walk-quadruped"),
            _ => Some("walk-biped"),
        };
        #[allow(clippy::cast_precision_loss)]
        let middle =
            legs.iter().map(|(_, centre)| *centre).sum::<Vec3>() / legs.len().max(1) as f32;
        for (index, centre) in legs {
            let right = centre.z > middle.z;
            let front = centre.x > middle.x;
            // The clips' phases: biped left 0, right 0.5; quadruped rear-left 0, front-left
            // 0.25, rear-right 0.5, front-right 0.75.
            parts[index].phase = match (gait == Some("walk-quadruped"), right, front) {
                (true, false, true) => 0.25,
                (true, true, true) => 0.75,
                (_, true, _) => 0.5,
                (_, false, _) => 0.0,
            };
        }
        Ok(Self {
            parts,
            mods,
            floor,
            top,
            back,
            gait,
        })
    }

    /// Mod `id` on the weapon `part` (kit entry `weapon`, catalog key `on`).
    fn modification(
        kit: &KitData,
        id: shooter_sim::robot::ModId,
        weapon: usize,
        part: usize,
        on: &str,
    ) -> Result<ModelMod, String> {
        let key = BETA
            .modification(id)
            .ok_or_else(|| format!("robot: mod {} is not in the catalog", id.0))?
            .key;
        let entry = kit
            .entry(key, "default")
            .ok_or_else(|| format!("robot: mod {key} has no kit entry"))?;
        let gear = kit.entries[entry].gear.as_ref();
        let mounts_on = gear
            .and_then(|gear| gear.mounts_on.as_deref())
            .ok_or_else(|| format!("robot: mod {key} names no anchor to sit on"))?;
        let anchor = kit.entries[weapon]
            .gear
            .as_ref()
            .and_then(|gear| gear.anchor(mounts_on))
            .ok_or_else(|| format!("robot: {on} has no {mounts_on} anchor for {key}"))?
            .clone();
        Ok(ModelMod {
            part,
            key,
            entry,
            anchor,
            muzzle: gear.and_then(|gear| gear.anchor("muzzle")).cloned(),
        })
    }

    /// The parts in the subtree of `index` (itself included).
    #[must_use]
    pub fn subtree(&self, index: usize) -> Vec<usize> {
        (0..self.parts.len())
            .filter(|&at| {
                let mut at = Some(at);
                while let Some(here) = at {
                    if here == index {
                        return true;
                    }
                    at = self.parts[here].parent;
                }
                false
            })
            .collect()
    }
}

/// Seconds between two of `weapon`'s shots in `build` (its mod's tuning included).
fn refire(build: &Assembly, weapon: Weapon) -> f32 {
    #[allow(clippy::cast_precision_loss)]
    let ticks = BETA.tuning(build, weapon).refire_ticks(weapon) as f32;
    ticks / f32::from(TICK_HZ)
}
