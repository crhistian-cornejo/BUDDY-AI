//! buddy-core: everything Buddy decides lives here, once, for both apps.
//!
//! The Mac app reaches it through UniFFI (feature `ffi`); the Windows app (Tauri) calls it directly.
//! Apps only draw what the core hands them: sprites, settings and the event stream.

pub mod briefing;
pub mod events;
pub mod log;
pub mod orchestrator;
pub mod parley;
pub mod paths;
pub mod pet;
pub mod pixel;
pub mod providers;
pub mod router;
pub mod sessions;
pub mod store;
pub mod usage;
pub mod voice;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub use events::{Event, EventBus};
pub use pet::{PetBrain, PetContext, PetPlan, PetRect, clamp_to_area};
pub use pixel::{Sprite, SpriteState};

#[cfg(feature = "ffi")]
uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error)]
#[cfg_attr(feature = "ffi", derive(uniffi::Error))]
#[cfg_attr(feature = "ffi", uniffi(flat_error))]
pub enum CoreError {
    #[error("base de datos: {0}")]
    Store(String),
    #[error("personaje: {0}")]
    Character(String),
    #[error("archivo: {0}")]
    Io(String),
}

impl From<rusqlite::Error> for CoreError {
    fn from(e: rusqlite::Error) -> Self {
        CoreError::Store(e.to_string())
    }
}

impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        CoreError::Io(e.to_string())
    }
}

/// Receives core events on a background thread (Mac, through UniFFI). The thread blocks while nothing happens.
#[cfg_attr(feature = "ffi", uniffi::export(with_foreign))]
pub trait EventListener: Send + Sync {
    fn on_event(&self, event: Event);
}

/// The core as one object: the app creates it once at launch and keeps it.
#[cfg_attr(feature = "ffi", derive(uniffi::Object))]
pub struct BuddyCore {
    data_dir: PathBuf,
    store: Mutex<store::Store>,
    bus: EventBus,
}

impl BuddyCore {
    /// Opens (or creates) the data folder and the SQLite base. An empty `data_dir` uses the platform default.
    pub fn open(data_dir: impl Into<PathBuf>) -> Result<Self, CoreError> {
        let mut data_dir: PathBuf = data_dir.into();
        if data_dir.as_os_str().is_empty() {
            data_dir = paths::default_data_dir();
        }
        std::fs::create_dir_all(&data_dir)?;
        log::init(&data_dir);
        let store = store::Store::open(&data_dir.join("buddy.sqlite"))?;
        log::line(format!("núcleo abierto (esquema v{})", store.schema_version()?));
        Ok(Self { data_dir, store: Mutex::new(store), bus: EventBus::default() })
    }

    /// Rust-side subscription (Windows app, tests): one channel per subscriber.
    pub fn events(&self) -> std::sync::mpsc::Receiver<Event> {
        self.bus.subscribe()
    }

    fn with_store<T>(&self, f: impl FnOnce(&store::Store) -> Result<T, CoreError>) -> Result<T, CoreError> {
        let guard = self.store.lock().unwrap_or_else(|p| p.into_inner());
        f(&guard)
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl BuddyCore {
    #[cfg_attr(feature = "ffi", uniffi::constructor)]
    pub fn new(data_dir: String) -> Result<Arc<Self>, CoreError> {
        Self::open(data_dir).map(Arc::new)
    }

    /// The same greeting on both platforms: proof that the app is talking to the core.
    pub fn hello(&self) -> String {
        hello()
    }

    pub fn data_dir(&self) -> String {
        self.data_dir.to_string_lossy().into_owned()
    }

    /// A built-in character, rasterized to ARGB pixels ready to paint.
    pub fn sprite(&self, id: String) -> Result<Sprite, CoreError> {
        pixel::builtin_sprite(&id)
    }

    pub fn sprite_ids(&self) -> Vec<String> {
        pixel::builtin_ids().iter().map(|s| s.to_string()).collect()
    }

    pub fn setting(&self, key: String) -> Result<Option<String>, CoreError> {
        self.with_store(|s| s.setting(&key))
    }

    pub fn set_setting(&self, key: String, value: String) -> Result<(), CoreError> {
        self.with_store(|s| s.set_setting(&key, &value))?;
        self.bus.publish(Event::SettingChanged { key });
        Ok(())
    }

    /// Forwards every event to `listener` from a dedicated thread, until the core is dropped.
    pub fn subscribe(&self, listener: Arc<dyn EventListener>) {
        let rx = self.bus.subscribe();
        std::thread::Builder::new()
            .name("buddy-events".into())
            .spawn(move || {
                while let Ok(event) = rx.recv() {
                    listener.on_event(event);
                }
            })
            .ok();
    }
}

pub fn hello() -> String {
    format!("¡Hola! Soy Buddy (núcleo {}).", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_and_round_trips_settings_with_an_event() {
        let dir = tempfile::tempdir().unwrap();
        let core = BuddyCore::open(dir.path()).unwrap();
        let rx = core.events();
        assert_eq!(core.setting("pet.position".into()).unwrap(), None);
        core.set_setting("pet.position".into(), "10,20".into()).unwrap();
        assert_eq!(core.setting("pet.position".into()).unwrap().as_deref(), Some("10,20"));
        assert_eq!(rx.try_recv().unwrap(), Event::SettingChanged { key: "pet.position".into() });
        assert!(dir.path().join("buddy.sqlite").exists());
    }

    #[test]
    fn serves_the_base_sprite() {
        let dir = tempfile::tempdir().unwrap();
        let core = BuddyCore::open(dir.path()).unwrap();
        assert!(core.sprite_ids().contains(&"buddy-base".to_string()));
        let sprite = core.sprite("buddy-base".into()).unwrap();
        assert_eq!(sprite.size, 48);
        assert!(core.sprite("nadie".into()).is_err());
        assert!(core.hello().starts_with("¡Hola! Soy Buddy"));
    }
}
