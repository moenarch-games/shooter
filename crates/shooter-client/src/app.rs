//! The window, the fixed-tick loop and the frame: input in, simulation ticks, render out.

use crate::audio::Sounds;
use crate::controls::{Controls, Look, PRIMARY_BUTTON, PadState, standard};
use crate::effects::Effects;
use crate::gpu::Gpu;
use crate::scene::{self, Actors, DummyView, METRES_PER_UNIT};
use render_engine::glam::Vec3;
use shooter_sim::dummies::DUMMY_HEALTH;
use shooter_sim::player::{EYE_HEIGHT_UNITS, PLAYER_HALF_EXTENTS, SLIDE_EYE_HEIGHT_UNITS};
use shooter_sim::{Arena, Event, Intent, Session, TICK_HZ};
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
    sounds: Sounds,
    /// Weapon effects and the moving actors, once the GPU is up.
    effects: Option<Effects>,
    actors: Option<Actors>,
    /// Each dummy's hit glow, 1 when just hit.
    glows: Vec<f32>,
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
        let dummies = session.dummies.len();
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
            sounds: Sounds::new(),
            effects: None,
            actors: None,
            glows: vec![0.0; dummies],
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
            self.sounds.unlock();
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
                pitch: self.look.sim_pitch(),
                // Only a captured mouse shoots: the click that captures it does not.
                fire: self.captured && self.controls.fire_held(),
                select: self.controls.selected(),
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
            let events = &self.session.events;
            self.sounds.tick(report, moved, events, elapsed_ms);
            for glow in &mut self.glows {
                *glow = (*glow - 4.0 * tick).max(0.0);
            }
            for event in events {
                if let Event::Damaged { dummy, .. } = *event {
                    self.glows[dummy] = 1.0;
                }
            }
            if let (Some(gpu), Some(effects)) = (&mut self.gpu, &mut self.effects) {
                effects.play(&mut gpu.world, events);
                effects.rockets(&mut gpu.world, &self.session.rockets);
            }
        }
        let ticks = ticks.min(MAX_TICKS_PER_FRAME);

        let blend = self.accumulator / tick;
        let center = Vec3::from(self.previous_center).lerp(Vec3::from(self.center), blend);
        #[allow(clippy::cast_precision_loss)]
        let eye_above_center = self.previous_eye + (self.eye - self.previous_eye) * blend
            - PLAYER_HALF_EXTENTS.y as f32;
        let eye = (center + Vec3::Y * eye_above_center) * METRES_PER_UNIT;
        let forward = Vec3::from(self.look.forward());
        let (_, right, _) = scene::basis(forward);
        self.sounds.listen(eye.to_array(), right.to_array());
        self.sounds.pump();

        let world = &self.session.world;
        let rockets: Vec<Vec3> = self
            .session
            .rockets
            .iter()
            .map(|rocket| Vec3::from(rocket.position_units()) * METRES_PER_UNIT)
            .collect();
        let point_lights = self
            .effects
            .as_mut()
            .map(|effects| effects.lights(seconds, &rockets))
            .unwrap_or_default();
        let scene = scene::frame(eye, forward, point_lights);
        let Some(gpu) = &mut self.gpu else {
            return;
        };
        if let Some(actors) = &mut self.actors {
            #[allow(clippy::cast_precision_loss)]
            let dummies: Vec<DummyView> = self
                .session
                .dummies
                .iter()
                .zip(&self.glows)
                .map(|(dummy, glow)| DummyView {
                    center: dummy
                        .center(world)
                        .map(|center| Vec3::from(center) * METRES_PER_UNIT),
                    health: dummy.health as f32 / DUMMY_HEALTH as f32,
                    glow: *glow,
                })
                .collect();
            let held = self.session.arsenal.held;
            actors.update(&mut gpu.world, &dummies, &rockets, held, eye, forward);
        }
        if let Err(error) = gpu.render(&scene, ticks) {
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
                self.effects = Some(Effects::new(&mut gpu.world));
                self.actors = Some(Actors::new(&mut gpu.world, self.session.dummies.len()));
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
            WindowEvent::MouseInput { state, button, .. } => {
                let button = match button {
                    MouseButton::Left => PRIMARY_BUTTON,
                    MouseButton::Middle => 1,
                    MouseButton::Right => 2,
                    MouseButton::Back => 3,
                    MouseButton::Forward => 4,
                    MouseButton::Other(other) => other,
                };
                if state == ElementState::Pressed {
                    self.controls.mouse_down(button);
                } else {
                    self.controls.mouse_up(button);
                }
            }
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
