//! Reading the pinned robot kit: natively from a directory (`scripts/fetch-robot-kit.sh` puts it
//! in `target/robot-kit`; `SHOOTER_ROBOT_KIT` names another), in the browser over HTTP from
//! `robot-kit/` beside the page (`scripts/build-web.sh` copies it into `web/dist`). Both read the
//! manifest, then exactly the files its [`Plan`] lists, each bounded by [`MAX_FILE_BYTES`] (and
//! in the browser by [`FETCH_TIMEOUT_MS`]), and decode them the same way ([`decode`]).

use super::kit::{KitData, MAX_FILE_BYTES, MAX_TOTAL_BYTES, Plan, decode};
use std::collections::HashMap;

/// How long the browser waits for one bundle file (milliseconds).
pub const FETCH_TIMEOUT_MS: u32 = 20_000;

/// The files of one load, by bundle path.
pub type Files = HashMap<String, Vec<u8>>;

/// Decodes a fetched load, timing it.
///
/// # Errors
///
/// As [`decode`].
pub fn finish(plan: &Plan, files: &Files) -> Result<KitData, String> {
    let started = web_time::Instant::now();
    let kit = decode(plan, files)?;
    crate::log(&format!(
        "robot kit: {} entries, {} textures decoded in {:.0} ms",
        kit.entries.len(),
        kit.textures.len(),
        started.elapsed().as_secs_f64() * 1_000.0
    ));
    Ok(kit)
}

/// The native kit directory: `$SHOOTER_ROBOT_KIT`, else the workspace's `target/robot-kit`.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn directory() -> std::path::PathBuf {
    std::env::var_os("SHOOTER_ROBOT_KIT").map_or_else(
        || {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("target")
                .join("robot-kit")
        },
        std::path::PathBuf::from,
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn read_bounded(path: &std::path::Path) -> Result<Vec<u8>, String> {
    let fail = |error: std::io::Error| {
        format!(
            "robot kit: {}: {error} (fetch it with scripts/fetch-robot-kit.sh)",
            path.display()
        )
    };
    let length = std::fs::metadata(path).map_err(fail)?.len();
    if usize::try_from(length).map_or(true, |length| length > MAX_FILE_BYTES) {
        return Err(format!("robot kit: {} is too large", path.display()));
    }
    std::fs::read(path).map_err(fail)
}

/// Reads and decodes the kit in `directory`.
///
/// # Errors
///
/// If a file is missing, too large or not its pinned content, or does not decode.
#[cfg(not(target_arch = "wasm32"))]
pub fn read(directory: &std::path::Path) -> Result<KitData, String> {
    let plan = Plan::of(&read_bounded(&directory.join("manifest.json"))?)?;
    let mut files = Files::new();
    let mut total = 0;
    for file in plan.files() {
        let bytes = read_bounded(&directory.join(&file.path))?;
        total += bytes.len();
        if total > MAX_TOTAL_BYTES {
            return Err("robot kit: the load is too large".into());
        }
        files.insert(file.path, bytes);
    }
    finish(&plan, &files)
}

#[cfg(target_arch = "wasm32")]
mod web {
    use super::{FETCH_TIMEOUT_MS, Files, MAX_FILE_BYTES, MAX_TOTAL_BYTES, Plan};
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    fn js(error: wasm_bindgen::JsValue) -> String {
        error.as_string().unwrap_or_else(|| format!("{error:?}"))
    }

    /// Starts fetching `url`, bounded by [`FETCH_TIMEOUT_MS`].
    fn start(url: &str) -> Result<js_sys::Promise, String> {
        let window = web_sys::window().ok_or("robot kit: no window")?;
        let init = web_sys::RequestInit::new();
        init.set_signal(Some(&web_sys::AbortSignal::timeout_with_u32(
            FETCH_TIMEOUT_MS,
        )));
        Ok(window.fetch_with_str_and_init(url, &init))
    }

    /// The body of a started fetch of `url`, at most [`MAX_FILE_BYTES`].
    async fn body(url: &str, request: js_sys::Promise) -> Result<Vec<u8>, String> {
        let fail = |what: String| format!("robot kit: {url}: {what}");
        let response: web_sys::Response = JsFuture::from(request)
            .await
            .map_err(|error| fail(js(error)))?
            .dyn_into()
            .map_err(|_| fail("not a response".into()))?;
        if !response.ok() {
            return Err(fail(format!("HTTP {}", response.status())));
        }
        let declared = response
            .headers()
            .get("content-length")
            .ok()
            .flatten()
            .and_then(|length| length.parse::<usize>().ok());
        if declared.is_some_and(|length| length > MAX_FILE_BYTES) {
            return Err(fail("too large".into()));
        }
        let buffer = JsFuture::from(response.array_buffer().map_err(|error| fail(js(error)))?)
            .await
            .map_err(|error| fail(js(error)))?;
        let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
        if bytes.len() > MAX_FILE_BYTES {
            return Err(fail("too large".into()));
        }
        Ok(bytes)
    }

    /// Fetches the kit's manifest and the files its plan lists from `base` (a URL ending in
    /// `/`), every request started at once.
    pub async fn fetch(base: &str) -> Result<(Plan, Files), String> {
        let url = format!("{base}manifest.json");
        let plan = Plan::of(&body(&url, start(&url)?).await?)?;
        let requests = plan
            .files()
            .into_iter()
            .map(|file| {
                let url = format!("{base}{}", file.path);
                start(&url).map(|request| (file.path, url, request))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut files = Files::new();
        let mut total = 0;
        for (path, url, request) in requests {
            let bytes = body(&url, request).await?;
            total += bytes.len();
            if total > MAX_TOTAL_BYTES {
                return Err("robot kit: the load is too large".into());
            }
            files.insert(path, bytes);
        }
        Ok((plan, files))
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::fetch;
