//! The drawn UI's pass ([`crate::ui`]): after the frame, over it, on either renderer. One
//! instanced pipeline draws rounded rectangles (anti-aliased by their signed distance) and glyphs
//! (coverage from a one-channel atlas the glyphs are rasterized into at the size they are drawn,
//! on first use).

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use render_engine::glam::Vec4;
use render_engine::wgpu;

use super::lite::Growing;
use crate::ui::{FONT, Item};

/// The glyph atlas' side in texels.
const ATLAS: u32 = 1024;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    /// Clip-space left, top, right, bottom.
    rect: [f32; 4],
    /// Atlas left, top, right, bottom.
    uv: [f32; 4],
    color: [f32; 4],
    /// Width and height in pixels, corner radius in pixels, 1 for a glyph.
    shape: [f32; 4],
}

/// A glyph's bitmap in the atlas and its offset from the pen.
#[derive(Clone, Copy, Debug)]
struct Placed {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    xmin: i32,
    ymin: i32,
}

/// The UI pass: its pipeline, the glyph atlas and the instances.
pub struct UiRenderer {
    pipeline: wgpu::RenderPipeline,
    atlas: wgpu::Texture,
    group: wgpu::BindGroup,
    instances: Growing,
    glyphs: HashMap<(char, u32), Placed>,
    /// The shelf being filled: its next free column, its top and its height.
    shelf: (u32, u32, u32),
    /// The target is not sRGB: colours are encoded before they are written.
    encode_srgb: bool,
}

impl UiRenderer {
    #[must_use]
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("ui.wgsl"));
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui atlas"),
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
        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("ui glyph atlas"),
            size: wgpu::Extent3d {
                width: ATLAS,
                height: ATLAS,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // Glyphs are drawn texel for pixel, so nearest sampling reads them exactly.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ui glyph sampler"),
            ..Default::default()
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui atlas"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &atlas.create_view(&wgpu::TextureViewDescriptor::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![
            0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4
        ];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("ui_vs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attributes,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("ui_fs"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            atlas,
            group,
            instances: Growing::new(device, "ui instances", wgpu::BufferUsages::VERTEX),
            glyphs: HashMap::new(),
            shelf: (0, 0, 0),
            encode_srgb: !format.is_srgb(),
        }
    }

    /// `character` at `size` pixels in the atlas, rasterized and uploaded on first use. A full
    /// atlas starts over (glyphs placed earlier in the frame may show wrongly for that frame).
    fn place(&mut self, queue: &wgpu::Queue, character: char, size: u32) -> Placed {
        if let Some(placed) = self.glyphs.get(&(character, size)) {
            return *placed;
        }
        #[allow(clippy::cast_precision_loss)]
        let (metrics, bitmap) = FONT.rasterize(character, size as f32);
        let (width, height) = (
            u32::try_from(metrics.width).unwrap_or(0).min(ATLAS),
            u32::try_from(metrics.height).unwrap_or(0).min(ATLAS),
        );
        let (mut x, mut y, mut row) = self.shelf;
        if width > 0 && height > 0 {
            if x + width > ATLAS {
                (x, y, row) = (0, y + row + 1, 0);
            }
            if y + height > ATLAS {
                self.glyphs.clear();
                (x, y, row) = (0, 0, 0);
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x, y, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &bitmap,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            self.shelf = (x + width + 1, y, row.max(height));
        }
        let placed = Placed {
            x,
            y,
            width,
            height,
            xmin: metrics.xmin,
            ymin: metrics.ymin,
        };
        self.glyphs.insert((character, size), placed);
        placed
    }

    /// Draws `items`, laid out for a `layout`-sized frame, over `target` (`target_size` pixels).
    #[allow(clippy::cast_precision_loss, clippy::too_many_arguments)]
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        target_size: (u32, u32),
        layout: (u32, u32),
        items: &[Item],
    ) {
        if items.is_empty() {
            return;
        }
        let (width, height) = (target_size.0 as f32, target_size.1 as f32);
        let (rx, ry) = (
            width / layout.0.max(1) as f32,
            height / layout.1.max(1) as f32,
        );
        let clip = |left: f32, top: f32, right: f32, bottom: f32| {
            [
                left / width * 2.0 - 1.0,
                1.0 - top / height * 2.0,
                right / width * 2.0 - 1.0,
                1.0 - bottom / height * 2.0,
            ]
        };
        let encode_srgb = self.encode_srgb;
        let encode = |color: Vec4| {
            if encode_srgb {
                let channel = |c: f32| {
                    if c <= 0.003_130_8 {
                        c * 12.92
                    } else {
                        1.055 * c.powf(1.0 / 2.4) - 0.055
                    }
                };
                [
                    channel(color.x),
                    channel(color.y),
                    channel(color.z),
                    color.w,
                ]
            } else {
                color.to_array()
            }
        };
        let mut instances = Vec::with_capacity(items.len());
        for item in items {
            match *item {
                Item::Rect {
                    position,
                    size,
                    radius,
                    color,
                } => {
                    let (left, top) = (position.x * rx, position.y * ry);
                    let (w, h) = (size.x * rx, size.y * ry);
                    instances.push(Instance {
                        rect: clip(left, top, left + w, top + h),
                        uv: [0.0; 4],
                        color: encode(color),
                        shape: [w, h, radius * ry, 0.0],
                    });
                }
                Item::Glyph {
                    pen,
                    character,
                    size,
                    color,
                } => {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let pixels = (size * ry).round().max(1.0) as u32;
                    let placed = self.place(queue, character, pixels);
                    if placed.width == 0 {
                        continue;
                    }
                    let (w, h) = (placed.width as f32, placed.height as f32);
                    let left = (pen.x * rx).round() + placed.xmin as f32;
                    let top = (pen.y * ry).round() - placed.ymin as f32 - h;
                    let texel = 1.0 / ATLAS as f32;
                    let (u, v) = (placed.x as f32 * texel, placed.y as f32 * texel);
                    instances.push(Instance {
                        rect: clip(left, top, left + w, top + h),
                        uv: [u, v, u + w * texel, v + h * texel],
                        color: encode(color),
                        shape: [w, h, 0.0, 1.0],
                    });
                }
            }
        }
        if instances.is_empty() {
            return;
        }
        self.instances
            .write(device, queue, bytemuck::cast_slice(&instances));
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ui"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        pass.set_vertex_buffer(0, self.instances.buffer.slice(..));
        pass.draw(0..4, 0..u32::try_from(instances.len()).unwrap_or(u32::MAX));
    }
}
