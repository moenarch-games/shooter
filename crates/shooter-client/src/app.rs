//! The window, the fixed-tick loop and the frame: input in, the match ticks (in-process practice
//! or a hosted match), render out.

use crate::audio::Sounds;
use crate::camera::{self, Follow};
use crate::controls::{Controls, Look, PRIMARY_BUTTON, PadState, Selection, WheelDelta, standard};
use crate::effects::Effects;
use crate::game::Game;
use crate::gpu::Gpu;
use crate::hud::{Hud, HudText};
use crate::overlay::{Overlay, Vitals};
use crate::robots::{self, RobotState, RobotView, Robots, track::Tracker};
use crate::scene::{self, Actors, EYE_ABOVE_CENTER, FighterView, METRES_PER_UNIT, SWING_SECONDS};
use render_engine::glam::{Mat4, Vec3};
use shooter_sim::aim::{self, RETICLE_RANGE_UNITS};
use shooter_sim::damage::{Hitboxes, PartDamage};
use shooter_sim::player::{EYE_HEIGHT_UNITS, PLAYER_HALF_EXTENTS, SLIDE_EYE_HEIGHT_UNITS};
use shooter_sim::robot::PartId;
use shooter_sim::scenarios::scripted_intent;

use shooter_sim::session::{body_of, slot_of};
use shooter_sim::weapons;
use shooter_sim::{ArenaModule, Event, FighterState, Intent, MapId, TICK_HZ, Weapon};
use std::sync::Arc;
use web_time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{
    DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorGrabMode, Window, WindowId};

/// The GPU arrives as a user event: in the browser it is created asynchronously.
pub type GpuReady = Result<Gpu, String>;

/// The browser's robot kit fetch result, filled in when it finishes.
#[cfg(target_arch = "wasm32")]
type KitInbox = std::rc::Rc<
    std::cell::RefCell<Option<Result<(Vec<robots::kit::Plan>, robots::load::Files), String>>>,
>;

/// Where the browser fetches the kits: `robot-kit/` and `weapon-kit/` beside the page
/// (`scripts/build-web.sh`).
#[cfg(target_arch = "wasm32")]
const KIT_URL: &str = "";

/// The HUD's note while the kit is not drawn yet.
const KIT_LOADING: &str = "robot kit loading: box placeholders stand in";

/// Starts the browser's robot kit fetch.
#[cfg(target_arch = "wasm32")]
fn fetch_kit() -> KitInbox {
    let inbox = KitInbox::default();
    let filled = inbox.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let fetched = robots::load::fetch(KIT_URL).await;
        *filled.borrow_mut() = Some(fetched);
    });
    inbox
}

/// Never simulate more than this many ticks in one frame (after a stall, time is dropped).
const MAX_TICKS_PER_FRAME: u32 = 8;

/// Standing eye height (units) as `f32`.
#[allow(clippy::cast_precision_loss)]
const EYE: f32 = EYE_HEIGHT_UNITS as f32;
/// How far (units) the eye moves toward its target height each tick.
const EYE_DIP_PER_TICK: f32 = 12.0;

/// A `shooter_sim::trig` yaw in radians.
fn radians(yaw: u16) -> f32 {
    f32::from(yaw) / 65_536.0 * std::f32::consts::TAU
}

/// Where a yaw and pitch (`trig` steps) look.
fn looking(yaw: u16, pitch: i16) -> Vec3 {
    let (sy, cy) = radians(yaw).sin_cos();
    let (sp, cp) = (f32::from(pitch) / 65_536.0 * std::f32::consts::TAU).sin_cos();
    Vec3::new(sy * cp, sp, cy * cp)
}

/// How far through its drawn swing a blade is (0..=1), `seconds` after the swing began.
fn swing_progress(seconds: f32) -> Option<f32> {
    (seconds < SWING_SECONDS).then(|| seconds / SWING_SECONDS)
}

pub struct App {
    /// None in the offscreen profile run, which has no event loop.
    proxy: Option<EventLoopProxy<GpuReady>>,
    /// The offscreen profile run's tick: the local fighter plays
    /// [`scripted_intent`]`(0, tick)` instead of the controls.
    scripted: Option<u32>,
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    game: Game,
    /// The map drawn in the GPU's world, once the game knows it and the GPU is up.
    drawn: Option<MapId>,
    controls: Controls,
    look: Look,
    pads: Option<gilrs::Gilrs>,
    sounds: Sounds,
    hud: Hud,
    /// The drawn HUD: the weapon panel and the chassis and energy bars.
    overlay: Overlay,
    /// The weapon the controls ask for (a select key, or the next and previous steps).
    selection: Selection,
    shown: HudText,
    /// Tab held: the scoreboard shows.
    board: bool,
    /// Weapon effects and the moving actors, once the GPU is up.
    effects: Option<Effects>,
    actors: Option<Actors>,
    /// The fighters drawn as their robot builds, once the kit is in the GPU's world.
    robots: Option<Robots>,
    /// Why the fighters are boxes, while they are (the kit loading or unavailable): in the HUD.
    kit_status: Option<String>,
    /// The browser's kit fetch, once it is done.
    #[cfg(target_arch = "wasm32")]
    kit_inbox: KitInbox,
    /// Shots and deaths per slot, for the robots' poses.
    tracker: Tracker,
    /// This frame's ticks' events, rockets and pellets, whose effects start once the robots are
    /// posed (shots are drawn from their weapons' muzzles).
    pending: Vec<Pending>,
    /// Each slot's hit glow, 1 when just hit.
    glows: Vec<f32>,
    /// Seconds since each slot's last blade swing (others', from the match's events), and since
    /// the local fighter's (as predicted).
    swings: Vec<f32>,
    local_swing: f32,
    started: Instant,
    last_frame: Instant,
    /// Seconds not yet simulated.
    accumulator: f32,
    previous_center: [f32; 3],
    center: [f32; 3],
    /// Eye height above the feet (units), dipping while sliding.
    previous_eye: f32,
    eye: f32,
    /// The third-person camera following the local fighter.
    follow: Follow,
    /// The pointer is locked to the game (native: grabbed; web: Pointer Lock).
    captured: bool,
    /// The browser profile run (`?frames=<n>`), timing each animation frame.
    #[cfg(target_arch = "wasm32")]
    profile: Option<crate::web::Profile>,
}

impl App {
    #[must_use]
    pub fn new(proxy: EventLoopProxy<GpuReady>, game: Game) -> Self {
        let now = Instant::now();
        let center = game.local_center().unwrap_or_default();
        Self {
            proxy: Some(proxy),
            scripted: None,
            window: None,
            gpu: None,
            look: Look::new(radians(game.map().map_or(0, |map| map.arena().spawn_yaw))),
            game,
            drawn: None,
            controls: Controls::new(),
            pads: gilrs::Gilrs::new()
                .inspect_err(|error| crate::log(&format!("gamepads unavailable: {error}")))
                .ok(),
            sounds: Sounds::new(),
            hud: Hud::default(),
            overlay: Overlay::default(),
            selection: Selection::default(),
            shown: HudText::default(),
            board: false,
            effects: None,
            actors: None,
            robots: None,
            kit_status: Some(KIT_LOADING.to_owned()),
            #[cfg(target_arch = "wasm32")]
            kit_inbox: fetch_kit(),
            tracker: Tracker::default(),
            pending: Vec::new(),
            glows: vec![0.0; usize::from(u8::MAX)],
            swings: vec![f32::INFINITY; usize::from(u8::MAX)],
            local_swing: f32::INFINITY,
            started: now,
            last_frame: now,
            accumulator: 0.0,
            previous_center: center,
            center,
            previous_eye: EYE,
            eye: EYE,
            follow: Follow::default(),
            captured: false,
            #[cfg(target_arch = "wasm32")]
            profile: None,
        }
    }

    /// The browser profile run ([`crate::web::Profile`]): `frames` animation frames of one
    /// tick each, the local fighter scripted as in the native offscreen run.
    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub fn profiled(mut self, frames: u32) -> Self {
        self.scripted = Some(0);
        self.profile = Some(crate::web::Profile::new(
            frames,
            self.game.map().unwrap_or(MapId::DEFAULT),
        ));
        self
    }

    /// One frame of the browser profile run; false once it is over.
    #[cfg(target_arch = "wasm32")]
    fn profiled_frame(&mut self) -> bool {
        if self.gpu.is_none() {
            // Not drawing yet: the timed frames start once the GPU is up.
            return true;
        }
        let Some(started) = self.profile.as_mut().map(crate::web::Profile::begin) else {
            return false;
        };
        if !self.advance(Instant::now(), 1.0 / f32::from(TICK_HZ)) {
            crate::fail("the scripted match failed");
            return false;
        }
        let Some(profile) = &mut self.profile else {
            return false;
        };
        if profile.end(started) {
            profile.finish();
            return false;
        }
        true
    }

    /// The offscreen profile run ([`crate::offscreen`]): `game` drawn on `gpu`, the local
    /// fighter scripted, no window, event loop or gamepads.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn scripted(game: Game, gpu: Gpu) -> Self {
        let now = Instant::now();
        let center = game.local_center().unwrap_or_default();
        let mut app = Self {
            proxy: None,
            scripted: Some(0),
            window: None,
            gpu: None,
            look: Look::new(radians(game.map().map_or(0, |map| map.arena().spawn_yaw))),
            game,
            drawn: None,
            controls: Controls::new(),
            pads: None,
            sounds: Sounds::new(),
            hud: Hud::default(),
            overlay: Overlay::default(),
            selection: Selection::default(),
            shown: HudText::default(),
            board: false,
            effects: None,
            actors: None,
            robots: None,
            kit_status: Some(KIT_LOADING.to_owned()),
            tracker: Tracker::default(),
            pending: Vec::new(),
            glows: vec![0.0; usize::from(u8::MAX)],
            swings: vec![f32::INFINITY; usize::from(u8::MAX)],
            local_swing: f32::INFINITY,
            started: now,
            last_frame: now,
            accumulator: 0.0,
            previous_center: center,
            center,
            previous_eye: EYE,
            eye: EYE,
            follow: Follow::default(),
            captured: false,
        };
        app.attach(gpu);
        app
    }

    /// One frame of the offscreen profile run: exactly one tick, then the drawing.
    ///
    /// # Errors
    ///
    /// If the match fails.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn scripted_frame(&mut self) -> Result<(), String> {
        let tick = 1.0 / f32::from(TICK_HZ);
        if self.advance(Instant::now(), tick) {
            Ok(())
        } else {
            Err("the scripted match failed".into())
        }
    }

    /// The offscreen run's GPU (`--capture` reads its frames).
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub const fn gpu(&self) -> Option<&Gpu> {
        self.gpu.as_ref()
    }

    /// The GPU is up: the effects and actors go into its world (and the arena, once known).
    fn attach(&mut self, mut gpu: Gpu) {
        self.effects = Some(Effects::new(&mut gpu.world));
        self.actors = Some(Actors::new(&mut gpu.world));
        #[cfg(not(target_arch = "wasm32"))]
        self.robots_ready(&mut gpu, robots::load::read());
        if let Some(window) = &self.window {
            let size = window.inner_size();
            gpu.resize(size.width, size.height);
            window.request_redraw();
        }
        self.gpu = Some(gpu);
        self.draw_map();
        self.last_frame = Instant::now();
    }

    /// The kit is read (or failed): its robots go into `gpu`'s world, or the HUD says why the
    /// fighters stay boxes.
    fn robots_ready(&mut self, gpu: &mut Gpu, kit: Result<robots::kit::KitData, String>) {
        match kit.and_then(|kit| Robots::new(&mut gpu.world, kit)) {
            Ok(robots) => {
                self.robots = Some(robots);
                self.kit_status = None;
            }
            Err(error) => {
                crate::log(&error);
                self.kit_status = Some(format!("{error}: box placeholders stand in"));
            }
        }
    }

    /// In the browser: the kit fetch is done and the GPU is up.
    #[cfg(target_arch = "wasm32")]
    fn poll_kit(&mut self) {
        if self.gpu.is_none() {
            return;
        }
        let Some(fetched) = self.kit_inbox.borrow_mut().take() else {
            return;
        };
        let kit = fetched.and_then(|(plans, files)| robots::load::finish(&plans, &files));
        if let Some(mut gpu) = self.gpu.take() {
            self.robots_ready(&mut gpu, kit);
            self.gpu = Some(gpu);
        }
    }

    /// Puts the match's arena into the GPU's world the first time both are there. A host keeps
    /// a match on one map, so it is drawn once.
    fn draw_map(&mut self) {
        let (Some(gpu), Some(map), None) = (&mut self.gpu, self.game.map(), self.drawn) else {
            return;
        };
        scene::build(&mut gpu.world, &map.arena());
        self.drawn = Some(map);
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

    /// One local tick: the controls become an intent, the match ticks, and its events play.
    fn tick(&mut self, now: Instant) -> bool {
        let (forward, strafe) = self.controls.movement();
        let intent = if let Some(tick) = &mut self.scripted {
            let intent = scripted_intent(0, *tick);
            *tick += 1;
            self.look = Look::new(radians(intent.yaw));
            intent
        } else {
            // The wheel and Q/E step through the weapons the robot still has, from the one it
            // holds as predicted; the host decides.
            let steps = self.controls.take_steps();
            let select = match self.game.local_fighter() {
                Some((me, _)) => self.selection.intent(
                    self.controls.selected(),
                    steps,
                    me.arsenal.held,
                    |weapon| me.parts.armed(&me.build, weapon),
                ),
                None => {
                    self.selection = Selection::default();
                    self.controls.selected()
                }
            };
            Intent {
                forward,
                strafe,
                yaw: self.look.sim_yaw(),
                jump: self.controls.jump_held(),
                sprint: self.controls.sprint_held(),
                crouch: self.controls.crouch_held(),
                pitch: self.look.sim_pitch(),
                // Only a captured mouse shoots: the click that captures it does not.
                fire: self.captured && self.controls.fire_held(),
                melee: self.captured && self.controls.melee_held(),
                select,
            }
        };
        let tick = match self.game.tick(intent) {
            Ok(tick) => tick,
            Err(error) => {
                crate::log(&format!("the match failed: {error}"));
                return false;
            }
        };
        let local = self.game.local_slot();
        self.previous_eye = self.eye;
        let target = if self.game.local_sliding() {
            SLIDE_EYE_HEIGHT_UNITS
        } else {
            EYE_HEIGHT_UNITS
        };
        #[allow(clippy::cast_precision_loss)]
        let target = target as f32;
        // The camera dips into a slide and rises out of it over a few ticks.
        self.eye += (target - self.eye).clamp(-EYE_DIP_PER_TICK, EYE_DIP_PER_TICK);
        self.previous_center = self.center;
        let respawned = tick
            .events
            .iter()
            .any(|event| matches!(event, Event::Spawned { slot, .. } if Some(*slot) == local));
        if let Some(center) = self.game.local_center() {
            if respawned {
                self.previous_center = center;
                self.follow.reset();
            }
            self.center = center;
        }
        let moved = (self.center[0] - self.previous_center[0])
            .hypot(self.center[2] - self.previous_center[2]);
        let elapsed_ms = (now - self.started).as_secs_f64() * 1_000.0;
        self.sounds
            .tick(tick.report, moved, &tick.events, local, elapsed_ms);
        let seconds = 1.0 / f32::from(TICK_HZ);
        for glow in &mut self.glows {
            *glow = (*glow - 4.0 * seconds).max(0.0);
        }
        for swing in &mut self.swings {
            *swing += seconds;
        }
        self.local_swing += seconds;
        self.tracker
            .tick(&tick.events, local, tick.report.fired, seconds);
        if tick.report.fired == Some(Weapon::EnergyBlade) {
            self.local_swing = 0.0;
        }
        for event in &tick.events {
            match *event {
                Event::Hit { victim, .. } => self.glows[usize::from(victim)] = 1.0,
                Event::Fired {
                    slot,
                    weapon: Weapon::EnergyBlade,
                    ..
                } if Some(slot) != local => self.swings[usize::from(slot)] = 0.0,
                Event::Spawned { slot, yaw } if Some(slot) == local => {
                    self.look = Look::new(radians(yaw));
                }
                _ => {}
            }
        }
        if let Some(snapshot) = self.game.snapshot() {
            self.hud.events(snapshot, &tick.events, local);
            if self.gpu.is_some() && self.effects.is_some() {
                self.pending.push(Pending {
                    events: tick.events.clone(),
                    rockets: snapshot.rockets.clone(),
                    pellets: snapshot.pellets.clone(),
                });
            }
        }
        true
    }

    fn frame(&mut self) {
        let now = Instant::now();
        let seconds = (now - self.last_frame).as_secs_f32().min(0.25);
        self.last_frame = now;
        self.advance(now, seconds);
    }

    /// `seconds` of play: the ticks they hold, then the drawing. False once the match fails.
    fn advance(&mut self, now: Instant, seconds: f32) -> bool {
        #[cfg(target_arch = "wasm32")]
        self.poll_kit();
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
            if !self.tick(now) {
                return false;
            }
        }
        let ticks = ticks.min(MAX_TICKS_PER_FRAME);
        self.draw_map();

        let blend = self.accumulator / tick;
        let center = Vec3::from(self.previous_center).lerp(Vec3::from(self.center), blend);
        #[allow(clippy::cast_precision_loss)]
        let eye_above_center = self.previous_eye + (self.eye - self.previous_eye) * blend
            - PLAYER_HALF_EXTENTS.y as f32;
        let eye = (center + Vec3::Y * eye_above_center) * METRES_PER_UNIT;
        let forward = Vec3::from(self.look.forward());
        let (_, right, _) = camera::basis(forward);
        self.sounds.listen(eye.to_array(), right.to_array());
        self.sounds.pump();
        let camera = self.follow.place(self.game.world(), eye, forward, seconds);

        let local = self.game.local_slot();
        let snapshot = self.game.snapshot();
        let rockets: Vec<Vec3> = snapshot
            .map(|snapshot| {
                snapshot
                    .rockets
                    .iter()
                    .map(|rocket| Vec3::from(rocket.position) * METRES_PER_UNIT)
                    .collect()
            })
            .unwrap_or_default();
        // The modules lying on their pads (ready to take).
        #[allow(clippy::cast_precision_loss)]
        let modules: Vec<(Vec3, ArenaModule)> = snapshot
            .iter()
            .flat_map(|snapshot| &snapshot.pickups)
            .filter(|pickup| pickup.ready_in == 0)
            .map(|pickup| {
                let center = [pickup.center.x, pickup.center.y, pickup.center.z];
                (
                    Vec3::from(center.map(|axis| axis as f32)) * METRES_PER_UNIT,
                    pickup.module,
                )
            })
            .collect();
        let me = local.and_then(|slot| snapshot.and_then(|snapshot| snapshot.slot(slot)));
        let alive = me.filter(|me| me.alive && self.game.local_center().is_some());
        // The robots' poses: each fighter shown alive, and the dead where they fell.
        let mut robot_views: Vec<RobotView> = Vec::new();
        #[allow(clippy::cast_precision_loss)]
        let mut fighters: Vec<FighterView> = alive
            .map(|me| FighterView {
                // Not drawn while the camera is pulled into it.
                center: (self.follow.reach() >= camera::HIDE_WITHIN)
                    .then(|| center * METRES_PER_UNIT),
                eye_above_center: eye_above_center * METRES_PER_UNIT,
                forward,
                health: crate::hud::chassis_share(me),
                glow: self.glows[usize::from(me.slot)],
                held: me.held,
                hook: self
                    .game
                    .local_hook()
                    .map(|hook| Vec3::from(hook) * METRES_PER_UNIT),
                swing: swing_progress(self.local_swing),
                slot: me.slot,
                robot: false,
                grapple_from: None,
            })
            .into_iter()
            .collect();
        if let (Some(me), Some(view)) = (alive, fighters.first()) {
            let at = center * METRES_PER_UNIT;
            self.tracker.seen(me.slot, at, forward, me.build, me.held);
            let frame = robots::track::rest_frame(&me.build, at, forward);
            self.tracker.parts(me.slot, &me.build, &me.parts, frame);
            if view.center.is_some() {
                let moved = Vec3::from(self.center) - Vec3::from(self.previous_center);
                let (fired, shots) = self.tracker.fired(me.slot);
                robot_views.push(RobotView {
                    slot: me.slot,
                    build: me.build,
                    state: RobotState {
                        center: at,
                        forward,
                        velocity: moved * f32::from(TICK_HZ) * METRES_PER_UNIT,
                        sliding: self.game.local_sliding(),
                        hook: view.hook,
                        since_fired: self.tracker.since_fired(me.slot),
                        fired,
                        shots,
                        held: me.held,
                        energy: energy_share(me),
                        swing: view.swing,
                        dead_for: None,
                    },
                    glow: view.glow,
                    parts: robots::PartStates::default(),
                });
            }
        }
        // Where the others' parts are drawn (units), which is where the host checks a hitscan shot.
        let mut seen = Vec::new();
        #[allow(clippy::cast_precision_loss)]
        for fighter in snapshot.iter().flat_map(|snapshot| &snapshot.fighters) {
            if Some(fighter.slot) == local {
                continue;
            }
            let slot = usize::from(fighter.slot);
            let drawn = self.game.other_center(fighter.slot, blend);
            let view = FighterView {
                center: drawn.map(|center| Vec3::from(center) * METRES_PER_UNIT),
                eye_above_center: EYE_ABOVE_CENTER,
                forward: looking(fighter.yaw, fighter.pitch),
                health: crate::hud::chassis_share(fighter),
                glow: self.glows[slot],
                held: fighter.held,
                hook: fighter
                    .hook
                    .map(|hook| Vec3::from(hook.anchor_units()) * METRES_PER_UNIT),
                swing: swing_progress(self.swings[slot]),
                slot: fighter.slot,
                robot: false,
                grapple_from: None,
            };
            if let Some(center) = drawn.filter(|_| fighter.alive) {
                seen.push((
                    slot,
                    Hitboxes::place(&fighter.build, &fighter.parts, center, fighter.yaw),
                ));
                let at = Vec3::from(center) * METRES_PER_UNIT;
                self.tracker
                    .seen(fighter.slot, at, view.forward, fighter.build, fighter.held);
                let frame = robots::track::rest_frame(&fighter.build, at, view.forward);
                self.tracker
                    .parts(fighter.slot, &fighter.build, &fighter.parts, frame);
                let (fired, shots) = self.tracker.fired(fighter.slot);
                robot_views.push(RobotView {
                    slot: fighter.slot,
                    build: fighter.build,
                    state: RobotState {
                        center: at,
                        forward: view.forward,
                        velocity: Vec3::from(fighter.velocity)
                            * f32::from(TICK_HZ)
                            * METRES_PER_UNIT,
                        sliding: fighter.motion.sliding,
                        hook: view.hook,
                        since_fired: self.tracker.since_fired(fighter.slot),
                        fired,
                        shots,
                        held: fighter.held,
                        energy: energy_share(fighter),
                        swing: view.swing,
                        dead_for: None,
                    },
                    glow: view.glow,
                    parts: robots::PartStates::default(),
                });
            }
            fighters.push(view);
        }
        for (slot, remains) in self.tracker.dead() {
            robot_views.push(RobotView {
                slot,
                build: remains.build,
                state: RobotState {
                    center: remains.center,
                    forward: remains.forward,
                    velocity: Vec3::ZERO,
                    sliding: false,
                    hook: None,
                    since_fired: f32::INFINITY,
                    fired: remains.held,
                    shots: 0,
                    held: remains.held,
                    energy: 0.0,
                    swing: None,
                    dead_for: Some(remains.dead_for),
                },
                glow: 0.0,
                parts: robots::PartStates::default(),
            });
        }
        // The reticle covers what a shot fired now meets: from the eye along the aim the
        // simulation gets, against the arena and the others as drawn.
        let reticle = match (alive, self.game.world()) {
            (Some(me), Some(world)) => {
                let sight = aim::sight(
                    world,
                    (eye / METRES_PER_UNIT).to_array(),
                    weapons::aim(self.look.sim_yaw(), self.look.sim_pitch()),
                    RETICLE_RANGE_UNITS,
                    body_of(me.slot),
                    |body| slot_of(body).is_none(),
                    seen.iter().map(|(slot, parts)| (*slot, parts)),
                );
                Some(camera::reticle(
                    camera,
                    Vec3::from(sight.end) * METRES_PER_UNIT,
                    forward,
                ))
            }
            _ => None,
        };

        // Each robot's lost parts (falling or fallen where they came off) and its damaged ones,
        // from the host's part state.
        let part_states: Vec<(Detached, Vec<PartId>)> = robot_views
            .iter()
            .map(|view| {
                let damaged = snapshot
                    .and_then(|snapshot| snapshot.slot(view.slot))
                    .filter(|fighter| fighter.alive && view.state.dead_for.is_none())
                    .map(damaged_parts)
                    .unwrap_or_default();
                (self.tracker.detached(view.slot), damaged)
            })
            .collect();
        let robot_views: Vec<RobotView> = robot_views
            .iter()
            .zip(&part_states)
            .map(|(view, (detached, damaged))| RobotView {
                parts: robots::PartStates {
                    detached,
                    damaged,
                    ..view.parts
                },
                ..*view
            })
            .collect();
        let drawn = match (&mut self.gpu, &mut self.robots) {
            (Some(gpu), Some(robots)) => robots.update(&mut gpu.world, &robot_views, seconds),
            _ => Ok(Vec::new()),
        };
        let drawn = drawn.unwrap_or_else(|error| {
            // A build the kit cannot draw: every fighter falls back to its box, and says why.
            crate::log(&error);
            self.robots = None;
            self.kit_status = Some(format!("{error}: box placeholders stand in"));
            Vec::new()
        });
        if let (Some(gpu), Some(effects)) = (&mut self.gpu, &mut self.effects) {
            let muzzle = |slot: u8, weapon: Weapon| {
                drawn
                    .iter()
                    .find(|robot| robot.slot == slot)?
                    .muzzles
                    .iter()
                    .find(|muzzle| muzzle.weapon == weapon)
                    .copied()
            };
            for tick in self.pending.drain(..) {
                effects.play(&mut gpu.world, &tick.events, muzzle);
                effects.rockets(&mut gpu.world, &tick.rockets);
                effects.pellets(&mut gpu.world, &tick.pellets);
            }
        }
        for robot in &drawn {
            if let Some(view) = fighters.iter_mut().find(|view| view.slot == robot.slot) {
                view.robot = true;
                view.grapple_from = robot.grapple;
            }
        }
        let mut text = self
            .hud
            .text(snapshot, local, self.game.status(), self.board, seconds);
        text.robot = self.kit_status.clone().unwrap_or_else(|| {
            drawn
                .iter()
                .find(|robot| Some(robot.slot) == local)
                .map(robot_line)
                .unwrap_or_default()
        });
        #[cfg(target_arch = "wasm32")]
        crate::hud::show(&text, &mut self.shown);
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(window) = &self.window {
            crate::hud::show(&text, &mut self.shown, window);
        }

        let point_lights = self
            .effects
            .as_mut()
            .map(|effects| effects.lights(seconds, &rockets))
            .unwrap_or_default();
        let mut scene = scene::frame(camera, forward, point_lights);
        // The drawn HUD: the held weapon (as predicted), the carried ones, chassis and energy.
        let vitals = self
            .game
            .local_fighter()
            .map(|(me, tick)| Vitals::of(me, tick));
        let Some(gpu) = &mut self.gpu else {
            return true;
        };
        let (width, height) = gpu.size();
        scene.overlay = self.overlay.draw(vitals.as_ref(), width, height, seconds);
        if let Some(actors) = &mut self.actors {
            actors.update(&mut gpu.world, &fighters, &rockets, &modules, reticle);
        }
        if let Err(error) = gpu.render(&scene, ticks) {
            crate::log(&error);
        }
        true
    }
}

/// One tick's effects, started after the frame's robots are posed.
struct Pending {
    events: Vec<Event>,
    rockets: Vec<shooter_sim::wire::RocketState>,
    pellets: Vec<shooter_sim::wire::PelletState>,
}

/// A robot's lost parts and where each one's robot frame is ([`robots::PartStates::detached`]).
type Detached = Vec<(PartId, Mat4)>;

/// `fighter`'s energy as a share of its pool's capacity (its weapons' charge glow).
fn energy_share(fighter: &FighterState) -> f32 {
    let pool = shooter_sim::energy::Pool::attached(&fighter.build, &fighter.parts);
    #[allow(clippy::cast_precision_loss)]
    let share = fighter.energy as f32 / pool.capacity.max(1) as f32;
    share.clamp(0.0, 1.0)
}

/// The parts `fighter` still has that have taken damage (drawn in their damaged variant where
/// the kit has one).
fn damaged_parts(fighter: &FighterState) -> Vec<PartId> {
    let whole = PartDamage::fresh(&fighter.build);
    fighter
        .build
        .parts()
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            fighter.parts.attached(*index)
                && fighter.parts.health()[*index] < whole.health()[*index]
        })
        .map(|(_, part)| part.id)
        .collect()
}

/// The HUD's robot line for the local fighter: its animation state and its placeholder parts,
/// "robot: run (walk-biped) + fire · placeholders (asset-tooling#634): robot.wheel, …".
fn robot_line(robot: &robots::Drawn) -> String {
    let mut owners: Vec<&str> = robot.placeholders.iter().map(|(_, owner)| *owner).collect();
    owners.dedup();
    let placeholders: Vec<String> = owners
        .iter()
        .map(|owner| {
            let keys: Vec<&str> = robot
                .placeholders
                .iter()
                .filter(|(_, by)| by == owner)
                .map(|(key, _)| *key)
                .collect();
            format!(" · placeholders ({owner}): {}", keys.join(", "))
        })
        .collect();
    format!("robot: {}{}", robot.label, placeholders.concat())
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
        let Some(proxy) = self.proxy.clone() else {
            return;
        };
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
            Ok(gpu) => self.attach(gpu),
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
                self.board = false;
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
            // The wheel cycles weapons while the game has the mouse (winit reports the browser's
            // wheel events here too, in lines or pixels as the page got them).
            WindowEvent::MouseWheel { delta, .. } if self.captured => {
                let delta = match delta {
                    MouseScrollDelta::LineDelta(_, lines) => WheelDelta::Lines(lines),
                    MouseScrollDelta::PixelDelta(pixels) => WheelDelta::Pixels(pixels.y),
                };
                let now_ms = (Instant::now() - self.started).as_secs_f64() * 1_000.0;
                self.controls.wheel(delta, now_ms);
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
                if code == KeyCode::Tab {
                    self.board = event.state == ElementState::Pressed;
                    return;
                }
                if event.state == ElementState::Pressed {
                    self.controls.key_down(code, event.repeat);
                } else {
                    self.controls.key_up(code);
                }
            }
            #[cfg(target_arch = "wasm32")]
            WindowEvent::RedrawRequested if self.profile.is_some() => {
                if self.profiled_frame()
                    && let Some(window) = &self.window
                {
                    window.request_redraw();
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
