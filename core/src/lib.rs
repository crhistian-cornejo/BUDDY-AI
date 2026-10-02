//! buddy-core: everything Buddy decides lives here, once, for both apps.
//!
//! The Mac app reaches it through UniFFI (feature `ffi`); the Windows app (Tauri) calls it directly.
//! Apps only draw what the core hands them: sprites, settings and the event stream.

pub mod briefing;
pub mod chat;
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
pub mod tools;
pub mod usage;
pub mod voice;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub use chat::ChatEngine;
pub use events::{Event, EventBus};
pub use orchestrator::Agent;
pub use providers::{ProviderId, ProviderStatus};
pub use store::{ChatMessage, ChatSummary, SourceLink};
pub use pet::{PetBrain, PetContext, PetPlan, PetRect, clamp_to_area};
pub use pixel::{FaceRect, Sprite, SpriteState};
pub use tools::{FocusStatus, Shortcut};
pub use sessions::{HookPreview, HookStatusInfo, SessionHub, SessionInfo};

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
    /// Agent hooks: installing them, or the channel the relay talks to.
    #[error("{0}")]
    Hooks(String),
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
    store: Arc<Mutex<store::Store>>,
    bus: Arc<EventBus>,
    chat: Arc<ChatEngine>,
    focus: tools::Focus,
    sessions: Arc<SessionHub>,
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
        let providers: Vec<Arc<dyn providers::Provider>> =
            vec![Arc::new(providers::claude::Claude::new()), Arc::new(providers::codex::Codex::new())];
        Self::with_providers(data_dir, store, providers)
    }

    /// The core with the given providers (tests use scripted ones).
    pub fn with_providers(
        data_dir: PathBuf,
        store: store::Store,
        providers: Vec<Arc<dyn providers::Provider>>,
    ) -> Result<Self, CoreError> {
        let store = Arc::new(Mutex::new(store));
        let bus = Arc::new(EventBus::default());
        let chat = Arc::new(ChatEngine::new(data_dir.clone(), store.clone(), bus.clone(), providers));
        let sessions = Arc::new(SessionHub::new(data_dir.clone(), bus.clone()));
        Ok(Self { data_dir, store, bus, chat, sessions, focus: tools::Focus::default() })
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

    /// Sends a message to Buddy; the answer arrives as events. Returns the chat id (a new one when `chat_id` is None).
    /// `attachments` are paths the user chose (dropped, picked): they are copied into Buddy's own folder first.
    pub fn send_message(&self, chat_id: Option<String>, text: String, attachments: Vec<String>) -> Result<String, CoreError> {
        self.chat.send(chat_id, text, attachments)
    }

    /// Stops the answer being written in that chat.
    pub fn cancel_chat(&self, chat_id: String) {
        self.chat.cancel(&chat_id);
    }

    pub fn chats(&self, limit: u32) -> Result<Vec<ChatSummary>, CoreError> {
        self.with_store(|s| s.chats(limit))
    }

    /// Chats matching every word of `query` in their title or messages (case and accents ignored).
    pub fn search_chats(&self, query: String, limit: u32) -> Result<Vec<ChatSummary>, CoreError> {
        self.with_store(|s| s.search_chats(&query, limit))
    }

    /// Writes the last answer of the chat again.
    pub fn regenerate(&self, chat_id: String) -> Result<(), CoreError> {
        self.chat.regenerate(&chat_id)
    }

    pub fn messages(&self, chat_id: String) -> Result<Vec<ChatMessage>, CoreError> {
        self.with_store(|s| s.messages(&chat_id))
    }

    pub fn delete_chat(&self, chat_id: String) -> Result<(), CoreError> {
        self.chat.cancel(&chat_id);
        self.with_store(|s| s.delete_chat(&chat_id))
    }

    /// Which provider CLIs are installed.
    pub fn providers(&self) -> Vec<ProviderStatus> {
        self.chat.statuses()
    }

    /// Buddy and its specialists, as defined in the data folder.
    pub fn agents(&self) -> Vec<Agent> {
        self.chat.agents()
    }

    /// Starts listening to Claude Code / Codex hooks: copies the app's bundled relay from `relay_path` (empty = skip)
    /// to `<data_dir>/bin/buddy-hook[.exe]`, then opens the local socket (Mac) or pipe (Windows) once. Never blocks.
    pub fn start_sessions(&self, relay_path: String) -> Result<(), CoreError> {
        self.sessions.start(&relay_path)
    }

    /// Whether each agent's hooks are installed, the agent is there, and the relay is in place.
    pub fn hooks_status(&self) -> Vec<HookStatusInfo> {
        self.sessions.hooks_status()
    }

    /// The change installing (or removing) Buddy's hooks would make to `agent`'s (`claude` | `codex`) config file.
    pub fn hooks_preview(&self, agent: String, install: bool) -> Result<HookPreview, CoreError> {
        self.sessions.hooks_preview(&agent, install)
    }

    /// Applies the change the user saw (refused if the file changed since `hooks_preview`). Takes a dated backup
    /// first and returns its path (empty when there was no file yet).
    pub fn hooks_write(&self, agent: String, install: bool, fingerprint: String) -> Result<String, CoreError> {
        self.sessions.hooks_write(&agent, install, &fingerprint)
    }

    /// The user's click on an approval card (`ApprovalRequest`).
    pub fn answer_approval(&self, request_id: String, allow: bool) {
        self.sessions.answer_approval(&request_id, allow);
    }

    /// Claude Code / Codex sessions Buddy heard from, most recent first.
    pub fn sessions(&self) -> Vec<SessionInfo> {
        self.sessions.sessions()
    }

    /// Starts a focus block of `minutes` (replacing a running one).
    pub fn focus_start(&self, minutes: u32) -> FocusStatus {
        self.focus.start(minutes, &self.bus)
    }

    pub fn focus_stop(&self) {
        self.focus.stop(&self.bus);
    }

    pub fn focus_status(&self) -> FocusStatus {
        self.focus.status()
    }

    /// The pinned apps, folders, files and pages.
    pub fn shortcuts(&self) -> Result<Vec<Shortcut>, CoreError> {
        self.with_store(tools::shortcuts)
    }

    pub fn add_shortcut(&self, target: String) -> Result<Vec<Shortcut>, CoreError> {
        self.with_store(|s| tools::add_shortcut(s, &target))
    }

    pub fn remove_shortcut(&self, id: String) -> Result<Vec<Shortcut>, CoreError> {
        self.with_store(|s| tools::remove_shortcut(s, &id))
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
