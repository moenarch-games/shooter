//! The GPU: a window's surface (or an offscreen texture for the profile run), the device and
//! the renderer drawing the client's [`World`]: render-engine where the adapter has what it needs
//! (WebGPU in the browser, Vulkan, Metal or DX12 natively), else the lite renderer
//! ([`crate::gfx::lite`]) on WebGL 2. The browser falls back by itself where WebGPU is missing;
//! `?renderer=lite` in the browser and `SHOOTER_RENDERER=lite` natively force the lite one.

use crate::gfx::World;
use crate::gfx::lite::{FrameSizes, LiteRenderer, LiteWorld, RenderScale};
use crate::gfx::ui::UiRenderer;
use crate::ui::Item;
use render_engine::renderer::{Renderer, Viewport};
use render_engine::scene::Scene;
use render_engine::wgpu;
use std::sync::Arc;
use winit::window::Window;

/// Where frames go.
enum Target {
    /// A window's surface, presented each frame.
    Surface {
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
    },
    /// The offscreen profile run's texture (`--frames`): it waits for each frame, and
    /// `--capture` reads some back ([`Gpu::read_frame`]).
    #[cfg(not(target_arch = "wasm32"))]
    Offscreen(wgpu::Texture),
}

/// What draws the world.
enum Backend {
    Engine {
        renderer: Box<Renderer>,
        viewport: Box<Viewport>,
    },
    Lite(Box<LiteRenderer>),
}

pub struct Gpu {
    target: Target,
    /// The sRGB view of the target the renderer writes (WebGPU canvases are not sRGB).
    view_format: wgpu::TextureFormat,
    device: wgpu::Device,
    queue: wgpu::Queue,
    backend: Backend,
    pub world: World,
    size: (u32, u32),
    /// The share of `size` the lite renderer draws the 3D scene at (dynamic resolution); the
    /// overlay is always drawn at full size.
    scale: f32,
    /// The surface's share of `size` ([`canvas_for`]): in the browser the canvas's backing
    /// store, which the page stretches over the window.
    canvas: f32,
    /// The lite renderer's dynamic resolution in the browser and when the last frame was drawn;
    /// `None` keeps `scale` fixed.
    render_scale: Option<(RenderScale, Option<web_time::Instant>)>,
    /// The drawn UI's pass, over either renderer's frame at full size.
    ui: UiRenderer,
}

/// A fixed scene scale for the lite renderer: `?scale=0.5` in the browser,
/// `SHOOTER_RENDER_SCALE=0.5` natively. Without one the browser's follows the frame rate and a
/// native window's stays at 1.
fn requested_scale() -> Option<f32> {
    #[cfg(not(target_arch = "wasm32"))]
    let requested = std::env::var("SHOOTER_RENDER_SCALE").ok();
    #[cfg(target_arch = "wasm32")]
    let requested = crate::web::query("scale");
    requested
        .and_then(|scale| scale.parse::<f32>().ok())
        .filter(|scale| scale.is_finite() && *scale > 0.0)
        .map(|scale| scale.clamp(0.25, 1.0))
}

/// `size` at `scale`, at least a pixel each way.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn scaled(size: (u32, u32), scale: f32) -> (u32, u32) {
    if scale >= 0.999 {
        return size;
    }
    (
        ((size.0 as f32 * scale).round() as u32).max(1),
        ((size.1 as f32 * scale).round() as u32).max(1),
    )
}

/// The browser canvas's share of the window for a scene drawn at `scale`: full size, until the
/// scene is at half size or below; then half size, which the page stretches. Below that the
/// scene is upscaled inside it, so in software WebGL the full-size upscale and present (each a
/// pass over every pixel) stop costing a frame's budget, while the overlay, drawn at half size,
/// stays readable. A native window always draws its full size.
fn canvas_for(scale: f32) -> f32 {
    // The controller's steps land just above a half (0.71 by √0.5 is 0.502).
    if cfg!(target_arch = "wasm32") && scale <= CANVAS_HALF + 0.02 {
        CANVAS_HALF
    } else {
        1.0
    }
}

const CANVAS_HALF: f32 = 0.5;

/// Whether the lite renderer is asked for: `SHOOTER_RENDERER=lite` natively, `?renderer=lite`
/// in the browser.
#[must_use]
pub fn lite_requested() -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    let requested = std::env::var("SHOOTER_RENDERER").ok();
    #[cfg(target_arch = "wasm32")]
    let requested = crate::web::query("renderer");
    requested.is_some_and(|renderer| renderer.eq_ignore_ascii_case("lite"))
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

/// The lite renderer's device: WebGL 2's limits (so a native run checks the same), at the
/// adapter's texture sizes.
async fn lite_device(adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue), String> {
    adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("shooter lite"),
            required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                .using_resolution(adapter.limits()),
            ..Default::default()
        })
        .await
        .map_err(|error| format!("no GPU device: {error}"))
}

/// The render-engine backend and its world for `view_format` at `width`×`height`.
fn engine(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    view_format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> (Backend, World) {
    let renderer = Renderer::new(device, queue, view_format);
    let world = World::engine(render_engine::world::World::new(&renderer));
    let viewport = Viewport::new(&renderer, width, height);
    (
        Backend::Engine {
            renderer: Box::new(renderer),
            viewport: Box::new(viewport),
        },
        world,
    )
}

/// The lite backend and its world.
fn lite(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    view_format: wgpu::TextureFormat,
) -> (Backend, World) {
    let world = LiteWorld::new(device, queue);
    let renderer = LiteRenderer::new(device, &world, view_format);
    (
        Backend::Lite(Box::new(renderer)),
        World::Lite(Box::new(world)),
    )
}

/// Whether the page has `navigator.gpu`.
#[cfg(target_arch = "wasm32")]
fn has_webgpu() -> bool {
    web_sys::window().is_some_and(|window| {
        js_sys::Reflect::get(&window.navigator(), &"gpu".into())
            .is_ok_and(|gpu| !gpu.is_undefined() && !gpu.is_null())
    })
}

impl Gpu {
    /// The GPU of `window`: render-engine unless the lite renderer is asked for or, in the
    /// browser, WebGPU has no adapter or device (then WebGL 2 and the lite renderer).
    pub async fn new(window: Arc<Window>) -> Result<Self, String> {
        #[cfg(target_arch = "wasm32")]
        {
            if !lite_requested() && has_webgpu() {
                // The adapter and device come before the surface: a canvas that once gave a
                // WebGPU context cannot give a WebGL one.
                let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                    backends: wgpu::Backends::BROWSER_WEBGPU,
                    ..wgpu::InstanceDescriptor::new_without_display_handle()
                });
                let adapter = instance
                    .request_adapter(&wgpu::RequestAdapterOptions {
                        power_preference: wgpu::PowerPreference::HighPerformance,
                        ..Default::default()
                    })
                    .await;
                match adapter {
                    Ok(adapter) => match device(&adapter).await {
                        Ok((device, queue)) => {
                            return Self::on_surface(
                                window, &instance, &adapter, device, queue, false,
                            );
                        }
                        Err(error) => crate::log(&format!("WebGPU: {error}; trying WebGL 2")),
                    },
                    Err(error) => {
                        crate::log(&format!("WebGPU: no adapter ({error}); trying WebGL 2"))
                    }
                }
            }
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::GL,
                ..wgpu::InstanceDescriptor::new_without_display_handle()
            });
            let surface = instance
                .create_surface(window.clone())
                .map_err(|error| format!("no WebGL 2 surface: {error}"))?;
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: Some(&surface),
                    ..Default::default()
                })
                .await
                .map_err(|error| {
                    format!("no GPU adapter: this browser has neither WebGPU nor WebGL 2 ({error})")
                })?;
            let (device, queue) = lite_device(&adapter).await?;
            Self::configure(window, surface, &adapter, device, queue, true)
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let instance = wgpu::Instance::default();
            let lite = lite_requested();
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    ..Default::default()
                })
                .await
                .map_err(|error| format!("no GPU adapter: {error}"))?;
            let (device, queue) = if lite {
                lite_device(&adapter).await?
            } else {
                device(&adapter).await?
            };
            Self::on_surface(window, &instance, &adapter, device, queue, lite)
        }
    }

    fn on_surface(
        window: Arc<Window>,
        instance: &wgpu::Instance,
        adapter: &wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        lite: bool,
    ) -> Result<Self, String> {
        let surface = instance
            .create_surface(window.clone())
            .map_err(|error| format!("no GPU surface: {error}"))?;
        Self::configure(window, surface, adapter, device, queue, lite)
    }

    fn configure(
        window: Arc<Window>,
        surface: wgpu::Surface<'static>,
        adapter: &wgpu::Adapter,
        device: wgpu::Device,
        queue: wgpu::Queue,
        lite: bool,
    ) -> Result<Self, String> {
        let size = window.inner_size();
        let info = adapter.get_info();
        crate::log(&format!(
            "{} renderer on {} ({:?})",
            if lite { "lite" } else { "render-engine" },
            info.name,
            info.backend
        ));
        let mut config = surface
            .get_default_config(adapter, size.width.max(1), size.height.max(1))
            .ok_or("the surface has no format the adapter supports")?;
        let capabilities = surface.get_capabilities(adapter);
        if let Some(srgb) = capabilities.formats.iter().copied().find(|f| f.is_srgb()) {
            config.format = srgb;
        }
        // The lite renderer encodes sRGB itself where the surface has no sRGB format (WebGL 2
        // has no view formats).
        let view_format = if lite {
            config.format
        } else {
            config.format.add_srgb_suffix()
        };
        if view_format != config.format {
            config.view_formats = vec![view_format];
        }
        config.present_mode = wgpu::PresentMode::AutoVsync;
        let size = (config.width, config.height);
        // Dynamic resolution is the lite renderer's: automatic in the browser, where software
        // WebGL needs it, fixed when asked for.
        let scale = if lite {
            requested_scale().unwrap_or(1.0)
        } else {
            1.0
        };
        let browser_lite = lite && cfg!(target_arch = "wasm32");
        let canvas = canvas_for(scale);
        (config.width, config.height) = scaled(size, canvas);
        surface.configure(&device, &config);
        let (backend, world) = if lite {
            self::lite(&device, &queue, view_format)
        } else {
            engine(&device, &queue, view_format, config.width, config.height)
        };
        let ui = UiRenderer::new(&device, view_format);
        Ok(Self {
            size,
            scale,
            canvas,
            render_scale: (browser_lite && requested_scale().is_none())
                .then(|| (RenderScale::default(), None)),
            target: Target::Surface { surface, config },
            view_format,
            device,
            queue,
            ui,
            backend,
            world,
        })
    }

    /// Whether the lite renderer draws.
    #[must_use]
    pub fn is_lite(&self) -> bool {
        matches!(self.backend, Backend::Lite(_))
    }

    /// A `width`×`height` offscreen target on the adapter wgpu picks without a window. As in
    /// render-engine's headless renderer, `WGPU_FORCE_FALLBACK_ADAPTER=1` picks the software
    /// one (lavapipe, as on a GPU-less Linux runner); `SHOOTER_RENDERER=lite` draws with the
    /// lite renderer (`WGPU_BACKEND=gl` puts it on OpenGL, as WebGL 2 is).
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
        let lite = lite_requested();
        let info = adapter.get_info();
        crate::log(&format!(
            "offscreen {} on {} ({:?}, {})",
            if lite { "lite" } else { "render-engine" },
            info.name,
            info.backend,
            info.driver
        ));
        let (device, queue) = if lite {
            pollster::block_on(lite_device(&adapter))?
        } else {
            pollster::block_on(device(&adapter))?
        };
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
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let (backend, world) = if lite {
            self::lite(&device, &queue, view_format)
        } else {
            engine(&device, &queue, view_format, width, height)
        };
        let ui = UiRenderer::new(&device, view_format);
        Ok(Self {
            target: Target::Offscreen(texture),
            view_format,
            device,
            queue,
            ui,
            backend,
            world,
            size: (width, height),
            render_scale: None,
            canvas: 1.0,
            scale: if lite {
                requested_scale().unwrap_or(1.0)
            } else {
                1.0
            },
        })
    }

    /// The drawn frame's width and height in pixels.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// The target's own size: the surface's (the canvas's backing store in the browser), or the
    /// offscreen texture's.
    fn target_size(&self) -> (u32, u32) {
        match &self.target {
            Target::Surface { config, .. } => (config.width, config.height),
            #[cfg(not(target_arch = "wasm32"))]
            Target::Offscreen(texture) => (texture.width(), texture.height()),
        }
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
        self.size = (width.min(limit), height.min(limit));
        (config.width, config.height) = scaled(self.size, self.canvas);
        surface.configure(&self.device, config);
        if let Backend::Engine { renderer, viewport } = &mut self.backend {
            viewport.resize(renderer, config.width, config.height);
        }
    }

    /// Steps the browser's dynamic resolution with the time since the last frame.
    #[cfg_attr(target_arch = "wasm32", allow(irrefutable_let_patterns))]
    fn follow_frame_rate(&mut self) {
        let Some((controller, last)) = &mut self.render_scale else {
            return;
        };
        let now = web_time::Instant::now();
        let Some(last) = last.replace(now) else {
            return;
        };
        let scale = controller.update((now - last).as_secs_f32() * 1_000.0);
        if (scale - self.scale).abs() < 0.01 {
            return;
        }
        self.scale = scale;
        let canvas = canvas_for(scale);
        crate::log(&format!("render scale {scale:.2}, canvas {canvas:.2}"));
        if (canvas - self.canvas).abs() > 0.001 {
            self.canvas = canvas;
            if let Target::Surface { surface, config } = &mut self.target {
                (config.width, config.height) = scaled(self.size, canvas);
                surface.configure(&self.device, config);
            }
        }
    }

    /// Draws `scene` and over it the UI `ui` ([`crate::ui`]), first stepping the particles
    /// `particle_steps` simulation ticks (they run at the simulation's fixed rate, so effects look
    /// the same at any frame rate).
    pub fn render(
        &mut self,
        scene: &Scene,
        ui: &[Item],
        particle_steps: u32,
    ) -> Result<(), String> {
        self.follow_frame_rate();
        let scene_size = scaled(self.size, self.scale);
        let target_size = self.target_size();
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
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let tick = 1.0 / f32::from(shooter_sim::TICK_HZ);
        match (&mut self.backend, &mut self.world) {
            (Backend::Engine { renderer, viewport }, World::Engine(world)) => {
                renderer.prepare(viewport, &world.world, scene, &[]);
                for _ in 0..particle_steps {
                    world.world.step_particles(&mut encoder, tick, None);
                }
                renderer.encode(&mut encoder, viewport, &world.world, &view, None);
            }
            (Backend::Lite(renderer), World::Lite(world)) => {
                for _ in 0..particle_steps {
                    world.step_particles(tick);
                }
                renderer.encode(
                    world,
                    scene,
                    &mut encoder,
                    &view,
                    FrameSizes {
                        target: target_size,
                        layout: self.size,
                        scene: scene_size,
                    },
                );
            }
            _ => unreachable!("a world is made with its renderer"),
        }
        self.ui.encode(
            &self.device,
            &self.queue,
            &mut encoder,
            &view,
            target_size,
            self.size,
            ui,
        );
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

    /// The last offscreen frame: width, height and tightly packed sRGB RGBA8 rows.
    ///
    /// # Errors
    ///
    /// If this is no offscreen GPU or the read-back fails.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read_frame(&self) -> Result<(u32, u32, Vec<u8>), String> {
        let Target::Offscreen(texture) = &self.target else {
            return Err("only an offscreen frame can be read back".into());
        };
        let (width, height) = (texture.width(), texture.height());
        let row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame read-back"),
            size: u64::from(row * height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(height),
                },
            },
            texture.size(),
        );
        self.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| format!("frame read-back failed: {error}"))?;
        let mapped = slice
            .get_mapped_range()
            .map_err(|error| format!("frame read-back failed: {error:?}"))?;
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for line in mapped.chunks(row as usize) {
            pixels.extend_from_slice(&line[..(width * 4) as usize]);
        }
        Ok((width, height, pixels))
    }
}
