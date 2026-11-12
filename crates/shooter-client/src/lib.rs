//! The shooter's client: one crate for the native window (wgpu on Vulkan/Metal/DX12) and the
//! browser (WebGPU, `wasm32`), rendered by render-engine, with input-bindings controls and
//! game-audio footsteps and gunfire, render-engine weapon lights and particles over the shared
//! `shooter-sim` simulation.

pub mod app;
pub mod audio;
pub mod controls;
pub mod effects;
pub mod game;
pub mod gpu;
pub mod hud;
pub mod interpolation;
pub mod net;
pub mod scene;

/// The match to play: natively `--connect <url> [--cert <pem>] [--name <name>]` joins a hosted
/// deathmatch (the certificate is a development host's self-signed one; the name is remembered
/// in the config directory for later runs, [`remembered_name`]); in the browser the page's
/// `?connect=<url>[&certHash=<sha-256 hex>][&name=<name>]` does (the page remembers the name).
/// Anything else is offline practice against bots.
fn game(#[cfg(not(target_arch = "wasm32"))] args: Args) -> game::Game {
    #[cfg(not(target_arch = "wasm32"))]
    let (url, trust, name) = (args.connect, args.cert, remembered_name(args.name));
    #[cfg(target_arch = "wasm32")]
    let (url, trust, name) = {
        let query = web::query("connect");
        let hash = web::query("certHash");
        let name = web::query("name");
        match hash.as_deref().map(parse_hash) {
            Some(None) => {
                fail("certHash is not a SHA-256 in hex");
                (None, None, name)
            }
            hash => (query, hash.flatten(), name),
        }
    };
    if let Some(url) = url {
        match game::online::Online::connect(url, trust, name.as_deref()) {
            Ok(online) => return game::Game::Online(Box::new(online)),
            Err(error) => fail(&error),
        }
    }
    game::Game::practice()
}

/// The native command line.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct Args {
    connect: Option<String>,
    cert: Option<std::path::PathBuf>,
    name: Option<String>,
    /// `--frames <n>`: the offscreen profile run instead of a window ([`offscreen`]).
    frames: Option<u32>,
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
                "--frames" => parsed.frames = args.next().and_then(|n| n.parse().ok()),
                other => log(&format!("unknown argument {other}")),
            }
        }
        parsed
    }
}

/// The offscreen profile run's frame size.
pub const OFFSCREEN_SIZE: (u32, u32) = (1280, 720);

/// The profile run (`shooter-client --frames <n>`): `frames` frames of a practice match at one
/// tick per frame, the local fighter played by the replays' scripted person 0
/// ([`shooter_sim::scenarios::scripted_intent`]), drawn to an offscreen [`OFFSCREEN_SIZE`]
/// target with no window, as fast as the GPU finishes them. Prints frame times; nothing gates
/// on them.
///
/// # Errors
///
/// If there is no GPU adapter or the match fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn offscreen(frames: u32) -> Result<(), String> {
    let (width, height) = OFFSCREEN_SIZE;
    let gpu = gpu::Gpu::offscreen(width, height)?;
    let mut app = app::App::scripted(game::Game::practice(), gpu);
    let mut times = Vec::with_capacity(frames as usize);
    let started = std::time::Instant::now();
    for _ in 0..frames {
        let frame = std::time::Instant::now();
        app.scripted_frame()?;
        times.push(frame.elapsed().as_secs_f64() * 1_000.0);
    }
    let total = started.elapsed().as_secs_f64();
    let stats = FrameStats::of(times);
    println!(
        "offscreen {width}x{height}: {frames} frames in {total:.2} s; frame ms mean {:.2} p50 {:.2} p95 {:.2} max {:.2}",
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

/// The player name file: `shooter/name` in `$XDG_CONFIG_HOME` (else `~/.config`, or
/// `%APPDATA%` on Windows).
#[cfg(not(target_arch = "wasm32"))]
fn name_file() -> Option<std::path::PathBuf> {
    let env = |key| std::env::var_os(key).filter(|value| !value.is_empty());
    let config = env("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| env("HOME").map(|home| std::path::Path::new(&home).join(".config")))
        .or_else(|| env("APPDATA").map(std::path::PathBuf::from))?;
    Some(config.join("shooter").join("name"))
}

/// The name to play under: `given` (`--name`, then remembered for later runs) else the one
/// remembered last time. A file that cannot be read or written only loses the memory.
#[cfg(not(target_arch = "wasm32"))]
fn remembered_name(given: Option<String>) -> Option<String> {
    let file = name_file();
    let given = given.and_then(|name| shooter_sim::names::sanitize(&name));
    match (given, file) {
        (Some(name), Some(file)) => {
            let saved = file
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&file, &name));
            if let Err(error) = saved {
                log(&format!("could not remember the name: {error}"));
            }
            Some(name)
        }
        (given, None) => given,
        (None, Some(file)) => std::fs::read_to_string(file)
            .ok()
            .and_then(|name| shooter_sim::names::sanitize(&name)),
    }
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
    if let Some(frames) = args.frames {
        if let Err(error) = offscreen(frames) {
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
    #[cfg(target_arch = "wasm32")]
    let game = match profile {
        Some(_) => game::Game::practice(),
        None => game(),
    };
    #[cfg(not(target_arch = "wasm32"))]
    let game = game(args);
    let app = app::App::new(event_loop.create_proxy(), game);
    #[cfg(target_arch = "wasm32")]
    let app = match profile {
        Some(frames) => app.profiled(frames),
        None => app,
    };
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
        console_error_panic_hook::set_once();
        super::run();
    }

    fn document() -> Option<web_sys::Document> {
        web_sys::window()?.document()
    }

    /// Whether the page holds Pointer Lock (the browser releases it on Escape).
    pub fn pointer_locked() -> bool {
        document().is_some_and(|document| document.pointer_lock_element().is_some())
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

    /// The browser profile run (`?frames=<n>`): like the native `--frames`, `frames` frames of
    /// a practice match at one tick per animation frame, the local fighter played by the
    /// replays' scripted person 0. When done it sets `window.shooterProfile` to its frame
    /// times; nothing gates on them.
    pub struct Profile {
        pub frames: u32,
        /// Milliseconds in the frame's work (the tick, the scene and the GPU submission).
        work: Vec<f64>,
        /// Milliseconds between consecutive frames' starts (including waiting on the GPU).
        intervals: Vec<f64>,
        first: Option<f64>,
        last: Option<f64>,
    }

    impl Profile {
        #[must_use]
        pub fn new(frames: u32) -> Self {
            Self {
                frames,
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
            self.work.len() >= self.frames as usize
        }

        /// Sets `window.shooterProfile` to the frame times.
        pub fn finish(&self) {
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
                r#"{{"frames":{},"width":{width},"height":{height},"seconds":{total:.3},"workMs":{},"intervalMs":{}}}"#,
                self.work.len(),
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
}
