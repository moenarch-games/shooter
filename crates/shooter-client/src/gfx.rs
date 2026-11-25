//! The client's drawn content: meshes, textures, materials, skins, instances and particle
//! emitters under the client's own handles, on one of two renderers.
//!
//! - **render-engine** (WebGPU in the browser, Vulkan, Metal or DX12 natively): the game's look,
//!   with shadows, clustered lights and GPU particles.
//! - **lite** ([`lite`]): a small forward renderer that needs neither compute shaders nor storage
//!   buffers, so it runs on WebGL 2. The browser falls back to it where WebGPU is missing
//!   (render-engine has no WebGL 2 backend: its `docs/webgl2-fallback.md`). It draws the same
//!   [`Scene`](render_engine::scene::Scene) and world with the sun, ambient and point lights,
//!   base-colour, emissive and metallic-roughness textures, CPU-simulated particles and the
//!   overlay, without shadows, normal maps or MSAA.
//!
//! The handles are the client's, not render-engine's (whose cannot be made without its world),
//! so the same code builds content for either.

pub mod lite;

use render_engine::glam::{Mat4, Vec3, Vec4};
pub use render_engine::material::AlphaMode;
use render_engine::material::TextureData;
use render_engine::mesh::MeshData;
use render_engine::particles::Emitter;
use render_engine::world as engine;

/// A mesh in a [`World`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MeshId(u32);

/// A texture in a [`World`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextureId(u32);

/// A material in a [`World`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MaterialId(u32);

/// A particle emitter in a [`World`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EmitterId(u32);

/// A joint palette in a [`World`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SkinId(u32);

/// A retained instance in a [`World`], drawn every frame until removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InstanceId(u32);

/// One drawn copy of a mesh (render-engine's `Instance`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Instance {
    pub mesh: MeshId,
    pub material: MaterialId,
    /// Model matrix; a negative determinant (mirroring) flips the winding.
    pub world_from_local: Mat4,
    /// The joint palette of a skinned mesh, in the mesh's local space.
    pub skin: Option<SkinId>,
}

/// A metallic-roughness material (render-engine's `StandardMaterial`, with the client's
/// texture handles).
#[derive(Clone, Copy, Debug)]
pub struct StandardMaterial {
    pub base_color: Vec4,
    pub base_color_texture: Option<TextureId>,
    /// Emitted radiance in cd/m² (exposed like the rest).
    pub emissive: Vec3,
    pub emissive_texture: Option<TextureId>,
    pub metallic: f32,
    pub perceptual_roughness: f32,
    /// Metallic in blue, roughness in green.
    pub metallic_roughness_texture: Option<TextureId>,
    pub reflectance: f32,
    pub normal_map_texture: Option<TextureId>,
    pub occlusion_texture: Option<TextureId>,
    pub alpha_mode: AlphaMode,
    pub blend_shadow_alpha_cutoff: f32,
    pub double_sided: bool,
    pub unlit: bool,
}

impl Default for StandardMaterial {
    fn default() -> Self {
        let engine = render_engine::material::StandardMaterial::default();
        Self {
            base_color: engine.base_color,
            base_color_texture: None,
            emissive: engine.emissive,
            emissive_texture: None,
            metallic: engine.metallic,
            perceptual_roughness: engine.perceptual_roughness,
            metallic_roughness_texture: None,
            reflectance: engine.reflectance,
            normal_map_texture: None,
            occlusion_texture: None,
            alpha_mode: engine.alpha_mode,
            blend_shadow_alpha_cutoff: engine.blend_shadow_alpha_cutoff,
            double_sided: engine.double_sided,
            unlit: engine.unlit,
        }
    }
}

impl StandardMaterial {
    /// An untextured material of `base_color`.
    #[must_use]
    pub fn color(base_color: Vec3) -> Self {
        Self {
            base_color: base_color.extend(1.0),
            ..Self::default()
        }
    }
}

/// Values under stable `u32` keys, freed keys reused.
pub(crate) struct Slab<T> {
    items: Vec<Option<T>>,
    free: Vec<u32>,
}

impl<T> Default for Slab<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            free: Vec::new(),
        }
    }
}

impl<T> Slab<T> {
    pub(crate) fn insert(&mut self, value: T) -> u32 {
        if let Some(key) = self.free.pop() {
            self.items[key as usize] = Some(value);
            key
        } else {
            self.items.push(Some(value));
            u32::try_from(self.items.len() - 1).expect("fewer than 2^32 items")
        }
    }

    pub(crate) fn remove(&mut self, key: u32) -> Option<T> {
        let value = self.items.get_mut(key as usize)?.take();
        if value.is_some() {
            self.free.push(key);
        }
        value
    }

    pub(crate) fn get(&self, key: u32) -> Option<&T> {
        self.items.get(key as usize)?.as_ref()
    }

    pub(crate) fn get_mut(&mut self, key: u32) -> Option<&mut T> {
        self.items.get_mut(key as usize)?.as_mut()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (u32, &T)> {
        self.items
            .iter()
            .enumerate()
            .filter_map(|(key, value)| Some((key as u32, value.as_ref()?)))
    }

    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = (u32, &mut T)> {
        self.items
            .iter_mut()
            .enumerate()
            .filter_map(|(key, value)| Some((key as u32, value.as_mut()?)))
    }
}

/// render-engine's world under the client's handles.
pub struct EngineWorld {
    pub(crate) world: engine::World,
    meshes: Slab<engine::MeshId>,
    textures: Slab<engine::TextureId>,
    materials: Slab<engine::MaterialId>,
    emitters: Slab<engine::EmitterId>,
    skins: Slab<engine::SkinId>,
    /// The client's instance beside render-engine's, for [`World::instance`].
    instances: Slab<(engine::InstanceId, Instance)>,
}

impl EngineWorld {
    fn texture(&self, id: Option<TextureId>) -> Option<engine::TextureId> {
        id.and_then(|id| self.textures.get(id.0).copied())
    }

    fn material(&self, m: &StandardMaterial) -> render_engine::material::StandardMaterial {
        render_engine::material::StandardMaterial {
            base_color: m.base_color,
            base_color_texture: self.texture(m.base_color_texture),
            emissive: m.emissive,
            emissive_texture: self.texture(m.emissive_texture),
            metallic: m.metallic,
            perceptual_roughness: m.perceptual_roughness,
            metallic_roughness_texture: self.texture(m.metallic_roughness_texture),
            reflectance: m.reflectance,
            normal_map_texture: self.texture(m.normal_map_texture),
            occlusion_texture: self.texture(m.occlusion_texture),
            alpha_mode: m.alpha_mode,
            blend_shadow_alpha_cutoff: m.blend_shadow_alpha_cutoff,
            double_sided: m.double_sided,
            unlit: m.unlit,
        }
    }

    fn instance(&self, instance: Instance) -> engine::Instance {
        engine::Instance {
            mesh: *self
                .meshes
                .get(instance.mesh.0)
                .expect("instance of a removed mesh"),
            material: *self
                .materials
                .get(instance.material.0)
                .expect("instance of a removed material"),
            world_from_local: instance.world_from_local,
            skin: instance
                .skin
                .map(|skin| *self.skins.get(skin.0).expect("instance of a removed skin")),
        }
    }
}

/// The drawn content, on render-engine or on the lite renderer.
pub enum World {
    Engine(Box<EngineWorld>),
    Lite(Box<lite::LiteWorld>),
}

impl World {
    pub(crate) fn engine(world: engine::World) -> Self {
        Self::Engine(Box::new(EngineWorld {
            world,
            meshes: Slab::default(),
            textures: Slab::default(),
            materials: Slab::default(),
            emitters: Slab::default(),
            skins: Slab::default(),
            instances: Slab::default(),
        }))
    }

    pub fn add_mesh(&mut self, mesh: &MeshData) -> MeshId {
        MeshId(match self {
            Self::Engine(e) => {
                let id = e.world.add_mesh(mesh);
                e.meshes.insert(id)
            }
            Self::Lite(l) => l.add_mesh(mesh),
        })
    }

    pub fn add_texture(&mut self, texture: &TextureData) -> TextureId {
        TextureId(match self {
            Self::Engine(e) => {
                let id = e.world.add_texture(texture);
                e.textures.insert(id)
            }
            Self::Lite(l) => l.add_texture(texture),
        })
    }

    pub fn add_material(&mut self, material: &StandardMaterial) -> MaterialId {
        MaterialId(match self {
            Self::Engine(e) => {
                let id = e.world.add_material(&e.material(material));
                e.materials.insert(id)
            }
            Self::Lite(l) => l.add_material(material),
        })
    }

    pub fn set_material(&mut self, id: MaterialId, material: &StandardMaterial) {
        match self {
            Self::Engine(e) => {
                let converted = e.material(material);
                let engine_id = *e
                    .materials
                    .get(id.0)
                    .expect("set_material on a removed material");
                e.world.set_material(engine_id, &converted);
            }
            Self::Lite(l) => l.set_material(id.0, material),
        }
    }

    pub fn remove_material(&mut self, id: MaterialId) {
        match self {
            Self::Engine(e) => {
                if let Some(engine_id) = e.materials.remove(id.0) {
                    e.world.remove_material(engine_id);
                }
            }
            Self::Lite(l) => l.remove_material(id.0),
        }
    }

    pub fn add_skin(&mut self, joints: &[Mat4]) -> SkinId {
        SkinId(match self {
            Self::Engine(e) => {
                let id = e.world.add_skin(joints);
                e.skins.insert(id)
            }
            Self::Lite(l) => l.add_skin(joints),
        })
    }

    pub fn set_skin(&mut self, id: SkinId, joints: &[Mat4]) {
        match self {
            Self::Engine(e) => {
                let engine_id = *e.skins.get(id.0).expect("set_skin on a removed skin");
                e.world.set_skin(engine_id, joints);
            }
            Self::Lite(l) => l.set_skin(id.0, joints),
        }
    }

    pub fn remove_skin(&mut self, id: SkinId) {
        match self {
            Self::Engine(e) => {
                if let Some(engine_id) = e.skins.remove(id.0) {
                    e.world.remove_skin(engine_id);
                }
            }
            Self::Lite(l) => l.remove_skin(id.0),
        }
    }

    pub fn add_instance(&mut self, instance: Instance) -> InstanceId {
        InstanceId(match self {
            Self::Engine(e) => {
                let id = e.world.add_instance(e.instance(instance));
                e.instances.insert((id, instance))
            }
            Self::Lite(l) => l.add_instance(instance),
        })
    }

    pub fn set_instance(&mut self, id: InstanceId, instance: Instance) {
        match self {
            Self::Engine(e) => {
                let converted = e.instance(instance);
                let slot = e
                    .instances
                    .get_mut(id.0)
                    .expect("set_instance on a removed instance");
                slot.1 = instance;
                let engine_id = slot.0;
                e.world.set_instance(engine_id, converted);
            }
            Self::Lite(l) => l.set_instance(id.0, instance),
        }
    }

    /// The instance as last added or set.
    #[must_use]
    pub fn instance(&self, id: InstanceId) -> Instance {
        match self {
            Self::Engine(e) => {
                e.instances
                    .get(id.0)
                    .expect("instance of a removed instance")
                    .1
            }
            Self::Lite(l) => l.instance(id.0),
        }
    }

    pub fn remove_instance(&mut self, id: InstanceId) {
        match self {
            Self::Engine(e) => {
                if let Some((engine_id, _)) = e.instances.remove(id.0) {
                    e.world.remove_instance(engine_id);
                }
            }
            Self::Lite(l) => l.remove_instance(id.0),
        }
    }

    pub fn add_emitter(&mut self, emitter: &Emitter) -> EmitterId {
        EmitterId(match self {
            Self::Engine(e) => {
                let id = e.world.add_emitter(emitter);
                e.emitters.insert(id)
            }
            Self::Lite(l) => l.add_emitter(emitter),
        })
    }

    /// `count` particles spawned at `world_from_emitter` in the next particle step.
    pub fn burst(&mut self, id: EmitterId, count: u32, world_from_emitter: Mat4) {
        match self {
            Self::Engine(e) => {
                let engine_id = *e.emitters.get(id.0).expect("burst on a removed emitter");
                e.world.burst(engine_id, count, world_from_emitter);
            }
            Self::Lite(l) => l.burst(id.0, count, world_from_emitter),
        }
    }
}
