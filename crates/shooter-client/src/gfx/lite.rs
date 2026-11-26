//! The lite renderer: the client's world drawn with forward shading that needs no compute
//! shaders, storage buffers, cube-array textures or indirect draws, so wgpu runs it on WebGL 2
//! (and on any native adapter). It is the browser's fallback where WebGPU is missing
//! ([`super`]): one pass into the target with the sun, the ambient light and up to
//! [`MAX_LIGHTS`] point lights, textures, meshes skinned from a uniform joint palette, CPU-simulated particles and the
//! overlay, and linear distance fog as render-engine's (its other falloffs and sun glow are left out);
//! no shadows, sky, normal maps or MSAA.
//!
//! Draws are batched: opaque instances of one mesh and material are one instanced draw, and
//! instances outside the view frustum are skipped on the CPU.

use super::{AlphaMode, Instance, Slab, StandardMaterial};
use bytemuck::{Pod, Zeroable};
use render_engine::glam::{Mat3, Mat4, Vec2, Vec3, Vec4};
use render_engine::material::{Sampler, TextureData};
use render_engine::mesh::MeshData;
use render_engine::particles::{
    ColorBlend, Emitter, InitModifier, Orient, ParticleBlend, ShapeDimension, Value,
};
use render_engine::scene::{FogFalloff, Scene};
use render_engine::wgpu;
use std::collections::HashMap;
use wgpu::util::DeviceExt;

/// Point lights shaded per fragment (the nearest in view win).
pub const MAX_LIGHTS: usize = 8;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    uv: [f32; 2],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FrameUniform {
    clip_from_world: [[f32; 4]; 4],
    eye: [f32; 4],
    sun_direction: [f32; 4],
    sun_color: [f32; 4],
    ambient: [f32; 4],
    params: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    /// Linear fog: colour (exposed) and how much it hides at most; then start and end metres.
    fog: [f32; 4],
    fog_range: [f32; 4],
    lights: [[f32; 4]; 2 * MAX_LIGHTS],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct MaterialUniform {
    base_color: [f32; 4],
    emissive: [f32; 4],
    surface: [f32; 4],
    mode: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ParticleInstance {
    position: [f32; 3],
    size: [f32; 4],
    color: [f32; 4],
    velocity: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct OverlayVertex {
    position: [f32; 2],
    color: [f32; 4],
    /// Pixels from the quad's centre.
    local: [f32; 2],
    /// Half width, half height, corner radius (pixels), unused.
    shape: [f32; 4],
    uv: [f32; 2],
}

/// A skinned vertex's `JOINTS_0` and `WEIGHTS_0` (`lite.wgsl` `SkinIn`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct SkinVertex {
    joints: [u16; 4],
    weights: [f32; 4],
}

/// Joints a skin's palette holds (`lite.wgsl` `MAX_JOINTS`): the kits' rigs have at most 31.
pub const MAX_JOINTS: usize = 32;

/// One palette in the frame's joint buffer: [`MAX_JOINTS`] matrices, a multiple of the 256-byte
/// uniform offset alignment.
const PALETTE_BYTES: u64 = (MAX_JOINTS * 64) as u64;

struct Mesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    /// Bounding sphere in the mesh's space.
    center: Vec3,
    radius: f32,
    /// A skinned mesh's joints and weights, posed on the GPU.
    skin: Option<wgpu::Buffer>,
}

struct Texture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// Its overlay bind group (texture and sampler), made on first use.
    overlay: Option<wgpu::BindGroup>,
    sampler: Sampler,
}

struct Material {
    desc: StandardMaterial,
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

struct Particle {
    position: Vec3,
    velocity: Vec3,
    age: f32,
    lifetime: f32,
    size: f32,
    color: Vec4,
}

struct CpuEmitter {
    desc: Emitter,
    particles: Vec<Particle>,
    bursts: Vec<(u32, Mat4)>,
    rng: u32,
}

/// The lite renderer's content (the [`super::World`] on it).
pub struct LiteWorld {
    device: wgpu::Device,
    queue: wgpu::Queue,
    material_layout: wgpu::BindGroupLayout,
    white: wgpu::TextureView,
    samplers: HashMap<SamplerKey, wgpu::Sampler>,
    meshes: Slab<Mesh>,
    textures: Slab<Texture>,
    materials: Slab<Material>,
    skins: Slab<Vec<Mat4>>,
    instances: Slab<Instance>,
    emitters: Slab<CpuEmitter>,
}

type SamplerKey = (
    wgpu::AddressMode,
    wgpu::AddressMode,
    wgpu::FilterMode,
    wgpu::FilterMode,
    wgpu::MipmapFilterMode,
);

fn material_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let texture = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("lite material"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            texture(1),
            texture(2),
            texture(3),
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

impl LiteWorld {
    #[must_use]
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let white = device
            .create_texture_with_data(
                queue,
                &wgpu::TextureDescriptor {
                    label: Some("lite white"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::util::TextureDataOrder::LayerMajor,
                &[255; 4],
            )
            .create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            device: device.clone(),
            queue: queue.clone(),
            material_layout: material_layout(device),
            white,
            samplers: HashMap::new(),
            meshes: Slab::default(),
            textures: Slab::default(),
            materials: Slab::default(),
            skins: Slab::default(),
            instances: Slab::default(),
            emitters: Slab::default(),
        }
    }

    pub(super) fn add_mesh(&mut self, mesh: &MeshData) -> u32 {
        let vertices: Vec<Vertex> = mesh
            .positions
            .iter()
            .enumerate()
            .map(|(i, position)| Vertex {
                position: position.to_array(),
                normal: mesh.normals.get(i).copied().unwrap_or(Vec3::Y).to_array(),
                uv: mesh.uvs.get(i).copied().unwrap_or(Vec2::ZERO).to_array(),
                color: mesh.colors.get(i).copied().unwrap_or(Vec4::ONE).to_array(),
            })
            .collect();
        let indices: Vec<u32> = if mesh.indices.is_empty() {
            (0..u32::try_from(vertices.len()).unwrap_or(u32::MAX)).collect()
        } else {
            mesh.indices.clone()
        };
        let (low, high) = mesh.positions.iter().fold(
            (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
            |(low, high), p| (low.min(*p), high.max(*p)),
        );
        let center = if mesh.positions.is_empty() {
            Vec3::ZERO
        } else {
            (low + high) / 2.0
        };
        let radius = mesh
            .positions
            .iter()
            .map(|p| p.distance(center))
            .fold(0.0, f32::max);
        let skinned = !mesh.joints.is_empty() && mesh.joints.len() == mesh.positions.len();
        let buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("lite vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
        let skin = skinned.then(|| {
            let skin: Vec<SkinVertex> = mesh
                .joints
                .iter()
                .enumerate()
                .map(|(i, joints)| SkinVertex {
                    joints: *joints,
                    weights: mesh.weights.get(i).copied().unwrap_or(Vec4::X).to_array(),
                })
                .collect();
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("lite skin"),
                    contents: bytemuck::cast_slice(&skin),
                    usage: wgpu::BufferUsages::VERTEX,
                })
        });
        let index_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("lite indices"),
                contents: bytemuck::cast_slice(&indices),
                usage: wgpu::BufferUsages::INDEX,
            });
        self.meshes.insert(Mesh {
            vertices: buffer,
            indices: index_buffer,
            index_count: u32::try_from(indices.len()).unwrap_or(u32::MAX),
            center,
            radius,
            skin,
        })
    }

    pub(super) fn add_texture(&mut self, texture: &TextureData) -> u32 {
        let levels = u32::try_from(texture.levels.len().max(1)).unwrap_or(1);
        let data: Vec<u8> = texture.levels.concat();
        let gpu = self.device.create_texture_with_data(
            &self.queue,
            &wgpu::TextureDescriptor {
                label: Some("lite texture"),
                size: wgpu::Extent3d {
                    width: texture.width,
                    height: texture.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: levels,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: texture.format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::MipMajor,
            &data,
        );
        self.textures.insert(Texture {
            view: gpu.create_view(&wgpu::TextureViewDescriptor::default()),
            texture: gpu,
            sampler: texture.sampler,
            overlay: None,
        })
    }

    /// Overwrites a `size` region of `id`'s level 0 at `origin` with tightly packed `pixels`.
    pub(super) fn write_texture(
        &mut self,
        id: u32,
        origin: (u32, u32),
        size: (u32, u32),
        pixels: &[u8],
    ) {
        let Some(texture) = self.textures.get(id) else {
            return;
        };
        let block = texture.texture.format().block_copy_size(None).unwrap_or(4);
        if size.0 == 0 || size.1 == 0 {
            return;
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: origin.0,
                    y: origin.1,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size.0 * block),
                rows_per_image: Some(size.1),
            },
            wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
        );
    }

    /// The overlay bind group of texture `id` (white for none or a removed one), made once.
    fn overlay_group(
        &mut self,
        id: Option<u32>,
        layout: &wgpu::BindGroupLayout,
    ) -> wgpu::BindGroup {
        if let Some(group) = id
            .and_then(|id| self.textures.get(id))
            .and_then(|texture| texture.overlay.clone())
        {
            return group;
        }
        let (view, sampler) = match id.and_then(|id| self.textures.get(id)) {
            Some(texture) => (texture.view.clone(), texture.sampler),
            None => (self.white.clone(), Sampler::default()),
        };
        let sampler = self.sampler(sampler);
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lite overlay image"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        if let Some(texture) = id.and_then(|id| self.textures.get_mut(id)) {
            texture.overlay = Some(group.clone());
        }
        group
    }

    fn sampler(&mut self, sampler: Sampler) -> wgpu::Sampler {
        let key = (
            sampler.address_mode_u,
            sampler.address_mode_v,
            sampler.mag_filter,
            sampler.min_filter,
            sampler.mipmap_filter,
        );
        let device = &self.device;
        self.samplers
            .entry(key)
            .or_insert_with(|| {
                device.create_sampler(&wgpu::SamplerDescriptor {
                    label: Some("lite sampler"),
                    address_mode_u: sampler.address_mode_u,
                    address_mode_v: sampler.address_mode_v,
                    mag_filter: sampler.mag_filter,
                    min_filter: sampler.min_filter,
                    mipmap_filter: sampler.mipmap_filter,
                    ..Default::default()
                })
            })
            .clone()
    }

    fn material_uniform(material: &StandardMaterial) -> MaterialUniform {
        let (mask, cutoff) = match material.alpha_mode {
            AlphaMode::Mask(cutoff) => (1.0, cutoff),
            AlphaMode::Opaque | AlphaMode::Blend => (0.0, 0.0),
        };
        MaterialUniform {
            base_color: material.base_color.to_array(),
            emissive: material.emissive.extend(0.0).to_array(),
            surface: [
                material.metallic,
                material.perceptual_roughness,
                material.reflectance,
                cutoff,
            ],
            mode: [
                mask,
                f32::from(u8::from(material.unlit)),
                f32::from(u8::from(material.double_sided)),
                0.0,
            ],
        }
    }

    fn material_bind_group(
        &mut self,
        material: &StandardMaterial,
        buffer: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        let sampler_desc = material
            .base_color_texture
            .or(material.emissive_texture)
            .or(material.metallic_roughness_texture)
            .and_then(|id| self.textures.get(id.0))
            .map_or_else(Sampler::default, |texture| texture.sampler);
        let sampler = self.sampler(sampler_desc);
        let view = |id: Option<super::TextureId>| {
            id.and_then(|id| self.textures.get(id.0))
                .map_or(&self.white, |texture| &texture.view)
        };
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lite material"),
            layout: &self.material_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(view(material.base_color_texture)),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(view(material.emissive_texture)),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(view(
                        material.metallic_roughness_texture,
                    )),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        })
    }

    pub(super) fn add_material(&mut self, material: &StandardMaterial) -> u32 {
        let buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("lite material"),
                contents: bytemuck::bytes_of(&Self::material_uniform(material)),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });
        let bind_group = self.material_bind_group(material, &buffer);
        self.materials.insert(Material {
            desc: *material,
            buffer,
            bind_group,
        })
    }

    pub(super) fn set_material(&mut self, id: u32, material: &StandardMaterial) {
        let Some(old) = self.materials.get(id) else {
            return;
        };
        let textures_changed = (
            old.desc.base_color_texture,
            old.desc.emissive_texture,
            old.desc.metallic_roughness_texture,
        ) != (
            material.base_color_texture,
            material.emissive_texture,
            material.metallic_roughness_texture,
        );
        let buffer = old.buffer.clone();
        self.queue.write_buffer(
            &buffer,
            0,
            bytemuck::bytes_of(&Self::material_uniform(material)),
        );
        let bind_group = textures_changed.then(|| self.material_bind_group(material, &buffer));
        let slot = self.materials.get_mut(id).expect("checked above");
        slot.desc = *material;
        if let Some(bind_group) = bind_group {
            slot.bind_group = bind_group;
        }
    }

    pub(super) fn remove_material(&mut self, id: u32) {
        self.materials.remove(id);
    }

    pub(super) fn add_skin(&mut self, joints: &[Mat4]) -> u32 {
        self.skins.insert(joints.to_vec())
    }

    pub(super) fn set_skin(&mut self, id: u32, joints: &[Mat4]) {
        if let Some(skin) = self.skins.get_mut(id) {
            skin.clear();
            skin.extend_from_slice(joints);
        }
    }

    pub(super) fn remove_skin(&mut self, id: u32) {
        self.skins.remove(id);
    }

    pub(super) fn add_instance(&mut self, instance: Instance) -> u32 {
        self.instances.insert(instance)
    }

    pub(super) fn set_instance(&mut self, id: u32, instance: Instance) {
        if let Some(slot) = self.instances.get_mut(id) {
            *slot = instance;
        }
    }

    pub(super) fn instance(&self, id: u32) -> Instance {
        *self
            .instances
            .get(id)
            .expect("instance of a removed instance")
    }

    pub(super) fn remove_instance(&mut self, id: u32) {
        self.instances.remove(id);
    }

    pub(super) fn add_emitter(&mut self, emitter: &Emitter) -> u32 {
        self.emitters.insert(CpuEmitter {
            desc: emitter.clone(),
            particles: Vec::new(),
            bursts: Vec::new(),
            rng: render_engine::particles::pcg_hash(emitter.seed) | 1,
        })
    }

    pub(super) fn burst(&mut self, id: u32, count: u32, world_from_emitter: Mat4) {
        if count > 0
            && let Some(emitter) = self.emitters.get_mut(id)
        {
            emitter.bursts.push((count, world_from_emitter));
        }
    }

    /// One particle step of `dt` seconds: the bursts spawn, then every particle ages,
    /// accelerates and moves; the dead are dropped.
    pub fn step_particles(&mut self, dt: f32) {
        for (_, emitter) in self.emitters.iter_mut() {
            emitter.step(dt);
        }
    }
}

/// xorshift32: the particles' own randomness (they are effects, nothing simulates with them).
fn random(state: &mut u32) -> f32 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    #[allow(clippy::cast_precision_loss)]
    let unit = (x >> 8) as f32 / (1u32 << 24) as f32;
    unit
}

fn uniform_f32(value: Value<f32>, rng: &mut u32) -> f32 {
    match value {
        Value::Const(v) => v,
        Value::Uniform { min, max } => min + (max - min) * random(rng),
    }
}

fn uniform_vec3(value: Value<Vec3>, rng: &mut u32) -> Vec3 {
    match value {
        Value::Const(v) => v,
        Value::Uniform { min, max } => {
            min + (max - min) * Vec3::new(random(rng), random(rng), random(rng))
        }
    }
}

fn uniform_vec4(value: Value<Vec4>, rng: &mut u32) -> Vec4 {
    match value {
        Value::Const(v) => v,
        Value::Uniform { min, max } => {
            min + (max - min) * Vec4::new(random(rng), random(rng), random(rng), random(rng))
        }
    }
}

/// A random unit vector.
fn on_sphere(rng: &mut u32) -> Vec3 {
    let z = random(rng) * 2.0 - 1.0;
    let angle = random(rng) * std::f32::consts::TAU;
    let r = (1.0 - z * z).max(0.0).sqrt();
    Vec3::new(r * angle.cos(), r * angle.sin(), z)
}

impl CpuEmitter {
    fn spawn(&mut self, world_from_emitter: Mat4) -> Particle {
        let rng = &mut self.rng;
        let mut p = Particle {
            position: Vec3::ZERO,
            velocity: Vec3::ZERO,
            age: 0.0,
            lifetime: f32::INFINITY,
            size: 1.0,
            color: Vec4::ONE,
        };
        for modifier in &self.desc.init {
            match *modifier {
                InitModifier::PositionSphere {
                    center,
                    radius,
                    dimension,
                } => {
                    let r = match dimension {
                        ShapeDimension::Volume => radius * random(rng).cbrt(),
                        ShapeDimension::Surface => radius,
                    };
                    p.position = center + on_sphere(rng) * r;
                }
                InitModifier::Position(value) => p.position = uniform_vec3(value, rng),
                InitModifier::VelocitySphere { center, speed } => {
                    p.velocity =
                        (p.position - center).normalize_or_zero() * uniform_f32(speed, rng);
                }
                InitModifier::Velocity(value) => p.velocity = uniform_vec3(value, rng),
                InitModifier::VelocityCone {
                    direction,
                    spread,
                    speed,
                } => {
                    // A direction within `spread` radians of the axis, uniform over the cap.
                    let axis = direction.normalize_or(Vec3::Z);
                    let cos_max = spread.cos();
                    let cos = 1.0 - random(rng) * (1.0 - cos_max);
                    let sin = (1.0 - cos * cos).max(0.0).sqrt();
                    let angle = random(rng) * std::f32::consts::TAU;
                    let (u, v) = axis.any_orthonormal_pair();
                    let dir = axis * cos + (u * angle.cos() + v * angle.sin()) * sin;
                    p.velocity = dir * uniform_f32(speed, rng);
                }
                InitModifier::Lifetime(value) => p.lifetime = uniform_f32(value, rng),
                InitModifier::Age(value) => p.age = uniform_f32(value, rng),
                InitModifier::Size(value) => p.size = uniform_f32(value, rng),
                InitModifier::Color(value) => p.color = uniform_vec4(value, rng),
            }
        }
        p.position = world_from_emitter.transform_point3(p.position);
        p.velocity = world_from_emitter.transform_vector3(p.velocity);
        p
    }

    fn step(&mut self, dt: f32) {
        let capacity = self.desc.capacity as usize;
        let bursts = std::mem::take(&mut self.bursts);
        for (count, at) in bursts {
            for _ in 0..count {
                if self.particles.len() >= capacity {
                    break;
                }
                let particle = self.spawn(at);
                self.particles.push(particle);
            }
        }
        let accel = self.desc.accel;
        self.particles.retain_mut(|p| {
            p.age += dt;
            p.velocity += accel * dt;
            p.position += p.velocity * dt;
            p.age < p.lifetime
        });
    }

    /// Size and colour now, as render-engine's `render_attributes`.
    fn look(&self, p: &Particle) -> (Vec2, Vec4) {
        let t = p.age / p.lifetime;
        let size = self
            .desc
            .size_over_life
            .as_ref()
            .and_then(|g| g.sample(t))
            .unwrap_or(Vec2::splat(p.size));
        let color = match self
            .desc
            .color_over_life
            .as_ref()
            .and_then(|c| Some((c.blend, c.gradient.sample(t)?)))
        {
            None => p.color,
            Some((ColorBlend::Overwrite, g)) => g,
            Some((ColorBlend::Multiply, g)) => p.color * g,
            Some((ColorBlend::Add, g)) => p.color + g,
        };
        (size, color)
    }
}

/// Frustum planes (inward normals in xyz, distance in w) of a reverse-Z infinite projection: the
/// four sides and the near plane.
fn frustum(clip_from_world: Mat4) -> [Vec4; 5] {
    let rows = [
        clip_from_world.row(0),
        clip_from_world.row(1),
        clip_from_world.row(2),
        clip_from_world.row(3),
    ];
    let normalize = |plane: Vec4| plane / plane.truncate().length().max(1e-12);
    [
        normalize(rows[3] + rows[0]),
        normalize(rows[3] - rows[0]),
        normalize(rows[3] + rows[1]),
        normalize(rows[3] - rows[1]),
        normalize(rows[3] - rows[2]),
    ]
}

fn visible(planes: &[Vec4; 5], center: Vec3, radius: f32) -> bool {
    planes
        .iter()
        .all(|plane| plane.truncate().dot(center) + plane.w >= -radius)
}

/// The camera's view matrix, as render-engine's `look_at` (Bevy `Transform::looking_at`).
fn look_at(eye: Vec3, target: Vec3) -> Mat4 {
    let back = -(target - eye).normalize_or(Vec3::NEG_Z);
    let right = Vec3::Y.cross(back).normalize_or(Vec3::X);
    let rotation = Mat3::from_cols(right, back.cross(right), back);
    (Mat4::from_translation(eye) * Mat4::from_mat3(rotation)).inverse()
}

/// Bevy's right-handed infinite reverse-Z perspective projection.
fn perspective(fov_y: f32, aspect: f32, near: f32) -> Mat4 {
    let h = 1.0 / (0.5 * fov_y).tan();
    Mat4::from_cols(
        Vec4::new(h / aspect, 0.0, 0.0, 0.0),
        Vec4::new(0.0, h, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 0.0, -1.0),
        Vec4::new(0.0, 0.0, near, 0.0),
    )
}

/// A buffer that grows to what a frame needs.
pub(super) struct Growing {
    pub(super) buffer: wgpu::Buffer,
    usage: wgpu::BufferUsages,
    label: &'static str,
}

impl Growing {
    pub(super) fn new(
        device: &wgpu::Device,
        label: &'static str,
        usage: wgpu::BufferUsages,
    ) -> Self {
        let usage = usage | wgpu::BufferUsages::COPY_DST;
        Self {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 4096,
                usage,
                mapped_at_creation: false,
            }),
            usage,
            label,
        }
    }

    pub(super) fn write(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let needed = bytes.len() as u64;
        if self.buffer.size() < needed {
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(self.label),
                size: needed.next_power_of_two(),
                usage: self.usage,
                mapped_at_creation: false,
            });
        }
        queue.write_buffer(&self.buffer, 0, bytes);
    }
}

/// One draw of the frame's meshes.
struct Draw {
    pipeline: usize,
    material: u32,
    mesh: u32,
    /// A skinned draw's palette, its index in the frame's joint buffer.
    palette: Option<u32>,
    /// First and count of its model matrices in the instance buffer.
    first: u32,
    count: u32,
}

/// The lite renderer's pipelines and per-frame buffers.
pub struct LiteRenderer {
    frame_buffer: wgpu::Buffer,
    frame_group: wgpu::BindGroup,
    /// Index: `skinned * 8 + blend * 4 + double_sided * 2 + mirrored`.
    mesh_pipelines: Vec<wgpu::RenderPipeline>,
    skin_layout: wgpu::BindGroupLayout,
    /// The frame's joint palettes ([`PALETTE_BYTES`] each) and their bind group, remade when the
    /// buffer grows.
    palettes: Growing,
    palette_group: Option<(u64, wgpu::BindGroup)>,
    /// Additive, then alpha-blended.
    particle_pipelines: [wgpu::RenderPipeline; 2],
    overlay_pipeline: wgpu::RenderPipeline,
    /// An overlay image's texture and sampler.
    overlay_image_layout: wgpu::BindGroupLayout,
    depth: Option<((u32, u32), wgpu::TextureView)>,
    instances: Growing,
    particles: Growing,
    overlay: Growing,
    /// The target is not sRGB: the shaders encode.
    encode_srgb: bool,
    format: wgpu::TextureFormat,
    /// The overlay over the upscaled scene, in a pass without depth.
    overlay_flat: wgpu::RenderPipeline,
    blit_pipeline: wgpu::RenderPipeline,
    blit_layout: wgpu::BindGroupLayout,
    blit_sampler: wgpu::Sampler,
    /// The scaled scene target at its size, and its bind group for the upscale.
    scaled: Option<((u32, u32), wgpu::TextureView, wgpu::BindGroup)>,
}

/// The sizes of a lite frame, in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameSizes {
    /// The target's own size (in the browser the canvas's backing store).
    pub target: (u32, u32),
    /// The size the scene's camera and the overlay were laid out for (the window's).
    pub layout: (u32, u32),
    /// The size the 3D scene is drawn at, at most the target's.
    pub scene: (u32, u32),
}

/// Dynamic resolution: the render scale that keeps frames within the display's refresh, from
/// the time between frames. It drops quickly when frames run long and probes back up slowly, so
/// WebGL 2 drawn in software (SwiftShader, no GPU) keeps close to 60 frames a second while a GPU
/// stays at full resolution.
#[derive(Clone, Debug)]
pub struct RenderScale {
    pub scale: f32,
    /// Smoothed time between frames, in milliseconds.
    average: f32,
    /// Frames since the scale last changed.
    settled: u32,
    /// The scale that last ran too slow, not probed again for a while.
    ceiling: f32,
    /// Frames seen: the first [`WARM_UP_FRAMES`] (uploads, shader compiles) are not judged.
    seen: u32,
}

impl Default for RenderScale {
    fn default() -> Self {
        Self {
            scale: 1.0,
            average: TARGET_MS,
            settled: 0,
            ceiling: 1.0,
            seen: 0,
        }
    }
}

/// A 60 Hz frame, with a little slack for timer jitter.
const TARGET_MS: f32 = 17.5;
const MIN_SCALE: f32 = 0.3;
const WARM_UP_FRAMES: u32 = 90;

impl RenderScale {
    /// Takes the time since the last frame (`ms`) and returns the scale for the next one.
    pub fn update(&mut self, ms: f32) -> f32 {
        // A hidden tab or a load stall says nothing about the drawing.
        self.seen = self.seen.saturating_add(1);
        if !(0.0..250.0).contains(&ms) || self.seen <= WARM_UP_FRAMES {
            return self.scale;
        }
        self.average += (ms - self.average) * 0.1;
        self.settled = self.settled.saturating_add(1);
        if self.settled >= 20 && self.average > TARGET_MS + 2.5 {
            // Pixels cost in proportion to area: aim the area at the frame budget.
            let area = (TARGET_MS / self.average).clamp(0.5, 0.95);
            self.ceiling = self.scale;
            self.scale = (self.scale * area.sqrt()).max(MIN_SCALE);
            self.settled = 0;
            self.average = TARGET_MS;
        } else if self.scale < 1.0 && self.settled >= 180 && self.average <= TARGET_MS {
            // Probe up, but stay under the scale that last ran too slow for longer.
            let next = (self.scale + 0.05).min(1.0);
            if next < self.ceiling - 0.01 || self.settled >= 600 {
                self.scale = next;
                self.settled = 0;
            }
        }
        self.scale
    }
}

impl LiteRenderer {
    #[must_use]
    pub fn new(device: &wgpu::Device, world: &LiteWorld, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("lite.wgsl"));
        let frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lite frame"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let frame_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lite frame"),
            size: std::mem::size_of::<FrameUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lite frame"),
            layout: &frame_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: frame_buffer.as_entire_binding(),
            }],
        });
        let mesh_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lite mesh"),
            bind_group_layouts: &[Some(&frame_layout), Some(&world.material_layout)],
            immediate_size: 0,
        });
        let skin_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lite skin"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: wgpu::BufferSize::new(PALETTE_BYTES),
                },
                count: None,
            }],
        });
        let skinned_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lite skinned mesh"),
            bind_group_layouts: &[
                Some(&frame_layout),
                Some(&world.material_layout),
                Some(&skin_layout),
            ],
            immediate_size: 0,
        });
        let frame_only = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lite frame only"),
            bind_group_layouts: &[Some(&frame_layout)],
            immediate_size: 0,
        });
        let alpha = Some(wgpu::BlendState::ALPHA_BLENDING);
        let additive = Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        });
        let depth = |write: bool, compare: wgpu::CompareFunction| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(write),
            depth_compare: Some(compare),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        };
        let mesh_attributes = wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Float32x4
        ];
        let model_attributes = wgpu::vertex_attr_array![
            4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4
        ];
        let skin_attributes = wgpu::vertex_attr_array![8 => Uint16x4, 9 => Float32x4];
        let mesh_pipelines = (0..16)
            .map(|index| {
                let skinned = index & 8 != 0;
                let blend = index & 4 != 0;
                let double_sided = index & 2 != 0;
                let mirrored = index & 1 != 0;
                let buffers = [
                    Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &mesh_attributes,
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: 64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &model_attributes,
                    }),
                    Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<SkinVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &skin_attributes,
                    }),
                ];
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("lite mesh"),
                    layout: Some(if skinned {
                        &skinned_layout
                    } else {
                        &mesh_layout
                    }),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some(if skinned {
                            "mesh_skinned_vs"
                        } else {
                            "mesh_vs"
                        }),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        buffers: if skinned { &buffers } else { &buffers[..2] },
                    },
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleList,
                        front_face: if mirrored {
                            wgpu::FrontFace::Cw
                        } else {
                            wgpu::FrontFace::Ccw
                        },
                        cull_mode: (!double_sided).then_some(wgpu::Face::Back),
                        ..Default::default()
                    },
                    depth_stencil: Some(depth(!blend, wgpu::CompareFunction::GreaterEqual)),
                    multisample: wgpu::MultisampleState::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("mesh_fs"),
                        compilation_options: wgpu::PipelineCompilationOptions::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: if blend { alpha } else { None },
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                })
            })
            .collect();
        let particle_attributes = wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32x4, 2 => Float32x4, 3 => Float32x3
        ];
        let particle_pipeline = |blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("lite particles"),
                layout: Some(&frame_only),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("particle_vs"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<ParticleInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &particle_attributes,
                    })],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(depth(false, wgpu::CompareFunction::GreaterEqual)),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("particle_fs"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        // The overlay: its own shader (`lite_overlay.wgsl`), its image at group 0.
        let overlay_shader = device.create_shader_module(wgpu::include_wgsl!("lite_overlay.wgsl"));
        let overlay_image_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("lite overlay image"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let overlay_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lite overlay"),
            bind_group_layouts: &[Some(&overlay_image_layout)],
            immediate_size: 0,
        });
        let overlay_attributes = wgpu::vertex_attr_array![
            0 => Float32x2, 1 => Float32x4, 2 => Float32x2, 3 => Float32x4, 4 => Float32x2
        ];
        let encode_constant = [("ENCODE_SRGB", if format.is_srgb() { 0.0 } else { 1.0 })];
        let overlay = |depth_stencil: Option<wgpu::DepthStencilState>| {
            let options = wgpu::PipelineCompilationOptions {
                constants: &encode_constant,
                ..Default::default()
            };
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("lite overlay"),
                layout: Some(&overlay_layout),
                vertex: wgpu::VertexState {
                    module: &overlay_shader,
                    entry_point: Some("overlay_vs"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<OverlayVertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &overlay_attributes,
                    })],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &overlay_shader,
                    entry_point: Some("overlay_fs"),
                    compilation_options: options,
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: alpha,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let overlay_pipeline = overlay(Some(depth(false, wgpu::CompareFunction::Always)));
        let overlay_flat = overlay(None);
        let blit_shader = device.create_shader_module(wgpu::include_wgsl!("lite_blit.wgsl"));
        let blit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lite upscale"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let blit_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lite upscale"),
            bind_group_layouts: &[Some(&blit_layout)],
            immediate_size: 0,
        });
        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("lite upscale"),
            layout: Some(&blit_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &blit_shader,
                entry_point: Some("blit_vs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &blit_shader,
                entry_point: Some("blit_fs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let blit_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("lite upscale"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            overlay_flat,
            format,
            blit_pipeline,
            blit_layout,
            blit_sampler,
            scaled: None,
            frame_buffer,
            frame_group,
            mesh_pipelines,
            skin_layout,
            palettes: Growing::new(device, "lite palettes", wgpu::BufferUsages::UNIFORM),
            palette_group: None,
            particle_pipelines: [particle_pipeline(additive), particle_pipeline(alpha)],
            overlay_pipeline,
            overlay_image_layout,
            depth: None,
            instances: Growing::new(device, "lite instances", wgpu::BufferUsages::VERTEX),
            particles: Growing::new(device, "lite particles", wgpu::BufferUsages::VERTEX),
            overlay: Growing::new(device, "lite overlay", wgpu::BufferUsages::VERTEX),
            encode_srgb: !format.is_srgb(),
        }
    }

    fn depth_view(&mut self, device: &wgpu::Device, size: (u32, u32)) -> wgpu::TextureView {
        if let Some((at, view)) = &self.depth
            && *at == size
        {
            return view.clone();
        }
        let view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("lite depth"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.depth = Some((size, view.clone()));
        view
    }

    /// Draws `scene` and `world` into `target` with `encoder`: the 3D scene at
    /// [`FrameSizes::scene`] (dynamic resolution, [`RenderScale`]; smaller than the target, it
    /// is upscaled), the overlay at the target's size.
    #[allow(clippy::too_many_lines)]
    pub fn encode(
        &mut self,
        world: &mut LiteWorld,
        scene: &Scene,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        sizes: FrameSizes,
        overlay_quads: &[super::OverlayQuad],
    ) {
        let FrameSizes {
            target: target_size,
            layout: size,
            scene: scene_size,
        } = sizes;
        let device = world.device.clone();
        let queue = world.queue.clone();
        #[allow(clippy::cast_precision_loss)]
        let aspect = size.0.max(1) as f32 / size.1.max(1) as f32;
        let camera = &scene.camera;
        let view = look_at(camera.eye, camera.target);
        let clip_from_world = perspective(camera.fov_y, aspect, camera.near) * view;
        let planes = frustum(clip_from_world);
        let world_from_view = view.inverse();

        // The frame: the sun, the ambient light and the nearest point lights in view.
        let mut lights: Vec<_> = scene
            .point_lights
            .iter()
            .filter(|light| visible(&planes, light.position, light.range))
            .collect();
        lights.sort_by(|a, b| {
            a.position
                .distance_squared(camera.eye)
                .total_cmp(&b.position.distance_squared(camera.eye))
        });
        lights.truncate(MAX_LIGHTS);
        let mut frame = FrameUniform {
            clip_from_world: clip_from_world.to_cols_array_2d(),
            eye: camera.eye.extend(1.0).to_array(),
            sun_direction: [0.0; 4],
            sun_color: [0.0; 4],
            ambient: (scene.ambient.color * scene.ambient.brightness)
                .extend(0.0)
                .to_array(),
            #[allow(clippy::cast_precision_loss)]
            params: [
                camera.exposure(),
                lights.len() as f32,
                f32::from(u8::from(self.encode_srgb)),
                0.0,
            ],
            right: world_from_view.x_axis.to_array(),
            up: world_from_view.y_axis.to_array(),
            fog: [0.0; 4],
            fog_range: [0.0, 1.0, 0.0, 0.0],
            lights: [[0.0; 4]; 2 * MAX_LIGHTS],
        };
        if let Some(fog) = &scene.fog
            && let FogFalloff::Linear { start, end } = fog.falloff
        {
            frame.fog = fog.color.to_array();
            frame.fog_range = [start, end.max(start + 1e-3), 0.0, 0.0];
        }
        if let Some(sun) = &scene.sun {
            frame.sun_direction = (-sun.direction.normalize_or(Vec3::NEG_Y))
                .extend(1.0)
                .to_array();
            frame.sun_color = (sun.color * sun.illuminance).extend(0.0).to_array();
        }
        for (i, light) in lights.iter().enumerate() {
            frame.lights[2 * i] = light
                .position
                .extend(1.0 / (light.range * light.range).max(1e-6))
                .to_array();
            frame.lights[2 * i + 1] = (light.color
                * (light.intensity / (4.0 * std::f32::consts::PI)))
                .extend(0.0)
                .to_array();
        }
        queue.write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&frame));

        // The meshes: opaque ones batched by pipeline, material and mesh, blended ones back to
        // front, skinned ones each with its palette.
        struct Item {
            pipeline: usize,
            material: u32,
            mesh: u32,
            palette: Option<u32>,
            model: Mat4,
            distance: f32,
        }
        let mut opaque = Vec::new();
        let mut blended = Vec::new();
        // The palettes of the skins drawn this frame, each once.
        let mut palettes: Vec<[f32; 16]> = Vec::new();
        let mut palette_of: HashMap<u32, u32> = HashMap::new();
        for (_, instance) in world.instances.iter() {
            let (Some(mesh), Some(material)) = (
                world.meshes.get(instance.mesh.0),
                world.materials.get(instance.material.0),
            ) else {
                continue;
            };
            let model = instance.world_from_local;
            let scale = model
                .x_axis
                .truncate()
                .length()
                .max(model.y_axis.truncate().length())
                .max(model.z_axis.truncate().length());
            let center = model.transform_point3(mesh.center);
            let skin = instance
                .skin
                .filter(|_| mesh.skin.is_some())
                .and_then(|skin| Some((skin.0, world.skins.get(skin.0)?)));
            // A posed mesh can leave its rest bounds: skinned ones are not culled.
            if skin.is_none() && !visible(&planes, center, mesh.radius * scale) {
                continue;
            }
            let palette = skin.map(|(id, joints)| {
                *palette_of.entry(id).or_insert_with(|| {
                    let index = u32::try_from(palettes.len() / MAX_JOINTS).unwrap_or(u32::MAX);
                    palettes.extend(
                        joints
                            .iter()
                            .map(Mat4::to_cols_array)
                            .chain(std::iter::repeat(Mat4::IDENTITY.to_cols_array()))
                            .take(MAX_JOINTS),
                    );
                    index
                })
            });
            let blend = matches!(material.desc.alpha_mode, AlphaMode::Blend);
            let pipeline = usize::from(palette.is_some()) * 8
                + usize::from(blend) * 4
                + usize::from(material.desc.double_sided) * 2
                + usize::from(model.determinant() < 0.0);
            let item = Item {
                pipeline,
                material: instance.material.0,
                mesh: instance.mesh.0,
                palette,
                model,
                distance: center.distance_squared(camera.eye),
            };
            if blend {
                blended.push(item);
            } else {
                opaque.push(item);
            }
        }
        opaque.sort_by_key(|item| (item.pipeline, item.material, item.mesh, item.palette));
        blended.sort_by(|a, b| b.distance.total_cmp(&a.distance));
        let mut models: Vec<[f32; 16]> = Vec::with_capacity(opaque.len() + blended.len());
        let mut draws: Vec<Draw> = Vec::new();
        for item in opaque.iter().chain(&blended) {
            let first = u32::try_from(models.len()).unwrap_or(u32::MAX);
            models.push(item.model.to_cols_array());
            match draws.last_mut() {
                Some(last)
                    if !matches!(
                        world
                            .materials
                            .get(item.material)
                            .map(|m| m.desc.alpha_mode),
                        Some(AlphaMode::Blend)
                    ) && last.pipeline == item.pipeline
                        && last.material == item.material
                        && last.mesh == item.mesh
                        && last.palette == item.palette
                        && last.first + last.count == first =>
                {
                    last.count += 1;
                }
                _ => draws.push(Draw {
                    pipeline: item.pipeline,
                    material: item.material,
                    mesh: item.mesh,
                    palette: item.palette,
                    first,
                    count: 1,
                }),
            }
        }
        self.instances
            .write(&device, &queue, bytemuck::cast_slice(&models));
        if !palettes.is_empty() {
            self.palettes
                .write(&device, &queue, bytemuck::cast_slice(&palettes));
            let size = self.palettes.buffer.size();
            if self
                .palette_group
                .as_ref()
                .is_none_or(|(at, _)| *at != size)
            {
                let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("lite palettes"),
                    layout: &self.skin_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.palettes.buffer,
                            offset: 0,
                            size: wgpu::BufferSize::new(PALETTE_BYTES),
                        }),
                    }],
                });
                self.palette_group = Some((size, group));
            }
        }

        // The particles, additive first.
        let mut particles: [Vec<ParticleInstance>; 2] = [Vec::new(), Vec::new()];
        for (_, emitter) in world.emitters.iter() {
            let list = match emitter.desc.blend {
                ParticleBlend::Add => &mut particles[0],
                ParticleBlend::Alpha => &mut particles[1],
            };
            let along = f32::from(u8::from(emitter.desc.orient == Orient::AlongVelocity));
            for p in &emitter.particles {
                let (size, color) = emitter.look(p);
                list.push(ParticleInstance {
                    position: p.position.to_array(),
                    size: [size.x, size.y, along, emitter.desc.roundness],
                    color: color.to_array(),
                    velocity: p.velocity.to_array(),
                });
            }
        }
        let additive_count = u32::try_from(particles[0].len()).unwrap_or(u32::MAX);
        let alpha_count = u32::try_from(particles[1].len()).unwrap_or(u32::MAX);
        let mut all_particles = std::mem::take(&mut particles[0]);
        all_particles.append(&mut particles[1]);
        self.particles
            .write(&device, &queue, bytemuck::cast_slice(&all_particles));

        // The overlay, in clip space, in runs of quads sharing an image; its signed distances in
        // target pixels (the overlay is laid out at the layout size).
        #[allow(clippy::cast_precision_loss)]
        let (width, height) = (size.0.max(1) as f32, size.1.max(1) as f32);
        #[allow(clippy::cast_precision_loss)]
        let pixels = target_size.1.max(1) as f32 / height;
        let mut overlay = Vec::with_capacity(overlay_quads.len() * 6);
        let mut overlay_batches: Vec<(Option<u32>, std::ops::Range<u32>)> = Vec::new();
        for quad in overlay_quads {
            let texture = quad.image.map(|image| image.texture.0);
            let start = u32::try_from(overlay.len()).unwrap_or(u32::MAX);
            match overlay_batches.last_mut() {
                Some((batch, _)) if *batch == texture => {}
                _ => overlay_batches.push((texture, start..start)),
            }
            let clip = |p: Vec2| [p.x / width * 2.0 - 1.0, 1.0 - p.y / height * 2.0];
            let half = quad.size / 2.0;
            let radius = quad.corner_radius.clamp(0.0, half.x.min(half.y));
            let shape = [half.x * pixels, half.y * pixels, radius * pixels, 0.0];
            let (uv_min, uv_max) = quad
                .image
                .map_or((Vec2::ZERO, Vec2::ZERO), |i| (i.uv_min, i.uv_max));
            let color = quad.color.to_array();
            for (cx, cy) in [
                (0.0, 0.0),
                (1.0, 0.0),
                (1.0, 1.0),
                (0.0, 0.0),
                (1.0, 1.0),
                (0.0, 1.0),
            ] {
                let corner = Vec2::new(cx, cy);
                overlay.push(OverlayVertex {
                    position: clip(quad.position + quad.size * corner),
                    color,
                    local: ((corner - 0.5) * quad.size * pixels).to_array(),
                    shape,
                    uv: (uv_min + (uv_max - uv_min) * corner).to_array(),
                });
            }
            if let Some((_, vertices)) = overlay_batches.last_mut() {
                vertices.end = u32::try_from(overlay.len()).unwrap_or(u32::MAX);
            }
        }
        self.overlay
            .write(&device, &queue, bytemuck::cast_slice(&overlay));
        let overlay_draws: Vec<(wgpu::BindGroup, std::ops::Range<u32>)> = overlay_batches
            .into_iter()
            .map(|(texture, vertices)| {
                (
                    world.overlay_group(texture, &self.overlay_image_layout),
                    vertices,
                )
            })
            .collect();

        // Dynamic resolution: a scene smaller than the target goes into its own texture,
        // upscaled before the overlay.
        let scene_size = (
            scene_size.0.clamp(1, target_size.0.max(1)),
            scene_size.1.clamp(1, target_size.1.max(1)),
        );
        let scaled = (scene_size != target_size).then(|| self.scaled_target(&device, scene_size));
        let scene_target = scaled.as_ref().map_or(target, |(view, _)| view);
        let depth = self.depth_view(&device, scene_size);
        let clear = tonemap(scene.clear_color);
        let clear = if self.encode_srgb {
            clear.map(encode_srgb)
        } else {
            clear
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("lite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: scene_target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(clear.x),
                        g: f64::from(clear.y),
                        b: f64::from(clear.z),
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &self.frame_group, &[]);
        let mut bound_pipeline = usize::MAX;
        let mut bound_material = u32::MAX;
        for draw in &draws {
            let (Some(mesh), Some(material)) = (
                world.meshes.get(draw.mesh),
                world.materials.get(draw.material),
            ) else {
                continue;
            };
            if bound_pipeline != draw.pipeline {
                pass.set_pipeline(&self.mesh_pipelines[draw.pipeline]);
                bound_pipeline = draw.pipeline;
                bound_material = u32::MAX;
            }
            if bound_material != draw.material {
                pass.set_bind_group(1, &material.bind_group, &[]);
                bound_material = draw.material;
            }
            if let (Some(palette), Some(skin), Some((_, group))) =
                (draw.palette, &mesh.skin, &self.palette_group)
            {
                let offset = u64::from(palette) * PALETTE_BYTES;
                pass.set_bind_group(2, group, &[u32::try_from(offset).unwrap_or(u32::MAX)]);
                pass.set_vertex_buffer(2, skin.slice(..));
            }
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            let offset = u64::from(draw.first) * 64;
            pass.set_vertex_buffer(1, self.instances.buffer.slice(offset..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, 0..draw.count);
        }
        let stride = std::mem::size_of::<ParticleInstance>() as u64;
        for (index, (first, count)) in [(0, additive_count), (additive_count, alpha_count)]
            .into_iter()
            .enumerate()
        {
            if count == 0 {
                continue;
            }
            pass.set_pipeline(&self.particle_pipelines[index]);
            pass.set_vertex_buffer(0, self.particles.buffer.slice(u64::from(first) * stride..));
            pass.draw(0..6, 0..count);
        }
        let Some((_, blit_group)) = scaled else {
            if !overlay_draws.is_empty() {
                pass.set_pipeline(&self.overlay_pipeline);
                pass.set_vertex_buffer(0, self.overlay.buffer.slice(..));
                for (group, vertices) in &overlay_draws {
                    pass.set_bind_group(0, group, &[]);
                    pass.draw(vertices.clone(), 0..1);
                }
            }
            return;
        };
        drop(pass);
        // The scaled scene stretched over the target, then the overlay at full size.
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("lite upscale"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // The upscale covers every pixel.
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.blit_pipeline);
        pass.set_bind_group(0, &blit_group, &[]);
        pass.draw(0..3, 0..1);
        if !overlay_draws.is_empty() {
            pass.set_pipeline(&self.overlay_flat);
            pass.set_vertex_buffer(0, self.overlay.buffer.slice(..));
            for (group, vertices) in &overlay_draws {
                pass.set_bind_group(0, group, &[]);
                pass.draw(vertices.clone(), 0..1);
            }
        }
    }

    /// The scaled scene target of `size` and its upscale bind group.
    fn scaled_target(
        &mut self,
        device: &wgpu::Device,
        size: (u32, u32),
    ) -> (wgpu::TextureView, wgpu::BindGroup) {
        if let Some((at, view, group)) = &self.scaled
            && *at == size
        {
            return (view.clone(), group.clone());
        }
        let view = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("lite scaled scene"),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: self.format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lite upscale"),
            layout: &self.blit_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.blit_sampler),
                },
            ],
        });
        self.scaled = Some((size, view.clone(), group.clone()));
        (view, group)
    }
}

/// The shader's tone mapping (`lite.wgsl` `tonemap`), for the clear colour.
fn tonemap(color: Vec3) -> Vec3 {
    let c = color.max(Vec3::ZERO);
    let per_channel = c / (c + 1.0);
    let l = c.dot(Vec3::new(0.2126, 0.7152, 0.0722));
    let by_luminance = c / (l + 1.0);
    let t = (l / (l + 1.0)).clamp(0.0, 1.0);
    by_luminance.lerp(per_channel, t * t).min(Vec3::ONE)
}

fn encode_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use render_engine::particles::Spawner;

    fn emitter(capacity: u32) -> CpuEmitter {
        CpuEmitter {
            desc: Emitter {
                capacity,
                spawner: Spawner::once(0.0),
                seed: 7,
                init: vec![
                    InitModifier::VelocityCone {
                        direction: Vec3::Z,
                        spread: 0.3,
                        speed: Value::Const(2.0),
                    },
                    InitModifier::Lifetime(Value::Const(0.5)),
                ],
                accel: Vec3::ZERO,
                size_over_life: None,
                color_over_life: None,
                orient: Orient::CameraPlane,
                texture: None,
                roundness: 1.0,
                depth_fade: 0.0,
                lighting: None,
                fog: false,
                blend: ParticleBlend::Add,
                active: false,
            },
            particles: Vec::new(),
            bursts: Vec::new(),
            rng: 1,
        }
    }

    #[test]
    fn bursts_spawn_within_capacity_move_along_the_cone_and_die_of_age() {
        let mut e = emitter(10);
        e.bursts.push((25, Mat4::from_translation(Vec3::X)));
        e.step(0.1);
        assert_eq!(e.particles.len(), 10);
        for p in &e.particles {
            assert!((p.velocity.length() - 2.0).abs() < 1e-4);
            assert!(p.velocity.normalize().dot(Vec3::Z) >= 0.3f32.cos() - 1e-4);
            assert!((p.position - (Vec3::X + p.velocity * 0.1)).length() < 1e-4);
        }
        for _ in 0..4 {
            e.step(0.1);
        }
        assert!(e.particles.is_empty());
    }

    #[test]
    fn the_render_scale_drops_on_slow_frames_and_recovers_on_fast_ones() {
        let mut scale = RenderScale::default();
        for _ in 0..200 {
            scale.update(16.7);
        }
        assert!(
            (scale.scale - 1.0).abs() < f32::EPSILON,
            "a GPU at 60 Hz keeps full size"
        );
        for _ in 0..400 {
            scale.update(70.0);
        }
        assert!(
            scale.scale <= 0.5,
            "70 ms frames cut the scale: {}",
            scale.scale
        );
        assert!(scale.scale >= MIN_SCALE);
        let low = scale.scale;
        for _ in 0..2_000 {
            scale.update(16.7);
        }
        assert!(scale.scale > low, "fast frames win the resolution back");
        // Stalls (a hidden tab) are ignored.
        let before = scale.scale;
        scale.update(5_000.0);
        assert!((scale.scale - before).abs() < f32::EPSILON);
    }

    #[test]
    fn the_frustum_keeps_what_is_ahead_and_drops_what_is_behind() {
        let clip_from_world = perspective(1.0, 16.0 / 9.0, 0.05)
            * look_at(Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 1.0, -1.0));
        let planes = frustum(clip_from_world);
        assert!(visible(&planes, Vec3::new(0.0, 1.0, -10.0), 0.5));
        assert!(!visible(&planes, Vec3::new(0.0, 1.0, 10.0), 0.5));
        assert!(!visible(&planes, Vec3::new(100.0, 1.0, -10.0), 0.5));
        // A sphere reaching into view from the side is kept.
        assert!(visible(&planes, Vec3::new(12.0, 1.0, -10.0), 5.0));
    }
}
