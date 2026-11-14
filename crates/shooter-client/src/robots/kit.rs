//! The asset-tooling robot kit (release `shooter-combat-robot`, Atlas D5.i46.1): which bundle
//! files the clients read, their content pins and size bounds, and the decoding of its glTF
//! scenes (meshes, skins, the leg's walk clips) and KTX2 (Basis Universal) textures into
//! render-engine data. Both clients read the same files through [`Plan`] and [`decode`]: natively
//! from a directory, in the browser over HTTP (`crate::robots::load`).
//!
//! **Pins.** The release is rolling, so the shooter pins it (`assets/robot-kit.pin.json`): the
//! tar's SHA-256 (checked by `scripts/fetch-robot-kit.sh`) and the bundle manifest's (checked
//! here). The manifest pins every scene and resource by SHA-256 and length; a file that differs
//! is refused, as is anything over [`MAX_FILE_BYTES`] or a load over [`MAX_TOTAL_BYTES`].
//!
//! **IDs.** Kit entries are keyed by the simulation's part keys (`shooter_sim::robot::parts`).
//! Every kit key must be a catalog part and every catalog part must be either in the kit or a
//! declared placeholder ([`placeholder_owner`]: the weapons of asset-tooling#594, the wheels and
//! tracks of asset-tooling#634); anything else fails the load loudly.

use render_engine::glam::{Mat4, Quat, Vec2, Vec3, Vec4};
use render_engine::material::{Sampler, TextureData};
use render_engine::mesh::MeshData;
use render_engine::wgpu;
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use shooter_sim::robot::BETA;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// The pinned release (`assets/robot-kit.pin.json`).
pub const PIN_JSON: &str = include_str!("../../../../assets/robot-kit.pin.json");
/// The most one bundle file may hold (the largest, the normal atlas, is about 0.45 MB).
pub const MAX_FILE_BYTES: usize = 4 << 20;
/// The most one load may read (the drawn closure is about 1.5 MB).
pub const MAX_TOTAL_BYTES: usize = 8 << 20;
/// The variants the clients draw: every part's full detail, and the armor plate's damaged look.
pub const VARIANTS: [&str; 2] = ["default", "damaged"];
/// The atlas's top mip level is skipped: at third-person distances a 1024 px atlas (256 px a
/// cell) is plenty, and it keeps the transcoded RGBA8 textures at about 22 MB instead of 89 MB.
pub const SKIPPED_MIPS: u32 = 1;

/// The bundle manifest's pinned SHA-256 (hex).
///
/// # Panics
///
/// If the pin file is malformed (it is checked in by hand; the tests read it).
#[must_use]
pub fn pinned_manifest() -> String {
    let pin: Json = serde_json::from_str(PIN_JSON).expect("assets/robot-kit.pin.json is JSON");
    pin["manifestSha256"]
        .as_str()
        .expect("the pin names the manifest's SHA-256")
        .to_owned()
}

/// The issue that makes a catalog part no shipped kit draws yet, if it is one: those draw as
/// labelled placeholders. Any other catalog part missing from the kit is an error.
#[must_use]
pub fn placeholder_owner(key: &str) -> Option<&'static str> {
    if key.starts_with("weapon.") {
        Some("asset-tooling#594")
    } else if matches!(key, "robot.wheel" | "robot.track") {
        Some("asset-tooling#634")
    } else {
        None
    }
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

/// One drawn kit entry: a part key and variant, its scene and the resources it reads.
#[derive(Clone, Debug)]
pub struct EntryRef {
    pub key: String,
    pub variant: String,
    pub scene: FileRef,
    pub resources: Vec<FileRef>,
}

/// What a load reads, from the pinned manifest.
#[derive(Clone, Debug)]
pub struct Plan {
    pub entries: Vec<EntryRef>,
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
    /// The drawn entries of the bundle manifest `manifest`, after checking it against the pin
    /// and its keys against the simulation's catalog.
    ///
    /// # Errors
    ///
    /// If the manifest is not the pinned one or not a bundle manifest, a kit key is no catalog
    /// part, or a catalog part is neither in the kit nor a declared placeholder.
    pub fn of(manifest: &[u8]) -> Result<Self, String> {
        Self::checked(manifest, &pinned_manifest())
    }

    /// [`Plan::of`] against `pin` instead of the checked-in pin (tests).
    ///
    /// # Errors
    ///
    /// As [`Plan::of`].
    pub fn checked(manifest: &[u8], pin: &str) -> Result<Self, String> {
        if manifest.len() > MAX_FILE_BYTES {
            return Err("robot kit: the manifest is too large".into());
        }
        let hash = sha256(manifest);
        if hash != pin {
            return Err(format!(
                "robot kit: the manifest's SHA-256 is {hash}, pinned {pin} (refetch the kit with scripts/fetch-robot-kit.sh)"
            ));
        }
        let json: Json = serde_json::from_slice(manifest)
            .map_err(|error| format!("robot kit: manifest: {error}"))?;
        let assets = json["assets"]
            .as_array()
            .ok_or("robot kit: the manifest lists no assets")?;
        let mut kit_keys = BTreeSet::new();
        let mut entries = Vec::new();
        for asset in assets {
            let key = asset["key"]
                .as_str()
                .ok_or("robot kit: an asset has no key")?;
            let variant = asset["variant"]
                .as_str()
                .ok_or_else(|| format!("robot kit: {key} has no variant"))?;
            if !BETA.parts.iter().any(|part| part.key == key) {
                return Err(format!(
                    "robot kit: entry {key} is no part of the simulation's catalog"
                ));
            }
            kit_keys.insert(key.to_owned());
            if !VARIANTS.contains(&variant) {
                continue;
            }
            let scene = file_ref(asset)?;
            let resources = asset["resources"]
                .as_array()
                .ok_or_else(|| format!("robot kit: {key} lists no resources"))?
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
        for part in BETA.parts {
            if !kit_keys.contains(part.key) && placeholder_owner(part.key).is_none() {
                return Err(format!(
                    "robot kit: catalog part {} is neither in the kit nor a declared placeholder",
                    part.key
                ));
            }
        }
        if !entries
            .iter()
            .any(|entry| entry.key == "robot.armor.plate" && entry.variant == "damaged")
        {
            return Err("robot kit: robot.armor.plate has no damaged variant".into());
        }
        Ok(Self { entries })
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
        files.into_values().collect()
    }
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

/// One rotation track of a clip, sampled linearly.
#[derive(Clone, Debug, PartialEq)]
pub struct Channel {
    pub joint: usize,
    pub times: Vec<f32>,
    pub rotations: Vec<Quat>,
}

impl Channel {
    /// The rotation at `time` (seconds), held at the ends.
    #[must_use]
    pub fn sample(&self, time: f32) -> Quat {
        let (Some(first), Some(last)) = (self.times.first(), self.times.last()) else {
            return Quat::IDENTITY;
        };
        if time <= *first {
            return self.rotations[0];
        }
        if time >= *last {
            return self.rotations[self.rotations.len() - 1];
        }
        let next = self.times.partition_point(|at| *at <= time);
        let (t0, t1) = (self.times[next - 1], self.times[next]);
        let blend = if t1 > t0 {
            (time - t0) / (t1 - t0)
        } else {
            0.0
        };
        self.rotations[next - 1]
            .slerp(self.rotations[next], blend)
            .normalize()
    }
}

/// A looping joint animation (the leg's walk clips).
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub name: String,
    /// One cycle (seconds).
    pub duration: f32,
    pub channels: Vec<Channel>,
}

impl Clip {
    /// Sets the joints the clip turns to their rotations `phase` (0..1, wrapping) through it.
    pub fn sample(&self, phase: f32, rotations: &mut [Quat]) {
        let time = phase.rem_euclid(1.0) * self.duration;
        for channel in &self.channels {
            if let Some(rotation) = rotations.get_mut(channel.joint) {
                *rotation = channel.sample(time);
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

    /// Each joint's transform in the skinned mesh's frame, with `rotations` (one a joint).
    #[must_use]
    pub fn globals(&self, rotations: &[Quat]) -> Vec<Mat4> {
        let mut globals: Vec<Option<Mat4>> = vec![None; self.joints.len()];
        // Parents come first in the kit; a few passes settle any other order.
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
                    let rotation = rotations.get(index).copied().unwrap_or(joint.rotation);
                    globals[index] = Some(
                        parent
                            * Mat4::from_scale_rotation_translation(
                                joint.scale,
                                rotation,
                                joint.translation,
                            ),
                    );
                }
            }
        }
        globals.into_iter().map(Option::unwrap_or_default).collect()
    }

    /// The joint palette render-engine skins with: each joint's posed transform times its
    /// inverse bind matrix (identity everywhere at rest).
    #[must_use]
    pub fn palette(&self, rotations: &[Quat]) -> Vec<Mat4> {
        self.globals(rotations)
            .into_iter()
            .zip(&self.joints)
            .map(|(global, joint)| global * joint.inverse_bind)
            .collect()
    }
}

/// The kit's one material, as its glTF describes it.
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
}

/// The decoded kit: what the clients upload and pose.
#[derive(Debug)]
pub struct KitData {
    pub entries: Vec<KitEntry>,
    pub materials: Vec<KitMaterial>,
    pub textures: Vec<TextureData>,
}

impl KitData {
    /// The entry for part `key` in `variant`.
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
    plan: &Plan,
    files: &'a HashMap<String, Vec<u8>>,
) -> Result<HashMap<String, &'a [u8]>, String> {
    let mut total = 0;
    let mut out = HashMap::new();
    for file in plan.files() {
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

/// Decodes the drawn entries of `plan` from `files` (bundle path → bytes).
///
/// # Errors
///
/// If a file is missing, too large or not its pinned content, or a scene is not the one-mesh
/// part the kit standard describes.
pub fn decode(plan: &Plan, files: &HashMap<String, Vec<u8>>) -> Result<KitData, String> {
    let files = checked(plan, files)?;
    let mut textures: Vec<TextureData> = Vec::new();
    let mut texture_ids: HashMap<(String, bool), usize> = HashMap::new();
    let mut materials: Vec<KitMaterial> = Vec::new();
    let mut entries = Vec::new();
    for entry in &plan.entries {
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
                        if channel.target().property() != gltf::animation::Property::Rotation {
                            return Err(fail("kit clips turn joints only"));
                        }
                        if channel.sampler().interpolation()
                            != gltf::animation::Interpolation::Linear
                        {
                            return Err(fail("kit clips are sampled linearly"));
                        }
                        let reader = channel.reader(get);
                        let times: Vec<f32> = reader
                            .read_inputs()
                            .ok_or_else(|| fail("a clip without times"))?
                            .collect();
                        let Some(gltf::animation::util::ReadOutputs::Rotations(rotations)) =
                            reader.read_outputs()
                        else {
                            return Err(fail("a rotation channel without rotations"));
                        };
                        let rotations: Vec<Quat> =
                            rotations.into_f32().map(Quat::from_array).collect();
                        if rotations.len() != times.len() || times.is_empty() {
                            return Err(fail("a clip's times and rotations do not match"));
                        }
                        duration = duration.max(times[times.len() - 1]);
                        channels.push(Channel {
                            joint,
                            times,
                            rotations,
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
        entries.push(KitEntry {
            key: entry.key.clone(),
            variant: entry.variant.clone(),
            mesh: data,
            material,
            rig,
        });
    }
    Ok(KitData {
        entries,
        materials,
        textures,
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
    fn only_the_weapons_wheels_and_tracks_are_declared_placeholders() {
        assert_eq!(
            placeholder_owner("weapon.rail-gun"),
            Some("asset-tooling#594")
        );
        assert_eq!(placeholder_owner("robot.track"), Some("asset-tooling#634"));
        assert_eq!(placeholder_owner("robot.leg"), None);
        assert_eq!(pinned_manifest().len(), 64);
    }

    #[test]
    fn a_manifest_off_its_pin_is_refused() {
        let error = Plan::of(br#"{"assets": []}"#).unwrap_err();
        assert!(error.contains("pinned"), "{error}");
    }

    fn manifest(assets: &str) -> (Vec<u8>, String) {
        let bytes = format!(r#"{{"assets": [{assets}]}}"#).into_bytes();
        let pin = sha256(&bytes);
        (bytes, pin)
    }

    #[test]
    fn unknown_kit_keys_and_unmapped_catalog_parts_fail_loudly() {
        let (bytes, pin) = manifest(
            r#"{"key": "robot.jetpack", "variant": "default", "path": "scenes/x.gltf",
                "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                "resources": []}"#,
        );
        let error = Plan::checked(&bytes, &pin).unwrap_err();
        assert!(error.contains("robot.jetpack"), "{error}");
        // A kit with only the core leaves the other robot parts unmapped.
        let (bytes, pin) = manifest(
            r#"{"key": "robot.core", "variant": "default", "path": "scenes/x.gltf",
                "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                "resources": []}"#,
        );
        let error = Plan::checked(&bytes, &pin).unwrap_err();
        assert!(error.contains("neither in the kit"), "{error}");
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
                rotations: vec![Quat::IDENTITY, half, Quat::IDENTITY],
            }],
        };
        let mut rotations = [Quat::IDENTITY];
        clip.sample(0.25, &mut rotations);
        assert!(rotations[0].angle_between(Quat::from_rotation_x(0.25)) < 1e-4);
        clip.sample(1.5, &mut rotations);
        assert!(rotations[0].angle_between(half) < 1e-4);
    }
}
