//! The asset-tooling kits the fighters are drawn from (Atlas D5.i46.1): the robot part kit
//! (release `shooter-combat-robot`), the weapons kit (release `shooter-weapons`, the eight
//! weapons and four mods) and the locomotion kit (release `shooter-robot-locomotion`, the wheel
//! and the track). Which bundle files the clients read, their content pins and size bounds, and
//! the decoding of their glTF scenes (meshes, skins, clips), KTX2 (Basis Universal) textures and
//! the socket-grid descriptors (the weapons' anchors, glow channels and states; the wheels' and
//! tracks' axles and roll clip) into render-engine data. Both clients read the same files through
//! [`Plan`] and [`decode`]: natively from a directory, in the browser over HTTP
//! (`crate::robots::load`). The arena kit (release `shooter-arena`, [`Bundle::Arena`]) is read the
//! same way in a load of its own, for the kit-built maps (`crate::arena_art`): its modules' intact
//! and broken looks, a module's static mesh nodes baked into one mesh.
//!
//! **Pins.** The releases are rolling, so the shooter pins each ([`Bundle::pin_json`],
//! `assets/*-kit.pin.json`): the tar's SHA-256 (checked by `scripts/fetch-robot-kit.sh`) and the
//! bundle manifest's (checked here). A manifest pins every scene, resource and attachment by
//! SHA-256 and length; a file that differs is refused, as is anything over [`MAX_FILE_BYTES`] or
//! a load over [`MAX_TOTAL_BYTES`].
//!
//! **IDs.** Kit entries are keyed by the simulation's part and mod keys
//! (`shooter_sim::robot::parts`, `BETA.mods`). Every kit key must be a catalog part or mod of
//! that kit, and every catalog part and mod must be in a kit (the arena kit's keys are the
//! simulation's arena modules, `shooter_sim::arena_kit::Module`); anything else fails the load
//! loudly ([`complete`]). Some catalog parts have no model of their own yet: the repair module
//! (`robot.repair-module`, shooter#61) is drawn as the robot kit's sensor until the robot kit
//! ships one (moritzbrantner/asset-tooling#742), and the mount rails and the adapter
//! (shooter#126) as its beams fitted to their cuboids (moritzbrantner/asset-tooling#743):
//! [`drawn_key`], [`stand_in`].

use render_engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use render_engine::material::{Sampler, TextureData};
use render_engine::mesh::MeshData;
use render_engine::wgpu;
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use shooter_sim::arena_kit::Module;
use shooter_sim::robot::{BETA, Kind, Locomotion};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// The most one bundle file may hold (the largest, the robot kit's normal atlas, is about
/// 0.45 MB).
pub const MAX_FILE_BYTES: usize = 4 << 20;
/// The most one load (all three kits) may read (the drawn closures are about 1.5 MB, 1 MB and
/// 1.4 MB).
pub const MAX_TOTAL_BYTES: usize = 8 << 20;
/// The variants the clients draw: every part's full detail, and the damaged look where a kit has
/// one (the armor plate, the wheel and the track). (The `lod1` and `lod2` levels are not read: at
/// the arena's distances a weapon is a few hundred triangles and a wheel about a thousand.)
pub const VARIANTS: [&str; 2] = ["default", "damaged"];
/// The arena kit's drawn variants: every module intact, and broken where the kit has a broken
/// look (`shooter_sim::arena_kit::Variant`). Its `lod1` levels are not read.
pub const ARENA_VARIANTS: [&str; 2] = ["default", "broken"];
/// The atlases' top mip level is skipped: at third-person distances a 1024 px atlas is plenty,
/// and it keeps the transcoded RGBA8 textures at a quarter of their full size (about 22 MB for
/// the robot kit, 6 MB for the weapons and 6 MB for the wheels and tracks).
pub const SKIPPED_MIPS: u32 = 1;
/// The attachment of the weapons and locomotion kits that describes their anchors, channels and
/// states, and the wheels' and tracks' axles and roll.
pub const DESCRIPTOR: &str = "socket-grid-descriptor";
/// The locomotion kit's descriptor schema (asset-tooling
/// `schemas/socket-grid-kit-descriptor-v3.schema.json`: v2 plus each part's `locomotion` block).
pub const LOCOMOTION_SCHEMA: &str = "socket-grid-kit-descriptor-v3";

/// One pinned asset-tooling release the clients draw from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bundle {
    /// `shooter-combat-robot`: the robot parts but the wheels and tracks.
    Robot,
    /// `shooter-weapons`: the weapons and their mods.
    Weapons,
    /// `shooter-robot-locomotion`: the wheel and the track.
    Locomotion,
    /// `shooter-arena`: the arena modules the kit-built maps are drawn from (`crate::arena_art`).
    Arena,
}

impl Bundle {
    /// Every pinned kit.
    pub const ALL: [Self; 4] = [Self::Robot, Self::Weapons, Self::Locomotion, Self::Arena];
    /// The kits the fighters are drawn from: one load, [`crate::robots::Robots`].
    pub const FIGHTERS: [Self; 3] = [Self::Robot, Self::Weapons, Self::Locomotion];

    /// Its pin (`assets/robot-kit.pin.json`, `assets/weapon-kit.pin.json`,
    /// `assets/locomotion-kit.pin.json`).
    #[must_use]
    pub const fn pin_json(self) -> &'static str {
        match self {
            Self::Robot => include_str!("../../../../assets/robot-kit.pin.json"),
            Self::Weapons => include_str!("../../../../assets/weapon-kit.pin.json"),
            Self::Locomotion => include_str!("../../../../assets/locomotion-kit.pin.json"),
            Self::Arena => include_str!("../../../../assets/arena-kit.pin.json"),
        }
    }

    /// The directory it is fetched into (`target/…` natively, beside the page in the browser).
    #[must_use]
    pub const fn directory(self) -> &'static str {
        match self {
            Self::Robot => "robot-kit",
            Self::Weapons => "weapon-kit",
            Self::Locomotion => "locomotion-kit",
            Self::Arena => "arena-kit",
        }
    }

    /// The bundle variants the clients draw from it ([`VARIANTS`], or the arena modules' intact
    /// and broken looks): no `lod1` levels.
    #[must_use]
    pub const fn variants(self) -> [&'static str; 2] {
        match self {
            Self::Robot | Self::Weapons | Self::Locomotion => VARIANTS,
            Self::Arena => ARENA_VARIANTS,
        }
    }

    /// Whether `key` may be an entry of this kit: a robot part, a weapon or mod, a wheel or
    /// track, an arena module of the simulation's table (`shooter_sim::arena_kit`).
    fn holds(self, key: &str) -> bool {
        let part = BETA.parts.iter().find(|part| part.key == key);
        let rolls = |kind: Kind| {
            matches!(
                kind,
                Kind::Locomotion(Locomotion::Wheel | Locomotion::Track)
            )
        };
        match self {
            Self::Robot => {
                part.is_some_and(|part| !matches!(part.kind, Kind::Weapon(_)) && !rolls(part.kind))
            }
            Self::Weapons => {
                part.is_some_and(|part| matches!(part.kind, Kind::Weapon(_)))
                    || BETA.mods.iter().any(|modification| modification.key == key)
            }
            Self::Locomotion => part.is_some_and(|part| rolls(part.kind)),
            Self::Arena => Module::ALL.iter().any(|module| module.key() == key),
        }
    }
}

/// The kit entry catalog part `key` is drawn with: its own, but for the parts the robot kit has
/// no model for yet: the repair module draws as the sensor (the nearest small utility module,
/// moritzbrantner/asset-tooling#742), the mount rails as the beams of their length and the
/// adapter as the short beam (the nearest structural parts, moritzbrantner/asset-tooling#743),
/// fitted to their cuboids by [`stand_in`].
#[must_use]
pub fn drawn_key(key: &str) -> &str {
    use shooter_sim::robot::parts::{
        ADAPTER, BEAM_LONG, BEAM_SHORT, MOUNT_RAIL_LONG, MOUNT_RAIL_SHORT, REPAIR_MODULE, SENSOR,
    };
    match key {
        _ if key == REPAIR_MODULE.key => SENSOR.key,
        _ if key == MOUNT_RAIL_SHORT.key || key == ADAPTER.key => BEAM_SHORT.key,
        _ if key == MOUNT_RAIL_LONG.key => BEAM_LONG.key,
        _ => key,
    }
}

/// How catalog part `key`'s stand-in mesh ([`drawn_key`]) is fitted to the part's cuboid, in the
/// part's frame (metres from its cuboid's minimum corner): a mount rail's beam laid flat along
/// the rail's y (the beam's z turned onto it), the adapter's short beam at half its length;
/// identity for a part drawn with its own model.
#[must_use]
pub fn stand_in(key: &str) -> Mat4 {
    use shooter_sim::robot::parts::{ADAPTER, MOUNT_RAIL_LONG, MOUNT_RAIL_SHORT};
    let cell = crate::robots::model::CELL;
    if key == MOUNT_RAIL_SHORT.key || key == MOUNT_RAIL_LONG.key {
        Mat4::from_translation(Vec3::new(0.0, 0.0, cell))
            * Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2)
    } else if key == ADAPTER.key {
        Mat4::from_scale(Vec3::new(1.0, 1.0, 0.5))
    } else {
        Mat4::IDENTITY
    }
}

/// `bundle`'s pinned manifest SHA-256 (hex).
///
/// # Panics
///
/// If the pin file is malformed (it is checked in by hand; the tests read it).
#[must_use]
pub fn pinned_manifest(bundle: Bundle) -> String {
    let pin: Json = serde_json::from_str(bundle.pin_json()).expect("a kit pin is JSON");
    pin["manifestSha256"]
        .as_str()
        .expect("the pin names the manifest's SHA-256")
        .to_owned()
}

/// `bundle`'s pinned release tar SHA-256 (hex): the browser fetches the tar
/// (`crate::robots::load`).
///
/// # Panics
///
/// As [`pinned_manifest`].
#[must_use]
pub fn pinned_tar(bundle: Bundle) -> String {
    let pin: Json = serde_json::from_str(bundle.pin_json()).expect("a kit pin is JSON");
    pin["sha256"]
        .as_str()
        .expect("the pin names the tar's SHA-256")
        .to_owned()
}

/// `bytes`' SHA-256 in lowercase hex.
#[must_use]
pub fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// One pinned bundle file.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct FileRef {
    /// Relative to the bundle's root, e.g. `scenes/robot.core/default.gltf`.
    pub path: String,
    pub sha256: String,
    /// Its length, where the manifest records it.
    pub byte_length: Option<usize>,
}

/// One drawn kit entry: a part or mod key and variant, its scene and the resources it reads.
#[derive(Clone, Debug)]
pub struct EntryRef {
    pub key: String,
    pub variant: String,
    pub scene: FileRef,
    pub resources: Vec<FileRef>,
}

/// What a load reads from one kit, from its pinned manifest.
#[derive(Clone, Debug)]
pub struct Plan {
    pub bundle: Bundle,
    pub entries: Vec<EntryRef>,
    /// The weapons kit's socket-grid descriptor.
    pub descriptor: Option<FileRef>,
}

/// A bundle path the client may read: relative, no `..`, no scheme or backslash.
fn safe_path(path: &str) -> Result<&str, String> {
    let bad = path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.split('/').any(|part| part.is_empty() || part == "..");
    if bad {
        Err(format!("robot kit: refusing bundle path {path:?}"))
    } else {
        Ok(path)
    }
}

fn file_ref(value: &Json) -> Result<FileRef, String> {
    let path = value["path"]
        .as_str()
        .ok_or("robot kit: a manifest file has no path")?;
    let sha256 = value["sha256"]
        .as_str()
        .filter(|hash| hash.len() == 64)
        .ok_or_else(|| format!("robot kit: {path} has no SHA-256"))?;
    Ok(FileRef {
        path: safe_path(path)?.to_owned(),
        sha256: sha256.to_owned(),
        byte_length: value["byteLength"]
            .as_u64()
            .and_then(|length| usize::try_from(length).ok()),
    })
}

impl Plan {
    /// The drawn entries of `bundle`'s manifest `manifest`, after checking it against the pin
    /// and its keys against the simulation's catalog.
    ///
    /// # Errors
    ///
    /// If the manifest is not the pinned one or not a bundle manifest, or a kit key is no
    /// catalog part or mod of that kit.
    pub fn of(bundle: Bundle, manifest: &[u8]) -> Result<Self, String> {
        Self::checked(bundle, manifest, &pinned_manifest(bundle))
    }

    /// [`Plan::of`] against `pin` instead of the checked-in pin (tests).
    ///
    /// # Errors
    ///
    /// As [`Plan::of`].
    pub fn checked(bundle: Bundle, manifest: &[u8], pin: &str) -> Result<Self, String> {
        let name = bundle.directory();
        if manifest.len() > MAX_FILE_BYTES {
            return Err(format!("{name}: the manifest is too large"));
        }
        let hash = sha256(manifest);
        if hash != pin {
            return Err(format!(
                "{name}: the manifest's SHA-256 is {hash}, pinned {pin} (refetch the kits with scripts/fetch-robot-kit.sh)"
            ));
        }
        let json: Json = serde_json::from_slice(manifest)
            .map_err(|error| format!("{name}: manifest: {error}"))?;
        let assets = json["assets"]
            .as_array()
            .ok_or_else(|| format!("{name}: the manifest lists no assets"))?;
        let mut entries = Vec::new();
        let mut descriptor = None;
        for asset in assets {
            let key = asset["key"]
                .as_str()
                .ok_or_else(|| format!("{name}: an asset has no key"))?;
            let variant = asset["variant"]
                .as_str()
                .ok_or_else(|| format!("{name}: {key} has no variant"))?;
            if !bundle.holds(key) {
                return Err(format!(
                    "{name}: entry {key} is no part or mod of the simulation's catalog this kit draws"
                ));
            }
            if !bundle.variants().contains(&variant) {
                continue;
            }
            for attachment in asset["attachments"].as_array().into_iter().flatten() {
                if attachment["name"].as_str() == Some(DESCRIPTOR) {
                    descriptor = Some(file_ref(attachment)?);
                }
            }
            let scene = file_ref(asset)?;
            let resources = asset["resources"]
                .as_array()
                .ok_or_else(|| format!("{name}: {key} lists no resources"))?
                .iter()
                .map(file_ref)
                .collect::<Result<Vec<_>, _>>()?;
            entries.push(EntryRef {
                key: key.to_owned(),
                variant: variant.to_owned(),
                scene,
                resources,
            });
        }
        match bundle {
            Bundle::Robot => {
                if !entries
                    .iter()
                    .any(|entry| entry.key == "robot.armor.plate" && entry.variant == "damaged")
                {
                    return Err(format!("{name}: robot.armor.plate has no damaged variant"));
                }
            }
            Bundle::Weapons => {
                if descriptor.is_none() {
                    return Err(format!("{name}: no {DESCRIPTOR} attachment"));
                }
            }
            Bundle::Locomotion => {
                if descriptor.is_none() {
                    return Err(format!("{name}: no {DESCRIPTOR} attachment"));
                }
                for key in ["robot.wheel", "robot.track"] {
                    if !entries
                        .iter()
                        .any(|entry| entry.key == key && entry.variant == "damaged")
                    {
                        return Err(format!("{name}: {key} has no damaged variant"));
                    }
                }
            }
            Bundle::Arena => {
                // Every module of the simulation's table is drawn, intact and (where the table
                // has broken proxies) broken.
                for module in Module::ALL {
                    let looks =
                        std::iter::once("default").chain(module.broken_proxies().map(|_| "broken"));
                    for variant in looks {
                        if !entries
                            .iter()
                            .any(|entry| entry.key == module.key() && entry.variant == variant)
                        {
                            return Err(format!(
                                "{name}: {} has no {variant} variant",
                                module.key()
                            ));
                        }
                    }
                }
            }
        }
        Ok(Self {
            bundle,
            entries,
            descriptor,
        })
    }

    /// Every file the drawn entries read, each once.
    #[must_use]
    pub fn files(&self) -> Vec<FileRef> {
        let mut files = BTreeMap::new();
        for entry in &self.entries {
            for file in std::iter::once(&entry.scene).chain(&entry.resources) {
                files
                    .entry(file.path.clone())
                    .or_insert_with(|| file.clone());
            }
        }
        if let Some(descriptor) = &self.descriptor {
            files.insert(descriptor.path.clone(), descriptor.clone());
        }
        files.into_values().collect()
    }
}

/// Checks that `plans` between them draw the whole catalog: every catalog part and every catalog
/// mod is in a kit.
///
/// # Errors
///
/// Naming the first catalog part or mod no kit draws.
pub fn complete(plans: &[Plan]) -> Result<(), String> {
    let keys: BTreeSet<&str> = plans
        .iter()
        .flat_map(|plan| &plan.entries)
        .map(|entry| entry.key.as_str())
        .collect();
    for part in BETA.parts {
        if !keys.contains(drawn_key(part.key)) {
            return Err(format!("robot kit: catalog part {} is in no kit", part.key));
        }
    }
    for modification in BETA.mods {
        if !keys.contains(modification.key) {
            return Err(format!(
                "robot kit: catalog mod {} is in no kit",
                modification.key
            ));
        }
    }
    Ok(())
}

/// `uri` (relative to the bundle file `base`) as a bundle path.
fn resolve(base: &str, uri: &str) -> Result<String, String> {
    let mut parts: Vec<&str> = base.split('/').collect();
    parts.pop();
    for part in uri.split('/') {
        match part {
            ".." => {
                parts
                    .pop()
                    .ok_or_else(|| format!("robot kit: {uri} leaves the bundle"))?;
            }
            "." | "" => {}
            part => parts.push(part),
        }
    }
    let path = parts.join("/");
    safe_path(&path)?;
    Ok(path)
}

/// One joint of a skinned kit part: where it rests (relative to its parent joint, or for a root
/// joint relative to the skinned mesh) and its inverse bind matrix.
#[derive(Clone, Debug, PartialEq)]
pub struct RigJoint {
    pub name: String,
    /// Its parent joint (an index into [`Rig::joints`]).
    pub parent: Option<usize>,
    /// For a root joint, the transform of the nodes above it; identity otherwise.
    pub above: Mat4,
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    pub inverse_bind: Mat4,
}

/// A joint's transform relative to its parent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Local {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Local {
    #[must_use]
    pub fn matrix(self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

/// What one track of a clip moves.
#[derive(Clone, Debug, PartialEq)]
pub enum Track {
    Translation(Vec<Vec3>),
    Rotation(Vec<Quat>),
    Scale(Vec<Vec3>),
}

/// One track of a clip, sampled linearly.
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub joint: usize,
    pub times: Vec<f32>,
    pub track: Track,
}

impl Channel {
    /// The keys around `time` (seconds) and how far between them it is, held at the ends.
    fn at(&self, time: f32) -> (usize, usize, f32) {
        let last = self.times.len().saturating_sub(1);
        if self.times.is_empty() || time <= self.times[0] {
            return (0, 0, 0.0);
        }
        if time >= self.times[last] {
            return (last, last, 0.0);
        }
        let next = self.times.partition_point(|at| *at <= time);
        let (t0, t1) = (self.times[next - 1], self.times[next]);
        let blend = if t1 > t0 {
            (time - t0) / (t1 - t0)
        } else {
            0.0
        };
        (next - 1, next, blend)
    }

    /// Sets what it moves of `local` to its value at `time` (seconds), held at the ends.
    pub fn apply(&self, time: f32, local: &mut Local) {
        let (from, to, blend) = self.at(time);
        match &self.track {
            Track::Translation(values) => {
                local.translation = values[from].lerp(values[to], blend);
            }
            Track::Rotation(values) => {
                local.rotation = values[from].slerp(values[to], blend).normalize();
            }
            Track::Scale(values) => local.scale = values[from].lerp(values[to], blend),
        }
    }

    /// Its rotation at `time` (seconds), if it is a rotation track.
    #[must_use]
    pub fn rotation(&self, time: f32) -> Option<Quat> {
        let Track::Rotation(values) = &self.track else {
            return None;
        };
        let (from, to, blend) = self.at(time);
        Some(values[from].slerp(values[to], blend).normalize())
    }
}

/// A joint animation: the leg's walk clips (looping), a weapon's states.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub name: String,
    /// Its length (seconds); one cycle of a looping clip.
    pub duration: f32,
    pub channels: Vec<Channel>,
}

impl Clip {
    /// Sets the joints the clip turns to their rotations `phase` (0..1, wrapping) through it.
    pub fn sample(&self, phase: f32, rotations: &mut [Quat]) {
        let time = phase.rem_euclid(1.0) * self.duration;
        for channel in &self.channels {
            if let (Some(rotation), Some(turned)) =
                (rotations.get_mut(channel.joint), channel.rotation(time))
            {
                *rotation = turned;
            }
        }
    }

    /// Sets everything the clip moves to its value `time` seconds in (held at its ends).
    pub fn pose(&self, time: f32, locals: &mut [Local]) {
        for channel in &self.channels {
            if let Some(local) = locals.get_mut(channel.joint) {
                channel.apply(time, local);
            }
        }
    }
}

/// A skinned kit part's skeleton and clips.
#[derive(Clone, Debug, PartialEq)]
pub struct Rig {
    pub joints: Vec<RigJoint>,
    pub clips: Vec<Clip>,
}

impl Rig {
    #[must_use]
    pub fn joint(&self, name: &str) -> Option<usize> {
        self.joints.iter().position(|joint| joint.name == name)
    }

    #[must_use]
    pub fn clip(&self, name: &str) -> Option<&Clip> {
        self.clips.iter().find(|clip| clip.name == name)
    }

    /// Every joint's rotation at rest.
    #[must_use]
    pub fn rest(&self) -> Vec<Quat> {
        self.joints.iter().map(|joint| joint.rotation).collect()
    }

    /// Every joint's transform at rest.
    #[must_use]
    pub fn rest_locals(&self) -> Vec<Local> {
        self.joints
            .iter()
            .map(|joint| Local {
                translation: joint.translation,
                rotation: joint.rotation,
                scale: joint.scale,
            })
            .collect()
    }

    /// Each joint's transform in the skinned mesh's frame, with `locals` (one a joint).
    #[must_use]
    pub fn globals_of(&self, locals: &[Local]) -> Vec<Mat4> {
        let mut globals: Vec<Option<Mat4>> = vec![None; self.joints.len()];
        // Parents come first in the kits; a few passes settle any other order.
        for _ in 0..self.joints.len() {
            for (index, joint) in self.joints.iter().enumerate() {
                if globals[index].is_some() {
                    continue;
                }
                let parent = match joint.parent {
                    None => Some(joint.above),
                    Some(parent) => globals[parent],
                };
                if let Some(parent) = parent {
                    let local = locals.get(index).map_or_else(
                        || {
                            Mat4::from_scale_rotation_translation(
                                joint.scale,
                                joint.rotation,
                                joint.translation,
                            )
                        },
                        |local| local.matrix(),
                    );
                    globals[index] = Some(parent * local);
                }
            }
        }
        globals.into_iter().map(Option::unwrap_or_default).collect()
    }

    /// Each joint's transform in the skinned mesh's frame, with `rotations` (one a joint).
    #[must_use]
    pub fn globals(&self, rotations: &[Quat]) -> Vec<Mat4> {
        self.globals_of(&self.turned(rotations))
    }

    fn turned(&self, rotations: &[Quat]) -> Vec<Local> {
        let mut locals = self.rest_locals();
        for (local, rotation) in locals.iter_mut().zip(rotations) {
            local.rotation = *rotation;
        }
        locals
    }

    /// The joint palette render-engine skins with: each joint's posed transform times its
    /// inverse bind matrix (identity everywhere at rest).
    #[must_use]
    pub fn palette(&self, rotations: &[Quat]) -> Vec<Mat4> {
        self.palette_of(&self.turned(rotations))
    }

    /// [`Rig::palette`] with whole joint transforms.
    #[must_use]
    pub fn palette_of(&self, locals: &[Local]) -> Vec<Mat4> {
        self.globals_of(locals)
            .into_iter()
            .zip(&self.joints)
            .map(|(global, joint)| global * joint.inverse_bind)
            .collect()
    }
}

/// A named frame on a weapon or mod (the descriptor's `anchors[]`), in its glTF part frame
/// (metres): a muzzle, a mod mount, the grapple's cable end.
#[derive(Clone, Debug, PartialEq)]
pub struct Anchor {
    pub name: String,
    /// The node (skin joint) it rides; `None` on a static mod.
    pub node: Option<String>,
    pub position: Vec3,
    /// The fire or mount direction.
    pub normal: Vec3,
    pub up: Vec3,
}

impl Anchor {
    /// The frame a mod sits in on this anchor: its origin on the position, glTF +Y along the
    /// normal and −Z along the up (so +X is up × normal), in the weapon's glTF frame.
    #[must_use]
    pub fn frame(&self) -> Mat4 {
        let y = self.normal.normalize_or(Vec3::Y);
        let z = -self.up.normalize_or(Vec3::Z);
        Mat4::from_cols(
            y.cross(z).extend(0.0),
            y.extend(0.0),
            z.extend(0.0),
            self.position.extend(1.0),
        )
    }
}

/// What the weapons kit's descriptor says about one weapon or mod.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gear {
    pub anchors: Vec<Anchor>,
    /// The nodes of its `charge` and `fire` glow channels (a node's scale is the level).
    pub charge: Option<String>,
    pub fire: Option<String>,
    /// Its states' clips (each must be one of the entry's animations).
    pub states: Vec<String>,
    /// A mod: the anchor it sits on (`mod-side`, `mod-muzzle`).
    pub mounts_on: Option<String>,
}

impl Gear {
    #[must_use]
    pub fn anchor(&self, name: &str) -> Option<&Anchor> {
        self.anchors.iter().find(|anchor| anchor.name == name)
    }
}

/// What the locomotion kit's descriptor says about a wheel or track (its v3 `locomotion` block):
/// where the simulation's rolling bodies are and how the kit's roll clip follows their turn.
#[derive(Clone, Debug, PartialEq)]
pub struct Rolling {
    /// The axle every wheel node turns about, in the part's glTF frame: the part's mount normal,
    /// as the simulation's revolute joints (`shooter_sim::mechanics`). A positive angle turns
    /// right-handed about it.
    pub axis: Vec3,
    /// Metres of ground a radian of turn rolls: the tyre's tread (0.18 m) or the belt round the
    /// track's wheels (0.1 m).
    pub radius: f32,
    /// The simulation's rolling bodies' centres (the wheel's sphere, the track's two road
    /// wheels), in the part's glTF frame (metres).
    pub centres: Vec<Vec3>,
    /// The looping clip that turns the wheels (and moves the belt), its length (seconds) and the
    /// turn one cycle of it makes (radians).
    pub clip: String,
    pub seconds: f32,
    pub radians_per_cycle: f32,
}

impl Rolling {
    /// The clip's time (seconds) for a turn of `angle` radians: `(angle mod 2π) / 2π` of it
    /// (the descriptor's `roll.rule`).
    #[must_use]
    pub fn time(&self, angle: f32) -> f32 {
        angle.rem_euclid(self.radians_per_cycle) / self.radians_per_cycle * self.seconds
    }
}

/// The wheels and tracks of the locomotion kit's v3 descriptor, by key, with the nodes each
/// one's clip turns.
fn rolling(descriptor: &[u8]) -> Result<HashMap<String, (Rolling, Vec<String>)>, String> {
    let fail = |what: &str| format!("locomotion-kit: descriptor: {what}");
    let json: Json =
        serde_json::from_slice(descriptor).map_err(|error| fail(&error.to_string()))?;
    if json["schema"].as_str() != Some(LOCOMOTION_SCHEMA) {
        return Err(fail(&format!("not a {LOCOMOTION_SCHEMA}")));
    }
    let mut out = HashMap::new();
    for part in json["parts"].as_array().into_iter().flatten() {
        let key = part["key"]
            .as_str()
            .ok_or_else(|| fail("a part has no key"))?;
        let block = &part["locomotion"];
        let malformed = || fail(&format!("{key} has a malformed locomotion block"));
        #[allow(clippy::cast_possible_truncation)]
        let number = |value: &Json| value.as_f64().map(|value| value as f32);
        let roll = &block["roll"];
        let rolling = Rolling {
            axis: vec3(&block["axis"]).ok_or_else(malformed)?,
            radius: number(&block["rollingRadius"]).ok_or_else(malformed)?,
            centres: block["bodies"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|body| vec3(&body["centre"]))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(malformed)?,
            clip: roll["animation"].as_str().ok_or_else(malformed)?.to_owned(),
            seconds: number(&roll["seconds"]).ok_or_else(malformed)?,
            radians_per_cycle: number(&roll["radiansPerCycle"]).ok_or_else(malformed)?,
        };
        let wheels = block["wheels"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|node| node.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(malformed)?;
        if rolling.centres.is_empty()
            || wheels.is_empty()
            || rolling.radius <= 0.0
            || rolling.seconds <= 0.0
            || rolling.radians_per_cycle <= 0.0
            || rolling.axis.length() < 0.5
        {
            return Err(malformed());
        }
        out.insert(key.to_owned(), (rolling, wheels));
    }
    Ok(out)
}

fn vec3(value: &Json) -> Option<Vec3> {
    let array = value.as_array()?;
    #[allow(clippy::cast_possible_truncation)]
    let axis = |index: usize| array.get(index)?.as_f64().map(|axis| axis as f32);
    (array.len() == 3).then_some(Vec3::new(axis(0)?, axis(1)?, axis(2)?))
}

/// The weapons and mods of a socket-grid descriptor, by key.
fn gear(descriptor: &[u8]) -> Result<HashMap<String, Gear>, String> {
    let fail = |what: &str| format!("weapon-kit: descriptor: {what}");
    let json: Json =
        serde_json::from_slice(descriptor).map_err(|error| fail(&error.to_string()))?;
    let mut out = HashMap::new();
    let items = json["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(json["mods"].as_array().into_iter().flatten());
    for item in items {
        let key = item["key"]
            .as_str()
            .ok_or_else(|| fail("an item has no key"))?;
        let anchors = item["anchors"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|anchor| {
                Some(Anchor {
                    name: anchor["name"].as_str()?.to_owned(),
                    node: anchor["node"].as_str().map(str::to_owned),
                    position: vec3(&anchor["position"])?,
                    normal: vec3(&anchor["normal"])?,
                    up: vec3(&anchor["up"])?,
                })
            })
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| fail(&format!("{key} has a malformed anchor")))?;
        let channel = |name: &str| {
            item["channels"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|channel| channel["name"].as_str() == Some(name))
                .and_then(|channel| channel["node"].as_str())
                .map(str::to_owned)
        };
        let states = item["states"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|state| state["animation"].as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| fail(&format!("{key} has a state without an animation")))?;
        out.insert(
            key.to_owned(),
            Gear {
                anchors,
                charge: channel("charge"),
                fire: channel("fire"),
                states,
                mounts_on: item["mountsOn"].as_str().map(str::to_owned),
            },
        );
    }
    Ok(out)
}

/// The kits' material, as a glTF describes it.
#[derive(Clone, Debug, PartialEq)]
pub struct KitMaterial {
    pub base_color: Vec4,
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: Vec3,
    pub double_sided: bool,
    /// Indices into [`KitData::textures`].
    pub color: Option<usize>,
    pub normal: Option<usize>,
    pub metallic_roughness: Option<usize>,
    pub occlusion: Option<usize>,
    pub emissive_texture: Option<usize>,
}

/// One drawn kit entry, decoded.
#[derive(Clone, Debug)]
pub struct KitEntry {
    pub key: String,
    pub variant: String,
    /// In the part's glTF frame (metres); skinned parts in their bind pose.
    pub mesh: MeshData,
    pub material: usize,
    pub rig: Option<Rig>,
    /// A weapon's or mod's anchors, channels and states (the weapons kit's descriptor).
    pub gear: Option<Gear>,
    /// A wheel's or track's axle and roll clip (the locomotion kit's descriptor).
    pub rolling: Option<Rolling>,
}

/// The decoded kits: what the clients upload and pose.
#[derive(Debug)]
pub struct KitData {
    pub entries: Vec<KitEntry>,
    pub materials: Vec<KitMaterial>,
    pub textures: Vec<TextureData>,
}

impl KitData {
    /// The entry for part or mod `key` in `variant`.
    #[must_use]
    pub fn entry(&self, key: &str, variant: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.key == key && entry.variant == variant)
    }
}

/// A KTX2 (Basis Universal) texture with its own mips, the top [`SKIPPED_MIPS`] levels dropped,
/// transcoded to RGBA8.
fn decode_texture(bytes: &[u8], srgb: bool) -> Result<TextureData, String> {
    let transcoder =
        basisu::Transcoder::new(bytes).map_err(|error| format!("robot kit: texture: {error:?}"))?;
    let (width, height) = transcoder.base_dimensions();
    let levels = transcoder.level_count();
    let skip = SKIPPED_MIPS.min(levels.saturating_sub(1));
    let levels = (skip..levels)
        .map(|level| {
            transcoder
                .transcode(
                    level,
                    basisu::TargetFormat::Rgba32,
                    basisu::DecodeFlags::NONE,
                )
                .map_err(|error| format!("robot kit: texture level {level}: {error:?}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(TextureData {
        width: (width >> skip).max(1),
        height: (height >> skip).max(1),
        format: if srgb {
            wgpu::TextureFormat::Rgba8UnormSrgb
        } else {
            wgpu::TextureFormat::Rgba8Unorm
        },
        levels,
        sampler: Sampler::default(),
    })
}

/// The files of one load, checked against their pins and the bounds.
fn checked<'a>(
    plans: &[Plan],
    files: &'a HashMap<String, Vec<u8>>,
) -> Result<HashMap<String, &'a [u8]>, String> {
    let mut total = 0;
    let mut out = HashMap::new();
    let wanted: BTreeMap<String, FileRef> = plans
        .iter()
        .flat_map(Plan::files)
        .map(|file| (file.path.clone(), file))
        .collect();
    for file in wanted.into_values() {
        let bytes = files
            .get(&file.path)
            .ok_or_else(|| format!("robot kit: {} is missing", file.path))?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(format!("robot kit: {} is too large", file.path));
        }
        total += bytes.len();
        if total > MAX_TOTAL_BYTES {
            return Err("robot kit: the load is too large".into());
        }
        if file.byte_length.is_some_and(|length| length != bytes.len()) {
            return Err(format!("robot kit: {} is not its pinned length", file.path));
        }
        let hash = sha256(bytes);
        if hash != file.sha256 {
            return Err(format!(
                "robot kit: {} has SHA-256 {hash}, pinned {}",
                file.path, file.sha256
            ));
        }
        out.insert(file.path, bytes.as_slice());
    }
    Ok(out)
}

/// Decodes the drawn entries of `plans` (one a kit) from `files` (bundle path → bytes; the kits'
/// paths are content-addressed or keyed, so they share one map).
///
/// # Errors
///
/// If the plans do not draw the whole catalog ([`complete`]), a file is missing, too large or
/// not its pinned content, a scene is not the one-mesh part the kit standard describes, or a
/// weapon or mod is not in the weapons kit's descriptor or lacks a state's clip.
pub fn decode(plans: &[Plan], files: &HashMap<String, Vec<u8>>) -> Result<KitData, String> {
    if plans.iter().any(|plan| plan.bundle != Bundle::Arena) {
        complete(plans)?;
    }
    let files = checked(plans, files)?;
    let mut textures: Vec<TextureData> = Vec::new();
    let mut texture_ids: HashMap<(String, bool), usize> = HashMap::new();
    let mut materials: Vec<KitMaterial> = Vec::new();
    let mut entries = Vec::new();
    for plan in plans {
        let (gear, rolling) = match (&plan.descriptor, plan.bundle) {
            (Some(descriptor), Bundle::Locomotion) => {
                (None, Some(rolling(files[&descriptor.path])?))
            }
            (Some(descriptor), _) => (Some(gear(files[&descriptor.path])?), None),
            (None, _) => (None, None),
        };
        for entry in &plan.entries {
            let mut decoded = scene(
                entry,
                &files,
                &mut textures,
                &mut texture_ids,
                &mut materials,
                plan.bundle == Bundle::Arena,
            )?;
            if let Some(gear) = &gear {
                let found = gear
                    .get(&entry.key)
                    .ok_or_else(|| format!("weapon-kit: {} is not in the descriptor", entry.key))?;
                for state in &found.states {
                    if decoded
                        .rig
                        .as_ref()
                        .and_then(|rig| rig.clip(state))
                        .is_none()
                    {
                        return Err(format!(
                            "weapon-kit: {} has no clip for its state {state}",
                            entry.key
                        ));
                    }
                }
                decoded.gear = Some(found.clone());
            }
            if let Some(rolling) = &rolling {
                let fail = |what: String| format!("locomotion-kit: {}: {what}", entry.key);
                let (found, wheels) = rolling
                    .get(&entry.key)
                    .ok_or_else(|| fail("not in the descriptor".into()))?;
                let rig = decoded
                    .rig
                    .as_ref()
                    .ok_or_else(|| fail("not rigged".into()))?;
                if rig.clip(&found.clip).is_none() {
                    return Err(fail(format!("no {} clip", found.clip)));
                }
                if let Some(missing) = wheels.iter().find(|node| rig.joint(node).is_none()) {
                    return Err(fail(format!("no {missing} node")));
                }
                decoded.rolling = Some(found.clone());
            }
            entries.push(decoded);
        }
    }
    Ok(KitData {
        entries,
        materials,
        textures,
    })
}

/// The one primitive of `node`'s mesh, if it has exactly one.
fn only_primitive<'a>(node: &gltf::Node<'a>) -> Option<gltf::Primitive<'a>> {
    let mut primitives = node.mesh()?.primitives();
    let first = primitives.next()?;
    primitives.next().is_none().then_some(first)
}

/// One primitive's vertices placed by `placed`: positions, normals, UVs, tangents, colours and
/// indices (no skin). None without positions.
fn static_mesh<'a, 's, F>(
    primitive: &'a gltf::Primitive<'a>,
    placed: Mat4,
    get: F,
) -> Option<MeshData>
where
    F: Clone + Fn(gltf::Buffer<'a>) -> Option<&'s [u8]>,
{
    let normal_matrix = placed.inverse().transpose();
    let reader = primitive.reader(get);
    let positions: Vec<Vec3> = reader
        .read_positions()?
        .map(|p| placed.transform_point3(Vec3::from(p)))
        .collect();
    Some(MeshData {
        normals: reader.read_normals().map_or_else(
            || vec![Vec3::Y; positions.len()],
            |normals| {
                normals
                    .map(|n| {
                        normal_matrix
                            .transform_vector3(Vec3::from(n))
                            .normalize_or(Vec3::Y)
                    })
                    .collect()
            },
        ),
        uvs: reader
            .read_tex_coords(0)
            .map(|uvs| uvs.into_f32().map(Vec2::from).collect())
            .unwrap_or_default(),
        tangents: reader
            .read_tangents()
            .map(|tangents| tangents.map(Vec4::from).collect())
            .unwrap_or_default(),
        colors: reader
            .read_colors(0)
            .map(|colors| colors.into_rgba_f32().map(Vec4::from).collect())
            .unwrap_or_default(),
        indices: reader.read_indices().map_or_else(
            || (0..u32::try_from(positions.len()).unwrap_or(u32::MAX)).collect(),
            |indices| indices.into_u32().collect(),
        ),
        positions,
        ..MeshData::default()
    })
}

/// `theirs` appended to `ours` where both have the attribute for every vertex, else none.
fn join<T>(ours: &mut Vec<T>, theirs: Vec<T>, counts: (usize, usize)) {
    if ours.len() == counts.0 && theirs.len() == counts.1 {
        ours.extend(theirs);
    } else {
        ours.clear();
    }
}

/// `more` appended to `data` (static meshes; an attribute one of them lacks is dropped).
fn append(data: &mut MeshData, more: MeshData) {
    let base = u32::try_from(data.positions.len()).unwrap_or(u32::MAX);
    let counts = (data.positions.len(), more.positions.len());
    join(&mut data.uvs, more.uvs, counts);
    join(&mut data.tangents, more.tangents, counts);
    join(&mut data.colors, more.colors, counts);
    data.normals.extend(more.normals);
    data.indices
        .extend(more.indices.into_iter().map(|index| index + base));
    data.positions.extend(more.positions);
}

/// One entry's scene, its textures and material added to the load's. With `parts` (the arena
/// kit) its static mesh nodes are baked into one mesh.
fn scene(
    entry: &EntryRef,
    files: &HashMap<String, &[u8]>,
    textures: &mut Vec<TextureData>,
    texture_ids: &mut HashMap<(String, bool), usize>,
    materials: &mut Vec<KitMaterial>,
    parts: bool,
) -> Result<KitEntry, String> {
    let path = &entry.scene.path;
    let bytes = files[path];
    let fail = |what: &str| format!("robot kit: {path}: {what}");
    let json: Json = serde_json::from_slice(bytes).map_err(|error| fail(&error.to_string()))?;
    let gltf = gltf::Gltf::from_slice_without_validation(bytes)
        .map_err(|error| fail(&error.to_string()))?;
    let buffers: Vec<&[u8]> = gltf
        .buffers()
        .map(|buffer| match buffer.source() {
            gltf::buffer::Source::Uri(uri) => {
                let file = resolve(path, uri)?;
                files
                    .get(&file)
                    .copied()
                    .ok_or_else(|| fail(&format!("{file} is not among its resources")))
            }
            gltf::buffer::Source::Bin => Err(fail("embedded buffers are not kit standard")),
        })
        .collect::<Result<_, _>>()?;
    let get = |buffer: gltf::Buffer| buffers.get(buffer.index()).copied();

    let mut texture = |index: Option<usize>, srgb: bool| -> Result<Option<usize>, String> {
        let Some(index) = index else {
            return Ok(None);
        };
        let entry = &json["textures"][index];
        let source = entry["extensions"]["KHR_texture_basisu"]["source"]
            .as_u64()
            .or_else(|| entry["source"].as_u64())
            .ok_or_else(|| fail("a texture has no source"))?;
        let uri = json["images"][usize::try_from(source).unwrap_or(usize::MAX)]["uri"]
            .as_str()
            .ok_or_else(|| fail("an image has no URI"))?;
        let file = resolve(path, uri)?;
        if let Some(id) = texture_ids.get(&(file.clone(), srgb)) {
            return Ok(Some(*id));
        }
        let bytes = files
            .get(&file)
            .ok_or_else(|| fail(&format!("{file} is not among its resources")))?;
        textures.push(decode_texture(bytes, srgb)?);
        texture_ids.insert((file, srgb), textures.len() - 1);
        Ok(Some(textures.len() - 1))
    };

    let scene = gltf
        .default_scene()
        .or_else(|| gltf.scenes().next())
        .ok_or_else(|| fail("no scene"))?;
    // Every node's parent and global transform.
    let count = gltf.nodes().count();
    let mut parents = vec![None; count];
    for node in gltf.nodes() {
        for child in node.children() {
            parents[child.index()] = Some(node.index());
        }
    }
    let locals: Vec<Mat4> = gltf
        .nodes()
        .map(|node| Mat4::from_cols_array_2d(&node.transform().matrix()))
        .collect();
    let global = |mut index: usize| {
        let mut matrix = locals[index];
        while let Some(parent) = parents[index] {
            matrix = locals[parent] * matrix;
            index = parent;
        }
        matrix
    };
    let mut meshes = Vec::new();
    let mut stack: Vec<gltf::Node> = scene.nodes().collect();
    while let Some(node) = stack.pop() {
        stack.extend(node.children());
        if node.mesh().is_some() {
            meshes.push(node);
        }
    }
    // A kit part is one mesh node; an arena module (the door: its two leaves) may be several
    // static ones, baked into one mesh.
    let (node, others) = match meshes.as_slice() {
        [node] => (node, &[][..]),
        [node, others @ ..] if parts && meshes.iter().all(|node| node.skin().is_none()) => {
            (node, others)
        }
        _ => return Err(fail("a kit part is exactly one mesh")),
    };
    let one_primitive = "a kit part is exactly one primitive";
    let primitive = only_primitive(node).ok_or_else(|| fail(one_primitive))?;
    let primitive = &primitive;
    let skin = node.skin();
    // A static mesh is baked into its node's place; a skinned one keeps its bind pose (glTF
    // skins ignore the mesh node's transform).
    let placed = if skin.is_some() {
        Mat4::IDENTITY
    } else {
        global(node.index())
    };
    let reader = primitive.reader(get);
    let mut data = static_mesh(primitive, placed, get).ok_or_else(|| fail("no positions"))?;
    for other in others {
        let part = only_primitive(other).ok_or_else(|| fail(one_primitive))?;
        if part.material().index() != primitive.material().index() {
            return Err(fail("a module's meshes share one material"));
        }
        let more =
            static_mesh(&part, global(other.index()), get).ok_or_else(|| fail("no positions"))?;
        append(&mut data, more);
    }
    let rig = match skin {
        None => None,
        Some(skin) => {
            data.joints = reader
                .read_joints(0)
                .ok_or_else(|| fail("a skinned mesh without JOINTS_0"))?
                .into_u16()
                .collect();
            data.weights = reader
                .read_weights(0)
                .ok_or_else(|| fail("a skinned mesh without WEIGHTS_0"))?
                .into_f32()
                .map(Vec4::from)
                .collect();
            let joints: Vec<gltf::Node> = skin.joints().collect();
            let inverse_binds: Vec<Mat4> = skin
                .reader(get)
                .read_inverse_bind_matrices()
                .map(|matrices| matrices.map(|m| Mat4::from_cols_array_2d(&m)).collect())
                .unwrap_or_else(|| vec![Mat4::IDENTITY; joints.len()]);
            if inverse_binds.len() != joints.len() {
                return Err(fail(
                    "a skin's inverse bind matrices do not match its joints",
                ));
            }
            let slot = |node: usize| joints.iter().position(|joint| joint.index() == node);
            let rig_joints = joints
                .iter()
                .zip(&inverse_binds)
                .map(|(joint, inverse_bind)| {
                    // The nearest ancestor that is a joint; the nodes between are folded
                    // into `above` for a root joint.
                    let mut above = Mat4::IDENTITY;
                    let mut at = parents[joint.index()];
                    let mut parent = None;
                    while let Some(index) = at {
                        if let Some(joint) = slot(index) {
                            parent = Some(joint);
                            break;
                        }
                        above = locals[index] * above;
                        at = parents[index];
                    }
                    let (translation, rotation, scale) = joint.transform().decomposed();
                    RigJoint {
                        name: joint.name().unwrap_or_default().to_owned(),
                        parent,
                        above: if parent.is_some() {
                            Mat4::IDENTITY
                        } else {
                            above
                        },
                        translation: Vec3::from(translation),
                        rotation: Quat::from_array(rotation),
                        scale: Vec3::from(scale),
                        inverse_bind: *inverse_bind,
                    }
                })
                .collect();
            let mut clips = Vec::new();
            for animation in gltf.animations() {
                let mut channels = Vec::new();
                let mut duration: f32 = 0.0;
                for channel in animation.channels() {
                    let Some(joint) = slot(channel.target().node().index()) else {
                        continue;
                    };
                    if channel.sampler().interpolation() != gltf::animation::Interpolation::Linear {
                        return Err(fail("kit clips are sampled linearly"));
                    }
                    let reader = channel.reader(get);
                    let times: Vec<f32> = reader
                        .read_inputs()
                        .ok_or_else(|| fail("a clip without times"))?
                        .collect();
                    let (track, keys) = match reader.read_outputs() {
                        Some(gltf::animation::util::ReadOutputs::Rotations(values)) => {
                            let values: Vec<Quat> =
                                values.into_f32().map(Quat::from_array).collect();
                            let keys = values.len();
                            (Track::Rotation(values), keys)
                        }
                        Some(gltf::animation::util::ReadOutputs::Translations(values)) => {
                            let values: Vec<Vec3> = values.map(Vec3::from).collect();
                            let keys = values.len();
                            (Track::Translation(values), keys)
                        }
                        Some(gltf::animation::util::ReadOutputs::Scales(values)) => {
                            let values: Vec<Vec3> = values.map(Vec3::from).collect();
                            let keys = values.len();
                            (Track::Scale(values), keys)
                        }
                        _ => return Err(fail("kit clips move, turn and scale joints only")),
                    };
                    if keys != times.len() || times.is_empty() {
                        return Err(fail("a clip's times and values do not match"));
                    }
                    duration = duration.max(times[times.len() - 1]);
                    channels.push(Channel {
                        joint,
                        times,
                        track,
                    });
                }
                if duration > 0.0 {
                    clips.push(Clip {
                        name: animation.name().unwrap_or_default().to_owned(),
                        duration,
                        channels,
                    });
                }
            }
            Some(Rig {
                joints: rig_joints,
                clips,
            })
        }
    };
    if data.tangents.len() != data.positions.len() && data.generate_tangents().is_err() {
        data.tangents.clear();
    }
    let material = primitive.material();
    let pbr = material.pbr_metallic_roughness();
    let kit_material = KitMaterial {
        base_color: Vec4::from(pbr.base_color_factor()),
        metallic: pbr.metallic_factor(),
        roughness: pbr.roughness_factor(),
        emissive: Vec3::from(material.emissive_factor()),
        double_sided: material.double_sided(),
        color: texture(pbr.base_color_texture().map(|t| t.texture().index()), true)?,
        normal: texture(
            material.normal_texture().map(|t| t.texture().index()),
            false,
        )?,
        metallic_roughness: texture(
            pbr.metallic_roughness_texture()
                .map(|t| t.texture().index()),
            false,
        )?,
        occlusion: texture(
            material.occlusion_texture().map(|t| t.texture().index()),
            false,
        )?,
        emissive_texture: texture(
            material.emissive_texture().map(|t| t.texture().index()),
            true,
        )?,
    };
    let material = materials
        .iter()
        .position(|known| *known == kit_material)
        .unwrap_or_else(|| {
            materials.push(kit_material);
            materials.len() - 1
        });
    Ok(KitEntry {
        key: entry.key.clone(),
        variant: entry.variant.clone(),
        mesh: data,
        material,
        rig,
        gear: None,
        rolling: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_paths_stay_inside_the_bundle() {
        assert_eq!(
            resolve("scenes/robot.core/default.gltf", "../../resources/a.bin").as_deref(),
            Ok("resources/a.bin")
        );
        assert!(resolve("scenes/robot.core/default.gltf", "../../../a.bin").is_err());
        assert!(safe_path("/etc/passwd").is_err());
        assert!(safe_path("scenes/../../x").is_err());
        assert!(safe_path("https://example.com/x").is_err());
        assert!(safe_path("scenes/robot.core/default.gltf").is_ok());
    }

    #[test]
    fn every_catalog_part_has_exactly_one_kit_and_every_kit_a_pin() {
        for part in BETA.parts {
            let kits = Bundle::ALL
                .iter()
                .filter(|bundle| bundle.holds(part.key))
                .count();
            assert_eq!(kits, 1, "{}", part.key);
        }
        assert!(Bundle::Locomotion.holds("robot.wheel"));
        assert!(Bundle::Locomotion.holds("robot.track"));
        assert!(!Bundle::Robot.holds("robot.track"));
        assert!(!Bundle::Locomotion.holds("robot.leg"));
        for bundle in Bundle::ALL {
            assert_eq!(pinned_manifest(bundle).len(), 64);
            assert_eq!(pinned_tar(bundle).len(), 64);
        }
    }

    #[test]
    fn a_manifest_off_its_pin_is_refused() {
        let error = Plan::of(Bundle::Robot, br#"{"assets": []}"#).unwrap_err();
        assert!(error.contains("pinned"), "{error}");
    }

    fn manifest(assets: &str) -> (Vec<u8>, String) {
        let bytes = format!(r#"{{"assets": [{assets}]}}"#).into_bytes();
        let pin = sha256(&bytes);
        (bytes, pin)
    }

    fn asset(key: &str) -> String {
        format!(
            r#"{{"key": "{key}", "variant": "default", "path": "scenes/x.gltf",
                "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                "resources": []}}"#
        )
    }

    #[test]
    fn unknown_kit_keys_and_unmapped_catalog_parts_fail_loudly() {
        let (bytes, pin) = manifest(&asset("robot.jetpack"));
        let error = Plan::checked(Bundle::Robot, &bytes, &pin).unwrap_err();
        assert!(error.contains("robot.jetpack"), "{error}");
        // Each kit holds only its own keys.
        let (bytes, pin) = manifest(&asset("weapon.laser"));
        let error = Plan::checked(Bundle::Robot, &bytes, &pin).unwrap_err();
        assert!(error.contains("weapon.laser"), "{error}");
        let (bytes, pin) = manifest(&asset("robot.sensor"));
        let error = Plan::checked(Bundle::Weapons, &bytes, &pin).unwrap_err();
        assert!(error.contains("robot.sensor"), "{error}");
        // A weapons kit without its descriptor is refused.
        let (bytes, pin) = manifest(&asset("mod.long-barrel"));
        let error = Plan::checked(Bundle::Weapons, &bytes, &pin).unwrap_err();
        assert!(error.contains(DESCRIPTOR), "{error}");
        // Kits with only the sensor leave the other parts and every mod unmapped.
        let plan = |key: &str| Plan {
            bundle: Bundle::Robot,
            entries: vec![EntryRef {
                key: key.into(),
                variant: "default".into(),
                scene: FileRef {
                    path: "scenes/x.gltf".into(),
                    sha256: String::new(),
                    byte_length: None,
                },
                resources: Vec::new(),
            }],
            descriptor: None,
        };
        let error = complete(&[plan("robot.sensor")]).unwrap_err();
        assert!(error.contains("in no kit"), "{error}");
        // The locomotion kit holds only the wheel and the track, with its descriptor and their
        // damaged looks.
        let (bytes, pin) = manifest(&asset("robot.leg"));
        let error = Plan::checked(Bundle::Locomotion, &bytes, &pin).unwrap_err();
        assert!(error.contains("robot.leg"), "{error}");
        let (bytes, pin) = manifest(&asset("robot.wheel"));
        let error = Plan::checked(Bundle::Locomotion, &bytes, &pin).unwrap_err();
        assert!(error.contains(DESCRIPTOR), "{error}");
    }

    #[test]
    fn a_locomotion_descriptor_is_read_and_checked() {
        let part = |key: &str, block: &str| format!(r#"{{"key": "{key}", "locomotion": {block}}}"#);
        let wheel = r#"{"axis": [0, 1, 0], "rollingRadius": 0.18,
            "bodies": [{"centre": [0.1, 0.1, -0.1], "radius": 0.18}], "wheels": ["wheel"],
            "roll": {"animation": "roll", "seconds": 1, "radiansPerCycle": 6.283185}}"#;
        let descriptor = |parts: &str, schema: &str| {
            format!(r#"{{"schema": "{schema}", "parts": [{parts}]}}"#).into_bytes()
        };
        let read = rolling(&descriptor(&part("robot.wheel", wheel), LOCOMOTION_SCHEMA)).unwrap();
        let (found, wheels) = &read["robot.wheel"];
        assert_eq!(found.axis, Vec3::Y);
        assert_eq!(found.centres, [Vec3::new(0.1, 0.1, -0.1)]);
        assert_eq!(wheels, &["wheel".to_owned()]);
        // Half a turn is half the clip; a turn back wraps forward.
        assert!((found.time(std::f32::consts::PI) - 0.5).abs() < 1e-5);
        assert!((found.time(-std::f32::consts::FRAC_PI_2) - 0.75).abs() < 1e-5);
        let error = rolling(&descriptor(
            &part("robot.wheel", wheel),
            "socket-grid-kit-descriptor-v2",
        ))
        .unwrap_err();
        assert!(error.contains(LOCOMOTION_SCHEMA), "{error}");
        let error = rolling(&descriptor(
            &part("robot.wheel", r#"{"axis": [0, 1, 0]}"#),
            LOCOMOTION_SCHEMA,
        ))
        .unwrap_err();
        assert!(error.contains("malformed"), "{error}");
    }

    #[test]
    fn clips_interpolate_and_wrap() {
        let half = Quat::from_rotation_x(0.5);
        let clip = Clip {
            name: "walk".into(),
            duration: 1.0,
            channels: vec![Channel {
                joint: 0,
                times: vec![0.0, 0.5, 1.0],
                track: Track::Rotation(vec![Quat::IDENTITY, half, Quat::IDENTITY]),
            }],
        };
        let mut rotations = [Quat::IDENTITY];
        clip.sample(0.25, &mut rotations);
        assert!(rotations[0].angle_between(Quat::from_rotation_x(0.25)) < 1e-4);
        clip.sample(1.5, &mut rotations);
        assert!(rotations[0].angle_between(half) < 1e-4);
    }

    #[test]
    fn clips_move_and_scale_and_hold_their_ends() {
        let clip = Clip {
            name: "fire".into(),
            duration: 0.2,
            channels: vec![
                Channel {
                    joint: 0,
                    times: vec![0.0, 0.2],
                    track: Track::Translation(vec![Vec3::ZERO, Vec3::Y]),
                },
                Channel {
                    joint: 1,
                    times: vec![0.0, 0.2],
                    track: Track::Scale(vec![Vec3::ONE, Vec3::ZERO]),
                },
            ],
        };
        let rest = Local {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        };
        let mut locals = [rest; 2];
        clip.pose(0.1, &mut locals);
        assert!((locals[0].translation - Vec3::Y * 0.5).length() < 1e-5);
        assert!((locals[1].scale - Vec3::splat(0.5)).length() < 1e-5);
        clip.pose(9.0, &mut locals);
        assert_eq!(locals[1].scale, Vec3::ZERO);
    }

    #[test]
    fn a_mod_frame_puts_its_up_along_the_anchors_up() {
        let anchor = Anchor {
            name: "mod-side".into(),
            node: None,
            position: Vec3::new(-0.05, 0.1, 0.0),
            normal: Vec3::NEG_X,
            up: Vec3::Y,
        };
        let frame = anchor.frame();
        assert!((frame.transform_vector3(Vec3::Y) - Vec3::NEG_X).length() < 1e-6);
        assert!((frame.transform_vector3(Vec3::NEG_Z) - Vec3::Y).length() < 1e-6);
        assert!((frame.transform_point3(Vec3::ZERO) - anchor.position).length() < 1e-6);
        assert!((frame.determinant() - 1.0).abs() < 1e-6);
    }
}
