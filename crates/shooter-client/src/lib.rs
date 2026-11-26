//! The shooter's client: one crate for the native window (wgpu on Vulkan/Metal/DX12) and the
//! browser (`wasm32`: WebGPU, else WebGL 2 through the lite renderer, [`gfx`]), rendered by
//! render-engine, with input-bindings controls and
//! game-audio footsteps and gunfire, render-engine weapon lights and particles over the shared
//! `shooter-sim` simulation.

pub mod app;
pub mod arena_art;
pub mod audio;
pub mod builder;
pub mod camera;
pub mod controls;
pub mod effects;
pub mod game;
pub mod gfx;
pub mod gpu;
pub mod hud;
pub mod interpolation;
pub mod menu;
pub mod net;
pub mod overlay;
pub mod prefs;
pub mod repair_menu;
pub mod robots;
pub mod scene;
pub mod touch;
pub mod ui;

use shooter_sim::MapId;

/// The match the command line or the page's query starts directly, if it asks for one:
/// natively `--connect <url> [--cert <pem>] [--name <name>]` joins a hosted deathmatch (the
/// certificate is a development host's self-signed one; the name is remembered for later runs,
/// [`remembered_name`]); in the browser the page's `?connect=<url>[&certHash=<sha-256
/// hex>][&name=<name>]` does (else the name from Settings). Online, the host picks the map.
/// Otherwise `--map <id>` (`?map=<id>`) is offline practice against bots on `map` (the greybox
/// test maps' way in). With neither, none: the main menu opens (shooter#154). Either way
/// `--loadout <name>` (`?loadout=<name>`) is the build to play ([`loadout`]: the builder before
/// a direct match starts from it, the menu's matches play it), returned beside the game.
fn game(
    #[cfg(not(target_arch = "wasm32"))] args: Args,
    map: Option<MapId>,
) -> (Option<game::Game>, Option<shooter_sim::Assembly>) {
    #[cfg(not(target_arch = "wasm32"))]
    let (url, trust, name) = (args.connect, args.cert, remembered_name(args.name));
    #[cfg(target_arch = "wasm32")]
    let (url, trust, name) = {
        let query = web::query("connect");
        let hash = web::query("certHash");
        let name =
            web::query("name").or_else(|| prefs::Prefs::load(prefs::platform().as_ref()).name);
        match hash.as_deref().map(parse_hash) {
            Some(None) => {
                fail("certHash is not a SHA-256 in hex");
                (None, None, name)
            }
            hash => (query, hash.flatten(), name),
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    let requested = args.loadout;
    #[cfg(target_arch = "wasm32")]
    let requested = web::query("loadout");
    let game = match url {
        Some(url) => Some(game::Game::Online(Box::new(game::online::Online::connect(
            url,
            trust,
            name.as_deref(),
        )))),
        None => map.map(game::Game::practice),
    };
    match loadout(requested.as_deref()) {
        Ok(build) => (game, build),
        Err(error) => {
            fail(&error);
            (game, None)
        }
    }
}

/// The loadout to play as: natively `--loadout <name>`, in the browser `?loadout=<name>`, one of
/// [`shooter_sim::robot::LOADOUTS`] (`standard`, `heavy`, `skirmisher`, `marksman`, `pyro`,
/// `mechanic`, `stoker`, `robo-dog`, `rover`, `mini-tank`, `steer-car`); none asks for nothing (the
/// standard build).
///
/// # Errors
///
/// If `requested` names no loadout.
pub fn loadout(requested: Option<&str>) -> Result<Option<shooter_sim::Assembly>, String> {
    requested
        .filter(|name| !name.trim().is_empty())
        .map(|name| {
            shooter_sim::robot::loadout(name).ok_or_else(|| {
                let names: Vec<&str> = shooter_sim::robot::LOADOUTS
                    .iter()
                    .map(|(name, _)| *name)
                    .collect();
                format!("unknown loadout {name:?} (one of {})", names.join(", "))
            })
        })
        .transpose()
}

/// The practice and profile-run map: natively `--map <id>`, in the browser `?map=<id>`, else
/// [`MapId::DEFAULT`]. An ID outside the catalog is refused.
///
/// # Errors
///
/// If `requested` names no catalogued map.
pub fn practice_map(requested: Option<&str>) -> Result<MapId, String> {
    requested.map_or(Ok(MapId::DEFAULT), |id| {
        id.parse()
            .map_err(|error: shooter_sim::UnknownMap| error.to_string())
    })
}

/// The profile run's staged scene (`--scene <name>`, `?scene=<name>`) on `map`
/// ([`shooter_sim::scenarios::Staged`]): none; `cover`, the standard build behind the map's
/// arena cover firing its rifle past it, its shots stopping on the cover short of the reticle
/// (shooter#102's captures); or `swing`, the standard build hooking high arena geometry with its
/// grapple module and reeled up off the floor on the rope (shooter#104's captures).
///
/// # Errors
///
/// If `requested` names another scene, or `map` has none of it.
pub fn scene(
    requested: Option<&str>,
    map: MapId,
) -> Result<Option<shooter_sim::scenarios::Staged>, String> {
    requested
        .map(|name| shooter_sim::scenarios::Staged::named(name, map))
        .transpose()
}

/// The native command line.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct Args {
    connect: Option<String>,
    cert: Option<std::path::PathBuf>,
    name: Option<String>,
    /// `--map <id>`: the practice map ([`practice_map`]).
    map: Option<String>,
    /// `--frames <n>`: the offscreen profile run instead of a window ([`offscreen`]).
    frames: Option<u32>,
    /// `--loadout <name>`: the loadout to play as ([`loadout`]).
    loadout: Option<String>,
    /// `--capture <dir>`: the offscreen run writes every [`CAPTURE_EVERY`]th frame there.
    capture: Option<std::path::PathBuf>,
    /// `--scene <name>`: the offscreen run plays a staged scene ([`scene`]).
    scene: Option<String>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Args {
    fn parse() -> Self {
        let mut parsed = Self::default();
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--connect" => parsed.connect = args.next(),
                "--cert" => parsed.cert = args.next().map(std::path::PathBuf::from),
                "--name" => parsed.name = args.next(),
                "--map" => parsed.map = args.next(),
                "--loadout" => parsed.loadout = args.next(),
                "--frames" => parsed.frames = args.next().and_then(|n| n.parse().ok()),
                "--capture" => parsed.capture = args.next().map(std::path::PathBuf::from),
                "--scene" => parsed.scene = args.next(),
                other => log(&format!("unknown argument {other}")),
            }
        }
        parsed
    }
}

/// The offscreen profile run's frame size.
pub const OFFSCREEN_SIZE: (u32, u32) = (1280, 720);

/// How often `--capture` writes a frame.
pub const CAPTURE_EVERY: u32 = 30;

/// Writes RGBA8 `pixels` (`width` × `height`) to `path` as a binary PPM image.
///
/// # Errors
///
/// If the file cannot be written.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_ppm(
    path: &std::path::Path,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> Result<(), String> {
    let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
    for pixel in pixels.chunks(4) {
        ppm.extend_from_slice(&pixel[..3]);
    }
    std::fs::write(path, ppm).map_err(|error| format!("{}: {error}", path.display()))
}

/// The profile run (`shooter-client --frames <n> [--map <id>] [--loadout <name>] [--scene
/// <name>] [--capture <dir>]`): `frames` frames of a practice match on `map` at one tick per
/// frame, the local fighter (as `build`, if given) played by the replays' scripted person 0
/// ([`shooter_sim::scenarios::scripted_intent`]) or staged by `stage` ([`scene`]), drawn to an offscreen [`OFFSCREEN_SIZE`]
/// target with no window, as fast as the GPU finishes them. Prints frame times; nothing gates
/// on them. With `capture`, every [`CAPTURE_EVERY`]th frame is also written there as a PPM
/// image (outside the timed frames).
///
/// # Errors
///
/// If there is no GPU adapter, the match fails or a capture cannot be written.
#[cfg(not(target_arch = "wasm32"))]
pub fn offscreen(
    frames: u32,
    map: MapId,
    build: Option<shooter_sim::Assembly>,
    stage: Option<shooter_sim::scenarios::Staged>,
    capture: Option<&std::path::Path>,
) -> Result<(), String> {
    let (width, height) = OFFSCREEN_SIZE;
    let gpu = gpu::Gpu::offscreen(width, height)?;
    let game = if stage.is_some() {
        game::Game::staged(map)
    } else {
        game::Game::practice(map)
    };
    let game = match build {
        Some(build) => game.with_loadout(build),
        None => game,
    };
    let mut app = app::App::scripted(game, gpu).staged(stage);
    if let Some(directory) = capture {
        std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    }
    let mut times = Vec::with_capacity(frames as usize);
    let started = std::time::Instant::now();
    for index in 1..=frames {
        let frame = std::time::Instant::now();
        app.scripted_frame()?;
        times.push(frame.elapsed().as_secs_f64() * 1_000.0);
        if let (Some(directory), Some(gpu)) = (capture, app.gpu())
            && index % CAPTURE_EVERY == 0
        {
            let (width, height, pixels) = gpu.read_frame()?;
            write_ppm(
                &directory.join(format!("frame-{index:04}.ppm")),
                width,
                height,
                &pixels,
            )?;
        }
    }
    let total = started.elapsed().as_secs_f64();
    let stats = FrameStats::of(times);
    println!(
        "offscreen {width}x{height} on {map}: {frames} frames in {total:.2} s; frame ms mean {:.2} p50 {:.2} p95 {:.2} max {:.2}",
        stats.mean, stats.p50, stats.p95, stats.max
    );
    Ok(())
}

/// A profile run's frame times in milliseconds: mean, median, 95th percentile and worst.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameStats {
    pub mean: f64,
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
}

impl FrameStats {
    #[must_use]
    pub fn of(mut times: Vec<f64>) -> Self {
        times.sort_by(f64::total_cmp);
        let at = |q: f64| {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::cast_precision_loss
            )]
            times
                .get(((times.len().saturating_sub(1)) as f64 * q).round() as usize)
                .copied()
                .unwrap_or(0.0)
        };
        #[allow(clippy::cast_precision_loss)]
        let mean = times.iter().sum::<f64>() / times.len().max(1) as f64;
        Self {
            mean,
            p50: at(0.5),
            p95: at(0.95),
            max: at(1.0),
        }
    }
}

/// The name to play under: `given` (`--name`, then remembered for later runs, as Settings'
/// name, [`prefs`]) else the one remembered. A store that cannot be read or written only loses
/// the memory.
#[cfg(not(target_arch = "wasm32"))]
fn remembered_name(given: Option<String>) -> Option<String> {
    let store = prefs::platform();
    let mut remembered = prefs::Prefs::load(store.as_ref());
    if let Some(name) = given.and_then(|name| shooter_sim::names::sanitize(&name)) {
        remembered.name = Some(name);
        if let Err(error) = remembered.save(store.as_ref()) {
            log(&format!("could not remember the name: {error}"));
        }
    }
    remembered.name
}

/// A SHA-256 written in hex, colons allowed (as `openssl x509 -fingerprint -sha256` prints it).
#[must_use]
pub fn parse_hash(text: &str) -> Option<[u8; 32]> {
    let digits: Vec<u8> = text
        .bytes()
        .filter(|byte| *byte != b':')
        .map(|byte| {
            char::from(byte)
                .to_digit(16)
                .and_then(|d| u8::try_from(d).ok())
        })
        .collect::<Option<_>>()?;
    let mut hash = [0; 32];
    if digits.len() != 2 * hash.len() {
        return None;
    }
    for (byte, pair) in hash.iter_mut().zip(digits.chunks(2)) {
        *byte = pair[0] << 4 | pair[1];
    }
    Some(hash)
}

use winit::event_loop::EventLoop;

/// Opens the window (or the page's canvas) and runs the game until it closes.
///
/// # Panics
///
/// If the platform has no event loop.
pub fn run() {
    #[cfg(not(target_arch = "wasm32"))]
    let args = Args::parse();
    #[cfg(not(target_arch = "wasm32"))]
    let requested = args.map.clone();
    #[cfg(target_arch = "wasm32")]
    let requested = web::query("map");
    let map = match practice_map(requested.as_deref()) {
        Ok(map) => map,
        Err(error) => {
            fail(&error);
            #[cfg(not(target_arch = "wasm32"))]
            std::process::exit(2);
            #[cfg(target_arch = "wasm32")]
            return;
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(frames) = args.frames {
        let build = loadout(args.loadout.as_deref()).unwrap_or_else(|error| {
            fail(&error);
            std::process::exit(2);
        });
        let stage = scene(args.scene.as_deref(), map).unwrap_or_else(|error| {
            fail(&error);
            std::process::exit(2);
        });
        if let Err(error) = offscreen(frames, map, build, stage, args.capture.as_deref()) {
            fail(&error);
            std::process::exit(1);
        }
        return;
    }
    let event_loop = EventLoop::<app::GpuReady>::with_user_event()
        .build()
        .expect("an event loop");
    // `?frames=<n>`: the browser profile run, a scripted practice match ([`web::Profile`]).
    #[cfg(target_arch = "wasm32")]
    let profile = web::query("frames").and_then(|n| n.parse::<u32>().ok());
    // `&scene=cover`: a staged scene instead of the scripted person ([`scene`]).
    #[cfg(target_arch = "wasm32")]
    let stage = match profile.map(|_| scene(web::query("scene").as_deref(), map)) {
        Some(Err(error)) => {
            fail(&error);
            return;
        }
        Some(Ok(stage)) => stage,
        None => None,
    };
    #[cfg(target_arch = "wasm32")]
    let (game, build) = match profile {
        Some(_) if stage.is_some() => (Some(game::Game::staged(map)), None),
        Some(_) => (Some(game::Game::practice(map)), None),
        None => game(requested.is_some().then_some(map)),
    };
    #[cfg(not(target_arch = "wasm32"))]
    let (game, build) = game(args, requested.is_some().then_some(map));
    // A direct start (a hosted match, or practice on a requested map) opens the builder before
    // the match (shooter#57) on the requested loadout; otherwise the main menu (shooter#154),
    // whose matches play the requested loadout for this run, else the active build kept from
    // before (shooter#155).
    let direct = game.is_some();
    let app = app::App::new(event_loop.create_proxy(), game);
    let app = match build {
        Some(build) => app.with_active(build),
        None => app,
    };
    let build = build.unwrap_or_default();
    #[cfg(target_arch = "wasm32")]
    let app = match profile {
        Some(frames) => app.profiled(frames).staged(stage),
        None if direct => app.with_builder(build),
        None => app,
    };
    #[cfg(not(target_arch = "wasm32"))]
    let app = if direct { app.with_builder(build) } else { app };
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut app = app;
        if let Err(error) = event_loop.run_app(&mut app) {
            fail(&error.to_string());
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        use winit::platform::web::EventLoopExtWebSys;
        event_loop.spawn_app(app);
    }
}

pub(crate) fn log(message: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("{message}");
    #[cfg(target_arch = "wasm32")]
    web_sys::console::warn_1(&message.into());
}

/// A fatal error: logged, and shown on the page in the browser.
pub(crate) fn fail(message: &str) {
    log(message);
    #[cfg(target_arch = "wasm32")]
    web::show_error(message);
}

#[cfg(target_arch = "wasm32")]
pub(crate) mod web {
    use wasm_bindgen::prelude::wasm_bindgen;

    #[wasm_bindgen(start)]
    pub fn start() {
        // Panics go to the console and the page (and a profile run's result).
        std::panic::set_hook(Box::new(|info| {
            console_error_panic_hook::hook(info);
            show_error(&info.to_string());
        }));
        super::run();
    }

    fn document() -> Option<web_sys::Document> {
        web_sys::window()?.document()
    }

    /// Whether the page holds Pointer Lock (the browser releases it on Escape).
    pub fn pointer_locked() -> bool {
        document().is_some_and(|document| document.pointer_lock_element().is_some())
    }

    /// The page's touch controls this frame (`window.shooterTouch`, [`crate::touch`]); the look
    /// pad's drag is taken (reset on the page).
    pub fn touch() -> crate::touch::Frame {
        use js_sys::Reflect;
        let Some(state) = web_sys::window()
            .and_then(|window| Reflect::get(&window, &"shooterTouch".into()).ok())
            .filter(wasm_bindgen::JsValue::is_object)
        else {
            return crate::touch::Frame::default();
        };
        let number = |name: &str| {
            Reflect::get(&state, &name.into())
                .ok()
                .and_then(|value| value.as_f64())
                .filter(|value| value.is_finite())
                .unwrap_or(0.0)
        };
        let look = [number("lookX"), number("lookY")];
        let _ = Reflect::set(&state, &"lookX".into(), &0.into());
        let _ = Reflect::set(&state, &"lookY".into(), &0.into());
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        crate::touch::Frame {
            on: Reflect::get(&state, &"on".into())
                .ok()
                .and_then(|value| value.as_bool())
                .unwrap_or(false),
            stick: [number("stickX"), number("stickY")].map(|axis| axis.clamp(-1.0, 1.0) as f32),
            look,
            held: number("held").clamp(0.0, f64::from(u32::MAX)) as u32,
        }
    }

    /// Marks the page's body while the builder is open (`data-builder`): the page hides its own
    /// overlays and keeps the canvas' right click (shooter#57).
    pub fn show_builder(open: bool) {
        let Some(body) = document().and_then(|document| document.body()) else {
            return;
        };
        let _ = if open {
            body.set_attribute("data-builder", "")
        } else {
            body.remove_attribute("data-builder")
        };
    }

    /// Marks the page's body while the main menu or the Workshop is open (`data-menu`, no
    /// match): the page hides its click-to-play overlay, HUD and touch controls (shooter#154).
    pub fn show_menu(open: bool) {
        let Some(body) = document().and_then(|document| document.body()) else {
            return;
        };
        let _ = if open {
            body.set_attribute("data-menu", "")
        } else {
            body.remove_attribute("data-menu")
        };
    }

    /// `window.shooterMenu = <json>`: what the menu, or the match or Workshop started from it,
    /// shows ([`crate::menu::Menu::readout`]), for scripted checks of the page (shooter#154).
    pub fn publish_menu(json: &str) {
        if let (Some(window), Ok(value)) = (web_sys::window(), js_sys::JSON::parse(json)) {
            let _ = js_sys::Reflect::set(&window, &"shooterMenu".into(), &value);
        }
    }

    /// The page's click-to-play line (`#mode`): what the match is.
    pub fn set_mode(text: &str) {
        if let Some(element) = document().and_then(|document| document.get_element_by_id("mode")) {
            element.set_text_content(Some(text));
        }
    }

    /// The page's prompt asking `question`, filled with `current`: what the person typed, or
    /// none if they cancelled (a phone's keyboard types into it too).
    pub fn prompt(question: &str, current: &str) -> Option<String> {
        web_sys::window()?
            .prompt_with_message_and_default(question, current)
            .ok()
            .flatten()
    }

    /// `window.shooterBuilder = <json>`: what the open builder shows
    /// ([`crate::builder::view::Builder::readout`]), for scripted checks of the page (shooter#124).
    pub fn publish_builder(json: &str) {
        if let (Some(window), Ok(value)) = (web_sys::window(), js_sys::JSON::parse(json)) {
            let _ = js_sys::Reflect::set(&window, &"shooterBuilder".into(), &value);
        }
    }

    /// A parameter of the page's query string, if it is set and not empty.
    pub fn query(name: &str) -> Option<String> {
        let search = web_sys::window()?.location().search().ok()?;
        web_sys::UrlSearchParams::new_with_str(&search)
            .ok()?
            .get(name)
            .filter(|value| !value.is_empty())
    }

    pub fn show_error(message: &str) {
        if let Some(element) = document().and_then(|document| document.get_element_by_id("error")) {
            element.set_text_content(Some(message));
            let _ = element.set_attribute("data-shown", "");
        }
        // A profile run that fails says so where the journey waits for its result.
        if query("frames").is_none() {
            return;
        }
        publish(&format!(
            r#"{{"error":{}}}"#,
            js_sys::JSON::stringify(&message.into())
                .ok()
                .and_then(|text| text.as_string())
                .unwrap_or_else(|| "\"error\"".into())
        ));
    }

    fn performance() -> Option<web_sys::Performance> {
        web_sys::window()?.performance()
    }

    /// The page clock in milliseconds.
    pub fn now() -> f64 {
        performance().map_or(0.0, |performance| performance.now())
    }

    /// The User Timing mark a profiled frame starts at.
    const FRAME_MARK: &str = "shooter-frame";
    /// The User Timing measure around a profiled frame: runtime-profiler attributes it as the
    /// JS-to-WASM boundary (its `runtime-profiler:js-to-wasm:<label>` convention).
    const FRAME_MEASURE: &str = "runtime-profiler:js-to-wasm:frame";

    /// The browser profile run (`?frames=<n>[&map=<id>]`): like the native `--frames`, `frames`
    /// frames of a practice match on `map` at one tick per animation frame, the local fighter played by the
    /// replays' scripted person 0. When done it sets `window.shooterProfile` to its frame
    /// times; nothing gates on them.
    pub struct Profile {
        pub frames: u32,
        map: shooter_sim::MapId,
        /// Milliseconds in the frame's work (the tick, the scene and the GPU submission).
        work: Vec<f64>,
        /// Milliseconds between consecutive frames' starts (including waiting on the GPU).
        intervals: Vec<f64>,
        first: Option<f64>,
        last: Option<f64>,
    }

    impl Profile {
        #[must_use]
        pub fn new(frames: u32, map: shooter_sim::MapId) -> Self {
            Self {
                frames,
                map,
                work: Vec::with_capacity(frames as usize),
                intervals: Vec::with_capacity(frames as usize),
                first: None,
                last: None,
            }
        }

        /// Starts a frame: its timestamp and User Timing mark.
        pub fn begin(&mut self) -> f64 {
            let now = now();
            if let Some(last) = self.last {
                self.intervals.push(now - last);
            }
            self.first.get_or_insert(now);
            self.last = Some(now);
            if let Some(performance) = performance() {
                let _ = performance.mark(FRAME_MARK);
            }
            now
        }

        /// Ends the frame begun at `started`; true once all frames are done.
        pub fn end(&mut self, started: f64) -> bool {
            self.work.push(now() - started);
            if let Some(performance) = performance() {
                let _ = performance.measure_with_start_mark(FRAME_MEASURE, FRAME_MARK);
            }
            if self.work.len().is_multiple_of(30) {
                crate::log(&format!(
                    "profile frame {} of {} at {:.1} s",
                    self.work.len(),
                    self.frames,
                    (now() - self.first.unwrap_or(started)) / 1_000.0
                ));
            }
            self.work.len() >= self.frames as usize
        }

        /// Sets `window.shooterProfile` to the frame times and the IDs of the placeables
        /// broken at the end (`broken`, shooter#60).
        pub fn finish(&self, broken: &[u16]) {
            let total = self.last.zip(self.first).map_or(0.0, |(l, f)| l - f) / 1_000.0;
            let json = |stats: crate::FrameStats| {
                format!(
                    r#"{{"mean":{:.3},"p50":{:.3},"p95":{:.3},"max":{:.3}}}"#,
                    stats.mean, stats.p50, stats.p95, stats.max
                )
            };
            let (width, height) = web_sys::window()
                .map(|window| {
                    let size = |value: Result<wasm_bindgen::JsValue, _>| {
                        value.ok().and_then(|v| v.as_f64()).unwrap_or(0.0)
                    };
                    (size(window.inner_width()), size(window.inner_height()))
                })
                .unwrap_or_default();
            publish(&format!(
                r#"{{"frames":{},"map":"{}","width":{width},"height":{height},"seconds":{total:.3},"workMs":{},"intervalMs":{},"broken":{broken:?}}}"#,
                self.work.len(),
                self.map,
                json(crate::FrameStats::of(self.work.clone())),
                json(crate::FrameStats::of(self.intervals.clone())),
            ));
        }
    }

    /// `window.shooterProfile = <json>`, logged too.
    fn publish(json: &str) {
        crate::log(&format!("shooterProfile {json}"));
        if let (Some(window), Ok(value)) = (web_sys::window(), js_sys::JSON::parse(json)) {
            let _ = js_sys::Reflect::set(&window, &"shooterProfile".into(), &value);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_certificate_hashes_in_plain_and_colon_hex() {
        let plain = "00ff".repeat(16);
        let colons = plain
            .as_bytes()
            .chunks(2)
            .map(|pair| std::str::from_utf8(pair).unwrap().to_uppercase())
            .collect::<Vec<_>>()
            .join(":");
        let expected: [u8; 32] = std::array::from_fn(|i| if i % 2 == 0 { 0 } else { 0xff });
        assert_eq!(super::parse_hash(&plain), Some(expected));
        assert_eq!(super::parse_hash(&colons), Some(expected));
        assert_eq!(super::parse_hash("abc"), None);
        assert_eq!(super::parse_hash(&"zz".repeat(32)), None);
    }

    #[test]
    fn the_practice_map_is_the_default_or_a_catalogued_one() {
        use super::{MapId, practice_map};
        assert_eq!(practice_map(None), Ok(MapId::DEFAULT));
        assert_eq!(practice_map(Some("greybox")), Ok(MapId::Greybox));
        let refused = practice_map(Some("nowhere")).unwrap_err();
        assert!(
            refused.contains("nowhere") && refused.contains("greybox"),
            "{refused}"
        );
    }

    #[test]
    fn a_loadout_is_a_named_one_or_none() {
        use super::loadout;
        use shooter_sim::robot::{MARKSMAN, PYRO, SKIRMISHER};
        assert_eq!(loadout(None), Ok(None));
        assert_eq!(loadout(Some("")), Ok(None));
        assert_eq!(loadout(Some("skirmisher")), Ok(Some(SKIRMISHER)));
        assert_eq!(loadout(Some("Marksman")), Ok(Some(MARKSMAN)));
        assert_eq!(loadout(Some("pyro")), Ok(Some(PYRO)));
        let refused = loadout(Some("sniper")).unwrap_err();
        assert!(
            refused.contains("sniper") && refused.contains("pyro"),
            "{refused}"
        );
    }

    #[test]
    fn the_hud_shows_the_equipped_mods() {
        use crate::game::Game;
        use crate::hud::Hud;
        use shooter_sim::robot::{MARKSMAN, STANDARD};
        let status = |build| {
            let game = Game::practice(super::MapId::DEFAULT).with_loadout(build);
            Hud::default()
                .text(game.snapshot(), game.local_slot(), None, false, 0.0)
                .status
        };
        let marksman = status(MARKSMAN);
        assert!(
            marksman.contains("mods: Energy rifle + long barrel, Rail gun + overcharger"),
            "{marksman}"
        );
        let standard = status(STANDARD);
        assert!(
            !standard.contains("mods") && !standard.contains("Energy rifle"),
            "{standard}"
        );
    }
}
