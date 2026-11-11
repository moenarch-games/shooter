//! The window's GPU: surface, device and render-engine's renderer, world and viewport.

use render_engine::renderer::{Renderer, Viewport};
use render_engine::scene::Scene;
use render_engine::wgpu;
use render_engine::world::World;
use std::sync::Arc;
use winit::window::Window;

pub struct Gpu {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    /// The sRGB view of the surface the renderer writes (WebGPU canvases are not sRGB).
    view_format: wgpu::TextureFormat,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
    pub world: World,
    viewport: Viewport,
}

impl Gpu {
    pub async fn new(window: Arc<Window>) -> Result<Self, String> {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window)
            .map_err(|error| format!("no GPU surface: {error}"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .map_err(|error| format!("no GPU adapter (WebGPU needed in the browser): {error}"))?;
        // Unclipped sun-shadow depth where the adapter has it (render-engine falls back otherwise).
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("shooter"),
                required_features: adapter.features() & wgpu::Features::DEPTH_CLIP_CONTROL,
                ..Default::default()
            })
            .await
            .map_err(|error| format!("no GPU device: {error}"))?;
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("the surface has no format the adapter supports")?;
        let capabilities = surface.get_capabilities(&adapter);
        if let Some(srgb) = capabilities.formats.iter().copied().find(|f| f.is_srgb()) {
            config.format = srgb;
        }
        let view_format = config.format.add_srgb_suffix();
        if view_format != config.format {
            config.view_formats = vec![view_format];
        }
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&device, &config);
        let renderer = Renderer::new(&device, &queue, view_format);
        let world = World::new(&renderer);
        let viewport = Viewport::new(&renderer, config.width, config.height);
        Ok(Self {
            surface,
            config,
            view_format,
            device,
            queue,
            renderer,
            world,
            viewport,
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        let limit = self.device.limits().max_texture_dimension_2d;
        self.config.width = width.min(limit);
        self.config.height = height.min(limit);
        self.surface.configure(&self.device, &self.config);
        self.viewport
            .resize(&self.renderer, self.config.width, self.config.height);
    }

    /// Draws `scene`, first stepping the particles `particle_steps` simulation ticks (they run
    /// at the simulation's fixed rate, so effects look the same at any frame rate).
    pub fn render(&mut self, scene: &Scene, particle_steps: u32) -> Result<(), String> {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Lost => return Err("GPU surface lost".into()),
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("GPU surface validation failed".into());
            }
        };
        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.view_format),
            ..Default::default()
        });
        self.renderer
            .prepare(&mut self.viewport, &self.world, scene, &[]);
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let tick = 1.0 / f32::from(shooter_sim::TICK_HZ);
        for _ in 0..particle_steps {
            self.world.step_particles(&mut encoder, tick, None);
        }
        self.renderer
            .encode(&mut encoder, &self.viewport, &self.world, &view, None);
        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
        Ok(())
    }
}
