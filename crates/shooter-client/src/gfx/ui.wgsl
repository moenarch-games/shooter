// The drawn UI (`gfx::ui`): instanced quads over the frame, each a rounded rectangle (its edge
// anti-aliased from its signed distance) or a glyph (coverage from the atlas). Uniforms only and
// no storage buffers, so it runs on WebGL 2 as well.

@group(0) @binding(0) var atlas: texture_2d<f32>;
@group(0) @binding(1) var atlas_sampler: sampler;

struct UiIn {
    @builtin(vertex_index) corner: u32,
    // Clip-space left, top, right, bottom.
    @location(0) rect: vec4<f32>,
    // Atlas left, top, right, bottom (a glyph's).
    @location(1) uv: vec4<f32>,
    // Already encoded for the target.
    @location(2) color: vec4<f32>,
    // Width and height in pixels, corner radius in pixels, 1 for a glyph.
    @location(3) shape: vec4<f32>,
}

struct UiOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    // Pixels from the rectangle's centre.
    @location(2) local: vec2<f32>,
    @location(3) shape: vec4<f32>,
}

@vertex
fn ui_vs(in: UiIn) -> UiOut {
    // A strip: top left, bottom left, top right, bottom right.
    let t = vec2(f32(in.corner >> 1u), f32(in.corner & 1u));
    let clip = vec2(mix(in.rect.x, in.rect.z, t.x), mix(in.rect.y, in.rect.w, t.y));
    let uv = vec2(mix(in.uv.x, in.uv.z, t.x), mix(in.uv.y, in.uv.w, t.y));
    let local = (t - 0.5) * in.shape.xy;
    return UiOut(vec4(clip, 0.0, 1.0), uv, in.color, local, in.shape);
}

@fragment
fn ui_fs(in: UiOut) -> @location(0) vec4<f32> {
    var coverage: f32;
    if (in.shape.w > 0.5) {
        coverage = textureSampleLevel(atlas, atlas_sampler, in.uv, 0.0).r;
    } else {
        let half = in.shape.xy * 0.5;
        let radius = min(in.shape.z, min(half.x, half.y));
        let q = abs(in.local) - half + radius;
        let distance = length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - radius;
        coverage = clamp(0.5 - distance, 0.0, 1.0);
    }
    return vec4(in.color.rgb, in.color.a * coverage);
}
