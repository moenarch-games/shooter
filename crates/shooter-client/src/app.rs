//! The window, the fixed-tick loop and the frame: input in, the match ticks (in-process practice
//! or a hosted match), render out.

use crate::arena_art::ArenaArt;
use crate::audio::Sounds;
use crate::builder::saved::{self, SavedBuild, SavedBuilds};
use crate::builder::view::{Action, Builder, Stage, death_prompt};
use crate::camera::{self, Follow};
use crate::controls::{Controls, Look, PRIMARY_BUTTON, PadState, Selection, WheelDelta, standard};
use crate::effects::Effects;
use crate::game::Game;
use crate::gpu::Gpu;
use crate::hud::{Hud, HudText};
use crate::menu::{Choice, Menu, Previews};
use crate::overlay::{Overlay, Vitals, repair_panel};
use crate::prefs::PrefStore;
use crate::repair_menu::RepairMenu;
use crate::robots::{self, RobotState, RobotView, Robots, track::Tracker};
use crate::scene::{
    self, Actors, DrawnArena, EYE_ABOVE_CENTER, FighterView, METRES_PER_UNIT, SWING_SECONDS,
};
use render_engine::glam::{Mat4, Vec3};
use shooter_sim::damage::{Hitboxes, PartDamage};
use shooter_sim::physics::Vec3i;
use shooter_sim::player::{EYE_HEIGHT_UNITS, PLAYER_HALF_EXTENTS, SLIDE_EYE_HEIGHT_UNITS};
use shooter_sim::robot::PartId;
use shooter_sim::scenarios::{Staged, scripted_intent};
use shooter_sim::{ArenaModule, Event, FighterState, Intent, MapId, Select, TICK_HZ, Weapon};
use std::rc::Rc;
use std::sync::Arc;
use web_time::Instant;
use winit::application::ApplicationHandler;
use winit::event::{
    DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent,
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

/// Where the browser fetches the kits: `robot-kit/`, `weapon-kit/`, `locomotion-kit/` and
/// `arena-kit/` beside the page (`scripts/build-web.sh`).
#[cfg(target_arch = "wasm32")]
const KIT_URL: &str = "";

/// The HUD's note while the kit is not drawn yet.
const KIT_LOADING: &str = "robot kit loading: box placeholders stand in";

/// Starts the browser's fetch of `bundles` (one load).
#[cfg(target_arch = "wasm32")]
fn fetch_kit(bundles: &'static [robots::kit::Bundle]) -> KitInbox {
    let inbox = KitInbox::default();
    let filled = inbox.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let fetched = robots::load::fetch(KIT_URL, bundles).await;
        *filled.borrow_mut() = Some(fetched);
    });
    inbox
}

/// Never simulate more than this many ticks in one frame (after a stall, time is dropped).
const MAX_TICKS_PER_FRAME: u32 = 8;

/// How long a match started from the menu says what it is (and always while the pointer is
/// free).
const MATCH_LABEL_SECONDS: f32 = 6.0;

/// `build` if the beta catalog takes it, else the standard build: a match from the menu always
/// starts on a valid robot.
fn valid_or_standard(build: shooter_sim::Assembly) -> shooter_sim::Assembly {
    match shooter_sim::robot::BETA.validate(&build.candidate()) {
        Ok(build) => build,
        Err(error) => {
            crate::log(&format!(
                "the build is refused ({error}): the standard build plays"
            ));
            shooter_sim::Assembly::default()
        }
    }
}

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
    /// The profile run's staged scene, if any: the local fighter stands where it says and plays
    /// its intent instead of [`scripted_intent`] (the `cover` scene, shooter#102; `swing`, #104).
    stage: Option<Staged>,
    window: Option<Arc<Window>>,
    gpu: Option<Gpu>,
    /// The match, while one plays; none in the main menu and the Workshop (shooter#154).
    game: Option<Game>,
    /// The main menu: shown while there is neither a match nor the builder.
    menu: Menu,
    /// The maps' previews in the GPU's world, once it is up.
    previews: Option<Previews>,
    /// The robot the next match from the menu plays (the Workshop's Done, the last Ready; kept
    /// across restarts, shooter#155), the save it was readied as (empty: none), and its control
    /// layout is the driver's.
    active: shooter_sim::Assembly,
    active_name: String,
    /// Where the preferences and the active build are kept, and the saved builds.
    prefs: Rc<dyn PrefStore>,
    builds: Rc<dyn SavedBuilds>,
    /// Seconds the menu's live view has run (its camera's slow turn).
    menu_seconds: f32,
    /// What the match started from the menu is (Play Online's bot match says it is offline),
    /// and seconds since it started.
    match_label: Option<String>,
    match_seconds: f32,
    /// The pointer in the window's pixels (the Leave button while the pointer is free).
    cursor: render_engine::glam::Vec2,
    /// The map drawn in the GPU's world (the match's, or the menu's live view's), once the GPU
    /// is up, and where its robot stands in the menu's view.
    drawn: Option<MapId>,
    backdrop: Backdrop,
    /// Its instances; its kit-built modules drawn as the match's snapshots say they stand
    /// (broken or whole).
    arena: Option<DrawnArena>,
    /// What the page last published of the menu or match (`window.shooterMenu`).
    #[cfg(target_arch = "wasm32")]
    menu_readout: String,
    /// What the page shows (`data-menu` on its body while the menu or Workshop is open).
    #[cfg(target_arch = "wasm32")]
    page_menu: bool,
    controls: Controls,
    look: Look,
    pads: Option<gilrs::Gilrs>,
    sounds: Sounds,
    hud: Hud,
    /// The drawn HUD: the weapon panel and the chassis and energy bars.
    overlay: Overlay,
    /// The weapon the controls ask for (a select key, or the next and previous steps).
    selection: Selection,
    repair_menu: RepairMenu,
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
    /// The arena kit in the GPU's world (a kit-built map waits for it), or why not; None while
    /// it loads.
    arena_art: Option<Result<ArenaArt, String>>,
    /// The browser's arena kit fetch, once it is done.
    #[cfg(target_arch = "wasm32")]
    arena_inbox: KitInbox,
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
    /// Precision aim's zoom (shooter#132): held, the view narrows and the mouse slows.
    precision: camera::Precision,
    /// The robot builder while it is open: before the match (nothing plays) or between lives
    /// (the robot dead, its respawn held until Ready).
    builder: Option<Builder>,
    /// Whether the page shows the builder (the browser hides its own overlays then).
    #[cfg(target_arch = "wasm32")]
    builder_shown: bool,
    /// What the page last published of the open builder (`window.shooterBuilder`).
    #[cfg(target_arch = "wasm32")]
    builder_readout: String,
    /// The page's touch controls held as keys and buttons (phones and tablets).
    #[cfg(target_arch = "wasm32")]
    touch: crate::touch::Touch,
    /// The pointer is locked to the game (native: grabbed; web: Pointer Lock), or the touch
    /// controls play.
    captured: bool,
    /// The person's control layout for the robot's actuators (shooter#74).
    driver: Driver,
    /// The browser profile run (`?frames=<n>`), timing each animation frame.
    #[cfg(target_arch = "wasm32")]
    profile: Option<crate::web::Profile>,
}

/// The person's control layout (Atlas D2.i70.7, [`shooter_sim::bindings`]) and what it needs
/// between ticks: the layout edited in the builder (none: the build's preset), its toggles and
/// cursor offsets, the mouse's movement since the last tick, and the robot's connected
/// actuators with the layout pruned to them. The same natively and in the browser: the mouse
/// counts arrive as the look's do (pointer-locked movement), so a cursor offset is the sum of
/// that movement, not a screen position.
#[derive(Debug, Default)]
struct Driver {
    layout: Option<shooter_sim::bindings::Bindings>,
    state: shooter_sim::bindings::BindingState,
    mouse: [f64; 2],
    /// The build and lost parts the actuators and the effective layout are for.
    resolved: Option<(
        shooter_sim::Assembly,
        u32,
        shooter_sim::mechanics::Layout,
        shooter_sim::bindings::Bindings,
    )>,
}

impl Driver {
    /// A new layout from the builder: its toggles and offsets start over.
    fn adopt(&mut self, layout: shooter_sim::bindings::Bindings) {
        self.layout = Some(layout);
        self.state = shooter_sim::bindings::BindingState::default();
        self.resolved = None;
    }

    /// The mouse moved (pointer locked).
    fn mouse(&mut self, dx: f64, dy: f64) {
        self.mouse[0] += dx;
        self.mouse[1] += dy;
    }

    /// The whole counts the mouse moved since the last tick (the rest carries over).
    #[allow(clippy::cast_possible_truncation)]
    fn take_mouse(&mut self) -> [i32; 2] {
        let whole = self.mouse.map(f64::trunc);
        self.mouse = [self.mouse[0] - whole[0], self.mouse[1] - whole[1]];
        whole.map(|counts| counts.clamp(-10_000.0, 10_000.0) as i32)
    }

    /// This tick's commands for the robot `build` with parts `lost`, and the guns it fires,
    /// from what is `pressed`.
    fn actions(
        &mut self,
        build: &shooter_sim::Assembly,
        lost: u32,
        pressed: shooter_sim::bindings::Pressed,
    ) -> shooter_sim::bindings::Actions {
        let stale = self
            .resolved
            .as_ref()
            .is_none_or(|(had, gone, ..)| had != build || *gone != lost);
        if stale {
            let actuators = shooter_sim::mechanics::Layout::connected(build, lost);
            let effective = self.layout.as_ref().map_or_else(
                || shooter_sim::bindings::Bindings::preset(&actuators),
                |layout| layout.pruned(&actuators),
            );
            self.resolved = Some((*build, lost, actuators, effective));
        }
        let Some((_, _, actuators, effective)) = &self.resolved else {
            return shooter_sim::bindings::Actions::default();
        };
        effective.act(actuators, &mut self.state, pressed)
    }
}

/// Where the menu's live view stands the person's robot in the drawn arena (metres): its centre
/// on open floor (on the assembly works the floor west of the bridge, looking in along it as
/// `--example arena_views` does; elsewhere by the spawn point nearest the middle), the flat way
/// from it to the camera, and the arena's half size.
#[derive(Clone, Copy, Debug, Default)]
struct Backdrop {
    robot: Vec3,
    view: Vec3,
    half: f32,
}

impl Backdrop {
    fn of(map: MapId, arena: &shooter_sim::Arena) -> Self {
        #[allow(clippy::cast_precision_loss)]
        let half = arena.half_size as f32 * METRES_PER_UNIT;
        let (feet, view) = if map == MapId::AssemblyWorks {
            (Vec3::new(-9.5, 0.0, 1.0), Vec3::new(-1.0, 0.0, 0.35))
        } else {
            let spawn = arena
                .spawn_points()
                .into_iter()
                .min_by_key(|spawn| i64::from(spawn.feet.x).pow(2) + i64::from(spawn.feet.z).pow(2))
                .unwrap_or(shooter_sim::arena::Spawn {
                    feet: arena.spawn,
                    yaw: arena.spawn_yaw,
                });
            #[allow(clippy::cast_precision_loss)]
            let feet = Vec3::new(
                spawn.feet.x as f32,
                spawn.feet.y as f32,
                spawn.feet.z as f32,
            ) * METRES_PER_UNIT;
            (feet, -Vec3::new(feet.x, 0.0, feet.z))
        };
        Self {
            robot: feet + Vec3::Y * robots::pose::HALF_HEIGHT,
            view: view.try_normalize().unwrap_or(Vec3::Z),
            half,
        }
    }
}

impl App {
    /// The app on `game`, or (none) on the main menu, its matches playing the active build kept
    /// from before ([`saved::read_active`]).
    #[must_use]
    pub fn new(proxy: EventLoopProxy<GpuReady>, game: Option<Game>) -> Self {
        let now = Instant::now();
        let center = game
            .as_ref()
            .and_then(Game::local_center)
            .unwrap_or_default();
        let yaw = game
            .as_ref()
            .and_then(Game::map)
            .map_or(0, |map| map.arena().spawn_yaw);
        let prefs = crate::prefs::platform();
        let mut app = Self {
            proxy: Some(proxy),
            scripted: None,
            stage: None,
            window: None,
            gpu: None,
            look: Look::new(radians(yaw)),
            game,
            menu: Menu::new(prefs.clone()),
            previews: None,
            active: shooter_sim::Assembly::default(),
            active_name: String::new(),
            prefs,
            builds: saved::platform(),
            menu_seconds: 0.0,
            match_label: None,
            match_seconds: 0.0,
            cursor: render_engine::glam::Vec2::ZERO,
            drawn: None,
            backdrop: Backdrop::default(),
            arena: None,
            #[cfg(target_arch = "wasm32")]
            menu_readout: String::new(),
            #[cfg(target_arch = "wasm32")]
            page_menu: false,
            controls: Controls::new(),
            pads: gilrs::Gilrs::new()
                .inspect_err(|error| crate::log(&format!("gamepads unavailable: {error}")))
                .ok(),
            sounds: Sounds::new(),
            hud: Hud::default(),
            overlay: Overlay::default(),
            selection: Selection::default(),
            repair_menu: RepairMenu::default(),
            shown: HudText::default(),
            board: false,
            effects: None,
            actors: None,
            robots: None,
            kit_status: Some(KIT_LOADING.to_owned()),
            #[cfg(target_arch = "wasm32")]
            kit_inbox: fetch_kit(&robots::kit::Bundle::FIGHTERS),
            arena_art: None,
            #[cfg(target_arch = "wasm32")]
            arena_inbox: fetch_kit(&[robots::kit::Bundle::Arena]),
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
            precision: camera::Precision::default(),
            builder: None,
            #[cfg(target_arch = "wasm32")]
            builder_shown: false,
            #[cfg(target_arch = "wasm32")]
            builder_readout: String::new(),
            #[cfg(target_arch = "wasm32")]
            touch: crate::touch::Touch::default(),
            captured: false,
            driver: Driver::default(),
            #[cfg(target_arch = "wasm32")]
            profile: None,
        };
        app.restore_active();
        app
    }

    /// Opens the builder before the match on `build` (shooter#57): nothing plays, and online
    /// nothing connects, until Ready.
    #[must_use]
    pub fn with_builder(mut self, build: shooter_sim::Assembly) -> Self {
        self.builder = Some(self.builder_on(build, Stage::Setup));
        self
    }

    /// The main menu's matches play `build` (`--loadout` / `?loadout`) with its preset controls
    /// for this run, until the Workshop or a Ready changes it, the active build kept from before
    /// left as it is; a build the beta catalog refuses falls back to the standard one.
    #[must_use]
    pub fn with_active(mut self, build: shooter_sim::Assembly) -> Self {
        self.active = valid_or_standard(build);
        self.active_name.clear();
        self.driver = Driver::default();
        self.show_active();
        self
    }

    /// The active build kept from before (shooter#155), with its control layout: none kept is
    /// the standard build, and one that no longer plays is too, the menu saying why (it stays
    /// kept until the next Done or Ready replaces it).
    fn restore_active(&mut self) {
        match saved::read_active(self.prefs.as_ref()) {
            None => {}
            Some(Ok(active)) => {
                self.active = active.build;
                self.active_name = active.name;
                match active.layout {
                    Some(layout) => self.driver.adopt(layout),
                    None => self.driver = Driver::default(),
                }
            }
            Some(Err(error)) => {
                let notice = format!(
                    "Your active robot no longer plays ({error}): the standard build plays"
                );
                crate::log(&notice);
                self.menu.notice = Some(notice);
            }
        }
        self.show_active();
    }

    /// The menu's robot line for the active build.
    fn show_active(&mut self) {
        let name = (!self.active_name.is_empty()).then_some(self.active_name.as_str());
        self.menu
            .set_robot(crate::menu::robot_line(&self.active, name));
    }

    /// The builder on `build` for `stage`, with the person's control layout and saved builds.
    fn builder_on(&self, build: shooter_sim::Assembly, stage: Stage) -> Builder {
        Builder::new(build, stage)
            .with_store(self.builds.clone())
            .with_controls(self.driver.layout.as_ref())
    }

    /// Opens the builder between lives on the build the robot comes back as: its respawn waits
    /// until Ready.
    fn edit_robot(&mut self) {
        if self.builder.is_some() {
            return;
        }
        let build = self
            .game
            .as_ref()
            .and_then(Game::configured)
            .unwrap_or_default();
        self.builder = Some(self.builder_on(build, Stage::BetweenLives));
        self.capture(false);
        self.controls.release_all();
    }

    /// Whether the local robot is dead in the match (its respawn counting down).
    fn local_dead(&self) -> bool {
        let Some(game) = &self.game else {
            return false;
        };
        let Some(slot) = game.local_slot() else {
            return false;
        };
        game.snapshot()
            .and_then(|snapshot| snapshot.slot(slot))
            .is_some_and(|me| !me.alive)
    }

    /// What the builder asked: Ready sends the build and takes the control layout (none: the
    /// build's preset). Before the match the match starts; between lives the builder waits
    /// until the match shows the build configured. In the Workshop, Done keeps the build for the
    /// menu's next match and Back drops the edit; either returns to the menu. Whatever was
    /// readied is the active build from then on, kept across restarts ([`Self::keep_active`]).
    fn builder_action(&mut self, action: Option<Action>) {
        let Some(action) = action else {
            return;
        };
        let workshop = self
            .builder
            .as_ref()
            .is_some_and(|builder| builder.stage == Stage::Workshop);
        let (build, layout) = match action {
            Action::Back => {
                if workshop {
                    self.back_to_menu(None);
                }
                return;
            }
            Action::Ready(build, layout) => (build, layout),
        };
        let unkept = self.keep_active(build, layout);
        if workshop {
            self.back_to_menu(unkept);
            return;
        }
        if let Some(game) = &mut self.game {
            game.configure(build);
        }
        if self
            .builder
            .as_ref()
            .is_some_and(|builder| builder.stage == Stage::Setup)
        {
            self.builder = None;
            self.last_frame = Instant::now();
        }
    }

    /// Makes `build` with `layout` (none: its preset) the active build: the driver takes the
    /// layout and the build is kept across restarts, named after the save holding exactly it
    /// (the builds panel's typed or selected one first), if any. Why it could not be kept, if
    /// it could not: it still plays this run.
    fn keep_active(
        &mut self,
        build: shooter_sim::Assembly,
        layout: Option<shooter_sim::bindings::Bindings>,
    ) -> Option<String> {
        self.active_name = self
            .builder
            .as_ref()
            .and_then(|builder| builder.shelf.saved_as(&build, layout.as_ref()))
            .unwrap_or_default();
        self.active = build;
        let kept = saved::write_active(
            self.prefs.as_ref(),
            &SavedBuild {
                name: self.active_name.clone(),
                build,
                layout: layout.clone(),
            },
        );
        match layout {
            Some(layout) => self.driver.adopt(layout),
            None => self.driver = Driver::default(),
        }
        self.show_active();
        let error = kept.err()?;
        let notice = format!("Your robot plays, but could not be kept for next time: {error}");
        crate::log(&notice);
        Some(notice)
    }

    /// Between lives, the builder closes once the host shows the readied build configured (the
    /// next tick lets go of Edit Robot and the robot comes back), or if the robot is alive
    /// again anyway (a new session after a reconnect).
    fn follow_builder(&mut self) {
        let (Some(builder), Some(game)) = (&self.builder, &self.game) else {
            return;
        };
        if builder.stage != Stage::BetweenLives {
            return;
        }
        let accepted = builder.ready.is_some() && builder.ready == game.configured();
        if accepted || game.local_fighter().is_some() {
            self.builder = None;
        }
    }

    /// What the menu asked for: a match against bots, the Workshop or a hosted match.
    fn menu_choice(&mut self, choice: Option<Choice>) {
        match choice {
            None => {}
            Some(Choice::Play {
                map,
                fighters,
                online,
            }) => {
                let label = if online {
                    format!(
                        "Play Online: bot match, offline for now · {} · {fighters} fighters",
                        crate::menu::map_name(map)
                    )
                } else {
                    format!(
                        "Practice · {} · {fighters} fighters",
                        crate::menu::map_name(map)
                    )
                };
                self.start_match(Game::bot_match(map, fighters), label);
            }
            Some(Choice::Workshop) => {
                self.builder = Some(self.builder_on(self.active, Stage::Workshop));
            }
            Some(Choice::Connect { url, certificate }) => {
                #[cfg(not(target_arch = "wasm32"))]
                let trust =
                    (!certificate.is_empty()).then(|| std::path::PathBuf::from(certificate));
                #[cfg(target_arch = "wasm32")]
                let trust = crate::parse_hash(&certificate);
                let name = self.menu.prefs.name.clone();
                let label = format!("Hosted match at {url}");
                let game = Game::Online(Box::new(crate::game::online::Online::connect(
                    url,
                    trust,
                    name.as_deref(),
                )));
                self.start_match(game, label);
            }
        }
    }

    /// Starts `game` from the menu with the active build and control layout, nothing of the
    /// last match left over; `label` says what it is.
    fn start_match(&mut self, game: Game, label: String) {
        let game = game.with_loadout(self.active);
        self.reset_match();
        let yaw = game.map().map_or(0, |map| map.arena().spawn_yaw);
        self.look = Look::new(radians(yaw));
        let center = game.local_center().unwrap_or_default();
        (self.previous_center, self.center) = (center, center);
        self.game = Some(game);
        self.builder = None;
        #[cfg(target_arch = "wasm32")]
        crate::web::set_mode(&format!("Click to play: {label}"));
        self.match_label = Some(label);
        self.last_frame = Instant::now();
    }

    /// Back to the main menu: the match (and its connection) ends, nothing of it is left drawn,
    /// and the build, control layout, saved builds and preferences stay. `notice` says why, if
    /// it was not the person's choice.
    fn back_to_menu(&mut self, notice: Option<String>) {
        self.game = None;
        self.builder = None;
        self.match_label = None;
        self.reset_match();
        self.capture(false);
        self.menu.reopen();
        self.menu.notice = notice;
        self.show_active();
        if let Some(gpu) = &mut self.gpu {
            if let Some(actors) = &mut self.actors {
                actors.update(&mut gpu.world, &[], &[], &[], None);
            }
            // The arena as authored again, nothing broken.
            if let (Some(Ok(art)), Some(placed)) = (
                &self.arena_art,
                self.arena.as_ref().and_then(|arena| arena.placed.as_ref()),
            ) {
                art.show(&mut gpu.world, placed, &[]);
            }
        }
        #[cfg(target_arch = "wasm32")]
        crate::hud::show(&HudText::default(), &mut self.shown);
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(window) = &self.window {
            window.set_title("shooter");
        }
        self.last_frame = Instant::now();
    }

    /// Forgets everything a match left in the frame's state (the build, layout and drawn arena
    /// stay).
    fn reset_match(&mut self) {
        self.hud = Hud::default();
        self.selection = Selection::default();
        self.repair_menu = RepairMenu::default();
        self.shown = HudText::default();
        self.board = false;
        self.tracker = Tracker::default();
        self.pending.clear();
        self.glows.fill(0.0);
        self.swings.fill(f32::INFINITY);
        self.local_swing = f32::INFINITY;
        self.accumulator = 0.0;
        self.previous_eye = EYE;
        self.eye = EYE;
        self.follow = Follow::default();
        self.precision = camera::Precision::default();
        self.controls.release_all();
        self.driver = Driver {
            layout: self.driver.layout.take(),
            ..Driver::default()
        };
        self.match_seconds = 0.0;
    }

    /// Whether the menu shows: no match and no builder.
    fn in_menu(&self) -> bool {
        self.game.is_none() && self.builder.is_none()
    }

    /// The match's Leave button (to the main menu) while the pointer is free, in the frame's
    /// pixels.
    fn leave_button(&self) -> Option<crate::menu::Rect> {
        if self.game.is_none() || self.builder.is_some() || self.captured {
            return None;
        }
        let (width, height) = self.gpu.as_ref()?.size();
        Some(crate::menu::leave_button(width, height))
    }

    /// The pointer in the frame's pixels.
    #[allow(clippy::cast_precision_loss)]
    fn pointer_pixels(&self) -> render_engine::glam::Vec2 {
        let (Some(window), Some(gpu)) = (&self.window, &self.gpu) else {
            return self.cursor;
        };
        let size = window.inner_size();
        let (width, height) = gpu.size();
        self.cursor
            * render_engine::glam::Vec2::new(
                width as f32 / size.width.max(1) as f32,
                height as f32 / size.height.max(1) as f32,
            )
    }

    /// Whether the pointer is on the Leave button.
    fn on_leave(&self) -> bool {
        self.leave_button()
            .is_some_and(|rect| rect.contains_pixels(self.pointer_pixels()))
    }

    /// The browser profile run ([`crate::web::Profile`]): `frames` animation frames of one
    /// tick each, the local fighter scripted as in the native offscreen run.
    #[cfg(target_arch = "wasm32")]
    #[must_use]
    pub fn profiled(mut self, frames: u32) -> Self {
        self.scripted = Some(0);
        self.profile = Some(crate::web::Profile::new(
            frames,
            self.game
                .as_ref()
                .and_then(Game::map)
                .unwrap_or(MapId::DEFAULT),
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
            // The cover broken by then (shooter#60), for captures to say what they show.
            let broken: Vec<u16> =
                self.game
                    .as_ref()
                    .and_then(Game::snapshot)
                    .map_or_else(Vec::new, |snapshot| {
                        snapshot
                            .scenery
                            .iter()
                            .filter(|placeable| placeable.health == 0)
                            .map(|placeable| placeable.id)
                            .collect()
                    });
            profile.finish(&broken);
            return false;
        }
        true
    }

    /// The offscreen profile run ([`crate::offscreen`]): `game` drawn on `gpu`, the local
    /// fighter scripted, no window, event loop or gamepads.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn scripted(game: Game, gpu: Gpu) -> Self {
        let mut app = Self::offscreen(Some(game), gpu);
        app.scripted = Some(0);
        app
    }

    /// The app drawing to the offscreen `gpu` with no window, event loop or gamepads: `game`,
    /// or (none) the main menu, which [`Self::offscreen_frame`] and [`Self::click`] drive (the
    /// `menu_views` example's captures).
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn offscreen(game: Option<Game>, gpu: Gpu) -> Self {
        Self::offscreen_with(
            game,
            gpu,
            Rc::new(crate::prefs::MemoryPrefs::default()),
            Rc::new(saved::MemoryBuilds::default()),
        )
    }

    /// [`Self::offscreen`] keeping its preferences and active build in `prefs` and its saved
    /// builds in `builds` (in memory for [`Self::offscreen`]: a capture never touches the
    /// person's): an app made again on the same stores is the client restarted.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn offscreen_with(
        game: Option<Game>,
        gpu: Gpu,
        prefs: Rc<dyn PrefStore>,
        builds: Rc<dyn SavedBuilds>,
    ) -> Self {
        let now = Instant::now();
        let center = game
            .as_ref()
            .and_then(Game::local_center)
            .unwrap_or_default();
        let yaw = game
            .as_ref()
            .and_then(Game::map)
            .map_or(0, |map| map.arena().spawn_yaw);
        let mut app = Self {
            proxy: None,
            scripted: None,
            stage: None,
            window: None,
            gpu: None,
            look: Look::new(radians(yaw)),
            game,
            menu: Menu::new(prefs.clone()),
            previews: None,
            active: shooter_sim::Assembly::default(),
            active_name: String::new(),
            prefs,
            builds,
            menu_seconds: 0.0,
            match_label: None,
            match_seconds: 0.0,
            cursor: render_engine::glam::Vec2::ZERO,
            drawn: None,
            backdrop: Backdrop::default(),
            arena: None,
            controls: Controls::new(),
            pads: None,
            sounds: Sounds::new(),
            hud: Hud::default(),
            overlay: Overlay::default(),
            selection: Selection::default(),
            repair_menu: RepairMenu::default(),
            shown: HudText::default(),
            board: false,
            effects: None,
            actors: None,
            robots: None,
            kit_status: Some(KIT_LOADING.to_owned()),
            arena_art: None,
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
            precision: camera::Precision::default(),
            builder: None,
            #[cfg(target_arch = "wasm32")]
            builder_shown: false,
            #[cfg(target_arch = "wasm32")]
            builder_readout: String::new(),
            captured: false,
            driver: Driver::default(),
        };
        app.restore_active();
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

    /// The profile run plays `stage` (a staged scene) instead of the scripted person.
    #[must_use]
    pub const fn staged(mut self, stage: Option<Staged>) -> Self {
        self.stage = stage;
        self
    }

    /// The offscreen run's GPU (`--capture` reads its frames).
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub const fn gpu(&self) -> Option<&Gpu> {
        self.gpu.as_ref()
    }

    /// One frame of `seconds` of an offscreen app ([`Self::offscreen`]).
    ///
    /// # Errors
    ///
    /// If the match fails.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn offscreen_frame(&mut self, seconds: f32) -> Result<(), String> {
        if self.advance(Instant::now(), seconds) {
            Ok(())
        } else {
            Err("the match failed".into())
        }
    }

    /// A click (the primary button down and up) at `x`, `y` in an offscreen app's frame
    /// pixels, as a person's on the menu, the Workshop or a match's Leave button.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn click(&mut self, x: f32, y: f32) {
        self.cursor = render_engine::glam::Vec2::new(x, y);
        self.menu.cursor_moved(x, y);
        if self.in_menu() {
            self.menu.mouse(true);
            let choice = self.menu.mouse(false);
            self.menu_choice(choice);
        } else if let Some(builder) = &mut self.builder {
            builder.cursor_moved(x, y);
            let pressed = builder.mouse(false, true);
            let released = builder.mouse(false, false);
            self.builder_action(pressed.or(released));
        } else if self.on_leave() {
            self.back_to_menu(None);
        }
    }

    /// A key pressed (and typed) in an offscreen app, as a person's on the menu or the Workshop.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn press(&mut self, code: KeyCode) {
        if self.in_menu() {
            let choice = self.menu.key(code, None);
            self.menu_choice(choice);
        } else if let Some(builder) = &mut self.builder {
            let action = builder.key(code);
            self.builder_action(action);
        }
    }

    /// `text` typed in an offscreen app's menu (into the field being edited).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn type_text(&mut self, text: &str) {
        for character in text.chars() {
            self.menu.key(KeyCode::KeyA, Some(&character.to_string()));
        }
    }

    /// Whether an offscreen app's match is on a host and has been admitted (its snapshots name
    /// the map), and its status line.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn hosted(&self) -> Option<(bool, Option<String>)> {
        match &self.game {
            Some(game @ Game::Online(_)) => Some((
                game.local_slot().is_some(),
                game.status().map(str::to_owned),
            )),
            _ => None,
        }
    }

    /// The menu's readout ([`Menu::readout`]): where its items are, in an offscreen app's frame
    /// pixels; none outside the menu.
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn menu_readout(&self) -> Option<String> {
        self.in_menu().then(|| self.menu.readout())
    }

    /// The Leave button of an offscreen app's match while the pointer is free (frame pixels).
    #[cfg(not(target_arch = "wasm32"))]
    #[must_use]
    pub fn leave_at(&self) -> Option<render_engine::glam::Vec2> {
        self.leave_button().map(crate::menu::Rect::centre)
    }

    /// The match's map and fighters, while one plays.
    #[must_use]
    pub fn playing(&self) -> Option<(Option<MapId>, Option<usize>)> {
        self.game.as_ref().map(|game| (game.map(), game.slots()))
    }

    /// The build the local robot of the match playing is configured as, and whether the match
    /// runs in this process (no host, nothing connecting).
    #[must_use]
    pub fn played(&self) -> Option<(Option<shooter_sim::Assembly>, bool)> {
        self.game
            .as_ref()
            .map(|game| (game.configured(), game.in_process()))
    }

    /// The active build the menu's matches play and the save it was readied as (empty: none).
    #[must_use]
    pub fn active(&self) -> (shooter_sim::Assembly, &str) {
        (self.active, &self.active_name)
    }

    /// The open builder's readout ([`Builder::readout`]), as the page's `window.shooterBuilder`.
    #[must_use]
    pub fn workbench_readout(&self) -> Option<String> {
        self.builder.as_ref().map(Builder::readout)
    }

    /// The GPU is up: the effects and actors go into its world (and the arena, once known).
    fn attach(&mut self, mut gpu: Gpu) {
        self.effects = Some(Effects::new(&mut gpu.world));
        self.actors = Some(Actors::new(&mut gpu.world));
        self.previews = Previews::new(&mut gpu.world)
            .inspect_err(|error| crate::log(error))
            .ok();
        #[cfg(not(target_arch = "wasm32"))]
        self.robots_ready(&mut gpu, robots::load::read());
        #[cfg(not(target_arch = "wasm32"))]
        self.arena_ready(&mut gpu, robots::load::read_arena());
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

    /// In the browser: the arena kit fetch is done and the GPU is up.
    #[cfg(target_arch = "wasm32")]
    fn poll_arena(&mut self) {
        if self.gpu.is_none() {
            return;
        }
        let Some(fetched) = self.arena_inbox.borrow_mut().take() else {
            return;
        };
        let kit = fetched.and_then(|(plans, files)| robots::load::finish(&plans, &files));
        if let Some(mut gpu) = self.gpu.take() {
            self.arena_ready(&mut gpu, kit);
            self.gpu = Some(gpu);
        }
    }

    /// The arena kit is read (or failed): its modules go into `gpu`'s world, or a kit-built map
    /// is drawn as boxes and the HUD says why.
    fn arena_ready(&mut self, gpu: &mut Gpu, kit: Result<robots::kit::KitData, String>) {
        let art = kit.and_then(|kit| ArenaArt::new(&mut gpu.world, kit));
        if let Err(error) = &art {
            crate::log(error);
        }
        self.arena_art = Some(art);
    }

    /// Why a kit-built map is drawn as boxes, if it is: for the HUD.
    fn arena_status(&self) -> Option<String> {
        let map = self.drawn?;
        let Some(Err(error)) = &self.arena_art else {
            return None;
        };
        (!map.arena().placements.is_empty())
            .then(|| format!("{error}: the arena is drawn as boxes"))
    }

    /// Puts the match's arena into the GPU's world once both are there (a host keeps a match
    /// on one map), or in the menu its live view's.
    fn draw_map(&mut self) {
        let map = match &self.game {
            Some(game) => game.map(),
            None => Some(self.menu.backdrop()),
        };
        if let Some(map) = map {
            self.show_arena(map);
        }
    }

    /// Draws `map`'s arena in the GPU's world in place of the one there (for a kit-built map,
    /// once the arena kit is read or has failed).
    fn show_arena(&mut self, map: MapId) {
        if self.drawn == Some(map) {
            return;
        }
        let Some(gpu) = &mut self.gpu else {
            return;
        };
        let arena = map.arena();
        if !arena.placements.is_empty() && self.arena_art.is_none() {
            return;
        }
        if let Some(old) = self.arena.take() {
            old.remove(&mut gpu.world);
        }
        let art = self.arena_art.as_ref().and_then(|art| art.as_ref().ok());
        self.arena = Some(scene::build(&mut gpu.world, &arena, art));
        self.backdrop = Backdrop::of(map, &arena);
        self.drawn = Some(map);
    }

    /// The drawn arena's placeables as the match's latest snapshot has them (the host's word
    /// online, the in-process match's in practice): the renderer keeps no damage of its own.
    fn show_scenery(&mut self) {
        let (Some(gpu), Some(placed), Some(Ok(art)), Some(snapshot)) = (
            &mut self.gpu,
            self.arena.as_ref().and_then(|arena| arena.placed.as_ref()),
            &self.arena_art,
            self.game.as_ref().and_then(Game::snapshot),
        ) else {
            return;
        };
        art.show(&mut gpu.world, placed, &snapshot.scenery);
    }

    /// One frame of the main menu: its live view (the arena it shows and the robot the next
    /// match plays, the camera turning slowly round it; in Practice the chosen arena from above)
    /// under the menu's items.
    fn menu_frame(&mut self, seconds: f32) {
        self.menu_seconds += seconds;
        self.draw_map();
        self.pending.clear();
        let Some(gpu) = &mut self.gpu else {
            return;
        };
        if let Some(window) = &self.window {
            let size = window.inner_size();
            self.menu.resize(size.width, size.height);
        }
        let spot = self.backdrop;
        let drawn = self.drawn == Some(self.menu.backdrop());
        let time = self.menu_seconds;
        let (eye, target) = if self.menu.screen == crate::menu::Screen::Practice {
            // The arena from high over it, turning.
            let angle = 0.7 + time * 0.05;
            (
                Vec3::new(angle.cos(), 0.0, angle.sin()) * 0.8 * spot.half
                    + Vec3::Y * 0.75 * spot.half,
                Vec3::ZERO,
            )
        } else {
            // The robot from in front, the arena behind it, the camera swaying slowly round it;
            // the robot right of the menu's column.
            let sway = 0.35 * (time * 0.12).sin();
            let (sin, cos) = sway.sin_cos();
            let back = Vec3::new(
                spot.view.x * cos - spot.view.z * sin,
                0.0,
                spot.view.x * sin + spot.view.z * cos,
            );
            let eye = spot.robot + back * 4.0 + Vec3::Y * 0.7;
            let right = Vec3::Y.cross(back).normalize_or_zero();
            (eye, spot.robot + Vec3::Y * 0.1 - right * 1.5)
        };
        let robot = drawn.then(|| RobotView {
            slot: crate::builder::view::BENCH_SLOT,
            build: self.active,
            state: RobotState {
                center: spot.robot,
                forward: spot.view,
                velocity: Vec3::ZERO,
                sliding: false,
                hook: None,
                since_fired: f32::INFINITY,
                fired: shooter_sim::Instance::default(),
                tube: 0,
                recent: robots::pose::Recent::NONE,
                held: shooter_sim::robot::BETA
                    .weapons(&self.active)
                    .next()
                    .unwrap_or_default(),
                energy: 1.0,
                swing: None,
                dead_for: None,
            },
            glow: 0.0,
            parts: robots::PartStates::default(),
            highlight: None,
        });
        if let Some(robots) = &mut self.robots {
            let views: Vec<RobotView> = robot.into_iter().collect();
            if let Err(error) = robots.update(&mut gpu.world, &views, seconds) {
                crate::log(&error);
            }
        }
        let (width, height) = gpu.size();
        let loading = if !drawn {
            Some("Loading the arena")
        } else if self.kit_status.is_some() {
            Some("Loading the robot kit")
        } else {
            None
        };
        let ui = self
            .menu
            .frame(width, height, self.previews.as_ref(), loading);
        let mut scene = scene::frame(
            eye,
            (target - eye).normalize_or(Vec3::NEG_Z),
            Vec::new(),
            &self
                .drawn
                .map_or(scene::Lighting::GREYBOX, scene::Lighting::of),
        );
        if !drawn {
            scene.clear_color = Vec3::new(0.05, 0.07, 0.1);
        }
        if let Err(error) = gpu.render(&mut scene, &ui, 0) {
            crate::log(&error);
        }
        #[cfg(target_arch = "wasm32")]
        self.publish(self.menu.readout());
    }

    /// `window.shooterMenu = <json>`, when it changed: what the menu (or the match, or the
    /// Workshop) shows, for scripted checks of the page.
    #[cfg(target_arch = "wasm32")]
    fn publish(&mut self, readout: String) {
        if readout != self.menu_readout {
            crate::web::publish_menu(&readout);
            self.menu_readout = readout;
        }
    }

    /// What a match shows for scripted checks: its map, fighters, label and the Leave button.
    #[cfg(target_arch = "wasm32")]
    fn match_readout(&self) -> String {
        let game = self.game.as_ref();
        let leave = self.leave_button().map(|rect| {
            let at = rect.centre();
            let (Some(window), Some(gpu)) = (&self.window, &self.gpu) else {
                return serde_json::json!({ "name": "Leave", "at": [at.x, at.y] });
            };
            let size = window.inner_size();
            let (width, height) = gpu.size();
            #[allow(clippy::cast_precision_loss)]
            let scale = (
                size.width as f32 / width.max(1) as f32,
                size.height as f32 / height.max(1) as f32,
            );
            serde_json::json!({
                "name": "Leave",
                "at": [(at.x * scale.0).round(), (at.y * scale.1).round()],
            })
        });
        serde_json::json!({
            "screen": if self.builder.as_ref().is_some_and(|b| b.stage == Stage::Workshop) {
                "Workshop"
            } else {
                "Match"
            },
            "map": game.and_then(Game::map).map(MapId::as_str),
            "fighters": game.and_then(Game::slots),
            "inProcess": game.is_some_and(Game::in_process),
            "label": self.match_label,
            "build": shooter_sim::wire::encode_build(&self.active)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
            "targets": leave.into_iter().collect::<Vec<_>>(),
        })
        .to_string()
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

    /// The page's touch controls ([`crate::touch`]): their stick and buttons pressed and released
    /// as keys and the primary mouse button, their look pad's drag as mouse movement.
    #[cfg(target_arch = "wasm32")]
    fn poll_touch(&mut self) {
        let mut frame = crate::web::touch();
        // The builder has the screen while it is open: the touch controls let go until it closes.
        frame.on &= self.builder.is_none() && self.game.is_some();
        if frame.on != self.touch.on() {
            // Tapping to play is the touch controls' capture: no pointer to lock.
            self.captured = frame.on;
            if frame.on {
                self.sounds.unlock();
            }
        }
        let changes = self.touch.update(&frame);
        for key in changes.released {
            self.controls.key_up(key);
        }
        for key in changes.pressed {
            self.controls.key_down(key, false);
        }
        match changes.fire {
            Some(true) => self.controls.mouse_down(PRIMARY_BUTTON),
            Some(false) => self.controls.mouse_up(PRIMARY_BUTTON),
            None => {}
        }
        if changes.edit && self.local_dead() {
            self.edit_robot();
        }
        if frame.on {
            self.board = self.touch.board();
            let [dx, dy] = frame
                .look
                .map(|pixels| pixels * crate::touch::LOOK_COUNTS_PER_PIXEL);
            if dx != 0.0 || dy != 0.0 {
                self.look.mouse_scaled(dx, dy, self.menu.prefs.look_share());
                self.driver.mouse(dx, dy);
            }
        }
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
        let repairs = self.controls.take_repairs();
        // Presses toggle the self-destruct (shooter#131): two in one tick cancel out.
        let destruct = self.controls.take_destructs() % 2 == 1;
        let intent = if let Some(tick) = self.scripted {
            self.scripted = Some(tick + 1);
            let intent = match self.stage {
                // Staged once the local fighter is in the match, then held there.
                Some(stage) => {
                    if tick == 1
                        && let Err(error) = self
                            .game
                            .as_mut()
                            .map_or(Ok(()), |game| game.stage(stage.feet(), stage.yaw()))
                    {
                        crate::log(&format!("the staged scene: {error}"));
                    }
                    stage.intent(tick)
                }
                None => scripted_intent(0, tick),
            };
            self.look = Look::new(radians(intent.yaw));
            self.look.pitch = f32::from(intent.pitch) / 65_536.0 * std::f32::consts::TAU;
            intent
        } else if self.builder.is_some() {
            // In the builder: Edit Robot held, the robot dead and waiting (D2.i70.11).
            self.repair_menu = RepairMenu::default();
            Intent {
                yaw: self.look.sim_yaw(),
                pitch: self.look.sim_pitch(),
                edit: true,
                ..Intent::IDLE
            }
        } else {
            // The wheel and Q/E step through the weapons the robot still has, from the one it
            // holds as predicted; the host decides. While the repair menu is open they move
            // through it instead, and R there repairs the part chosen (the host decides too).
            let mut steps = self.controls.take_steps();
            let repair = self.repair_menu.tick(
                self.game
                    .as_ref()
                    .and_then(Game::local_fighter)
                    .map(|(me, _)| me),
                repairs,
                &mut steps,
            );
            let select = match self.game.as_ref().and_then(Game::local_fighter) {
                Some((me, _)) => self.selection.intent(
                    self.controls.selected(),
                    steps,
                    me.arsenal.held,
                    &me.instances(),
                ),
                None => {
                    self.selection = Selection::default();
                    self.controls.selected().map(Select::Weapon)
                }
            };
            // The robot's actuators through the person's control layout (shooter#74): the keys
            // and buttons held and the mouse's movement while the game has it, and the guns its
            // direct weapon buttons and groups fire (shooter#127; only with the mouse captured,
            // like the primary trigger).
            let mouse = self.driver.take_mouse();
            let actions = match self.game.as_ref().and_then(Game::local_fighter) {
                Some((me, _)) => {
                    let pressed = self
                        .controls
                        .pressed(if self.captured { mouse } else { [0, 0] });
                    let pressed = if self.captured {
                        pressed
                    } else {
                        // Without the mouse, its buttons do nothing (the click that captures).
                        shooter_sim::bindings::Pressed {
                            buttons: 0,
                            ..pressed
                        }
                    };
                    let actions = self.driver.actions(&me.build, me.parts.lost(), pressed);
                    shooter_sim::bindings::Actions {
                        triggers: if self.captured {
                            actions.triggers
                        } else {
                            shooter_sim::weapons::Triggers::NONE
                        },
                        ..actions
                    }
                }
                None => shooter_sim::bindings::Actions::default(),
            };
            Intent {
                forward,
                strafe,
                motors: actions.motors,
                triggers: actions.triggers,
                yaw: self.look.sim_yaw(),
                jump: self.controls.jump_held(),
                sprint: self.controls.sprint_held(),
                crouch: self.controls.crouch_held(),
                pitch: self.look.sim_pitch(),
                // Only a captured mouse shoots: the click that captures it does not.
                fire: self.captured && self.controls.fire_held(),
                melee: self.captured && self.controls.melee_held(),
                // The grapple module's own key (shooter#104); the host refuses it without one.
                grapple: self.captured && self.controls.grapple_held(),
                select,
                edit: false,
                repair,
                // Armed or cancelled by its own key; the host decides.
                destruct,
            }
        };
        let Some(game) = self.game.as_mut() else {
            return true;
        };
        let tick = match game.tick(intent) {
            Ok(tick) => tick,
            Err(error) => {
                crate::log(&format!("the match failed: {error}"));
                return false;
            }
        };
        self.follow_builder();
        let local = self.game.as_ref().and_then(Game::local_slot);
        self.previous_eye = self.eye;
        let target = if self.game.as_ref().is_some_and(Game::local_sliding) {
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
        if let Some(center) = self.game.as_ref().and_then(Game::local_center) {
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
            .tick(&tick.events, local, tick.report.volley.shots(), seconds);
        if tick
            .report
            .volley
            .shots()
            .iter()
            .any(|fired| fired.weapon == Weapon::EnergyBlade)
        {
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
        if let Some(snapshot) = self.game.as_ref().and_then(Game::snapshot) {
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
        self.poll_arena();
        #[cfg(target_arch = "wasm32")]
        if self.captured && !self.touch.on() && !crate::web::pointer_locked() {
            // The browser let go of the pointer (Escape): stop playing until the next click.
            self.capture(false);
        }
        #[cfg(target_arch = "wasm32")]
        self.poll_touch();

        let pads = self.poll_pads();
        self.controls.poll_pads(&pads);
        self.look.stick(&pads, seconds);

        #[cfg(target_arch = "wasm32")]
        if self.page_menu != (self.game.is_none()) {
            self.page_menu = self.game.is_none();
            crate::web::show_menu(self.page_menu);
        }
        if self.in_menu() {
            self.menu_frame(seconds);
            return true;
        }
        let tick = 1.0 / f32::from(TICK_HZ);
        // Before the match nothing plays: the match starts at Ready (and there is none in the
        // Workshop).
        let setup = self.game.is_none()
            || self
                .builder
                .as_ref()
                .is_some_and(|builder| builder.stage == Stage::Setup);
        self.accumulator = if setup {
            0.0
        } else {
            self.accumulator + seconds
        };
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
        self.show_scenery();
        #[cfg(target_arch = "wasm32")]
        if self.builder_shown != self.builder.is_some() {
            self.builder_shown = self.builder.is_some();
            crate::web::show_builder(self.builder_shown);
        }
        if let Some(builder) = &mut self.builder {
            // The bench instead of the match: its effects are not shown.
            self.pending.clear();
            let Some(gpu) = &mut self.gpu else {
                return true;
            };
            // The pointer arrives in the window's pixels, the frame is the surface's.
            if let Some(window) = &self.window {
                let size = window.inner_size();
                builder.resize(size.width, size.height);
            }
            let (width, height) = gpu.size();
            let robots = self.robots.as_mut().map(|robots| (&mut gpu.world, robots));
            let (mut scene, ui) = builder.frame(robots, width, height, seconds);
            if let Err(error) = gpu.render(&mut scene, &ui, ticks) {
                crate::log(&error);
            }
            #[cfg(target_arch = "wasm32")]
            {
                let readout = builder.readout();
                if readout != self.builder_readout {
                    crate::web::publish_builder(&readout);
                    self.builder_readout = readout;
                }
                let readout = self.match_readout();
                self.publish(readout);
            }
            return true;
        }
        #[cfg(target_arch = "wasm32")]
        {
            let readout = self.match_readout();
            self.publish(readout);
        }
        let Some(game) = &self.game else {
            return true;
        };
        self.match_seconds += seconds;

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
        let camera = self.follow.place(game.world(), eye, forward, seconds);
        // Precision aim (shooter#132): only with the pointer captured, like every other action.
        self.precision
            .update(self.captured && self.controls.aim_held(), seconds);

        let local = game.local_slot();
        let snapshot = game.snapshot();
        // The physical debris where the host has it (shooter#75): its parts are drawn there, not
        // falling on their own.
        if let Some(snapshot) = snapshot {
            self.tracker.debris(&snapshot.debris);
        }
        let rockets: Vec<Vec3> = snapshot
            .map(|snapshot| {
                snapshot
                    .rockets
                    .iter()
                    .map(|rocket| Vec3::from(rocket.position) * METRES_PER_UNIT)
                    .collect()
            })
            .unwrap_or_default();
        // The modules lying on their pads (ready to take), and where dead carriers dropped them.
        #[allow(clippy::cast_precision_loss)]
        let ground = |center: Vec3i| {
            Vec3::from([center.x, center.y, center.z].map(|axis| axis as f32)) * METRES_PER_UNIT
        };
        let modules: Vec<(Vec3, ArenaModule)> = snapshot
            .iter()
            .flat_map(|snapshot| {
                let pads = snapshot
                    .pickups
                    .iter()
                    .filter(|pickup| pickup.ready_in == 0)
                    .map(|pickup| (ground(pickup.center), pickup.module));
                let drops = snapshot
                    .drops
                    .iter()
                    .map(|drop| (ground(drop.center), drop.carried.module));
                pads.chain(drops)
            })
            .collect();
        let me = local.and_then(|slot| snapshot.and_then(|snapshot| snapshot.slot(slot)));
        // Dead: the countdown, and Edit Robot (B) to suspend it.
        let respawn_in = me.filter(|me| !me.alive).map(|me| me.respawn_in);
        // Its self-destruct counting down (shooter#131): the ticks left and the blast it would
        // set off now.
        let destruct = me.filter(|me| me.alive).and_then(|me| {
            let charge =
                shooter_sim::destruct::Charge::of(&me.build, &me.parts, me.energy, me.fuel);
            Some((me.destruct_in?, charge.damage()))
        });
        // Every robot whose self-destruct counts down, and the ticks left: a warning light
        // pulses on it for everyone.
        let mut warnings: Vec<(Vec3, u32)> = Vec::new();
        let alive = me.filter(|me| me.alive && game.local_center().is_some());
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
                held: me.held.weapon,
                hook: game
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
            if let Some(ticks) = me.destruct_in {
                warnings.push((at, ticks));
            }
            self.tracker.seen(me.slot, at, forward, me.build, me.held);
            let frame = robots::track::rest_frame(&me.build, at, forward);
            self.tracker.parts(me.slot, &me.build, &me.parts, frame);
            if view.center.is_some() {
                let moved = Vec3::from(self.center) - Vec3::from(self.previous_center);
                let (fired, tube) = self.tracker.fired(me.slot);
                robot_views.push(RobotView {
                    slot: me.slot,
                    build: me.build,
                    state: RobotState {
                        center: at,
                        forward,
                        velocity: moved * f32::from(TICK_HZ) * METRES_PER_UNIT,
                        sliding: game.local_sliding(),
                        hook: view.hook,
                        since_fired: self.tracker.since_fired(me.slot),
                        fired,
                        tube,
                        recent: self.tracker.recent(me.slot),
                        held: me.held,
                        energy: energy_share(me),
                        swing: view.swing,
                        dead_for: None,
                    },
                    glow: view.glow,
                    parts: robots::PartStates::default(),
                    // The part the repair menu points at, or the one under repair.
                    highlight: game
                        .local_fighter()
                        .and_then(|(fighter, _)| self.repair_menu.highlight(fighter)),
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
            let drawn = game.other_center(fighter.slot, blend);
            let view = FighterView {
                center: drawn.map(|center| Vec3::from(center) * METRES_PER_UNIT),
                eye_above_center: EYE_ABOVE_CENTER,
                forward: looking(fighter.yaw, fighter.pitch),
                health: crate::hud::chassis_share(fighter),
                glow: self.glows[slot],
                held: fighter.held.weapon,
                hook: fighter
                    .hook
                    .map(|hook| Vec3::from(hook.anchor_units()) * METRES_PER_UNIT),
                swing: swing_progress(self.swings[slot]),
                slot: fighter.slot,
                robot: false,
                grapple_from: None,
            };
            if let Some(center) = drawn.filter(|_| fighter.alive) {
                if let Some(ticks) = fighter.destruct_in {
                    warnings.push((Vec3::from(center) * METRES_PER_UNIT, ticks));
                }
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
                let (fired, tube) = self.tracker.fired(fighter.slot);
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
                        tube,
                        recent: self.tracker.recent(fighter.slot),
                        held: fighter.held,
                        energy: energy_share(fighter),
                        swing: view.swing,
                        dead_for: None,
                    },
                    glow: view.glow,
                    parts: robots::PartStates::default(),
                    highlight: None,
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
                    tube: 0,
                    recent: robots::pose::Recent::NONE,
                    held: remains.held,
                    energy: 0.0,
                    swing: None,
                    dead_for: Some(remains.dead_for),
                },
                glow: 0.0,
                parts: robots::PartStates::default(),
                highlight: None,
            });
        }
        // The reticle covers what the camera targets, a blocked marker where a shot fired now
        // from the held weapon's muzzle would stop short of it, and every gun's mark where its
        // own shot would land ([`camera::Reticle::aimed`]).
        let reticle = match (alive, game.world()) {
            (Some(me), Some(world)) => Some(camera::Reticle::aimed(&camera::Aiming {
                world,
                fighter: game.local_fighter().map(|(fighter, _)| fighter),
                eye: (eye / METRES_PER_UNIT).to_array(),
                held: me.held,
                yaw: self.look.sim_yaw(),
                pitch: self.look.sim_pitch(),
                others: &seen,
                camera,
                forward,
            })),
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
        // Each living robot's bodies where the simulation has them (the local one predicted, the
        // others between their snapshots), drawn around its fighter's drawn centre.
        let frames: Vec<Option<robots::Frames>> = robot_views
            .iter()
            .map(|view| {
                if view.state.dead_for.is_some() {
                    return None;
                }
                let exact = if Some(view.slot) == local {
                    game.local_frames()
                } else {
                    game.other_frames(view.slot, blend)
                };
                exact.map(|frames| frames.at(view.state.center))
            })
            .collect();
        let robot_views: Vec<RobotView> = robot_views
            .iter()
            .zip(&part_states)
            .zip(&frames)
            .map(|((view, (detached, damaged)), frames)| RobotView {
                parts: robots::PartStates {
                    frames: frames.as_ref(),
                    detached,
                    damaged,
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
            .text(snapshot, local, game.status(), self.board, seconds);
        text.robot = self
            .kit_status
            .clone()
            .or_else(|| self.arena_status())
            .unwrap_or_else(|| {
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
            .map(|effects| effects.lights(seconds, &rockets, &warnings))
            .unwrap_or_default();
        let mut scene = scene::frame(
            camera,
            forward,
            point_lights,
            &self
                .drawn
                .map_or(scene::Lighting::GREYBOX, scene::Lighting::of),
        );
        scene.camera.fov_y = self.precision.fov_y();
        // The drawn HUD: the held weapon (as predicted), the carried ones, chassis and energy.
        let vitals = game.local_fighter().map(|(me, tick)| Vitals::of(me, tick));
        // And field repair: the quick menu, the repair under way or what there is to repair.
        let repair = game
            .local_fighter()
            .map(|(me, _)| self.repair_menu.view(me));
        // While the pointer is free the way back to the menu, and for a while after the start
        // (and while the pointer is free) what the match is.
        let leave = self.leave_button().map(|rect| (rect, self.on_leave()));
        let label = self
            .match_label
            .as_deref()
            .filter(|_| leave.is_some() || self.match_seconds < MATCH_LABEL_SECONDS);
        let Some(gpu) = &mut self.gpu else {
            return true;
        };
        let (width, height) = gpu.size();
        let mut ui = crate::overlay::scope(width, height, self.precision.zoom());
        ui.extend(self.overlay.draw(vitals.as_ref(), width, height, seconds));
        if let Some(repair) = &repair {
            ui.extend(repair_panel(repair, height));
        }
        if let Some(ticks) = respawn_in {
            ui.extend(death_prompt(
                f32::from(u16::try_from(ticks).unwrap_or(u16::MAX)) / f32::from(TICK_HZ),
                width,
                height,
            ));
        }
        if let Some((ticks, damage)) = destruct {
            ui.extend(crate::overlay::destruct_warning(
                ticks, damage, width, height,
            ));
        }
        ui.extend(crate::menu::match_items(width, height, leave, label));
        if let Some(actors) = &mut self.actors {
            actors.update(&mut gpu.world, &fighters, &rockets, &modules, reticle);
        }
        if let Err(error) = gpu.render(&mut scene, &ui, ticks) {
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

/// The HUD's robot line for the local fighter: its animation state, "robot: run (walk-biped) +
/// fire".
fn robot_line(robot: &robots::Drawn) -> String {
    format!("robot: {}", robot.label)
}

impl ApplicationHandler<GpuReady> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let title = if self.game.is_some() {
            "shooter — click to play"
        } else {
            "shooter"
        };
        #[allow(unused_mut)]
        let mut attributes = Window::default_attributes().with_title(title);
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
                if let Some(builder) = &mut self.builder {
                    builder.resize(size.width, size.height);
                }
            }
            WindowEvent::Focused(false) => {
                self.capture(false);
                self.controls.release_all();
                self.board = false;
            }
            // The builder has the pointer and the keys while it is open, the menu while it is.
            WindowEvent::CursorMoved { position, .. } => {
                #[allow(clippy::cast_possible_truncation)]
                let (x, y) = (position.x as f32, position.y as f32);
                self.cursor = render_engine::glam::Vec2::new(x, y);
                self.menu.cursor_moved(x, y);
                if let Some(builder) = &mut self.builder {
                    builder.cursor_moved(x, y);
                }
            }
            WindowEvent::MouseInput { state, button, .. } if self.in_menu() => {
                if button == MouseButton::Left {
                    let choice = self.menu.mouse(state == ElementState::Pressed);
                    self.menu_choice(choice);
                }
            }
            // A finger is the left button: a tap acts.
            WindowEvent::Touch(touch) if self.in_menu() => {
                #[allow(clippy::cast_possible_truncation)]
                self.menu
                    .cursor_moved(touch.location.x as f32, touch.location.y as f32);
                let choice = match touch.phase {
                    TouchPhase::Started => self.menu.mouse(true),
                    TouchPhase::Ended => self.menu.mouse(false),
                    TouchPhase::Moved | TouchPhase::Cancelled => None,
                };
                self.menu_choice(choice);
            }
            WindowEvent::KeyboardInput { event, .. } if self.in_menu() => {
                if let (PhysicalKey::Code(code), ElementState::Pressed) =
                    (event.physical_key, event.state)
                {
                    let choice = self.menu.key(code, event.text.as_deref());
                    self.menu_choice(choice);
                }
            }
            // In a match with the pointer free: the Leave button goes back to the menu.
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } if self.on_leave() => self.back_to_menu(None),
            WindowEvent::Touch(touch)
                if touch.phase == TouchPhase::Started && self.leave_button().is_some() =>
            {
                #[allow(clippy::cast_possible_truncation)]
                {
                    self.cursor = render_engine::glam::Vec2::new(
                        touch.location.x as f32,
                        touch.location.y as f32,
                    );
                }
                if self.on_leave() {
                    self.back_to_menu(None);
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                if let Some(builder) = &mut self.builder {
                    let state = modifiers.state();
                    builder.modifiers(state.shift_key(), state.control_key());
                }
            }
            WindowEvent::MouseInput { state, button, .. } if self.builder.is_some() => {
                let pressed = state == ElementState::Pressed;
                let action = match (button, &mut self.builder) {
                    (MouseButton::Left | MouseButton::Right, Some(builder)) => {
                        builder.mouse(button == MouseButton::Right, pressed)
                    }
                    _ => None,
                };
                self.builder_action(action);
            }
            // On a touch screen a finger is the left button: tap to pick and press, drag to place.
            WindowEvent::Touch(touch) if self.builder.is_some() => {
                let action = self.builder.as_mut().and_then(|builder| {
                    #[allow(clippy::cast_possible_truncation)]
                    builder.cursor_moved(touch.location.x as f32, touch.location.y as f32);
                    match touch.phase {
                        TouchPhase::Started => builder.mouse(false, true),
                        TouchPhase::Ended | TouchPhase::Cancelled => builder.mouse(false, false),
                        TouchPhase::Moved => None,
                    }
                });
                self.builder_action(action);
            }
            WindowEvent::MouseWheel { delta, .. } if self.builder.is_some() => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, lines) => lines,
                    #[allow(clippy::cast_possible_truncation)]
                    MouseScrollDelta::PixelDelta(pixels) => pixels.y as f32 / 100.0,
                };
                if let Some(builder) = &mut self.builder {
                    builder.wheel(lines);
                }
            }
            WindowEvent::KeyboardInput { event, .. } if self.builder.is_some() => {
                if let (PhysicalKey::Code(code), ElementState::Pressed, Some(builder)) =
                    (event.physical_key, event.state, &mut self.builder)
                {
                    let action = builder.key(code);
                    self.builder_action(action);
                }
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
                // Dead: Edit Robot instead of the automatic respawn.
                if code == KeyCode::KeyB
                    && event.state == ElementState::Pressed
                    && self.local_dead()
                {
                    self.edit_robot();
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
            self.look.mouse_scaled(
                dx,
                dy,
                self.precision.sensitivity() * self.menu.prefs.look_share(),
            );
            self.driver.mouse(dx, dy);
        }
    }
}

#[cfg(test)]
mod tests {
    use shooter_sim::robot::{ARMORY, HEAVY, STANDARD};

    #[test]
    fn the_menu_plays_a_valid_build_or_the_standard_one() {
        assert_eq!(super::valid_or_standard(HEAVY), HEAVY);
        // Over the beta's budget: refused, the standard build plays.
        assert!(
            shooter_sim::robot::BETA
                .validate(&ARMORY.candidate())
                .is_err()
        );
        assert_eq!(super::valid_or_standard(ARMORY), STANDARD);
    }
}
