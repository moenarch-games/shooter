// The lite renderer (`gfx::lite`): forward shading that WebGL 2 runs (uniform buffers only, no
// storage buffers or compute). The BRDF is render-engine's `lit.wgsl` (Bevy's) cut down: Lambert
// diffuse, GGX specular with Smith visibility and Schlick fresnel, the sun, point lights and the
// uniform ambient through Karis's split-sum approximation; no shadows, normal maps or
// environment maps. Every fragment is exposed (lit and emissive) and tone mapped on its own, as
// there is no HDR target.

const PI: f32 = 3.141592653589793;
const MAX_LIGHTS: u32 = 16u;

struct Frame {
    clip_from_world: mat4x4<f32>,
    eye: vec4<f32>,
    // xyz: direction to the sun, w: 1 when there is a sun.
    sun_direction: vec4<f32>,
    // The sun's colour times its illuminance (lux).
    sun_color: vec4<f32>,
    // The ambient colour times its brightness (cd/m²).
    ambient: vec4<f32>,
    // x: exposure, y: point lights, z: 1 to encode sRGB here (the target is not sRGB).
    params: vec4<f32>,
    // The camera's right and up axes (particles).
    right: vec4<f32>,
    up: vec4<f32>,
    // Per light: position and 1 / range², then colour in candela and unused.
    lights: array<vec4<f32>, 32>,
}

@group(0) @binding(0) var<uniform> frame: Frame;

struct Material {
    base_color: vec4<f32>,
    // rgb: emissive radiance (cd/m²).
    emissive: vec4<f32>,
    // metallic, perceptual roughness, reflectance, alpha cutoff (mask)
    surface: vec4<f32>,
    // x: 1 to cut out below the cutoff, y: 1 unlit, z: 1 double sided.
    mode: vec4<f32>,
}

@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var base_texture: texture_2d<f32>;
@group(1) @binding(2) var emissive_texture: texture_2d<f32>;
@group(1) @binding(3) var metallic_roughness_texture: texture_2d<f32>;
@group(1) @binding(4) var material_sampler: sampler;

fn luminance(v: vec3<f32>) -> f32 {
    return dot(v, vec3(0.2126, 0.7152, 0.0722));
}

// A cheap stand-in for render-engine's Tony McMapface LUT: Reinhard on each channel with the
// highlights pulled towards their luminance, so bright colours wash out to white rather than
// clipping to saturated primaries.
fn tonemap(color: vec3<f32>) -> vec3<f32> {
    let c = max(color, vec3(0.0));
    let per_channel = c / (c + 1.0);
    let l = luminance(c);
    let by_luminance = c / (l + 1.0);
    let t = saturate(l / (l + 1.0));
    return min(mix(by_luminance, per_channel, t * t), vec3(1.0));
}

fn encode(color: vec3<f32>) -> vec3<f32> {
    if (frame.params.z < 0.5) {
        return color;
    }
    let low = color * 12.92;
    let high = 1.055 * pow(color, vec3(1.0 / 2.4)) - 0.055;
    return select(high, low, color <= vec3(0.0031308));
}

fn display(color: vec3<f32>) -> vec3<f32> {
    return encode(tonemap(color));
}

// ---------------------------------------------------------------- meshes

struct MeshIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(4) model_0: vec4<f32>,
    @location(5) model_1: vec4<f32>,
    @location(6) model_2: vec4<f32>,
    @location(7) model_3: vec4<f32>,
}

struct MeshOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
}

@vertex
fn mesh_vs(in: MeshIn) -> MeshOut {
    let model = mat4x4<f32>(in.model_0, in.model_1, in.model_2, in.model_3);
    let world = model * vec4(in.position, 1.0);
    var out: MeshOut;
    out.clip = frame.clip_from_world * world;
    out.world = world.xyz;
    // Uniformly scaled models only need the rotation; a mirrored one keeps its outward normals.
    out.normal = (model * vec4(in.normal, 0.0)).xyz;
    out.uv = in.uv;
    out.color = in.color;
    return out;
}

fn perceptual_roughness_to_roughness(p: f32) -> f32 {
    let c = clamp(p, 0.089, 1.0);
    return c * c;
}

fn D_GGX(roughness: f32, NdotH: f32) -> f32 {
    let one_minus = 1.0 - NdotH * NdotH;
    let a = NdotH * roughness;
    let k = roughness / (one_minus + a * a);
    return k * k * (1.0 / PI);
}

fn V_SmithGGXCorrelated(roughness: f32, NdotV: f32, NdotL: f32) -> f32 {
    let a2 = roughness * roughness;
    let lambda_v = NdotL * sqrt((NdotV - a2 * NdotV) * NdotV + a2);
    let lambda_l = NdotV * sqrt((NdotL - a2 * NdotL) * NdotL + a2);
    return 0.5 / (lambda_v + lambda_l);
}

fn fresnel(f0: vec3<f32>, LdotH: f32) -> vec3<f32> {
    let f90 = saturate(dot(f0, vec3(50.0 * 0.33)));
    return f0 + (f90 - f0) * pow(1.0 - LdotH, 5.0);
}

fn F_AB(perceptual_roughness: f32, NdotV: f32) -> vec2<f32> {
    let c0 = vec4(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4(1.0, 0.0425, 1.04, -0.04);
    let r = perceptual_roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * NdotV)) * r.x + r.y;
    return max(vec2(-1.04, 1.04) * a004 + r.zw, vec2(0.00005));
}

// Lambert diffuse and GGX specular from direction `L`, times N·L.
fn brdf(N: vec3<f32>, V: vec3<f32>, L: vec3<f32>, NdotV: f32, roughness: f32,
        diffuse_color: vec3<f32>, F0: vec3<f32>) -> vec3<f32> {
    let NdotL = saturate(dot(N, L));
    let H = normalize(L + V);
    let NdotH = saturate(dot(N, H));
    let LdotH = saturate(dot(L, H));
    let specular = D_GGX(roughness, NdotH) * V_SmithGGXCorrelated(roughness, NdotV, NdotL)
        * fresnel(F0, LdotH);
    return (diffuse_color * (1.0 / PI) + specular) * NdotL;
}

fn shade(in: MeshOut, front: bool) -> vec4<f32> {
    // Sampled in uniform control flow, every texture every time (a missing one is white).
    let base_sample = textureSample(base_texture, material_sampler, in.uv);
    let emissive_sample = textureSample(emissive_texture, material_sampler, in.uv);
    let mr_sample = textureSample(metallic_roughness_texture, material_sampler, in.uv);
    let base = material.base_color * in.color * base_sample;
    if (material.mode.x > 0.5 && base.a < material.surface.w) {
        discard;
    }
    let exposure = frame.params.x;
    if (material.mode.y > 0.5) {
        return vec4(display(base.rgb), base.a);
    }
    var N = normalize(in.normal);
    if (!front) {
        N = -N;
    }
    let metallic = material.surface.x * mr_sample.b;
    let perceptual_roughness = material.surface.y * mr_sample.g;
    let reflectance = material.surface.z;
    let roughness = perceptual_roughness_to_roughness(perceptual_roughness);
    let V = normalize(frame.eye.xyz - in.world);
    let NdotV = max(dot(N, V), 0.0001);
    let diffuse_color = base.rgb * (1.0 - metallic);
    let F0 = mix(vec3(0.16 * reflectance * reflectance), base.rgb, metallic);

    var direct = vec3(0.0);
    let count = u32(frame.params.y);
    for (var i = 0u; i < MAX_LIGHTS; i++) {
        if (i >= count) {
            break;
        }
        let position = frame.lights[2u * i];
        let color = frame.lights[2u * i + 1u];
        let to_light = position.xyz - in.world;
        let distance_square = dot(to_light, to_light);
        let factor = distance_square * position.w;
        let window = saturate(1.0 - factor * factor);
        let attenuation = window * window / max(distance_square, 0.0001);
        if (attenuation > 0.0) {
            let L = to_light * inverseSqrt(distance_square);
            direct += brdf(N, V, L, NdotV, roughness, diffuse_color, F0) * color.rgb * attenuation;
        }
    }
    if (frame.sun_direction.w > 0.5) {
        direct += brdf(N, V, frame.sun_direction.xyz, NdotV, roughness, diffuse_color, F0)
            * frame.sun_color.rgb;
    }
    let f_ab = F_AB(perceptual_roughness, NdotV);
    let d_ab = F_AB(1.0, NdotV);
    let diffuse_ambient = diffuse_color * d_ab.x + d_ab.y;
    let specular_ambient = F0 * f_ab.x + f_ab.y;
    let specular_occlusion = saturate(dot(F0, vec3(50.0 * 0.33)));
    let indirect = (diffuse_ambient + specular_ambient * specular_occlusion) * frame.ambient.rgb;
    let emissive = material.emissive.rgb * emissive_sample.rgb * base.a;
    let radiance = (direct + indirect + emissive) * exposure;
    return vec4(display(radiance), base.a);
}

@fragment
fn mesh_fs(in: MeshOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return shade(in, front);
}

// ---------------------------------------------------------------- particles

struct ParticleIn {
    @builtin(vertex_index) vertex: u32,
    @location(0) position: vec3<f32>,
    // x, y: size; z: 1 to orient along the velocity; w: roundness.
    @location(1) size: vec4<f32>,
    @location(2) color: vec4<f32>,
    @location(3) velocity: vec3<f32>,
}

struct ParticleOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) roundness: f32,
}

@vertex
fn particle_vs(in: ParticleIn) -> ParticleOut {
    // Two triangles over a unit quad centred on the particle.
    var corner = vec2(-0.5, -0.5);
    switch in.vertex {
        case 1u, 4u: { corner = vec2(0.5, 0.5); }
        case 2u: { corner = vec2(-0.5, 0.5); }
        case 5u: { corner = vec2(0.5, -0.5); }
        default: {}
    }
    var axis_x = frame.right.xyz;
    var axis_y = frame.up.xyz;
    if (in.size.z > 0.5 && dot(in.velocity, in.velocity) > 1e-12) {
        let dir = normalize(in.position - frame.eye.xyz);
        axis_x = normalize(in.velocity);
        axis_y = cross(dir, axis_x);
    }
    let offset = corner * in.size.xy;
    let world = in.position + axis_x * offset.x + axis_y * offset.y;
    var out: ParticleOut;
    out.clip = frame.clip_from_world * vec4(world, 1.0);
    out.color = in.color;
    out.uv = vec2(corner.x + 0.5, 0.5 - corner.y);
    out.roundness = in.size.w;
    return out;
}

@fragment
fn particle_fs(in: ParticleOut) -> @location(0) vec4<f32> {
    if (in.roundness > 0.0) {
        let n = 2.0 / in.roundness;
        if (pow(abs(1.0 - 2.0 * in.uv.x), n) + pow(abs(1.0 - 2.0 * in.uv.y), n) > 1.0) {
            discard;
        }
    }
    return vec4(display(in.color.rgb), saturate(in.color.a));
}

// ---------------------------------------------------------------- overlay

struct OverlayIn {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
}

struct OverlayOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn overlay_vs(in: OverlayIn) -> OverlayOut {
    return OverlayOut(vec4(in.position, 0.0, 1.0), in.color);
}

// Overlay colours are linear and neither exposed nor tone mapped.
@fragment
fn overlay_fs(in: OverlayOut) -> @location(0) vec4<f32> {
    return vec4(encode(in.color.rgb), in.color.a);
}
