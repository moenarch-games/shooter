//! Robot builds (Atlas D2.i70.1, D2.i89.1): a robot is one generic component tree assembled
//! free-form on a grid of universal sockets, the curated part catalog it is assembled from, and
//! the one validation path every build passes, whether it comes from a client, a snapshot, a
//! replay or code.
//!
//! A [`Candidate`] is untrusted raw data (part, socket, orientation and mod codes as they
//! arrived); [`Catalog::validate`] turns it into an [`Assembly`], the only form the simulation
//! stores and sends, or rejects it with an [`AssemblyError`].
//!
//! **The tree.** Every [`Part`] has a stable [`PartId`] (unique within its build, the handle for
//! hit attribution and detachment, [`crate::damage`]), a part type from the catalog and, except
//! for the one root, a [`Mount`]: the parent part, the [`SocketId`] on the parent it attaches to and its
//! [`Orientation`] there. One field per part names one parent, so a part never has two; the root
//! is a chassis section; every other part reaches the root through its parents (no cycles).
//! Validated parts are stored in [`PartId`] order, so equal builds are equal byte for byte.
//!
//! **Sockets.** Parts are cuboids of [`CELL_MM`] cells (`cells`, in the part's own frame). A part
//! type exposes sockets on some of its faces ([`Faces`]): one per surface cell, numbered face by
//! face in [`Face::ALL`] order and, on a face, row by row ([`PartType::socket`]); a 20 × 40 × 80 cm
//! chassis has 28. A child sits on one socket of its parent with its mount face (its own `NegZ`
//! face) against the parent's face, and reserves its base (`cells[0]` × `cells[1]`, the
//! *footprint*) on that face from the socket on; no two children may reserve the same cell. Cells
//! past the face's edge overhang and reserve nothing (a turret wider than its rotor is fine).
//! A part's own mount face rests on its parent, so its sockets are interior and take nothing (the
//! root's are free). Larger parts therefore occupy several cells but stay attached to one parent
//! socket only. Footprints are checked per face; whether parts on different faces or parents
//! intersect in space is not (that needs the parts' real geometry, shooter#73).
//!
//! **Orientation** is exact and canonical: three binary angles (65 536 a turn, like the
//! fighters' yaw), applied in the socket's frame as yaw about the face's outward normal, then
//! pitch, then roll (intrinsic Z-Y-X). [`Orientation::canonical`] folds any triple into the one
//! representation of its rotation (pitch within a quarter turn either way, roll zero at a quarter
//! turn of pitch), so the same rotation always serialises the same way. Quarter, eighth and finer
//! binary turns are exact; a builder's snaps (shooter#57) choose among them. The footprint turns
//! with the yaw's nearest quarter turn; pitch and roll tilt the part on its socket without moving
//! its footprint. The socket frame of each face is fixed ([`Face::grid`]): `u` × `v` is the
//! outward normal.
//!
//! **Rules.** Only objective rules reject a build: known codes, one root chassis, existing parents
//! and sockets, no cycles, footprints on their faces without overlaps, mods on the weapons they
//! fit, the technical part-count ceiling [`MAX_PARTS`], and the build points. A weapon may be
//! mounted any number of times (shooter#125): two rail guns or four rocket launchers are valid
//! when the sockets and the build points allow them, and each copy is its own
//! [`crate::weapons::Instance`], addressed by its part. Nothing requires
//! locomotion: stationary, asymmetrical or grappling-only robots are valid. Every attachment is
//! rigid; only joint and locomotion parts move ([`crate::mechanics`] places every part in space
//! and builds the robot's bodies, joints and motors from the tree).
//!
//! **The vital part** (D2.i89.1, superseding D2.i70.2): the root chassis is the robot's vital
//! part; its destruction is the only kill. There is no separate control core: the part code it had
//! (30, `robot.core`) is retired, so an old build naming it is an unknown part.
//!
//! **Energy** (D2.i70.3 as D2.i89.1 amends it, D2.i70.4): the root chassis holds the baseline
//! energy capacity and regenerates it ([`ROOT_CAPACITY`], [`ROOT_REGEN`]; other chassis sections
//! hold none); each optional battery ([`Kind::Battery`]) adds stored capacity only (no
//! regeneration). Every part draws from that one shared pool ([`crate::energy`]): no wiring, no
//! circuits, and no heat or cooling. A build's capacity and regeneration are in its
//! [`Catalog::budget`].
//!
//! **Provisional data.** The cell size, chassis sizes, part values, build points and part ceiling
//! are tunable placeholders (Atlas Q2.i70.1, Q2.i70.2), not owner-approved balance. The part keys
//! are the shooter's proposal for the stable part and weapon IDs of the forthcoming robot and
//! weapon kits (moritzbrantner/asset-tooling#595, #594) and are reconciled with them when the kits
//! ship.
//!
//! **Weapon modifications** (Atlas D2.i46.8, shooter#51) are mounted on weapon parts: a weapon
//! part may carry one [`Mod`] from the catalog. A mod costs build points and adds mass like any
//! part, fits only the weapons it names, may make its weapon a heavy module (its footprint grows
//! to at least two cells long), and changes how that weapon fires ([`Tuning`]: damage, refire,
//! range and the energy a shot costs). Every mod that changes the fire trades something away.
//!
//! A build does not yet change how a fighter moves, but it is its arsenal (shooter#92): a fighter
//! holds, fires and swings only the weapons its build mounts and still has, a weapon without a
//! mod fires as shipped, and a mounted weapon's mod applies whenever that copy fires, on the host
//! and in a client's prediction alike. Every part has component health; [`crate::damage`] places the
//! parts on the fighter as hitboxes, resolves strikes onto them and detaches what is destroyed:
//! a lost weapon part no longer fires, a lost battery takes its energy with it, a robot without a connected
//! locomotion part cannot walk, and the root chassis is the only kill. [`crate::mechanics`]
//! builds any valid build as a mechanical robot, but the match's fighters do not use it yet (shooter#73's
//! second half); locomotion (shooter#58) and the builder (shooter#57) apply the rest.

use crate::weapons::{Delivery, Instance, Weapon};
use parts::{
    BATTERY, CHASSIS_L, CHASSIS_M, CHASSIS_S, ENERGY_BLADE, ENERGY_RIFLE, FLAMETHROWER, GRAPPLE,
    LASER, LEG, RAIL_GUN, REPAIR_MODULE, ROCKET_LAUNCHER, SENSOR, SHOTGUN,
};
use std::fmt;

/// The edge of one socket cell in millimetres (provisional, Atlas Q2.i70.2).
pub const CELL_MM: u32 = 200;
/// The technical part-count ceiling: the most parts one build may have (provisional, Atlas
/// Q2.i70.1), separate from the build points.
pub const MAX_PARTS: usize = 32;
/// The baseline energy capacity the root chassis holds (whole units, [`crate::energy`]): the
/// retired control core's (D2.i89.1; provisional, Atlas Q2.i70.1).
pub const ROOT_CAPACITY: u16 = 100;
/// The energy the root chassis regenerates a second (whole units): the retired control core's
/// (D2.i89.1; provisional, Atlas Q2.i70.1).
pub const ROOT_REGEN: u16 = 24;

/// A face of a part's cuboid, in the part's own frame. `NegZ` is its mount face.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub enum Face {
    PosX,
    NegX,
    PosY,
    NegY,
    PosZ,
    NegZ,
}

impl Face {
    /// Every face, in socket-numbering order.
    pub const ALL: [Self; 6] = [
        Self::PosX,
        Self::NegX,
        Self::PosY,
        Self::NegY,
        Self::PosZ,
        Self::NegZ,
    ];

    /// The face's socket grid on a cuboid of `cells`: `(u, v)` cell counts. The grid's axes are
    /// part axes chosen so that `u` × `v` is the face's outward normal: +X (y, z), −X (z, y),
    /// +Y (z, x), −Y (x, z), +Z (x, y), −Z (y, x); cells count from the cuboid's minimum corner.
    #[must_use]
    pub const fn grid(self, [x, y, z]: [u8; 3]) -> (u8, u8) {
        match self {
            Self::PosX => (y, z),
            Self::NegX => (z, y),
            Self::PosY => (z, x),
            Self::NegY => (x, z),
            Self::PosZ => (x, y),
            Self::NegZ => (y, x),
        }
    }

    const fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// The faces of a part type that carry sockets.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Faces(u8);

impl Faces {
    pub const NONE: Self = Self(0);
    pub const ALL: Self = Self(0b11_1111);
    /// Only the face opposite the mount (a joint's moving side).
    pub const TOP: Self = Self(Face::PosZ.bit());

    #[must_use]
    pub const fn contains(self, face: Face) -> bool {
        self.0 & face.bit() != 0
    }
}

/// What a part type is.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Kind {
    /// A cuboid chassis section; a build's root is one, and the root is the robot's vital part
    /// and its baseline energy (D2.i89.1).
    Chassis,
    /// A structural beam extending the tree.
    Beam,
    /// A movable joint: its base is rigid with its parent, its top face and what is mounted
    /// there turn about one axis ([`crate::mechanics`]).
    Joint(Joint),
    /// A battery: stored energy capacity only (D2.i70.3).
    Battery,
    Sensor,
    /// A repair module (D2.i70.9): when the player activates it, it restores a damaged part
    /// still connected within its radius of where it is mounted ([`crate::repair`]).
    Repair,
    /// Legs, wheels or tracks: optional (D2.i70.1), driven as they are mounted
    /// ([`crate::mechanics`]).
    Locomotion(Locomotion),
    Armor,
    /// A weapon module: one of the shipped weapons as a mountable part (D2.i46.8).
    Weapon(Weapon),
}

/// A movable joint part.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Joint {
    /// A servo about the part's own X axis (across its top face), within a quarter turn either
    /// way.
    Hinge,
    /// A continuous rotary motor about the part's own Z axis (its mount normal): a turret ring.
    Rotor,
}

/// A locomotion part.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Locomotion {
    /// One self-contained leg module: a hip and a knee it drives itself.
    Leg,
    /// A wheel turning about its mount normal.
    Wheel,
    /// A track: road wheels along its length turning about its mount normal.
    Track,
}

/// What a motorised part can do (zero for the rest): the most torque each of its motors gives
/// (N·m), the fastest it turns (tenths of a radian a second) and the energy it draws a second
/// while commanded ([`crate::energy`]). Provisional (Q2.i70.1).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct Drive {
    pub torque: u16,
    pub rate: u16,
    pub draw: u16,
}

impl Drive {
    pub const NONE: Self = Self {
        torque: 0,
        rate: 0,
        draw: 0,
    };
}

/// A part type's stable code in the catalog (on the wire).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct PartTypeId(pub u8);

/// A part's stable ID within its build.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct PartId(pub u8);

/// A socket's stable index on its part ([`PartType::socket`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct SocketId(pub u8);

/// Where a socket is on its part: a face and a cell of that face's grid ([`Face::grid`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Site {
    pub face: Face,
    pub u: u8,
    pub v: u8,
}

/// A catalog part type. Masses are kilograms and energy whole energy units
/// ([`crate::energy`]); all values are provisional (Q2.i70.1, Q2.i70.2).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PartType {
    pub id: PartTypeId,
    /// Its stable data ID, shared with the robot or weapon kit's part ID.
    pub key: &'static str,
    pub kind: Kind,
    /// Its cuboid in cells (x, y, z of its own frame; its base is x × y).
    pub cells: [u8; 3],
    /// The faces with sockets.
    pub sockets: Faces,
    pub mass: u16,
    /// Energy it stores (batteries; the root chassis' baseline is [`ROOT_CAPACITY`]).
    pub capacity: u16,
    /// Energy it regenerates a second (none; the root chassis' is [`ROOT_REGEN`]).
    pub regen: u16,
    /// Its component health ([`crate::damage`]): it works fully until this reaches zero, then
    /// it is destroyed.
    pub health: u16,
    pub points: u16,
    /// Its motors (joints and locomotion, [`crate::mechanics`]).
    pub drive: Drive,
}

impl PartType {
    /// How many sockets it has.
    #[must_use]
    pub const fn socket_count(&self) -> u16 {
        let mut count = 0;
        let mut index = 0;
        while index < Face::ALL.len() {
            let face = Face::ALL[index];
            if self.sockets.contains(face) {
                let (u, v) = face.grid(self.cells);
                count += u as u16 * v as u16;
            }
            index += 1;
        }
        count
    }

    /// The socket at cell `(u, v)` of `face`, if the face has sockets and the cell exists.
    #[must_use]
    pub const fn socket(&self, face: Face, u: u8, v: u8) -> Option<SocketId> {
        if !self.sockets.contains(face) {
            return None;
        }
        let (width, height) = face.grid(self.cells);
        if u >= width || v >= height {
            return None;
        }
        let mut base: u16 = 0;
        let mut index = 0;
        while index < Face::ALL.len() {
            let earlier = Face::ALL[index];
            if earlier as u8 == face as u8 {
                break;
            }
            if self.sockets.contains(earlier) {
                let (eu, ev) = earlier.grid(self.cells);
                base += eu as u16 * ev as u16;
            }
            index += 1;
        }
        let socket = base + v as u16 * width as u16 + u as u16;
        if socket > u8::MAX as u16 {
            return None;
        }
        Some(SocketId(socket as u8))
    }

    /// Where `socket` is, if it has it.
    #[must_use]
    pub fn site(&self, socket: SocketId) -> Option<Site> {
        let mut rest = u16::from(socket.0);
        for face in Face::ALL {
            if !self.sockets.contains(face) {
                continue;
            }
            let (width, height) = face.grid(self.cells);
            let count = u16::from(width) * u16::from(height);
            if rest < count {
                let width = u16::from(width);
                return Some(Site {
                    face,
                    u: u8::try_from(rest % width).ok()?,
                    v: u8::try_from(rest / width).ok()?,
                });
            }
            rest -= count;
        }
        None
    }
}

/// A weapon modification's stable code in the catalog (on the wire).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct ModId(pub u8);

/// How a mod changes its weapon's fire, in percent of the shipped weapon's values (100 changes
/// nothing). Damage and range round down (to at least 1), refire and energy round up; a
/// continuous weapon keeps firing every tick.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Tuning {
    pub damage_percent: u16,
    pub refire_percent: u16,
    pub range_percent: u16,
    /// What one activation (a shot, a swing, a hook, a continuous weapon's tick) costs from the
    /// shared energy pool.
    pub energy_percent: u16,
}

impl Tuning {
    /// The shipped weapon, unchanged.
    pub const BASE: Self = Self {
        damage_percent: 100,
        refire_percent: 100,
        range_percent: 100,
        energy_percent: 100,
    };

    /// The energy (milli-units, [`crate::energy`]) one activation of `weapon` costs.
    #[must_use]
    pub fn energy(self, weapon: Weapon) -> u32 {
        (weapon.energy() * u32::from(self.energy_percent)).div_ceil(100)
    }

    /// Ticks between two of `weapon`'s shots.
    #[must_use]
    pub fn refire_ticks(self, weapon: Weapon) -> u32 {
        let base = weapon.refire_ticks();
        if base <= 1 {
            return base;
        }
        (base * u32::from(self.refire_percent)).div_ceil(100).max(1)
    }

    /// How `weapon`'s shot reaches its target, tuned.
    #[must_use]
    pub fn delivery(self, weapon: Weapon) -> Delivery {
        let scale = |value: i32, percent: u16| (value * i32::from(percent) / 100).max(1);
        match weapon.delivery() {
            Delivery::Hitscan {
                damage,
                range_units,
            } => Delivery::Hitscan {
                damage: scale(damage, self.damage_percent),
                range_units: scale(range_units, self.range_percent),
            },
            Delivery::Pellets { damage } => Delivery::Pellets {
                damage: scale(damage, self.damage_percent),
            },
            Delivery::Flame { damage } => Delivery::Flame {
                damage: scale(damage, self.damage_percent),
            },
            other => other,
        }
    }
}

/// A weapon modification (D2.i46.8): what it costs a build, which weapons it fits and how it
/// changes their fire. Masses are kilograms.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Mod {
    pub id: ModId,
    /// Its stable data ID.
    pub key: &'static str,
    /// The weapons it fits.
    pub fits: &'static [Weapon],
    /// It makes its weapon a heavy module: a footprint at least two cells long.
    pub heavy: bool,
    pub mass: u16,
    pub points: u16,
    pub tuning: Tuning,
}

/// The part types and weapon mods builds are made of, and the build-point budget.
#[derive(Clone, Copy, Debug)]
pub struct Catalog {
    pub parts: &'static [PartType],
    pub mods: &'static [Mod],
    pub build_points: u16,
}

/// The beta's part types (every one available from the start, D2.i46.5), as constants so builds
/// can name sockets on them ([`PartType::socket`]). Provisional values (Q2.i70.1, Q2.i70.2).
pub mod parts {
    use super::{Drive, Faces, Joint, Kind, Locomotion, PartType, PartTypeId};
    use crate::weapons::Weapon;

    const fn part(
        id: u8,
        key: &'static str,
        kind: Kind,
        cells: [u8; 3],
        sockets: Faces,
        [mass, capacity, regen, points]: [u16; 4],
    ) -> PartType {
        PartType {
            id: PartTypeId(id),
            key,
            kind,
            cells,
            sockets,
            mass,
            capacity,
            regen,
            health: 0,
            points,
            drive: Drive::NONE,
        }
    }

    /// `part` with motors of `torque` N·m turning at most `rate` tenths of a radian a second and
    /// drawing `draw` energy a second while commanded.
    const fn driven(part: PartType, [torque, rate, draw]: [u16; 3]) -> PartType {
        PartType {
            drive: Drive { torque, rate, draw },
            ..part
        }
    }

    /// `part` with `health` component health ([`crate::damage`]).
    const fn tough(part: PartType, health: u16) -> PartType {
        PartType { health, ..part }
    }

    /// A light weapon is 1 × 1 × 2 cells with 40 health, a heavy one (the rail gun, the
    /// rockets) 1 × 2 × 2 with 60.
    const fn weapon(
        weapon: Weapon,
        key: &'static str,
        heavy: bool,
        [mass, points]: [u16; 2],
    ) -> PartType {
        let (cells, health) = if heavy {
            ([1, 2, 2], 60)
        } else {
            ([1, 1, 2], 40)
        };
        tough(
            part(
                60 + weapon.index(),
                key,
                Kind::Weapon(weapon),
                cells,
                Faces::NONE,
                [mass, 0, 0, points],
            ),
            health,
        )
    }

    // The rows of the table: code, key, kind, cells, socket faces, then mass, energy capacity,
    // energy regenerated a second, and points (weapons: mass and points; what an activation
    // costs is the weapon's, `Weapon::energy`), and last the component health
    // ([`crate::damage`]).
    // A chassis' health is what a root chassis needs as the robot's vital part (D2.i89.1): within
    // (255, 318], four unmodded rail hits (85) and three overcharged ones (106) kill from the
    // front, as the chassis and then the control core took before (shooter#89). Provisional.
    /// 20 × 40 × 80 cm: 28 sockets.
    #[rustfmt::skip]
    pub const CHASSIS_S: PartType = tough(part(1, "robot.chassis.20x40x80", Kind::Chassis, [1, 2, 4], Faces::ALL, [60, 0, 0, 10]), 270);
    /// 40 × 40 × 80 cm: 40 sockets.
    #[rustfmt::skip]
    pub const CHASSIS_M: PartType = tough(part(2, "robot.chassis.40x40x80", Kind::Chassis, [2, 2, 4], Faces::ALL, [110, 0, 0, 20]), 290);
    /// 40 × 60 × 80 cm: 52 sockets.
    #[rustfmt::skip]
    pub const CHASSIS_L: PartType = tough(part(3, "robot.chassis.40x60x80", Kind::Chassis, [2, 3, 4], Faces::ALL, [150, 0, 0, 30]), 310);
    /// 20 × 20 × 40 cm.
    #[rustfmt::skip]
    pub const BEAM_SHORT: PartType = tough(part(10, "robot.beam.20x20x40", Kind::Beam, [1, 1, 2], Faces::ALL, [5, 0, 0, 2]), 50);
    /// 20 × 20 × 80 cm.
    #[rustfmt::skip]
    pub const BEAM_LONG: PartType = tough(part(11, "robot.beam.20x20x80", Kind::Beam, [1, 1, 4], Faces::ALL, [10, 0, 0, 3]), 60);
    // Motorised parts end with their drive: torque (N·m), top rate (tenths of a radian a
    // second) and the energy drawn a second while commanded.
    /// A hinge servo: one socket, on its moving side.
    #[rustfmt::skip]
    pub const HINGE: PartType = tough(driven(part(20, "robot.joint.hinge", Kind::Joint(Joint::Hinge), [1, 1, 1], Faces::TOP, [8, 0, 0, 5]), [40, 30, 1]), 40);
    /// A continuous rotor (a turret's base): one socket, on its moving side.
    #[rustfmt::skip]
    pub const ROTOR: PartType = tough(driven(part(21, "robot.joint.rotor", Kind::Joint(Joint::Rotor), [1, 1, 1], Faces::TOP, [10, 0, 0, 6]), [60, 40, 2]), 40);
    // Code 30 was the control core (`robot.core`), removed when the root chassis became the
    // vital part (shooter#89); it stays retired, so an old build naming it is an unknown part.
    /// A 20 × 40 × 20 cm battery: 60 more stored energy and nothing else (D2.i70.3), with 60
    /// component health for per-part damage (shooter#56).
    #[rustfmt::skip]
    pub const BATTERY: PartType = tough(part(32, "robot.battery", Kind::Battery, [1, 2, 1], Faces::NONE, [20, 60, 0, 5]), 60);
    /// A 20 × 20 × 20 cm field repair module (D2.i70.9, shooter#61): it restores a chosen
    /// damaged part within its radius ([`crate::repair`]), with 40 component health like a
    /// light weapon.
    #[rustfmt::skip]
    pub const REPAIR_MODULE: PartType = tough(part(33, "robot.repair-module", Kind::Repair, [1, 1, 1], Faces::NONE, [12, 0, 0, 8]), 40);
    #[rustfmt::skip]
    pub const SENSOR: PartType = tough(part(31, "robot.sensor", Kind::Sensor, [1, 1, 1], Faces::NONE, [10, 0, 0, 5]), 30);
    /// One self-contained leg, 80 cm long: a hip and a knee servo, each with this drive.
    #[rustfmt::skip]
    pub const LEG: PartType = tough(driven(part(40, "robot.leg", Kind::Locomotion(Locomotion::Leg), [1, 1, 4], Faces::NONE, [25, 0, 0, 7]), [300, 80, 4]), 70);
    /// A 40 cm wheel.
    #[rustfmt::skip]
    pub const WHEEL: PartType = tough(driven(part(41, "robot.wheel", Kind::Locomotion(Locomotion::Wheel), [2, 2, 1], Faces::NONE, [20, 0, 0, 6]), [30, 120, 2]), 50);
    /// An 80 cm track: two road wheels, each with this drive.
    #[rustfmt::skip]
    pub const TRACK: PartType = tough(driven(part(42, "robot.track", Kind::Locomotion(Locomotion::Track), [1, 4, 2], Faces::NONE, [45, 0, 0, 12]), [40, 120, 3]), 90);
    /// A 40 × 40 cm plate.
    #[rustfmt::skip]
    pub const ARMOR_PLATE: PartType = tough(part(50, "robot.armor.plate", Kind::Armor, [2, 2, 1], Faces::NONE, [15, 0, 0, 4]), 120);

    #[rustfmt::skip]
    pub const ENERGY_RIFLE: PartType = weapon(Weapon::EnergyRifle, "weapon.energy-rifle", false, [12, 10]);
    #[rustfmt::skip]
    pub const RAIL_GUN: PartType = weapon(Weapon::RailGun, "weapon.rail-gun", true, [25, 20]);
    #[rustfmt::skip]
    pub const ROCKET_LAUNCHER: PartType = weapon(Weapon::RocketLauncher, "weapon.rocket-launcher", true, [30, 20]);
    /// The grapple module (Atlas D2.i104.2, shooter#104): an optional component on any free
    /// socket, mounted and lost like a weapon part (10 kg, 10 build points, 40 health), but
    /// never a gun in the rotation: it hooks on its own key ([`crate::grapple`]). Its hook's
    /// energy and interval are [`Weapon::energy`] and [`Weapon::refire_ticks`] of
    /// [`Weapon::Grapple`], its reach [`crate::grapple::RANGE_UNITS`]: provisional tuning. The
    /// weapons kit's `weapon.grapple-hook` model draws it.
    #[rustfmt::skip]
    pub const GRAPPLE: PartType = weapon(Weapon::Grapple, "weapon.grapple-hook", false, [10, 10]);
    #[rustfmt::skip]
    pub const ENERGY_BLADE: PartType = weapon(Weapon::EnergyBlade, "weapon.energy-blade", false, [8, 5]);
    #[rustfmt::skip]
    pub const SHOTGUN: PartType = weapon(Weapon::Shotgun, "weapon.shotgun", false, [15, 15]);
    #[rustfmt::skip]
    pub const LASER: PartType = weapon(Weapon::Laser, "weapon.laser", false, [14, 15]);
    #[rustfmt::skip]
    pub const FLAMETHROWER: PartType = weapon(Weapon::Flamethrower, "weapon.flamethrower", false, [18, 15]);
}

const fn modification(
    id: u8,
    key: &'static str,
    fits: &'static [Weapon],
    heavy: bool,
    [mass, points]: [u16; 2],
    [
        damage_percent,
        refire_percent,
        range_percent,
        energy_percent,
    ]: [u16; 4],
) -> Mod {
    Mod {
        id: ModId(id),
        key,
        fits,
        heavy,
        mass,
        points,
        tuning: Tuning {
            damage_percent,
            refire_percent,
            range_percent,
            energy_percent,
        },
    }
}

/// The weapons the firing mods fit: the single-shot guns whose damage is worth tuning.
const SINGLE_SHOT: &[Weapon] = &[Weapon::EnergyRifle, Weapon::RailGun, Weapon::Shotgun];
/// The hitscan beams a longer barrel reaches further with.
const BEAMS: &[Weapon] = &[Weapon::EnergyRifle, Weapon::RailGun, Weapon::Laser];
/// The single-shot guns whose shots an efficiency coil charges more slowly.
const SHOTS: &[Weapon] = &[
    Weapon::EnergyRifle,
    Weapon::RailGun,
    Weapon::RocketLauncher,
    Weapon::Shotgun,
];

pub const RAPID_CYCLER: ModId = ModId(1);
pub const OVERCHARGER: ModId = ModId(2);
pub const LONG_BARREL: ModId = ModId(3);
// Code 4 was the heat sink (shooter#51), removed with heat (shooter#72); it stays retired, so an
// old build naming it is an unknown mod.
pub const EFFICIENCY_COIL: ModId = ModId(5);

/// The beta's catalog: three chassis sizes, two beams, two joints, a battery, a sensor, a
/// repair module, legs, wheels, tracks, an armor plate, the eight shipped weapons and the four weapon mods.
/// Provisional values (Q2.i70.1, Q2.i70.2).
#[rustfmt::skip]
pub const BETA: Catalog = Catalog {
    parts: &[
        parts::CHASSIS_S, parts::CHASSIS_M, parts::CHASSIS_L,
        parts::BEAM_SHORT, parts::BEAM_LONG,
        parts::HINGE, parts::ROTOR,
        parts::BATTERY, parts::SENSOR, parts::REPAIR_MODULE,
        parts::LEG, parts::WHEEL, parts::TRACK,
        parts::ARMOR_PLATE,
        parts::ENERGY_RIFLE, parts::RAIL_GUN, parts::ROCKET_LAUNCHER, parts::GRAPPLE,
        parts::ENERGY_BLADE, parts::SHOTGUN, parts::LASER, parts::FLAMETHROWER,
    ],
    mods: &[
        //           code key                    fits         heavy  mass points  damage% refire% range% energy%
        // Faster fire for less damage a shot: more damage a second, but each shot costs as much,
        // so it drains the pool half again as fast.
        modification(1, "mod.rapid-cycler",     SINGLE_SHOT, false, [4,  10], [75,  65,  100, 100]),
        // More damage a shot for slower fire, at a higher energy cost a damage point.
        modification(2, "mod.overcharger",      SINGLE_SHOT, false, [6,  10], [125, 130, 100, 150]),
        // Half again the range, a little slower fire, and a heavy (two-cell) module.
        modification(3, "mod.long-barrel",      BEAMS,       true,  [10, 5],  [100, 110, 150, 100]),
        // Shots cost 60 % of the energy but charge a quarter slower: sustained fire on less.
        modification(5, "mod.efficiency-coil",  SHOTS,       false, [6,  5],  [100, 125, 100, 60]),
    ],
    // 110: the 120 every build had less the 10 its mandatory control core took (shooter#89), so
    // a build's headroom stays what it was.
    build_points: 110,
};

/// A part's orientation on its socket: binary angles (65 536 a turn), yaw about the socket's
/// outward normal, then pitch, then roll (intrinsic Z-Y-X), always in canonical form
/// ([`Orientation::canonical`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct Orientation {
    pub yaw: u16,
    /// Within a quarter turn either way.
    pub pitch: i16,
    /// Zero when the pitch is a whole quarter turn.
    pub roll: u16,
}

impl Orientation {
    pub const IDENTITY: Self = Self {
        yaw: 0,
        pitch: 0,
        roll: 0,
    };
    pub const QUARTER: u16 = 16_384;
    const HALF: u16 = 2 * Self::QUARTER;

    /// The one canonical representation of the rotation `yaw`, `pitch`, `roll` (any binary
    /// angles): a pitch past a quarter turn becomes its supplement with yaw and roll turned half
    /// way (the same rotation), and at a pitch of exactly a quarter turn, where yaw and roll turn
    /// about the same axis, the roll folds into the yaw. Exact integer arithmetic.
    #[must_use]
    pub const fn canonical(yaw: u16, pitch: u16, roll: u16) -> Self {
        let (mut yaw, pitch, mut roll) = if pitch <= Self::QUARTER || pitch >= 3 * Self::QUARTER {
            (yaw, pitch, roll)
        } else {
            (
                yaw.wrapping_add(Self::HALF),
                Self::HALF.wrapping_sub(pitch),
                roll.wrapping_add(Self::HALF),
            )
        };
        #[allow(clippy::cast_possible_wrap)]
        let pitch = pitch as i16;
        if pitch == Self::QUARTER as i16 {
            yaw = yaw.wrapping_sub(roll);
            roll = 0;
        } else if pitch == -(Self::QUARTER as i16) {
            yaw = yaw.wrapping_add(roll);
            roll = 0;
        }
        Self { yaw, pitch, roll }
    }

    /// The yaw's nearest quarter turn (0 to 3; a tie turns further): how the footprint turns.
    #[must_use]
    pub const fn quarter_turns(self) -> u8 {
        (self.yaw.wrapping_add(Self::QUARTER / 2) / Self::QUARTER) as u8
    }

    /// Its raw `(yaw, pitch, roll)` codes.
    #[must_use]
    pub const fn raw(self) -> (u16, u16, u16) {
        #[allow(clippy::cast_sign_loss)]
        (self.yaw, self.pitch as u16, self.roll)
    }
}

/// Where a part attaches: a socket of its parent, and how it is turned there.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Mount {
    pub parent: PartId,
    pub socket: SocketId,
    pub orientation: Orientation,
}

/// One part of a validated build.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct Part {
    pub id: PartId,
    pub kind: PartTypeId,
    /// `None` for the root.
    pub mount: Option<Mount>,
    /// The mod on it (weapon parts only).
    pub modification: Option<ModId>,
}

impl Part {
    const EMPTY: Self = Self {
        id: PartId(0),
        kind: PartTypeId(0),
        mount: None,
        modification: None,
    };

    const fn root(id: u8, kind: &PartType) -> Self {
        Self {
            id: PartId(id),
            kind: kind.id,
            mount: None,
            modification: None,
        }
    }

    /// `kind` on `parent` (of type `on`) at cell `(u, v)` of its `face`, unturned.
    const fn on(
        id: u8,
        kind: &PartType,
        parent: u8,
        on: &PartType,
        face: Face,
        u: u8,
        v: u8,
    ) -> Self {
        let Some(socket) = on.socket(face, u, v) else {
            panic!("no such socket");
        };
        Self {
            id: PartId(id),
            kind: kind.id,
            mount: Some(Mount {
                parent: PartId(parent),
                socket,
                orientation: Orientation::IDENTITY,
            }),
            modification: None,
        }
    }

    const fn with(self, modification: ModId) -> Self {
        Self {
            modification: Some(modification),
            ..self
        }
    }
}

/// Untrusted raw data for one part, as it arrived.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct RawPart {
    pub id: u8,
    pub kind: u8,
    pub mount: Option<RawMount>,
    pub modification: Option<u8>,
}

/// Untrusted raw data for a mount: any parent and socket code, any angles.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub struct RawMount {
    pub parent: u8,
    pub socket: u8,
    pub yaw: u16,
    pub pitch: u16,
    pub roll: u16,
}

/// Untrusted build data as it arrived, parts in any order. Only [`Catalog::validate`] turns it
/// into an [`Assembly`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Candidate {
    pub parts: Vec<RawPart>,
}

/// A validated robot build: its parts in [`PartId`] order. At most [`MAX_PARTS`] parts, held
/// inline so a build copies like the rest of a fighter's state.
#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub struct Assembly {
    parts: [Part; MAX_PARTS],
    len: u8,
}

impl fmt::Debug for Assembly {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(self.parts()).finish()
    }
}

/// `parts` (in ID order, at most [`MAX_PARTS`]) as an assembly: the named loadouts (valid in
/// [`BETA`], tested) and candidates before their rules are checked.
const fn authored(parts: &[Part]) -> Assembly {
    assert!(parts.len() <= MAX_PARTS, "too many parts");
    let mut assembly = Assembly {
        parts: [Part::EMPTY; MAX_PARTS],
        len: parts.len() as u8,
    };
    let mut index = 0;
    while index < parts.len() {
        assembly.parts[index] = parts[index];
        index += 1;
    }
    assembly
}

/// The frame the named loadouts share, on chassis `torso` (the root, the vital part): the sensor
/// on its top and two legs underneath (IDs 0, 2, 5 and 6). ID 1 was the retired control core's
/// and IDs 3 and 4 the arm beams' (removed, shooter#91); they stay unused, so the other IDs keep
/// theirs.
const fn frame(torso: &PartType) -> [Part; 4] {
    [
        Part::root(0, torso),
        Part::on(2, &SENSOR, 0, torso, Face::PosZ, 0, 0),
        Part::on(5, &LEG, 0, torso, Face::NegZ, 0, 0),
        Part::on(6, &LEG, 0, torso, Face::NegZ, 1, 0),
    ]
}

/// The four weapons IDs 7 to 10 on `frame`, then `extra` (at most one more part).
const fn with_weapons(frame: [Part; 4], weapons: [Part; 4], extra: Option<Part>) -> Assembly {
    let [a, b, c, d] = frame;
    let [e, f, g, h] = weapons;
    match extra {
        Some(i) => authored(&[a, b, c, d, e, f, g, h, i]),
        None => authored(&[a, b, c, d, e, f, g, h]),
    }
}

/// `weapon` on the front (+X, the aim) of chassis `torso` at cell `(u, v)`, unturned: its mount
/// face on the chassis and its muzzle, along its own +Z, pointing forward. On the front face `u`
/// counts from the robot's right to its left and `v` from the bottom up; a heavy weapon (or a
/// heavy mod) reserves the cell above too.
const fn forward(id: u8, weapon: &PartType, torso: &PartType, u: u8, v: u8) -> Part {
    Part::on(id, weapon, 0, torso, Face::PosX, u, v)
}

/// The build every fighter starts with (re-expressing the shipped standard build): the small
/// chassis as the torso, every weapon on its front facing the aim: the energy rifle low on the
/// left, the blade low on the right, the shotgun high on the left and the grapple high on the
/// right. Valid in [`BETA`].
pub const STANDARD: Assembly = with_weapons(
    frame(&CHASSIS_S),
    [
        forward(7, &ENERGY_RIFLE, &CHASSIS_S, 1, 2),
        forward(8, &ENERGY_BLADE, &CHASSIS_S, 0, 2),
        forward(9, &GRAPPLE, &CHASSIS_S, 0, 3),
        forward(10, &SHOTGUN, &CHASSIS_S, 1, 3),
    ],
    None,
);

/// A heavy build: the medium chassis with, on its front facing the aim, the rail gun high on the
/// left, the rocket launcher high on the right, the grapple below the rail gun and the blade
/// below the rockets and, for the energy its rail shots and rockets take, a battery on its back
/// (160 energy, not 100). Valid in [`BETA`].
pub const HEAVY: Assembly = with_weapons(
    frame(&CHASSIS_M),
    [
        forward(7, &RAIL_GUN, &CHASSIS_M, 1, 2),
        forward(8, &ENERGY_BLADE, &CHASSIS_M, 0, 1),
        forward(9, &ROCKET_LAUNCHER, &CHASSIS_M, 0, 2),
        forward(10, &GRAPPLE, &CHASSIS_M, 1, 1),
    ],
    Some(Part::on(11, &BATTERY, 0, &CHASSIS_M, Face::NegX, 3, 0)),
);

/// A fast, light loadout on the standard frame and mounts: the rapid-cycler rifle (which drains
/// the pool half again as fast as the shipped rifle), the blade, the grapple, and the shotgun
/// with an efficiency coil (cheaper blasts, so the pool lasts). Valid in [`BETA`].
pub const SKIRMISHER: Assembly = with_weapons(
    frame(&CHASSIS_S),
    [
        forward(7, &ENERGY_RIFLE, &CHASSIS_S, 1, 2).with(RAPID_CYCLER),
        forward(8, &ENERGY_BLADE, &CHASSIS_S, 0, 2),
        forward(9, &GRAPPLE, &CHASSIS_S, 0, 3),
        forward(10, &SHOTGUN, &CHASSIS_S, 1, 3).with(EFFICIENCY_COIL),
    ],
    None,
);

/// A heavy, long-range loadout: the medium chassis with, on its front facing the aim, the
/// long-barrel rifle (a two-cell module) high on the left, an overcharged rail gun (106 a hit
/// instead of 85, for half again the energy a shot) high on the right, the grapple below the
/// rifle and the blade below the rail gun; no battery, so it lives on the root chassis' 100.
/// Valid in [`BETA`].
pub const MARKSMAN: Assembly = with_weapons(
    frame(&CHASSIS_M),
    [
        forward(7, &ENERGY_RIFLE, &CHASSIS_M, 1, 2).with(LONG_BARREL),
        forward(8, &ENERGY_BLADE, &CHASSIS_M, 0, 1),
        forward(9, &RAIL_GUN, &CHASSIS_M, 0, 2).with(OVERCHARGER),
        forward(10, &GRAPPLE, &CHASSIS_M, 1, 1),
    ],
    None,
);

/// A close- and mid-range loadout on the standard frame: on the small chassis' front facing the
/// aim, the laser with a long barrel (a two-cell module: half again the beam's reach) high on
/// the left, the flamethrower high on the right, the blade below it and the grapple below the
/// laser; no battery, so it lives on the root chassis' 100. The one named loadout carrying the
/// laser and the flamethrower (shooter#96). Valid in [`BETA`].
pub const PYRO: Assembly = with_weapons(
    frame(&CHASSIS_S),
    [
        forward(7, &LASER, &CHASSIS_S, 1, 2).with(LONG_BARREL),
        forward(8, &ENERGY_BLADE, &CHASSIS_S, 0, 2),
        forward(9, &FLAMETHROWER, &CHASSIS_S, 0, 3),
        forward(10, &GRAPPLE, &CHASSIS_S, 1, 1),
    ],
    None,
);

/// A field-repair loadout (shooter#61): the standard build with a repair module (ID 11) on the
/// small chassis' back, its second cell up, whose radius reaches every other part
/// ([`crate::repair::RADIUS_UNITS`]). Valid in [`BETA`].
pub const MECHANIC: Assembly = with_weapons(
    frame(&CHASSIS_S),
    [
        forward(7, &ENERGY_RIFLE, &CHASSIS_S, 1, 2),
        forward(8, &ENERGY_BLADE, &CHASSIS_S, 0, 2),
        forward(9, &GRAPPLE, &CHASSIS_S, 0, 3),
        forward(10, &SHOTGUN, &CHASSIS_S, 1, 3),
    ],
    Some(Part::on(
        11,
        &REPAIR_MODULE,
        0,
        &CHASSIS_S,
        Face::NegX,
        1,
        0,
    )),
);

/// A fixture with two rail guns (shooter#125): the heavy frame with a rail gun high on either
/// side of the medium chassis' front (IDs 7 and 9), the blade below the left one, the grapple
/// below the right one and a battery on its back for the shots. Each copy is its own weapon: its
/// own mount, mod, health and refire. Valid in [`BETA`].
pub const TWIN_RAIL: Assembly = with_weapons(
    frame(&CHASSIS_M),
    [
        forward(7, &RAIL_GUN, &CHASSIS_M, 1, 2),
        forward(8, &ENERGY_BLADE, &CHASSIS_M, 1, 1),
        forward(9, &RAIL_GUN, &CHASSIS_M, 0, 2),
        forward(10, &GRAPPLE, &CHASSIS_M, 0, 1),
    ],
    Some(Part::on(11, &BATTERY, 0, &CHASSIS_M, Face::NegX, 3, 0)),
);

/// A fixture with four rocket launchers (shooter#125) filling the small chassis' front, two low
/// and two high (IDs 7 to 10), and nothing else: 109 of the 110 build points. One launcher fires
/// at a time, the one held. Valid in [`BETA`].
pub const ROCKET_POD: Assembly = with_weapons(
    frame(&CHASSIS_S),
    [
        forward(7, &ROCKET_LAUNCHER, &CHASSIS_S, 1, 0),
        forward(8, &ROCKET_LAUNCHER, &CHASSIS_S, 0, 0),
        forward(9, &ROCKET_LAUNCHER, &CHASSIS_S, 1, 2),
        forward(10, &ROCKET_LAUNCHER, &CHASSIS_S, 0, 2),
    ],
    None,
);

/// The test ranges' build: every weapon, unmodded, on the front of the large chassis (the
/// light ones in the bottom two rows, the rail gun and the rocket launcher above), on the shared
/// frame. A fighter fires only what its build carries (shooter#92), so the scripted weapon
/// scenarios and the weapon tests use it. It costs more build points than [`BETA`] allows, so
/// it is never a player's: the host refuses it like any invalid build.
pub const ARMORY: Assembly = authored(&[
    Part::root(0, &CHASSIS_L),
    Part::on(2, &SENSOR, 0, &CHASSIS_L, Face::PosZ, 0, 0),
    Part::on(5, &LEG, 0, &CHASSIS_L, Face::NegZ, 0, 0),
    Part::on(6, &LEG, 0, &CHASSIS_L, Face::NegZ, 1, 0),
    forward(7, &ENERGY_RIFLE, &CHASSIS_L, 0, 0),
    forward(8, &ENERGY_BLADE, &CHASSIS_L, 1, 0),
    forward(9, &GRAPPLE, &CHASSIS_L, 2, 0),
    forward(10, &SHOTGUN, &CHASSIS_L, 0, 1),
    forward(11, &LASER, &CHASSIS_L, 1, 1),
    forward(12, &FLAMETHROWER, &CHASSIS_L, 2, 1),
    forward(13, &RAIL_GUN, &CHASSIS_L, 0, 2),
    forward(14, &ROCKET_LAUNCHER, &CHASSIS_L, 1, 2),
]);

/// The named loadouts a person picks from until the builder (shooter#57): the unmodified
/// standard and heavy builds, the two contrasting modded ones and the pyro (the laser and the
/// flamethrower, shooter#96) and the mechanic (a field repair module, shooter#61), all
/// component trees. The builds with several copies of one weapon ([`TWIN_RAIL`], [`ROCKET_POD`])
/// are fixtures, not among them; the builder makes any such build.
pub const LOADOUTS: [(&str, Assembly); 6] = [
    ("standard", STANDARD),
    ("heavy", HEAVY),
    ("skirmisher", SKIRMISHER),
    ("marksman", MARKSMAN),
    ("pyro", PYRO),
    ("mechanic", MECHANIC),
];

/// The loadout called `name` in [`LOADOUTS`] (any case).
#[must_use]
pub fn loadout(name: &str) -> Option<Assembly> {
    LOADOUTS
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name.trim()))
        .map(|(_, build)| *build)
}

impl Default for Assembly {
    fn default() -> Self {
        STANDARD
    }
}

impl Assembly {
    /// Its parts, in ID order.
    #[must_use]
    pub fn parts(&self) -> &[Part] {
        &self.parts[..usize::from(self.len)]
    }

    #[must_use]
    pub fn part(&self, id: PartId) -> Option<&Part> {
        self.parts().iter().find(|part| part.id == id)
    }

    /// The parts mounted on `id`, in ID order.
    pub fn children(&self, id: PartId) -> impl Iterator<Item = &Part> + '_ {
        self.parts()
            .iter()
            .filter(move |part| part.mount.is_some_and(|mount| mount.parent == id))
    }

    /// Its canonical raw form (what the wire carries).
    #[must_use]
    pub fn candidate(&self) -> Candidate {
        Candidate {
            parts: self
                .parts()
                .iter()
                .map(|part| RawPart {
                    id: part.id.0,
                    kind: part.kind.0,
                    mount: part.mount.map(|mount| {
                        let (yaw, pitch, roll) = mount.orientation.raw();
                        RawMount {
                            parent: mount.parent.0,
                            socket: mount.socket.0,
                            yaw,
                            pitch,
                            roll,
                        }
                    }),
                    modification: part.modification.map(|id| id.0),
                })
                .collect(),
        }
    }
}

/// A build's totals against its limits.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Budget {
    pub parts: u32,
    /// Total mass (no limit: nothing carries it yet, shooter#73).
    pub mass: u32,
    /// The shared energy pool's capacity (whole units): the root chassis' and every battery's.
    pub capacity: u32,
    /// Energy regenerated a second (whole units): the root chassis'.
    pub regen: u32,
    pub points: u32,
    pub build_points: u32,
}

/// Why a candidate is not a valid build.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssemblyError {
    /// More parts than [`MAX_PARTS`].
    TooManyParts(usize),
    UnknownPart(u8),
    UnknownMod(u8),
    /// The same part ID twice (a part with two parents is one).
    DuplicateId(PartId),
    /// No part without a parent.
    NoRoot,
    /// Two parts without a parent.
    MultipleRoots(PartId, PartId),
    /// The root is not a chassis section.
    RootNotChassis(PartId),
    /// The part's parent is not in the build.
    UnknownParent(PartId),
    /// The part's parent has no such socket.
    NoSocket(PartId),
    /// The part sits on its parent's mount face, which rests on the parent's own parent.
    InteriorSocket(PartId),
    /// The part's parents never reach the root: they loop.
    Cycle(PartId),
    /// The second part's footprint covers a cell the first's holds.
    Overlap(PartId, PartId),
    /// A mod on a part that is not a weapon.
    NotAWeapon(PartId),
    /// The mod does not fit the weapon.
    DoesNotFit(ModId, Weapon),
    OverBudget {
        points: u32,
        build_points: u32,
    },
}

impl fmt::Display for AssemblyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyParts(parts) => {
                write!(formatter, "{parts} parts, at most {MAX_PARTS}")
            }
            Self::UnknownPart(code) => write!(formatter, "unknown part type {code}"),
            Self::UnknownMod(code) => write!(formatter, "unknown weapon mod {code}"),
            Self::DuplicateId(id) => write!(formatter, "part {} appears twice", id.0),
            Self::NoRoot => formatter.write_str("no root part"),
            Self::MultipleRoots(first, second) => {
                write!(
                    formatter,
                    "parts {} and {} are both roots",
                    first.0, second.0
                )
            }
            Self::RootNotChassis(id) => write!(formatter, "root part {} is no chassis", id.0),
            Self::UnknownParent(id) => write!(formatter, "part {}'s parent is missing", id.0),
            Self::NoSocket(id) => {
                write!(formatter, "part {}'s parent has no such socket", id.0)
            }
            Self::InteriorSocket(id) => {
                write!(formatter, "part {} sits on an interior socket", id.0)
            }
            Self::Cycle(id) => write!(formatter, "part {} is not connected to the root", id.0),
            Self::Overlap(first, second) => {
                write!(formatter, "parts {} and {} overlap", first.0, second.0)
            }
            Self::NotAWeapon(id) => write!(formatter, "a mod on part {}, no weapon", id.0),
            Self::DoesNotFit(id, weapon) => {
                write!(formatter, "mod {} does not fit {weapon:?}", id.0)
            }
            Self::OverBudget {
                points,
                build_points,
            } => write!(
                formatter,
                "costs {points} build points, the budget is {build_points}"
            ),
        }
    }
}

impl std::error::Error for AssemblyError {}

/// The energy capacity and regeneration a second (whole units) `part`, of type `kind`, gives the
/// shared pool: its type's, and the baseline too if it is the root chassis (D2.i89.1).
#[must_use]
pub fn energy(part: &Part, kind: &PartType) -> (u16, u16) {
    if part.mount.is_none() && kind.kind == Kind::Chassis {
        (kind.capacity + ROOT_CAPACITY, kind.regen + ROOT_REGEN)
    } else {
        (kind.capacity, kind.regen)
    }
}

/// The cells of a face of `grid` that a footprint of `(width, length)` reserves from `(u, v)`,
/// turned `quarters` quarter turns about the face's normal (cells past the face's edge overhang
/// and reserve nothing).
fn footprint(
    (width, length): (u8, u8),
    quarters: u8,
    (u, v): (u8, u8),
    (columns, rows): (u8, u8),
) -> impl Iterator<Item = (u8, u8)> {
    (0..i16::from(width))
        .flat_map(move |i| (0..i16::from(length)).map(move |j| (i, j)))
        .filter_map(move |(i, j)| {
            let (du, dv) = match quarters & 3 {
                0 => (i, j),
                1 => (-j, i),
                2 => (-i, -j),
                _ => (j, -i),
            };
            let cell_u = u8::try_from(i16::from(u) + du).ok()?;
            let cell_v = u8::try_from(i16::from(v) + dv).ok()?;
            (cell_u < columns && cell_v < rows).then_some((cell_u, cell_v))
        })
}

impl Catalog {
    #[must_use]
    pub fn part_type(&self, id: PartTypeId) -> Option<&PartType> {
        self.parts.iter().find(|part| part.id == id)
    }

    #[must_use]
    pub fn modification(&self, id: ModId) -> Option<&Mod> {
        self.mods.iter().find(|modification| modification.id == id)
    }

    /// The weapon `part` is, if it is one.
    #[must_use]
    pub fn weapon(&self, part: &Part) -> Option<Weapon> {
        match self.part_type(part.kind)?.kind {
            Kind::Weapon(weapon) => Some(weapon),
            _ => None,
        }
    }

    /// The weapons mounted in `build`, every copy its own instance, in part ID order.
    pub fn weapons<'a>(&'a self, build: &'a Assembly) -> impl Iterator<Item = Instance> + 'a {
        build.parts().iter().filter_map(|part| {
            Some(Instance {
                part: part.id,
                weapon: self.weapon(part)?,
            })
        })
    }

    /// The first copy of `weapon` mounted in `build` (the lowest part ID), if any.
    #[must_use]
    pub fn first(&self, build: &Assembly, weapon: Weapon) -> Option<Instance> {
        self.weapons(build)
            .find(|instance| instance.weapon == weapon)
    }

    /// The weapon mounted as part `part` of `build`, if it is one.
    #[must_use]
    pub fn instance(&self, build: &Assembly, part: PartId) -> Option<Instance> {
        let weapon = self.weapon(build.part(part)?)?;
        Some(Instance { part, weapon })
    }

    /// Which copy `instance` is of its weapon in `build`, and how many copies `build` mounts
    /// (lost ones too, so the numbers stay put): `(1, 1)` for a weapon mounted once, `(2, 4)`
    /// for the second of four rocket launchers by part ID. `None` if it is no weapon of the
    /// build.
    #[must_use]
    pub fn copy(&self, build: &Assembly, instance: Instance) -> Option<(u8, u8)> {
        let copies = || {
            self.weapons(build)
                .filter(move |other| other.weapon == instance.weapon)
        };
        let index = copies().position(|other| other == instance)?;
        let count = copies().count();
        Some((
            u8::try_from(index + 1).unwrap_or(u8::MAX),
            u8::try_from(count).unwrap_or(u8::MAX),
        ))
    }

    /// The mod on part `part` of `build`, if any.
    #[must_use]
    pub fn mod_at(&self, build: &Assembly, part: PartId) -> Option<ModId> {
        build.part(part).and_then(|part| part.modification)
    }

    /// The mods in `build` with the weapon each is on, in part ID order.
    pub fn mods<'a>(
        &'a self,
        build: &'a Assembly,
    ) -> impl Iterator<Item = (PartId, Weapon, ModId)> + 'a {
        build
            .parts()
            .iter()
            .filter_map(|part| Some((part.id, self.weapon(part)?, part.modification?)))
    }

    /// How the copy `instance` fires in `build`: its mod's tuning, else the shipped weapon's.
    #[must_use]
    pub fn tuning(&self, build: &Assembly, instance: Instance) -> Tuning {
        build
            .part(instance.part)
            .filter(|part| self.weapon(part) == Some(instance.weapon))
            .and_then(|part| part.modification)
            .and_then(|id| self.modification(id))
            .map_or(Tuning::BASE, |modification| modification.tuning)
    }

    /// The cells of `site`'s face on a parent of type `parent` that a part of type `kind` with
    /// `modification`, turned `orientation` there, reserves: its base (a heavy mod makes it at
    /// least two cells long) turned with the yaw's nearest quarter turn, cells past the face's
    /// edge left out. The footprint rule of [`Catalog::validate`].
    fn reserved(
        &self,
        kind: &PartType,
        modification: Option<ModId>,
        orientation: Orientation,
        site: Site,
        parent: &PartType,
    ) -> impl Iterator<Item = (u8, u8)> + use<> {
        let heavy = modification
            .and_then(|id| self.modification(id))
            .is_some_and(|modification| modification.heavy);
        let length = if heavy {
            kind.cells[1].max(2)
        } else {
            kind.cells[1]
        };
        footprint(
            (kind.cells[0], length),
            orientation.quarter_turns(),
            (site.u, site.v),
            site.face.grid(parent.cells),
        )
    }

    /// The sockets of `parent` in `build` that a part of type `kind` with `modification`,
    /// mounted on `socket` turned `orientation`, would reserve: its footprint exactly as
    /// [`Catalog::validate`] checks it, whether or not the placement is valid (the builder's
    /// preview, shooter#124). Empty if `build` has no such parent or socket, or the part type is
    /// unknown.
    #[must_use]
    pub fn footprint(
        &self,
        build: &Assembly,
        parent: PartId,
        socket: SocketId,
        kind: PartTypeId,
        modification: Option<ModId>,
        orientation: Orientation,
    ) -> Vec<SocketId> {
        let (Some(parent), Some(kind)) = (
            build
                .part(parent)
                .and_then(|part| self.part_type(part.kind)),
            self.part_type(kind),
        ) else {
            return Vec::new();
        };
        let Some(site) = parent.site(socket) else {
            return Vec::new();
        };
        let (yaw, pitch, roll) = orientation.raw();
        self.reserved(
            kind,
            modification,
            Orientation::canonical(yaw, pitch, roll),
            site,
            parent,
        )
        .filter_map(|(u, v)| parent.socket(site.face, u, v))
        .collect()
    }

    /// The sockets of `parent` in `build` that its children's footprints reserve, each with the
    /// child holding it (children in ID order).
    #[must_use]
    pub fn claims(&self, build: &Assembly, parent: PartId) -> Vec<(SocketId, PartId)> {
        build
            .children(parent)
            .filter_map(|child| Some((child, child.mount?)))
            .flat_map(|(child, mount)| {
                self.footprint(
                    build,
                    parent,
                    mount.socket,
                    child.kind,
                    child.modification,
                    mount.orientation,
                )
                .into_iter()
                .map(move |socket| (socket, child.id))
            })
            .collect()
    }

    /// The one validation path for a build, in a fixed order so the same candidate always fails
    /// the same way: the part ceiling and known codes; unique IDs (canonical ID order); one root
    /// chassis; parents and their sockets; connection to the root; footprints; mods; then the
    /// build points.
    ///
    /// # Errors
    ///
    /// The first rule the candidate breaks.
    pub fn validate(&self, candidate: &Candidate) -> Result<Assembly, AssemblyError> {
        if candidate.parts.len() > MAX_PARTS {
            return Err(AssemblyError::TooManyParts(candidate.parts.len()));
        }
        let mut parts = Vec::with_capacity(candidate.parts.len());
        for raw in &candidate.parts {
            let kind = self
                .part_type(PartTypeId(raw.kind))
                .ok_or(AssemblyError::UnknownPart(raw.kind))?;
            let modification = match raw.modification {
                Some(code) => Some(
                    self.modification(ModId(code))
                        .ok_or(AssemblyError::UnknownMod(code))?
                        .id,
                ),
                None => None,
            };
            parts.push(Part {
                id: PartId(raw.id),
                kind: kind.id,
                mount: raw.mount.map(|mount| Mount {
                    parent: PartId(mount.parent),
                    socket: SocketId(mount.socket),
                    orientation: Orientation::canonical(mount.yaw, mount.pitch, mount.roll),
                }),
                modification,
            });
        }
        parts.sort_by_key(|part| part.id);
        if let Some(pair) = parts.windows(2).find(|pair| pair[0].id == pair[1].id) {
            return Err(AssemblyError::DuplicateId(pair[0].id));
        }
        let assembly = authored(&parts);
        let type_of = |part: &Part| {
            self.part_type(part.kind)
                .expect("validated part types are in the catalog")
        };

        let mut roots = parts.iter().filter(|part| part.mount.is_none());
        let root = roots.next().ok_or(AssemblyError::NoRoot)?;
        if let Some(second) = roots.next() {
            return Err(AssemblyError::MultipleRoots(root.id, second.id));
        }
        if type_of(root).kind != Kind::Chassis {
            return Err(AssemblyError::RootNotChassis(root.id));
        }

        // Each child's site on its parent.
        let mut sites = Vec::with_capacity(parts.len());
        for part in &parts {
            let Some(mount) = part.mount else {
                continue;
            };
            let parent = assembly
                .part(mount.parent)
                .ok_or(AssemblyError::UnknownParent(part.id))?;
            let site = type_of(parent)
                .site(mount.socket)
                .ok_or(AssemblyError::NoSocket(part.id))?;
            if site.face == Face::NegZ && parent.mount.is_some() {
                return Err(AssemblyError::InteriorSocket(part.id));
            }
            sites.push((part, parent, site));
        }

        // Every part reaches the root within as many steps as there are parts.
        for part in &parts {
            let mut at = part;
            let mut steps = 0;
            while let Some(mount) = at.mount {
                steps += 1;
                if steps > parts.len() || mount.parent == part.id {
                    return Err(AssemblyError::Cycle(part.id));
                }
                at = assembly.part(mount.parent).expect("parents were checked");
            }
        }

        // Footprints: no cell of a face claimed twice.
        let mut claimed: Vec<(PartId, Face, (u8, u8), PartId)> = Vec::new();
        for &(part, parent, site) in &sites {
            let mount = part.mount.expect("children have mounts");
            for cell in self.reserved(
                type_of(part),
                part.modification,
                mount.orientation,
                site,
                type_of(parent),
            ) {
                if let Some(&(.., owner)) = claimed
                    .iter()
                    .find(|&&(on, face, at, _)| on == parent.id && face == site.face && at == cell)
                {
                    return Err(AssemblyError::Overlap(owner, part.id));
                }
                claimed.push((parent.id, site.face, cell, part.id));
            }
        }

        // Mods on the weapons they fit. A weapon may be mounted any number of times
        // (shooter#125): the sockets and the build points bound its copies.
        for part in &parts {
            if let Some(id) = part.modification {
                let weapon = self
                    .weapon(part)
                    .ok_or(AssemblyError::NotAWeapon(part.id))?;
                let modification = self.modification(id).expect("mods were checked");
                if !modification.fits.contains(&weapon) {
                    return Err(AssemblyError::DoesNotFit(id, weapon));
                }
            }
        }

        let budget = self.budget(&assembly);
        if budget.points > budget.build_points {
            return Err(AssemblyError::OverBudget {
                points: budget.points,
                build_points: budget.build_points,
            });
        }
        Ok(assembly)
    }

    /// `assembly`'s totals against its limits (parts or mods this catalog lacks count nothing).
    #[must_use]
    pub fn budget(&self, assembly: &Assembly) -> Budget {
        let mut budget = Budget {
            parts: u32::from(assembly.len),
            build_points: u32::from(self.build_points),
            ..Budget::default()
        };
        for part in assembly.parts() {
            if let Some(kind) = self.part_type(part.kind) {
                let (capacity, regen) = energy(part, kind);
                budget.mass += u32::from(kind.mass);
                budget.capacity += u32::from(capacity);
                budget.regen += u32::from(regen);
                budget.points += u32::from(kind.points);
            }
            if let Some(modification) = part.modification.and_then(|id| self.modification(id)) {
                budget.mass += u32::from(modification.mass);
                budget.points += u32::from(modification.points);
            }
        }
        budget
    }
}
