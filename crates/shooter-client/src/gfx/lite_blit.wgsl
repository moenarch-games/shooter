// The lite renderer's upscale (`gfx::lite`, dynamic resolution): the scene drawn at a fraction of
// the target's size, stretched over it with bilinear filtering before the overlay is drawn at full
// size. The scene texture has the target's format, so its colours pass through unchanged.

@group(0) @binding(0) var scene_texture: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;

struct BlitOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn blit_vs(@builtin(vertex_index) index: u32) -> BlitOut {
    // One triangle covering the screen.
    let uv = vec2(f32(index >> 1u), f32(index & 1u)) * 2.0;
    return BlitOut(vec4(uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 0.0, 1.0), uv);
}

@fragment
fn blit_fs(in: BlitOut) -> @location(0) vec4<f32> {
    return textureSample(scene_texture, scene_sampler, in.uv);
}
