//! The window, the fixed-tick loop and the frame: input in, simulation ticks, render out.

use crate::audio::Footsteps;
use crate::controls::{Controls, Look, PadState, standard};
use crate::gpu::Gpu;
use crate::scene::{self, METRES_PER_UNIT};
use render_engine::glam::Vec3;
use shooter_sim::player::{EYE_HEIGHT_UNITS, PLAYER_HALF_EXTENTS, SLIDE_EYE_HEIGHT_UNITS};
use shooter_sim::{Arena, Intent, Session, TICK_HZ};
use std::sync::Arc;
use web_time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

/// The GPU arrives as a user event: in the browser it is created asynchronously.
pub type GpuReady = Result<Gpu, String>;

/// Never simulate more than this many ticks in one frame (after a stall, time is dropped).
const MAX_TICKS_PER_FRAME: u32 = 8;

/// Standing eye height (units) as `f32`.
#[allow(clippy::cast_precision_loss)]
const EYE: f32 = EYE_HEIGHT_UNITS as f32;
/// How far (units) the eye moves toward its target height each tick.
const EYE_DIP_PER_TICK: f32 = 12.0;

pub struct App {
    proxy: EventLoopProxy<GpuReady>,
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    session: Session,
    controls: Controls,
    look: Look,
    pads: Option<gilrs::Gilrs>,
    footsteps: Footsteps,
    started: Instant,
    last_frame: Instant,
    /// Seconds not yet simulated.
    accumulator: f32,
    previous_center: [f32; 3],
    center: [f32; 3],
    /// Eye height above the feet (units), dipping while sliding.
    previous_eye: f32,
    eye: f32,
    /// The pointer is locked to the game (native: grabbed; web: Pointer Lock).
    captured: bool,
}

impl App {
    /// # Panics
    ///
    /// If the arena does not build.
    #[must_use]
    pub fn new(proxy: EventLoopProxy<GpuReady>) -> Self {
        let arena = Arena::greybox();
        let yaw = f32::from(arena.spawn_yaw) / 65_536.0 * std::f32::consts::TAU;
        let session = Session::new(arena).expect("the greybox arena builds");
        let center = session.player.center(&session.world);
        let now = Instant::now();
        Self {
            proxy,
            window: None,
            gpu: None,
            session,
            controls: Controls::new(),
            look: Look::new(yaw),
            pads: gilrs::Gilrs::new()
                .inspect_err(|error| crate::log(&format!("gamepads unavailable: {error}")))
                .ok(),
            footsteps: Footsteps::new(),
            started: now,
            last_frame: now,
            accumulator: 0.0,
            previous_center: center,
            center,
            previous_eye: EYE,
            eye: EYE,
            captured: false,
        }
    }

    fn capture(&mut self, captured: bool) {
        let Some(window) = &self.window else {
            return;
        };
        if captured {
            let grabbed = window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined));
            if let Err(error) = grabbed {
                crate::log(&format!("pointer lock refused: {error}"));
                return;
            }
            window.set_cursor_visible(false);
            self.footsteps.unlock();
        } else {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            window.set_cursor_visible(true);
        }
        self.captured = captured;
    }

    fn poll_pads(&mut self) -> PadState {
        let mut state = PadState::default();
        let Some(pads) = &mut self.pads else {
            return state;
        };
        while pads.next_event().is_some() {}
        use gilrs::{Axis, Button};
        const BUTTONS: [(Button, usize); 17] = [
            (Button::South, 0),
            (Button::East, 1),
            (Button::West, 2),
            (Button::North, 3),
            (Button::LeftTrigger, 4),
            (Button::RightTrigger, 5),
            (Button::LeftTrigger2, 6),
            (Button::RightTrigger2, 7),
            (Button::Select, 8),
            (Button::Start, 9),
            (Button::LeftThumb, 10),
            (Button::RightThumb, 11),
            (Button::DPadUp, 12),
            (Button::DPadDown, 13),
            (Button::DPadLeft, 14),
            (Button::DPadRight, 15),
            (Button::Mode, 16),
        ];
        // gilrs reports stick Y up positive; the standard layout has it down positive.
        const AXES: [(Axis, usize, f32); 4] = [
            (Axis::LeftStickX, standard::LEFT_X as usize, 1.0),
            (Axis::LeftStickY, standard::LEFT_Y as usize, -1.0),
            (Axis::RightStickX, standard::RIGHT_X, 1.0),
            (Axis::RightStickY, standard::RIGHT_Y, -1.0),
        ];
        for (_, pad) in pads.gamepads() {
            for (button, index) in BUTTONS {
                let value = pad.button_data(button).map_or(0.0, |data| data.value());
                state.buttons[index] = state.buttons[index].max(value);
            }
            for (axis, index, sign) in AXES {
                let value = pad.value(axis) * sign;
                if value.abs() > state.axes[index].abs() {
                    state.axes[index] = value;
                }
            }
        }
        state
    }

    fn frame(&mut self) {
        let now = Instant::now();
        let seconds = (now - self.last_frame).as_secs_f32().min(0.25);
        self.last_frame = now;

        #[cfg(target_arch = "wasm32")]
        if self.captured && !crate::web::pointer_locked() {
            // The browser let go of the pointer (Escape): stop playing until the next click.
            self.capture(false);
        }

        let pads = self.poll_pads();
        self.controls.poll_pads(&pads);
        self.look.stick(&pads, seconds);

        let tick = 1.0 / f32::from(TICK_HZ);
        self.accumulator += seconds;
        let mut ticks = 0;
        while self.accumulator >= tick {
            self.accumulator -= tick;
            ticks += 1;
            if ticks > MAX_TICKS_PER_FRAME {
                self.accumulator = 0.0;
                break;
            }
            let (forward, strafe) = self.controls.movement();
            let intent = Intent {
                forward,
                strafe,
                yaw: self.look.sim_yaw(),
                jump: self.controls.jump_held(),
                sprint: self.controls.sprint_held(),
                crouch: self.controls.crouch_held(),
            };
            let report = match self.session.tick(intent) {
                Ok(report) => report,
                Err(error) => {
                    crate::log(&format!("physics step failed: {error}"));
                    return;
                }
            };
            self.previous_eye = self.eye;
            let target = if report.sliding {
                SLIDE_EYE_HEIGHT_UNITS
            } else {
                EYE_HEIGHT_UNITS
            };
            #[allow(clippy::cast_precision_loss)]
            let target = target as f32;
            // The camera dips into a slide and rises out of it over a few ticks.
            self.eye += (target - self.eye).clamp(-EYE_DIP_PER_TICK, EYE_DIP_PER_TICK);
            self.previous_center = self.center;
            self.center = self.session.player.center(&self.session.world);
            let moved = (self.center[0] - self.previous_center[0])
                .hypot(self.center[2] - self.previous_center[2]);
            let elapsed_ms = (now - self.started).as_secs_f64() * 1_000.0;
            self.footsteps.tick(report, moved, elapsed_ms);
        }
        self.footsteps.pump();

        let blend = self.accumulator / tick;
        let center = Vec3::from(self.previous_center).lerp(Vec3::from(self.center), blend);
        #[allow(clippy::cast_precision_loss)]
        let eye_above_center = self.previous_eye + (self.eye - self.previous_eye) * blend
            - PLAYER_HALF_EXTENTS.y as f32;
        let eye = (center + Vec3::Y * eye_above_center) * METRES_PER_UNIT;
        let scene = scene::frame(eye, Vec3::from(self.look.forward()));
        if let Some(gpu) = &mut self.gpu
            && let Err(error) = gpu.render(&scene)
        {
            crate::log(&error);
        }
    }
}

impl ApplicationHandler<GpuReady> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        #[allow(unused_mut)]
        let mut attributes = Window::default_attributes().with_title("shooter — click to play");
        #[cfg(target_arch = "wasm32")]
        {
            use winit::platform::web::WindowAttributesExtWebSys;
            attributes = attributes.with_append(true);
        }
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                crate::log(&format!("no window: {error}"));
                event_loop.exit();
                return;
            }
        };
        self.window = Some(window.clone());
        let proxy = self.proxy.clone();
        let create = async move {
            let _ = proxy.send_event(Gpu::new(window).await);
        };
        #[cfg(not(target_arch = "wasm32"))]
        pollster::block_on(create);
        #[cfg(target_arch = "wasm32")]
        wasm_bindgen_futures::spawn_local(create);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, gpu: GpuReady) {
        match gpu {
            Ok(mut gpu) => {
                scene::build(&mut gpu.world, &self.session.arena);
                if let Some(window) = &self.window {
                    let size = window.inner_size();
                    gpu.resize(size.width, size.height);
                    window.request_redraw();
                }
                self.gpu = Some(gpu);
                self.last_frame = Instant::now();
            }
            Err(error) => {
                crate::fail(&error);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(size.width, size.height);
                }
            }
            WindowEvent::Focused(false) => {
                self.capture(false);
                self.controls.release_all();
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } if !self.captured => self.capture(true),
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };
                // Escape frees the mouse; keys keep moving the player while the window has focus.
                if code == KeyCode::Escape {
                    self.capture(false);
                    return;
                }
                if event.state == ElementState::Pressed {
                    self.controls.key_down(code, event.repeat);
                } else {
                    self.controls.key_up(code);
                }
            }
            WindowEvent::RedrawRequested => {
                self.frame();
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta: (dx, dy) } = event
            && self.captured
        {
            self.look.mouse(dx, dy);
        }
    }
}
