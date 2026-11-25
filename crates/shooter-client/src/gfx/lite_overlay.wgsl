// The lite renderer's overlay (`gfx::lite`), as render-engine draws its `Scene::overlay`: quads
// over the frame, their colour linear with straight alpha (neither exposed nor tone mapped),
// tinting their image (white for none), their rounded corners cut by the signed distance and
// anti-aliased over a pixel (render-engine's `overlay.wgsl`, after Bevy UI). Where the target is
// not sRGB the colour is encoded here.

override ENCODE_SRGB: bool = false;

@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;

struct OverlayIn {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
    // Pixels from the quad's centre.
    @location(2) local: vec2<f32>,
    // Half width, half height, corner radius (pixels).
    @location(3) shape: vec4<f32>,
    @location(4) uv: vec2<f32>,
}

struct OverlayOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) local: vec2<f32>,
    @location(2) shape: vec4<f32>,
    @location(3) uv: vec2<f32>,
}

@vertex
fn overlay_vs(in: OverlayIn) -> OverlayOut {
    return OverlayOut(vec4(in.position, 0.0, 1.0), in.color, in.local, in.shape, in.uv);
}

fn rounded_box(point: vec2<f32>, half: vec2<f32>, radius: f32) -> f32 {
    let q = abs(point) - half + radius;
    return length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

fn encode(color: vec3<f32>) -> vec3<f32> {
    let low = color * 12.92;
    let high = 1.055 * pow(color, vec3(1.0 / 2.4)) - 0.055;
    return select(high, low, color <= vec3(0.0031308));
}

@fragment
fn overlay_fs(in: OverlayOut) -> @location(0) vec4<f32> {
    let texel = textureSample(image, image_sampler, in.uv);
    let coverage = saturate(0.5 - rounded_box(in.local, in.shape.xy, in.shape.z));
    let color = in.color * texel;
    var rgb = color.rgb;
    if (ENCODE_SRGB) {
        rgb = encode(rgb);
    }
    return vec4(rgb, color.a * coverage);
}
