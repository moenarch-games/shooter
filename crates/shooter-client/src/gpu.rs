//! The GPU: a window's surface (or an offscreen texture for the profile run), the device and
//! render-engine's renderer, world and viewport.

use render_engine::renderer::{Renderer, Viewport};
use render_engine::scene::Scene;
use render_engine::wgpu;
use render_engine::world::World;
use std::sync::Arc;
use winit::window::Window;

/// Where frames go.
enum Target {
    /// A window's surface, presented each frame.
    Surface {
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
    },
    /// A texture nobody reads: the offscreen profile run (`--frames`) waits for each frame.
    #[cfg(not(target_arch = "wasm32"))]
    Offscreen(wgpu::Texture),
}

pub struct Gpu {
    target: Target,
    /// The sRGB view of the target the renderer writes (WebGPU canvases are not sRGB).
    view_format: wgpu::TextureFormat,
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
    pub world: World,
    viewport: Viewport,
}

/// The adapter's device, with unclipped sun-shadow depth where it has it (render-engine falls
/// back otherwise).
async fn device(adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue), String> {
    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("shooter"),
            required_features: adapter.features() & wgpu::Features::DEPTH_CLIP_CONTROL,
            ..Default::default()
        })
        .await
        .map_err(|error| format!("no GPU device: {error}"))
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
        let (device, queue) = device(&adapter).await?;
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
            target: Target::Surface { surface, config },
            view_format,
            device,
            queue,
            renderer,
            world,
            viewport,
        })
    }

    /// A `width`×`height` offscreen target on the adapter wgpu picks without a window. As in
    /// render-engine's headless renderer, `WGPU_FORCE_FALLBACK_ADAPTER=1` picks the software
    /// one (lavapipe, as on a GPU-less Linux runner).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn offscreen(width: u32, height: u32) -> Result<Self, String> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let force_fallback_adapter = std::env::var("WGPU_FORCE_FALLBACK_ADAPTER")
            .is_ok_and(|v| !(v.is_empty() || v == "0" || v == "false"));
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter,
            ..Default::default()
        }))
        .map_err(|error| format!("no GPU adapter: {error}"))?;
        let info = adapter.get_info();
        crate::log(&format!(
            "offscreen on {} ({:?}, {})",
            info.name, info.backend, info.driver
        ));
        let (device, queue) = pollster::block_on(device(&adapter))?;
        let view_format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen frame"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: view_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let renderer = Renderer::new(&device, &queue, view_format);
        let world = World::new(&renderer);
        let viewport = Viewport::new(&renderer, width, height);
        Ok(Self {
            target: Target::Offscreen(texture),
            view_format,
            device,
            queue,
            renderer,
            world,
            viewport,
        })
    }

    /// An offscreen target keeps its size.
    #[cfg_attr(target_arch = "wasm32", allow(irrefutable_let_patterns))]
    pub fn resize(&mut self, width: u32, height: u32) {
        let Target::Surface { surface, config } = &mut self.target else {
            return;
        };
        if width == 0 || height == 0 {
            return;
        }
        let limit = self.device.limits().max_texture_dimension_2d;
        config.width = width.min(limit);
        config.height = height.min(limit);
        surface.configure(&self.device, config);
        self.viewport
            .resize(&self.renderer, config.width, config.height);
    }

    /// Draws `scene`, first stepping the particles `particle_steps` simulation ticks (they run
    /// at the simulation's fixed rate, so effects look the same at any frame rate).
    pub fn render(&mut self, scene: &Scene, particle_steps: u32) -> Result<(), String> {
        let view_of = |texture: &wgpu::Texture| {
            texture.create_view(&wgpu::TextureViewDescriptor {
                format: Some(self.view_format),
                ..Default::default()
            })
        };
        let (view, frame) = match &self.target {
            Target::Surface { surface, config } => match surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame)
                | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                    (view_of(&frame.texture), Some(frame))
                }
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                    return Ok(());
                }
                wgpu::CurrentSurfaceTexture::Outdated => {
                    surface.configure(&self.device, config);
                    return Ok(());
                }
                wgpu::CurrentSurfaceTexture::Lost => return Err("GPU surface lost".into()),
                wgpu::CurrentSurfaceTexture::Validation => {
                    return Err("GPU surface validation failed".into());
                }
            },
            #[cfg(not(target_arch = "wasm32"))]
            Target::Offscreen(texture) => (view_of(texture), None),
        };
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
        match frame {
            Some(frame) => self.queue.present(frame),
            // Nothing presents an offscreen frame, so wait for it as presenting would.
            None => {
                self.device
                    .poll(wgpu::PollType::wait_indefinitely())
                    .map_err(|error| format!("GPU frame failed: {error}"))?;
            }
        }
        Ok(())
    }
}
