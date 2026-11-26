//! The person's preferences, set in the main menu's Settings ([`crate::menu`], shooter#154): the
//! name they play under online and the mouse's look sensitivity. Each is one small value under
//! its own key, kept where this client keeps such things: natively a file per key in the config
//! directory's `shooter/` (`$XDG_CONFIG_HOME`, else `~/.config`, or `%APPDATA%`; the name in
//! `shooter/name`, as `--name` has always remembered it), in the browser the page's
//! `localStorage` under `shooter.<key>` (the name in `shooter.name`, as the page has always kept
//! it). The active build (shooter#155) is kept here too, under `active`
//! ([`crate::builder::saved::read_active`]). A store that cannot be read or written only loses the
//! memory: the defaults play.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

/// Where preferences are kept, one text value per key.
pub trait PrefStore {
    fn read(&self, key: &str) -> Option<String>;
    /// Sets `key` to `value`, or forgets it for none.
    ///
    /// # Errors
    ///
    /// If the store cannot be written.
    fn write(&self, key: &str, value: Option<&str>) -> Result<(), String>;
}

/// Preferences that last as long as the process (tests, and a client with no config directory).
#[derive(Debug, Default)]
pub struct MemoryPrefs(RefCell<BTreeMap<String, String>>);

impl PrefStore for MemoryPrefs {
    fn read(&self, key: &str) -> Option<String> {
        self.0.borrow().get(key).cloned()
    }
    fn write(&self, key: &str, value: Option<&str>) -> Result<(), String> {
        let mut values = self.0.borrow_mut();
        match value {
            Some(value) => values.insert(key.to_owned(), value.to_owned()),
            None => values.remove(key),
        };
        Ok(())
    }
}

/// Preferences as files in the config directory's `shooter/`, one per key.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
pub struct FilePrefs {
    pub dir: std::path::PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl FilePrefs {
    /// `shooter/` in `$XDG_CONFIG_HOME` (else `~/.config`, or `%APPDATA%` on Windows).
    #[must_use]
    pub fn platform() -> Option<Self> {
        let env = |key| std::env::var_os(key).filter(|value| !value.is_empty());
        let config = env("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| env("HOME").map(|home| std::path::Path::new(&home).join(".config")))
            .or_else(|| env("APPDATA").map(std::path::PathBuf::from))?;
        Some(Self {
            dir: config.join("shooter"),
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl PrefStore for FilePrefs {
    fn read(&self, key: &str) -> Option<String> {
        std::fs::read_to_string(self.dir.join(key)).ok()
    }
    fn write(&self, key: &str, value: Option<&str>) -> Result<(), String> {
        let path = self.dir.join(key);
        match value {
            Some(value) => std::fs::create_dir_all(&self.dir)
                .and_then(|()| std::fs::write(&path, value))
                .map_err(|error| format!("{}: {error}", path.display())),
            None => match std::fs::remove_file(&path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    Err(format!("{}: {error}", path.display()))
                }
                _ => Ok(()),
            },
        }
    }
}

/// Preferences in the page's `localStorage`, under `shooter.<key>`.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalStoragePrefs;

#[cfg(target_arch = "wasm32")]
impl LocalStoragePrefs {
    fn storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok()?
    }
}

#[cfg(target_arch = "wasm32")]
impl PrefStore for LocalStoragePrefs {
    fn read(&self, key: &str) -> Option<String> {
        Self::storage()?.get_item(&format!("shooter.{key}")).ok()?
    }
    fn write(&self, key: &str, value: Option<&str>) -> Result<(), String> {
        let storage = Self::storage().ok_or("browser storage is off")?;
        let key = format!("shooter.{key}");
        match value {
            Some(value) => storage.set_item(&key, value),
            None => storage.remove_item(&key),
        }
        .map_err(|_| "can't write browser storage".to_owned())
    }
}

/// This client's store: the page's `localStorage` in the browser, the config directory natively
/// (in memory if there is none).
#[must_use]
pub fn platform() -> Rc<dyn PrefStore> {
    #[cfg(target_arch = "wasm32")]
    {
        Rc::new(LocalStoragePrefs)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        match FilePrefs::platform() {
            Some(files) => Rc::new(files),
            None => Rc::new(MemoryPrefs::default()),
        }
    }
}

/// The look sensitivity's steps, in percent of the normal one.
pub const SENSITIVITY_STEP: u16 = 10;
pub const SENSITIVITY_MIN: u16 = 20;
pub const SENSITIVITY_MAX: u16 = 300;
pub const SENSITIVITY_DEFAULT: u16 = 100;

const NAME: &str = "name";
const SENSITIVITY: &str = "sensitivity";

/// What the person set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Prefs {
    /// The name they play under online ([`shooter_sim::names::sanitize`]d); none: the host
    /// shows "Player N".
    pub name: Option<String>,
    /// The mouse's (and the touch look pad's) look sensitivity, in percent of the normal one.
    pub sensitivity: u16,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            name: None,
            sensitivity: SENSITIVITY_DEFAULT,
        }
    }
}

impl Prefs {
    /// What `store` holds; anything missing or unreadable is the default.
    #[must_use]
    pub fn load(store: &dyn PrefStore) -> Self {
        Self {
            name: store
                .read(NAME)
                .and_then(|name| shooter_sim::names::sanitize(&name)),
            sensitivity: store
                .read(SENSITIVITY)
                .and_then(|value| value.trim().parse::<u16>().ok())
                .map_or(SENSITIVITY_DEFAULT, |value| {
                    value.clamp(SENSITIVITY_MIN, SENSITIVITY_MAX)
                }),
        }
    }

    /// Writes every preference to `store`.
    ///
    /// # Errors
    ///
    /// If the store cannot be written.
    pub fn save(&self, store: &dyn PrefStore) -> Result<(), String> {
        store.write(NAME, self.name.as_deref())?;
        let sensitivity = self.sensitivity.to_string();
        store.write(
            SENSITIVITY,
            (self.sensitivity != SENSITIVITY_DEFAULT).then_some(sensitivity.as_str()),
        )
    }

    /// The look sensitivity as a share of the normal one.
    #[must_use]
    pub fn look_share(&self) -> f32 {
        f32::from(self.sensitivity) / 100.0
    }

    /// The sensitivity `steps` steps up (down, negative), within its range.
    pub fn step_sensitivity(&mut self, steps: i32) {
        let value = i32::from(self.sensitivity) + steps * i32::from(SENSITIVITY_STEP);
        self.sensitivity =
            u16::try_from(value.clamp(i32::from(SENSITIVITY_MIN), i32::from(SENSITIVITY_MAX)))
                .unwrap_or(SENSITIVITY_DEFAULT);
    }

    /// Sets the name from what the person typed ([`shooter_sim::names::sanitize`]d; nothing
    /// left of it clears it).
    pub fn set_name(&mut self, typed: &str) {
        self.name = shooter_sim::names::sanitize(typed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_come_back_from_their_store_and_default_without_one() {
        let store = MemoryPrefs::default();
        assert_eq!(Prefs::load(&store), Prefs::default());
        let mut prefs = Prefs::default();
        prefs.set_name("  Ada  Lovelace!! ");
        prefs.step_sensitivity(3);
        assert_eq!(prefs.name.as_deref(), Some("Ada Lovelace"));
        assert_eq!(prefs.sensitivity, 130);
        prefs.save(&store).unwrap();
        assert_eq!(Prefs::load(&store), prefs);
        // The name under the key `--name` and the page have always used.
        assert_eq!(store.read("name").as_deref(), Some("Ada Lovelace"));
        // Within its range, and cleared back to the default.
        prefs.step_sensitivity(-100);
        assert_eq!(prefs.sensitivity, SENSITIVITY_MIN);
        prefs.step_sensitivity(100);
        assert_eq!(prefs.sensitivity, SENSITIVITY_MAX);
        prefs.set_name("!!!");
        assert_eq!(prefs.name, None);
        store.write("sensitivity", Some("9999")).unwrap();
        assert_eq!(Prefs::load(&store).sensitivity, SENSITIVITY_MAX);
        store.write("sensitivity", Some("fast")).unwrap();
        assert_eq!(Prefs::load(&store).sensitivity, SENSITIVITY_DEFAULT);
    }
}
