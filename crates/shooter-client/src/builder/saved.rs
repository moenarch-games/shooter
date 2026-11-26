//! Saved robot builds (shooter#140): the person saves the build on the bench and its control
//! layout under a name, and later loads, overwrites or deletes it, before the match and between
//! lives. Both clients share everything here but where the saves live ([`SavedBuilds`]): files
//! in the user's data directory natively ([`FileBuilds`]), the page's `localStorage` in the
//! browser ([`LocalStorageBuilds`]).
//!
//! **Format.** A save is a small versioned envelope ([`encode`]): the magic `SHBD`, the format
//! version ([`VERSION`], u16), the name (a length byte and its bytes), the build in the
//! simulation's own encoding ([`shooter_sim::wire::encode_build`], a u16 length first) and the
//! control layout in its own ([`shooter_sim::wire::encode_bindings`], a flag byte first: none
//! means the build's preset). Stores keep its bytes as lowercase hex text.
//!
//! **Loading.** A save goes through the same validation as any build on the bench
//! ([`super::BENCH`], every rule but the build-point ceiling, so a build over budget that was
//! saved while it was being worked on comes back to be finished) and its layout through the
//! bindings check against that build ([`Bindings::validate`]). A save of another format version,
//! one that does not decode, or one the rules refuse is refused with its reason
//! ([`LoadError`]), and the bench keeps its build.

use super::BENCH;
use crate::prefs::PrefStore;
use shooter_sim::bindings::{BindingError, Bindings};
use shooter_sim::robot::{Assembly, AssemblyError, BETA};
use shooter_sim::wire::{WireError, decode_bindings, decode_build, encode_bindings, encode_build};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;
use std::rc::Rc;

/// The envelope's first bytes.
pub const MAGIC: [u8; 4] = *b"SHBD";
/// The save format this client writes and reads.
pub const VERSION: u16 = 1;
/// The longest name.
pub const MAX_NAME: usize = 16;

/// A build saved under a name, with the control layout the person edited for it (none: the
/// build's preset).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SavedBuild {
    pub name: String,
    pub build: Assembly,
    pub layout: Option<Bindings>,
}

/// Why a save does not load.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoadError {
    /// No save of that name.
    Missing(String),
    /// Not a saved build (no magic, not hex).
    NotASave,
    /// A save of another format version.
    Version(u16),
    /// A save that does not decode.
    Malformed(&'static str),
    /// A build the rules refuse.
    Build(AssemblyError),
    /// A layout the build cannot take.
    Layout(BindingError),
    /// The store failed.
    Store(String),
}

impl fmt::Display for LoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(name) => write!(formatter, "no saved build named {name}"),
            Self::NotASave => write!(formatter, "that is no saved build: delete it"),
            Self::Version(version) if *version > VERSION => write!(
                formatter,
                "saved by a newer client (format {version}, this one reads {VERSION}): update the game"
            ),
            Self::Version(version) => write!(
                formatter,
                "saved in an old format ({version}, now {VERSION}): build it again and save it"
            ),
            Self::Malformed(reason) => {
                write!(formatter, "the save is damaged ({reason}): delete it")
            }
            Self::Build(error) => write!(
                formatter,
                "the saved robot breaks a rule ({error}): delete it or save over it"
            ),
            Self::Layout(error) => write!(
                formatter,
                "the saved controls do not fit the robot ({error}): delete it or save over it"
            ),
            Self::Store(error) => write!(formatter, "{error}"),
        }
    }
}

impl From<WireError> for LoadError {
    fn from(error: WireError) -> Self {
        match error {
            WireError::Malformed(reason) => Self::Malformed(reason),
            WireError::Build(error) => Self::Build(error),
        }
    }
}

/// `name` as saves are named: letters (upper case), digits, `-` and `_` (a space becomes `_`),
/// at most [`MAX_NAME`] of them; none if nothing is left.
#[must_use]
pub fn sanitise(name: &str) -> Option<String> {
    let name: String = name
        .trim()
        .chars()
        .filter_map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => Some(c.to_ascii_uppercase()),
            ' ' => Some('_'),
            _ => None,
        })
        .take(MAX_NAME)
        .collect();
    (!name.is_empty()).then_some(name)
}

/// The save's bytes ([module docs](self)).
#[must_use]
pub fn envelope(saved: &SavedBuild) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend(VERSION.to_le_bytes());
    let name = saved.name.as_bytes();
    let name = &name[..name.len().min(usize::from(u8::MAX))];
    #[allow(clippy::cast_possible_truncation)]
    out.push(name.len() as u8);
    out.extend(name);
    let build = encode_build(&saved.build);
    #[allow(clippy::cast_possible_truncation)]
    out.extend((build.len() as u16).to_le_bytes());
    out.extend(build);
    match &saved.layout {
        Some(layout) => {
            let layout = encode_bindings(layout);
            out.push(1);
            #[allow(clippy::cast_possible_truncation)]
            out.extend((layout.len() as u16).to_le_bytes());
            out.extend(layout);
        }
        None => out.push(0),
    }
    out
}

/// The save as the stores keep it: [`envelope`] in hex.
#[must_use]
pub fn encode(saved: &SavedBuild) -> String {
    envelope(saved)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Inverse of [`encode`], validated ([module docs](self)).
///
/// # Errors
///
/// Why the save does not load.
pub fn decode(text: &str) -> Result<SavedBuild, LoadError> {
    let text = text.trim();
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return Err(LoadError::NotASave);
    }
    let bytes = (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| LoadError::NotASave)?;
    open(&bytes)
}

/// Inverse of [`envelope`], validated.
///
/// # Errors
///
/// Why the save does not load.
pub fn open(bytes: &[u8]) -> Result<SavedBuild, LoadError> {
    let mut input = bytes;
    let mut take = |count: usize| -> Result<&[u8], LoadError> {
        if input.len() < count {
            return Err(LoadError::Malformed("truncated"));
        }
        let (head, rest) = input.split_at(count);
        input = rest;
        Ok(head)
    };
    if take(MAGIC.len()).map_err(|_| LoadError::NotASave)? != MAGIC {
        return Err(LoadError::NotASave);
    }
    let version = u16::from_le_bytes([take(1)?[0], take(1)?[0]]);
    if version != VERSION {
        return Err(LoadError::Version(version));
    }
    let length = usize::from(take(1)?[0]);
    let name = String::from_utf8(take(length)?.to_vec())
        .map_err(|_| LoadError::Malformed("name is not UTF-8"))?;
    let length = usize::from(u16::from_le_bytes([take(1)?[0], take(1)?[0]]));
    let candidate = decode_build(take(length)?)?;
    let layout = match take(1)?[0] {
        0 => None,
        1 => {
            let length = usize::from(u16::from_le_bytes([take(1)?[0], take(1)?[0]]));
            Some(decode_bindings(take(length)?)?)
        }
        _ => return Err(LoadError::Malformed("unknown layout flag")),
    };
    if !input.is_empty() {
        return Err(LoadError::Malformed("trailing bytes"));
    }
    let build = BENCH.validate(&candidate).map_err(LoadError::Build)?;
    if let Some(layout) = &layout {
        layout.validate(&build).map_err(LoadError::Layout)?;
    }
    Ok(SavedBuild {
        name,
        build,
        layout,
    })
}

/// Where saves live: text under a sanitised name ([`sanitise`]). Every call that can fail
/// returns why (a full, blocked or missing store), never panics.
pub trait SavedBuilds: fmt::Debug {
    /// The saves' names, sorted.
    ///
    /// # Errors
    ///
    /// If the store cannot be read.
    fn names(&self) -> Result<Vec<String>, String>;
    /// The save called `name`, if there is one.
    ///
    /// # Errors
    ///
    /// If the store cannot be read.
    fn read(&self, name: &str) -> Result<Option<String>, String>;
    /// Keeps `text` as `name`, over any save of that name.
    ///
    /// # Errors
    ///
    /// If the store cannot be written (full, blocked).
    fn write(&self, name: &str, text: &str) -> Result<(), String>;
    /// Forgets the save called `name` (none: nothing happens).
    ///
    /// # Errors
    ///
    /// If the store cannot be written.
    fn delete(&self, name: &str) -> Result<(), String>;
}

/// Saves in memory: the fallback where no store can be found (they last as long as the game),
/// scripted views and tests.
#[derive(Debug, Default)]
pub struct MemoryBuilds(RefCell<BTreeMap<String, String>>);

impl SavedBuilds for MemoryBuilds {
    fn names(&self) -> Result<Vec<String>, String> {
        Ok(self.0.borrow().keys().cloned().collect())
    }
    fn read(&self, name: &str) -> Result<Option<String>, String> {
        Ok(self.0.borrow().get(name).cloned())
    }
    fn write(&self, name: &str, text: &str) -> Result<(), String> {
        self.0.borrow_mut().insert(name.to_owned(), text.to_owned());
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<(), String> {
        self.0.borrow_mut().remove(name);
        Ok(())
    }
}

/// Saves as files, one per build (`<NAME>.build`), in a directory.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
pub struct FileBuilds {
    pub dir: std::path::PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl FileBuilds {
    const EXTENSION: &str = "build";

    /// The user's data directory's `shooter/builds`: `%APPDATA%` on Windows,
    /// `~/Library/Application Support` on macOS, else `$XDG_DATA_HOME` or `~/.local/share`.
    #[must_use]
    pub fn platform() -> Option<Self> {
        let env = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
        let base = if cfg!(windows) {
            std::path::PathBuf::from(env("APPDATA")?)
        } else if cfg!(target_os = "macos") {
            std::path::PathBuf::from(env("HOME")?).join("Library/Application Support")
        } else {
            env("XDG_DATA_HOME").map_or_else(
                || env("HOME").map(|home| std::path::PathBuf::from(home).join(".local/share")),
                |data| Some(std::path::PathBuf::from(data)),
            )?
        };
        Some(Self {
            dir: base.join("shooter").join("builds"),
        })
    }

    fn path(&self, name: &str) -> Result<std::path::PathBuf, String> {
        let name = sanitise(name)
            .filter(|safe| safe == name)
            .ok_or_else(|| format!("{name} is no build name"))?;
        Ok(self.dir.join(format!("{name}.{}", Self::EXTENSION)))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl SavedBuilds for FileBuilds {
    fn names(&self) -> Result<Vec<String>, String> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(format!("can't read {}: {error}", self.dir.display())),
        };
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension()? != Self::EXTENSION {
                    return None;
                }
                let stem = path.file_stem()?.to_str()?;
                (sanitise(stem).as_deref() == Some(stem)).then(|| stem.to_owned())
            })
            .collect();
        names.sort();
        Ok(names)
    }
    fn read(&self, name: &str) -> Result<Option<String>, String> {
        let path = self.path(name)?;
        match std::fs::read_to_string(&path) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("can't read {}: {error}", path.display())),
        }
    }
    fn write(&self, name: &str, text: &str) -> Result<(), String> {
        let path = self.path(name)?;
        std::fs::create_dir_all(&self.dir)
            .and_then(|()| std::fs::write(&path, text))
            .map_err(|error| format!("can't save {}: {error}", path.display()))
    }
    fn delete(&self, name: &str) -> Result<(), String> {
        let path = self.path(name)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("can't delete {}: {error}", path.display())),
        }
    }
}

/// Saves in the page's `localStorage`, one key per build (`shooter.build.<NAME>`). A blocked
/// store (private windows, disabled storage) or a full one is an error, never a panic.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalStorageBuilds;

#[cfg(target_arch = "wasm32")]
impl LocalStorageBuilds {
    const PREFIX: &str = "shooter.build.";

    fn storage() -> Result<web_sys::Storage, String> {
        web_sys::window()
            .ok_or("no window")?
            .local_storage()
            .map_err(|error| format!("browser storage is blocked: {}", js(&error)))?
            .ok_or_else(|| "browser storage is off".to_owned())
    }
}

#[cfg(target_arch = "wasm32")]
fn js(error: &wasm_bindgen::JsValue) -> String {
    error
        .as_string()
        .or_else(|| {
            js_sys::Reflect::get(error, &"message".into())
                .ok()?
                .as_string()
        })
        .unwrap_or_else(|| format!("{error:?}"))
}

#[cfg(target_arch = "wasm32")]
impl SavedBuilds for LocalStorageBuilds {
    fn names(&self) -> Result<Vec<String>, String> {
        let storage = Self::storage()?;
        let length = storage
            .length()
            .map_err(|error| format!("can't read browser storage: {}", js(&error)))?;
        let mut names = Vec::new();
        for index in 0..length {
            let key = storage
                .key(index)
                .map_err(|error| format!("can't read browser storage: {}", js(&error)))?;
            if let Some(name) = key
                .as_deref()
                .and_then(|key| key.strip_prefix(Self::PREFIX))
                && sanitise(name).as_deref() == Some(name)
            {
                names.push(name.to_owned());
            }
        }
        names.sort();
        Ok(names)
    }
    fn read(&self, name: &str) -> Result<Option<String>, String> {
        Self::storage()?
            .get_item(&format!("{}{name}", Self::PREFIX))
            .map_err(|error| format!("can't read browser storage: {}", js(&error)))
    }
    fn write(&self, name: &str, text: &str) -> Result<(), String> {
        Self::storage()?
            .set_item(&format!("{}{name}", Self::PREFIX), text)
            .map_err(|error| format!("can't save in browser storage (full?): {}", js(&error)))
    }
    fn delete(&self, name: &str) -> Result<(), String> {
        Self::storage()?
            .remove_item(&format!("{}{name}", Self::PREFIX))
            .map_err(|error| format!("can't delete from browser storage: {}", js(&error)))
    }
}

/// This client's store: the page's `localStorage` in the browser, the data directory's files
/// natively (in memory if there is no data directory).
#[must_use]
pub fn platform() -> Rc<dyn SavedBuilds> {
    #[cfg(target_arch = "wasm32")]
    {
        Rc::new(LocalStorageBuilds)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        match FileBuilds::platform() {
            Some(files) => Rc::new(files),
            None => Rc::new(MemoryBuilds::default()),
        }
    }
}

/// The preference key the active build is kept under ([`read_active`]).
pub const ACTIVE: &str = "active";

/// The active build (shooter#155): the one the menu's matches play, readied last (the
/// Workshop's Done, a match's Ready), kept across restarts with the person's preferences
/// ([`crate::prefs`]: natively the config directory's `shooter/active`, in the browser
/// `localStorage`'s `shooter.active`) as a save in this module's envelope, named after the save
/// it was readied as (empty: one never saved, or edited since). None if nothing was readied
/// yet; else the build and layout, or why they no longer play: a save that does not open, or a
/// build the beta catalog now refuses in full (the build points too, unlike a save for the
/// bench). The store is left as it is either way: only the next Ready replaces it.
#[must_use]
pub fn read_active(store: &dyn PrefStore) -> Option<Result<SavedBuild, LoadError>> {
    let text = store.read(ACTIVE)?;
    Some(decode(&text).and_then(|saved| {
        BETA.validate(&saved.build.candidate())
            .map_err(LoadError::Build)?;
        Ok(saved)
    }))
}

/// Keeps `active` as the active build ([`read_active`]).
///
/// # Errors
///
/// If the store cannot be written.
pub fn write_active(store: &dyn PrefStore, active: &SavedBuild) -> Result<(), String> {
    store.write(ACTIVE, Some(&encode(active)))
}

/// A second press that a save or a delete waits for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Confirm {
    Overwrite(String),
    Delete(String),
}

/// The builds panel's state: the name being typed, the saves listed, the one selected, and the
/// overwrite or delete waiting for its second press.
#[derive(Clone, Debug)]
pub struct Shelf {
    store: Rc<dyn SavedBuilds>,
    pub name: String,
    pub names: Vec<String>,
    pub selected: Option<usize>,
    pub confirm: Option<Confirm>,
}

impl Shelf {
    #[must_use]
    pub fn new(store: Rc<dyn SavedBuilds>) -> Self {
        Self {
            store,
            name: String::new(),
            names: Vec::new(),
            selected: None,
            confirm: None,
        }
    }

    /// Lists the saves again (the selection follows its name).
    ///
    /// # Errors
    ///
    /// If the store cannot be read.
    pub fn refresh(&mut self) -> Result<(), String> {
        let selected = self.selected_name();
        self.names = self.store.names()?;
        self.selected = selected.and_then(|name| self.names.iter().position(|n| *n == name));
        Ok(())
    }

    fn selected_name(&self) -> Option<String> {
        self.selected
            .and_then(|index| self.names.get(index))
            .cloned()
    }

    /// Types `c` into the name (if it is allowed and there is room).
    pub fn type_char(&mut self, c: char) {
        self.confirm = None;
        if let Some(name) = sanitise(&format!("{}{c}", self.name)) {
            self.name = name;
        }
    }

    /// Erases the name's last letter.
    pub fn erase(&mut self) {
        self.confirm = None;
        self.name.pop();
    }

    /// Selects the save listed at `index`, its name in the field.
    pub fn select(&mut self, index: usize) {
        self.confirm = None;
        if let Some(name) = self.names.get(index) {
            self.selected = Some(index);
            self.name.clone_from(name);
        }
    }

    /// The name a load or delete is for: the typed name if one is listed, else the selected.
    fn target(&self) -> Option<String> {
        sanitise(&self.name)
            .filter(|name| self.names.contains(name))
            .or_else(|| self.selected_name())
    }

    /// Saves `build` and `layout` under the typed name; a name already saved waits for a second
    /// press. What happened, or why not.
    ///
    /// # Errors
    ///
    /// No name, or the store failed.
    pub fn save(&mut self, build: Assembly, layout: Option<Bindings>) -> Result<String, String> {
        let name = sanitise(&self.name).ok_or("type a name for the build first")?;
        let exists = self.store.read(&name)?.is_some();
        if exists && self.confirm != Some(Confirm::Overwrite(name.clone())) {
            self.confirm = Some(Confirm::Overwrite(name.clone()));
            return Ok(format!(
                "{name} is saved already: save again to overwrite it"
            ));
        }
        self.confirm = None;
        let text = encode(&SavedBuild {
            name: name.clone(),
            build,
            layout,
        });
        self.store.write(&name, &text)?;
        self.refresh()?;
        self.selected = self.names.iter().position(|n| *n == name);
        Ok(if exists {
            format!("overwrote {name}")
        } else {
            format!("saved {name}")
        })
    }

    /// The save to load: the typed name if it is listed, else the selected one.
    ///
    /// # Errors
    ///
    /// Nothing to load, the store failed or the save does not load.
    pub fn load(&mut self) -> Result<SavedBuild, LoadError> {
        self.confirm = None;
        let name = self
            .target()
            .ok_or_else(|| LoadError::Store("pick a saved build to load".to_owned()))?;
        let text = self
            .store
            .read(&name)
            .map_err(LoadError::Store)?
            .ok_or_else(|| LoadError::Missing(name.clone()))?;
        decode(&text)
    }

    /// Deletes the save to load ([`Self::load`]'s) on a second press.
    ///
    /// # Errors
    ///
    /// Nothing to delete, or the store failed.
    pub fn delete(&mut self) -> Result<String, String> {
        let name = self.target().ok_or("pick a saved build to delete")?;
        if self.confirm != Some(Confirm::Delete(name.clone())) {
            self.confirm = Some(Confirm::Delete(name.clone()));
            return Ok(format!("delete {name}? press delete again"));
        }
        self.confirm = None;
        self.store.delete(&name)?;
        self.selected = None;
        self.refresh()?;
        Ok(format!("deleted {name}"))
    }

    /// The name of a save holding exactly `build` and `layout`: the typed or selected one if it
    /// does, else the first listed that does. None if no save does (or the store cannot be read).
    #[must_use]
    pub fn saved_as(&self, build: &Assembly, layout: Option<&Bindings>) -> Option<String> {
        let names = self.store.names().ok()?;
        let holds = |name: &String| {
            self.store
                .read(name)
                .ok()
                .flatten()
                .and_then(|text| decode(&text).ok())
                .is_some_and(|saved| saved.build == *build && saved.layout.as_ref() == layout)
        };
        let preferred = sanitise(&self.name).or_else(|| self.selected_name());
        preferred
            .filter(|name| names.contains(name) && holds(name))
            .or_else(|| names.into_iter().find(holds))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shooter_sim::bindings::{Binding, Mode, Output, Source, Target};
    use shooter_sim::robot::{
        Candidate, HEAVY, MECHANIC, PartId, RawPart, STANDARD, parts::CHASSIS_M,
    };

    fn layout_of(build: &Assembly) -> Bindings {
        let mut layout = Bindings::preset_of(build);
        layout.bindings.truncate(1);
        layout
    }

    #[test]
    fn a_save_round_trips_the_identical_build_and_layout() {
        for (build, layout) in [
            (STANDARD, None),
            (HEAVY, None),
            (MECHANIC, Some(Bindings::preset_of(&MECHANIC))),
            (MECHANIC, Some(layout_of(&MECHANIC))),
        ] {
            let saved = SavedBuild {
                name: "MY-BOT_1".to_owned(),
                build,
                layout,
            };
            let text = encode(&saved);
            assert!(text.starts_with("53484244"), "{text}");
            assert_eq!(decode(&text), Ok(saved));
        }
    }

    #[test]
    fn another_format_version_is_refused_with_a_reason() {
        let saved = SavedBuild {
            name: "OLD".to_owned(),
            build: STANDARD,
            layout: None,
        };
        let mut bytes = envelope(&saved);
        bytes[4] = 0;
        assert_eq!(open(&bytes), Err(LoadError::Version(0)));
        assert!(LoadError::Version(0).to_string().contains("old format"));
        bytes[4] = 9;
        assert!(
            open(&bytes)
                .unwrap_err()
                .to_string()
                .contains("newer client")
        );
        assert_eq!(decode("not a save"), Err(LoadError::NotASave));
        assert_eq!(decode("0011"), Err(LoadError::NotASave));
        let mut cut = envelope(&saved);
        cut.pop();
        assert!(matches!(open(&cut), Err(LoadError::Malformed(_))));
    }

    #[test]
    fn an_invalid_build_or_layout_is_refused_with_its_reason() {
        // A part that names a parent there is no such part as.
        let candidate = Candidate {
            parts: vec![
                RawPart {
                    id: 0,
                    kind: CHASSIS_M.id.0,
                    mount: None,
                    modification: None,
                },
                RawPart {
                    id: 1,
                    kind: 250,
                    mount: None,
                    modification: None,
                },
            ],
        };
        let build_bytes = {
            // The wire encoding of the raw parts: count, then id, kind, flags.
            let mut bytes = vec![2];
            for part in &candidate.parts {
                bytes.extend([part.id, part.kind, 0]);
            }
            bytes
        };
        let mut bytes = MAGIC.to_vec();
        bytes.extend(VERSION.to_le_bytes());
        bytes.extend([3, b'B', b'A', b'D']);
        #[allow(clippy::cast_possible_truncation)]
        bytes.extend((build_bytes.len() as u16).to_le_bytes());
        bytes.extend(build_bytes);
        bytes.push(0);
        let error = open(&bytes).unwrap_err();
        assert!(matches!(error, LoadError::Build(_)), "{error:?}");
        assert!(error.to_string().contains("breaks a rule"), "{error}");

        // A layout driving a part the build has no actuator as.
        let layout = Bindings {
            gait: Bindings::preset_of(&STANDARD).gait,
            bindings: vec![Binding {
                source: Source::Key(0),
                mode: Mode::Hold,
                output: Output::Rate,
                invert: false,
                strength: 50,
                targets: vec![Target {
                    part: PartId(0),
                    invert: false,
                }],
            }],
        };
        let text = encode(&SavedBuild {
            name: "X".to_owned(),
            build: STANDARD,
            layout: Some(layout),
        });
        let error = decode(&text).unwrap_err();
        assert!(matches!(error, LoadError::Layout(_)), "{error:?}");
    }

    #[test]
    fn names_are_sanitised() {
        assert_eq!(sanitise("my bot"), Some("MY_BOT".to_owned()));
        assert_eq!(sanitise("../../etc/passwd"), Some("ETCPASSWD".to_owned()));
        assert_eq!(sanitise("a:b\\c?d*e.f"), Some("ABCDEF".to_owned()));
        assert_eq!(
            sanitise("x".repeat(40).as_str()).map(|n| n.len()),
            Some(MAX_NAME)
        );
        assert_eq!(sanitise("  "), None);
        assert_eq!(sanitise("..."), None);
        assert_eq!(sanitise("Ünïcode-7"), Some("NCODE-7".to_owned()));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_file_store_saves_lists_loads_overwrites_and_deletes() {
        let dir = std::env::temp_dir().join(format!(
            "shooter-saved-builds-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let files = FileBuilds {
            dir: dir.join("builds"),
        };
        // No directory yet: nothing saved.
        assert_eq!(files.names(), Ok(Vec::new()));
        assert_eq!(files.read("A"), Ok(None));
        let mut shelf = Shelf::new(Rc::new(files.clone()));
        shelf.name = "heavy one".to_owned();
        assert_eq!(shelf.save(HEAVY, None), Ok("saved HEAVY_ONE".to_owned()));
        shelf.name = "std".to_owned();
        shelf.save(STANDARD, None).unwrap();
        assert_eq!(
            files.names(),
            Ok(vec!["HEAVY_ONE".to_owned(), "STD".to_owned()])
        );
        assert!(dir.join("builds/HEAVY_ONE.build").is_file());
        // Not a save: ignored by the list.
        std::fs::write(dir.join("builds/notes.txt"), "x").unwrap();
        assert_eq!(files.names().unwrap().len(), 2);

        shelf.refresh().unwrap();
        shelf.select(0);
        assert_eq!(shelf.load().unwrap().build, HEAVY);

        // Overwrite waits for the second press.
        shelf.name = "STD".to_owned();
        let asked = shelf.save(HEAVY, None).unwrap();
        assert!(asked.contains("save again"), "{asked}");
        assert_eq!(shelf.load().unwrap().build, STANDARD);
        assert_eq!(
            shelf.save(HEAVY, None).unwrap(),
            "STD is saved already: save again to overwrite it"
        );
        assert_eq!(shelf.save(HEAVY, None).unwrap(), "overwrote STD");
        assert_eq!(shelf.load().unwrap().build, HEAVY);

        // Delete too.
        assert!(shelf.delete().unwrap().contains("again"));
        assert_eq!(files.names().unwrap().len(), 2);
        assert_eq!(shelf.delete(), Ok("deleted STD".to_owned()));
        assert_eq!(files.names(), Ok(vec!["HEAVY_ONE".to_owned()]));
        assert_eq!(shelf.names, vec!["HEAVY_ONE".to_owned()]);

        // A name that is no file name is refused, not written somewhere else.
        assert!(files.write("../X", "y").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A store that is full or switched off.
    #[derive(Debug)]
    struct Broken {
        read: bool,
    }

    impl SavedBuilds for Broken {
        fn names(&self) -> Result<Vec<String>, String> {
            if self.read {
                Ok(vec!["A".to_owned()])
            } else {
                Err("browser storage is off".to_owned())
            }
        }
        fn read(&self, name: &str) -> Result<Option<String>, String> {
            if self.read {
                Ok((name == "A").then(|| "53484244".to_owned()))
            } else {
                Err("browser storage is off".to_owned())
            }
        }
        fn write(&self, _: &str, _: &str) -> Result<(), String> {
            Err("can't save in browser storage (full?): QuotaExceededError".to_owned())
        }
        fn delete(&self, _: &str) -> Result<(), String> {
            Err("can't delete from browser storage: blocked".to_owned())
        }
    }

    #[test]
    fn a_full_or_blocked_store_is_an_error_not_a_panic() {
        let mut off = Shelf::new(Rc::new(Broken { read: false }));
        assert_eq!(off.refresh(), Err("browser storage is off".to_owned()));
        off.name = "A".to_owned();
        assert_eq!(
            off.save(STANDARD, None),
            Err("browser storage is off".to_owned())
        );
        assert!(matches!(off.load(), Err(LoadError::Store(_))));

        let mut full = Shelf::new(Rc::new(Broken { read: true }));
        full.refresh().unwrap();
        full.name = "NEW".to_owned();
        let error = full.save(STANDARD, None).unwrap_err();
        assert!(error.contains("full"), "{error}");
        full.select(0);
        assert!(matches!(full.load(), Err(LoadError::Malformed(_))));
        full.delete().unwrap();
        assert!(full.delete().unwrap_err().contains("blocked"));
    }
}

#[cfg(test)]
mod active {
    use super::*;
    use crate::prefs::MemoryPrefs;
    use shooter_sim::robot::{HEAVY, MECHANIC, STANDARD};

    /// The active build comes back after a restart (a new store on the same directory), named
    /// after its save, with its layout; nothing readied yet is none.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_active_build_survives_a_restart() {
        use crate::prefs::FilePrefs;
        let dir = std::env::temp_dir().join(format!("shooter-active-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = FilePrefs { dir: dir.clone() };
        assert_eq!(read_active(&store), None);
        let active = SavedBuild {
            name: "SCOUT".to_owned(),
            build: MECHANIC,
            layout: Some(Bindings::preset_of(&MECHANIC)),
        };
        write_active(&store, &active).unwrap();
        let restarted = FilePrefs { dir: dir.clone() };
        assert_eq!(read_active(&restarted), Some(Ok(active)));
        assert!(dir.join(ACTIVE).is_file());
        // One never saved under a name.
        let unnamed = SavedBuild {
            name: String::new(),
            build: HEAVY,
            layout: None,
        };
        write_active(&restarted, &unnamed).unwrap();
        assert_eq!(read_active(&store), Some(Ok(unnamed)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A stored active build that no longer plays (damaged, of another format, over the build
    /// points) is refused with its reason and left where it is.
    #[test]
    fn an_active_build_that_no_longer_plays_is_refused_and_kept() {
        let store = MemoryPrefs::default();
        store.write(ACTIVE, Some("not a save")).unwrap();
        assert_eq!(read_active(&store), Some(Err(LoadError::NotASave)));
        let over = SavedBuild {
            name: "BIG".to_owned(),
            build: crate::builder::over_budget(),
            layout: None,
        };
        // It opens on the bench, but does not play.
        assert_eq!(
            decode(&encode(&over)).map(|saved| saved.build),
            Ok(over.build)
        );
        write_active(&store, &over).unwrap();
        assert!(matches!(
            read_active(&store),
            Some(Err(LoadError::Build(AssemblyError::OverBudget { .. })))
        ));
        assert_eq!(store.read(ACTIVE), Some(encode(&over)));
    }

    /// The name an active build is readied as: the typed or selected save holding it exactly,
    /// else any that does, else none (edited since, or never saved).
    #[test]
    fn a_readied_build_is_named_after_the_save_holding_it() {
        let mut shelf = Shelf::new(Rc::new(MemoryBuilds::default()));
        assert_eq!(shelf.saved_as(&STANDARD, None), None);
        shelf.name = "FIRST".to_owned();
        shelf.save(HEAVY, None).unwrap();
        shelf.name = "SECOND".to_owned();
        shelf.save(HEAVY, None).unwrap();
        shelf.name = "OTHER".to_owned();
        shelf.save(MECHANIC, None).unwrap();
        assert_eq!(shelf.saved_as(&MECHANIC, None).as_deref(), Some("OTHER"));
        // The typed name first, then the first listed.
        shelf.name = "SECOND".to_owned();
        assert_eq!(shelf.saved_as(&HEAVY, None).as_deref(), Some("SECOND"));
        shelf.name.clear();
        shelf.selected = None;
        assert_eq!(shelf.saved_as(&HEAVY, None).as_deref(), Some("FIRST"));
        // Another layout or build is not that save.
        let layout = Bindings::preset_of(&HEAVY);
        assert_eq!(shelf.saved_as(&HEAVY, Some(&layout)), None);
        assert_eq!(shelf.saved_as(&STANDARD, None), None);
    }
}
