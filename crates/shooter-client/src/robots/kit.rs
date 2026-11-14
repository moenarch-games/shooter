//! The asset-tooling kits the fighters are drawn from (Atlas D5.i46.1): the robot part kit
//! (release `shooter-combat-robot`) and the weapons kit (release `shooter-weapons`, the eight
//! weapons and four mods). Which bundle files the clients read, their content pins and size
//! bounds, and the decoding of their glTF scenes (meshes, skins, clips), KTX2 (Basis Universal)
//! textures and the weapons kit's socket-grid descriptor (anchors, glow channels, states) into
//! render-engine data. Both clients read the same files through [`Plan`] and [`decode`]: natively
//! from a directory, in the browser over HTTP (`crate::robots::load`).
//!
//! **Pins.** The releases are rolling, so the shooter pins each ([`Bundle::pin_json`],
//! `assets/*-kit.pin.json`): the tar's SHA-256 (checked by `scripts/fetch-robot-kit.sh`) and the
//! bundle manifest's (checked here). A manifest pins every scene, resource and attachment by
//! SHA-256 and length; a file that differs is refused, as is anything over [`MAX_FILE_BYTES`] or
//! a load over [`MAX_TOTAL_BYTES`].
//!
//! **IDs.** Kit entries are keyed by the simulation's part and mod keys
//! (`shooter_sim::robot::parts`, `BETA.mods`). Every kit key must be a catalog part or mod, every
//! catalog mod must be in the weapons kit and every catalog part in a kit or a declared
//! placeholder ([`placeholder_owner`]: the wheels and tracks of asset-tooling#634); anything else
//! fails the load loudly ([`complete`]). The one exception is a robot-kit entry for a part the
//! simulation has retired, listed by name with the issue that retired it ([`RETIRED_KIT_PARTS`]:
//! the control core, shooter#89): the load skips it and nothing draws it.

use render_engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use render_engine::material::{Sampler, TextureData};
use render_engine::mesh::MeshData;
use render_engine::wgpu;
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use shooter_sim::robot::{BETA, Kind};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// The most one bundle file may hold (the largest, the robot kit's normal atlas, is about
/// 0.45 MB).
pub const MAX_FILE_BYTES: usize = 4 << 20;
/// The most one load (both kits) may read (the drawn closures are about 1.5 MB and 1 MB).
pub const MAX_TOTAL_BYTES: usize = 8 << 20;
/// The variants the clients draw: every part's full detail, and the armor plate's damaged look.
/// (The weapons kit's `lod1` and `lod2` are not read: at the arena's distances a weapon is a few
/// hundred triangles.)
pub const VARIANTS: [&str; 2] = ["default", "damaged"];
/// The atlases' top mip level is skipped: at third-person distances a 1024 px atlas is plenty,
/// and it keeps the transcoded RGBA8 textures at a quarter of their full size (about 22 MB for
/// the robot kit and 6 MB for the weapons).
pub const SKIPPED_MIPS: u32 = 1;
/// The attachment of the weapons kit that describes its anchors, channels and states.
pub const DESCRIPTOR: &str = "socket-grid-descriptor";

/// One pinned asset-tooling release the clients draw from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bundle {
    /// `shooter-combat-robot`: the robot parts.
    Robot,
    /// `shooter-weapons`: the weapons and their mods.
    Weapons,
}

impl Bundle {
    pub const ALL: [Self; 2] = [Self::Robot, Self::Weapons];

    /// Its pin (`assets/robot-kit.pin.json`, `assets/weapon-kit.pin.json`).
    #[must_use]
    pub const fn pin_json(self) -> &'static str {
        match self {
            Self::Robot => include_str!("../../../../assets/robot-kit.pin.json"),
            Self::Weapons => include_str!("../../../../assets/weapon-kit.pin.json"),
        }
    }

    /// The directory it is fetched into (`target/…` natively, beside the page in the browser).
    #[must_use]
    pub const fn directory(self) -> &'static str {
        match self {
            Self::Robot => "robot-kit",
            Self::Weapons => "weapon-kit",
        }
    }

    /// Whether `key` may be an entry of this kit: a robot part, or a weapon or mod.
    fn holds(self, key: &str) -> bool {
        let part = BETA.parts.iter().find(|part| part.key == key);
        match self {
            Self::Robot => part.is_some_and(|part| !matches!(part.kind, Kind::Weapon(_))),
            Self::Weapons => {
                part.is_some_and(|part| matches!(part.kind, Kind::Weapon(_)))
                    || BETA.mods.iter().any(|modification| modification.key == key)
            }
        }
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

/// The issue that makes a catalog part no shipped kit draws yet, if it is one: those draw as
/// labelled placeholders. Any other catalog part missing from the kits is an error.
#[must_use]
pub fn placeholder_owner(key: &str) -> Option<&'static str> {
    matches!(key, "robot.wheel" | "robot.track").then_some("asset-tooling#634")
}

/// Robot-kit entries the pinned release still ships for parts the simulation has retired, and
/// the issue that retired each: the load skips them (nothing draws them) instead of failing.
/// `robot.core` went when the root chassis became the vital part (shooter#89); a later
/// `shooter-combat-robot` release can drop it, and then this list empties.
pub const RETIRED_KIT_PARTS: [(&str, &str); 1] = [("robot.core", "shooter#89")];

/// The issue that retired `key` in `bundle`, if the kit may still carry it as a retired part.
#[must_use]
pub fn retired(bundle: Bundle, key: &str) -> Option<&'static str> {
    (bundle == Bundle::Robot)
        .then(|| {
            RETIRED_KIT_PARTS
                .iter()
                .find(|(retired, _)| *retired == key)
        })
        .flatten()
        .map(|(_, issue)| *issue)
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
            if retired(bundle, key).is_some() {
                continue;
            }
            if !bundle.holds(key) {
                return Err(format!(
                    "{name}: entry {key} is no part or mod of the simulation's catalog this kit draws"
                ));
            }
            if !VARIANTS.contains(&variant) {
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

/// Checks that `plans` between them draw the whole catalog: every catalog part is in a kit or a
/// declared placeholder, and every catalog mod is in a kit.
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
        if !keys.contains(part.key) && placeholder_owner(part.key).is_none() {
            return Err(format!(
                "robot kit: catalog part {} is neither in a kit nor a declared placeholder",
                part.key
            ));
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
    complete(plans)?;
    let files = checked(plans, files)?;
    let mut textures: Vec<TextureData> = Vec::new();
    let mut texture_ids: HashMap<(String, bool), usize> = HashMap::new();
    let mut materials: Vec<KitMaterial> = Vec::new();
    let mut entries = Vec::new();
    for plan in plans {
        let gear = match &plan.descriptor {
            Some(descriptor) => Some(gear(files[&descriptor.path])?),
            None => None,
        };
        for entry in &plan.entries {
            let mut decoded = scene(
                entry,
                &files,
                &mut textures,
                &mut texture_ids,
                &mut materials,
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
            entries.push(decoded);
        }
    }
    Ok(KitData {
        entries,
        materials,
        textures,
    })
}

/// One entry's scene, its textures and material added to the load's.
fn scene(
    entry: &EntryRef,
    files: &HashMap<String, &[u8]>,
    textures: &mut Vec<TextureData>,
    texture_ids: &mut HashMap<(String, bool), usize>,
    materials: &mut Vec<KitMaterial>,
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
    let [node] = meshes.as_slice() else {
        return Err(fail("a kit part is exactly one mesh"));
    };
    let mesh = node.mesh().ok_or_else(|| fail("no mesh"))?;
    let primitives: Vec<gltf::Primitive> = mesh.primitives().collect();
    let [primitive] = primitives.as_slice() else {
        return Err(fail("a kit part is exactly one primitive"));
    };
    let skin = node.skin();
    // A static mesh is baked into its node's place; a skinned one keeps its bind pose (glTF
    // skins ignore the mesh node's transform).
    let placed = if skin.is_some() {
        Mat4::IDENTITY
    } else {
        global(node.index())
    };
    let normal_matrix = placed.inverse().transpose();
    let reader = primitive.reader(get);
    let positions: Vec<Vec3> = reader
        .read_positions()
        .ok_or_else(|| fail("no positions"))?
        .map(|p| placed.transform_point3(Vec3::from(p)))
        .collect();
    let mut data = MeshData {
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
    };
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
    fn only_the_wheels_and_tracks_are_declared_placeholders() {
        assert_eq!(placeholder_owner("weapon.rail-gun"), None);
        assert_eq!(placeholder_owner("robot.track"), Some("asset-tooling#634"));
        assert_eq!(placeholder_owner("robot.leg"), None);
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
        assert!(error.contains("neither in a kit"), "{error}");
    }

    #[test]
    fn the_retired_control_core_is_skipped_by_name_in_the_robot_kit_only() {
        // The retired core is no catalog part any more...
        assert!(BETA.parts.iter().all(|part| part.key != "robot.core"));
        assert_eq!(retired(Bundle::Robot, "robot.core"), Some("shooter#89"));
        assert_eq!(retired(Bundle::Weapons, "robot.core"), None);
        assert_eq!(retired(Bundle::Robot, "robot.sensor"), None);
        // ...so a robot kit that still ships it loads without it...
        let damaged = asset("robot.armor.plate").replace("\"default\"", "\"damaged\"");
        let (bytes, pin) = manifest(&format!("{}, {damaged}", asset("robot.core")));
        let plan = Plan::checked(Bundle::Robot, &bytes, &pin).unwrap();
        assert_eq!(plan.entries.len(), 1);
        assert!(plan.entries.iter().all(|entry| entry.key != "robot.core"));
        // ...while a weapons kit naming it is still refused, like any other stray key.
        let (bytes, pin) = manifest(&asset("robot.core"));
        let error = Plan::checked(Bundle::Weapons, &bytes, &pin).unwrap_err();
        assert!(error.contains("robot.core"), "{error}");
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
