//! Reading the pinned kits ([`Bundle::ALL`]; the fighters' three in one load, the arena kit in
//! another): natively from directories (`scripts/fetch-robot-kit.sh` puts them in
//! `target/robot-kit`, `target/weapon-kit`, `target/locomotion-kit` and `target/arena-kit`;
//! `SHOOTER_ROBOT_KIT`, `SHOOTER_WEAPON_KIT`, `SHOOTER_LOCOMOTION_KIT` and `SHOOTER_ARENA_KIT` name
//! others), in the browser over HTTP as each kit's release tar, `robot-kit/kit.tar`,
//! `weapon-kit/kit.tar`, `locomotion-kit/kit.tar` and `arena-kit/kit.tar` beside the page
//! (`scripts/build-web.sh` copies them into `web/dist`), checked against its pinned SHA-256 and
//! read in memory ([`untar`]). One request a kit: Chromium's WebGPU on SwiftShader refuses
//! buffers mapped at creation (every mesh upload) once a page has made many requests
//! (shooter#85). Both read each manifest, then exactly the files its [`Plan`] lists, each bounded
//! by [`MAX_FILE_BYTES`] (and in the browser by [`FETCH_TIMEOUT_MS`] and [`MAX_TAR_BYTES`]), the
//! whole load by [`MAX_TOTAL_BYTES`], and decode them the same way ([`decode`]).

use super::kit::{Bundle, KitData, MAX_FILE_BYTES, MAX_TOTAL_BYTES, Plan, decode};
use std::collections::HashMap;

/// How long the browser waits for one kit's tar (milliseconds).
pub const FETCH_TIMEOUT_MS: u32 = 30_000;
/// The most one kit's release tar may hold (each is about 2 MB).
pub const MAX_TAR_BYTES: usize = 16 << 20;
/// The most files one kit's tar may hold (each has under a hundred).
pub const MAX_TAR_ENTRIES: usize = 1_024;

/// The files of one load, by bundle path (the kits' paths do not collide: resources are named
/// by their content, scenes by their keys).
pub type Files = HashMap<String, Vec<u8>>;

/// Decodes a fetched load, timing it.
///
/// # Errors
///
/// As [`decode`].
pub fn finish(plans: &[Plan], files: &Files) -> Result<KitData, String> {
    let started = web_time::Instant::now();
    let kit = decode(plans, files)?;
    crate::log(&format!(
        "robot kits: {} entries, {} textures decoded in {:.0} ms",
        kit.entries.len(),
        kit.textures.len(),
        started.elapsed().as_secs_f64() * 1_000.0
    ));
    Ok(kit)
}

/// The regular files of the ustar archive `tar` (a kit's release tar), by path, each at most
/// [`MAX_FILE_BYTES`] and at most [`MAX_TAR_ENTRIES`] of them.
///
/// # Errors
///
/// If it is not a plain ustar archive of regular files (directories are skipped), or breaks a
/// bound.
pub fn untar(tar: &[u8]) -> Result<Files, String> {
    let fail = |what: &str| format!("robot kit: tar: {what}");
    let field = |header: &[u8], at: usize, length: usize| -> Result<String, String> {
        let bytes = &header[at..at + length];
        let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(length);
        std::str::from_utf8(&bytes[..end])
            .map(str::to_owned)
            .map_err(|_| fail("a name is not UTF-8"))
    };
    let mut files = Files::new();
    let mut at = 0;
    while at + 512 <= tar.len() {
        let header = &tar[at..at + 512];
        if header.iter().all(|byte| *byte == 0) {
            break;
        }
        if &header[257..262] != b"ustar" {
            return Err(fail("not a ustar archive"));
        }
        let size = field(header, 124, 12)?;
        let size = usize::from_str_radix(size.trim_matches(|c: char| c == ' ' || c == '\0'), 8)
            .map_err(|_| fail("a size is not octal"))?;
        let name = field(header, 0, 100)?;
        let prefix = field(header, 345, 155)?;
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let path = path.trim_start_matches("./").to_owned();
        let start = at + 512;
        let end = start
            .checked_add(size)
            .filter(|end| *end <= tar.len())
            .ok_or_else(|| fail("an entry runs past the end"))?;
        match header[156] {
            b'0' | 0 => {
                if size > MAX_FILE_BYTES {
                    return Err(fail(&format!("{path} is too large")));
                }
                if files.len() >= MAX_TAR_ENTRIES {
                    return Err(fail("too many entries"));
                }
                files.insert(path, tar[start..end].to_vec());
            }
            b'5' => {}
            kind => {
                return Err(fail(&format!(
                    "{path} is not a regular file (type {})",
                    char::from(kind)
                )));
            }
        }
        at = start + size.div_ceil(512) * 512;
    }
    Ok(files)
}

/// The files `plan` reads, taken from a kit's `tar` entries, adding to `total`.
///
/// # Errors
///
/// If one is missing or the load grows past [`MAX_TOTAL_BYTES`].
pub fn take(
    plan: &Plan,
    entries: &mut Files,
    total: &mut usize,
    into: &mut Files,
) -> Result<(), String> {
    for file in plan.files() {
        let bytes = entries
            .remove(&file.path)
            .ok_or_else(|| format!("robot kit: {} is not in its tar", file.path))?;
        *total += bytes.len();
        if *total > MAX_TOTAL_BYTES {
            return Err("robot kit: the load is too large".into());
        }
        into.insert(file.path, bytes);
    }
    Ok(())
}

/// The native directory of `bundle`: `$SHOOTER_ROBOT_KIT` / `$SHOOTER_WEAPON_KIT` /
/// `$SHOOTER_LOCOMOTION_KIT` / `$SHOOTER_ARENA_KIT`, else the workspace's `target/robot-kit` /
/// `target/weapon-kit` / `target/locomotion-kit` / `target/arena-kit`.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn directory(bundle: Bundle) -> std::path::PathBuf {
    let variable = match bundle {
        Bundle::Robot => "SHOOTER_ROBOT_KIT",
        Bundle::Weapons => "SHOOTER_WEAPON_KIT",
        Bundle::Locomotion => "SHOOTER_LOCOMOTION_KIT",
        Bundle::Arena => "SHOOTER_ARENA_KIT",
    };
    std::env::var_os(variable).map_or_else(
        || {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("target")
                .join(bundle.directory())
        },
        std::path::PathBuf::from,
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn read_bounded(path: &std::path::Path) -> Result<Vec<u8>, String> {
    let fail = |error: std::io::Error| {
        format!(
            "robot kit: {}: {error} (fetch the kits with scripts/fetch-robot-kit.sh)",
            path.display()
        )
    };
    let length = std::fs::metadata(path).map_err(fail)?.len();
    if usize::try_from(length).map_or(true, |length| length > MAX_FILE_BYTES) {
        return Err(format!("robot kit: {} is too large", path.display()));
    }
    std::fs::read(path).map_err(fail)
}

/// Reads and decodes the fighters' kits ([`Bundle::FIGHTERS`]) in their [`directory`]s.
///
/// # Errors
///
/// As [`read_from`].
#[cfg(not(target_arch = "wasm32"))]
pub fn read() -> Result<KitData, String> {
    read_from(&Bundle::FIGHTERS.map(|bundle| (bundle, directory(bundle))))
}

/// Reads and decodes the arena kit ([`Bundle::Arena`]) in its [`directory`].
///
/// # Errors
///
/// As [`read_from`].
#[cfg(not(target_arch = "wasm32"))]
pub fn read_arena() -> Result<KitData, String> {
    read_from(&[(Bundle::Arena, directory(Bundle::Arena))])
}

/// Reads and decodes each kit from its directory.
///
/// # Errors
///
/// If a file is missing, too large or not its pinned content, or does not decode.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_from(directories: &[(Bundle, std::path::PathBuf)]) -> Result<KitData, String> {
    let mut plans = Vec::new();
    let mut files = Files::new();
    let mut total = 0;
    for (bundle, directory) in directories {
        let plan = Plan::of(*bundle, &read_bounded(&directory.join("manifest.json"))?)?;
        for file in plan.files() {
            let bytes = read_bounded(&directory.join(&file.path))?;
            total += bytes.len();
            if total > MAX_TOTAL_BYTES {
                return Err("robot kit: the load is too large".into());
            }
            files.insert(file.path, bytes);
        }
        plans.push(plan);
    }
    finish(&plans, &files)
}

#[cfg(target_arch = "wasm32")]
mod web {
    use super::{Bundle, FETCH_TIMEOUT_MS, Files, MAX_TAR_BYTES, Plan, take, untar};
    use crate::robots::kit::{pinned_tar, sha256};
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    fn js(error: wasm_bindgen::JsValue) -> String {
        error.as_string().unwrap_or_else(|| format!("{error:?}"))
    }

    /// The body of `url`, at most [`MAX_TAR_BYTES`], bounded by [`FETCH_TIMEOUT_MS`].
    async fn body(url: &str) -> Result<Vec<u8>, String> {
        let fail = |what: String| format!("robot kit: {url}: {what}");
        let window = web_sys::window().ok_or("robot kit: no window")?;
        let init = web_sys::RequestInit::new();
        init.set_signal(Some(&web_sys::AbortSignal::timeout_with_u32(
            FETCH_TIMEOUT_MS,
        )));
        let response: web_sys::Response =
            JsFuture::from(window.fetch_with_str_and_init(url, &init))
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
        if declared.is_some_and(|length| length > MAX_TAR_BYTES) {
            return Err(fail("too large".into()));
        }
        let buffer = JsFuture::from(response.array_buffer().map_err(|error| fail(js(error)))?)
            .await
            .map_err(|error| fail(js(error)))?;
        let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
        if bytes.len() > MAX_TAR_BYTES {
            return Err(fail("too large".into()));
        }
        Ok(bytes)
    }

    /// Fetches each kit's release tar from `base` (a URL ending in `/`, or empty for the page's
    /// directory) and its kit directory, checks it against its pin and takes the files its
    /// manifest's plan lists: `bundles`, one load.
    pub async fn fetch(base: &str, bundles: &[Bundle]) -> Result<(Vec<Plan>, Files), String> {
        let mut plans = Vec::new();
        let mut files = Files::new();
        let mut total = 0;
        for &bundle in bundles {
            let url = format!("{base}{}/kit.tar", bundle.directory());
            let tar = body(&url).await?;
            let (hash, pin) = (sha256(&tar), pinned_tar(bundle));
            if hash != pin {
                return Err(format!(
                    "robot kit: {url} has SHA-256 {hash}, pinned {pin} (rebuild with scripts/build-web.sh)"
                ));
            }
            let mut entries = untar(&tar)?;
            let manifest = entries
                .remove("manifest.json")
                .ok_or_else(|| format!("robot kit: {url} has no manifest"))?;
            let plan = Plan::of(bundle, &manifest)?;
            take(&plan, &mut entries, &mut total, &mut files)?;
            plans.push(plan);
        }
        Ok((plans, files))
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::fetch;
