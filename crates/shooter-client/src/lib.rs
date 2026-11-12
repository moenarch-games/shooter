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
fn game() -> game::Game {
    #[cfg(not(target_arch = "wasm32"))]
    let (url, trust, name) = {
        let mut args = std::env::args().skip(1);
        let (mut url, mut cert, mut name) = (None, None, None);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--connect" => url = args.next(),
                "--cert" => cert = args.next().map(std::path::PathBuf::from),
                "--name" => name = args.next(),
                other => log(&format!("unknown argument {other}")),
            }
        }
        (url, cert, remembered_name(name))
    };
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
    let event_loop = EventLoop::<app::GpuReady>::with_user_event()
        .build()
        .expect("an event loop");
    let app = app::App::new(event_loop.create_proxy(), game());
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
