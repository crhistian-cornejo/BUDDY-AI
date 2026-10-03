//! buddy-core: everything Buddy decides lives here, once, for both apps.
//!
//! The Mac app reaches it through UniFFI (feature `ffi`); the Windows app (Tauri) calls it directly.
//! Apps only draw what the core hands them: sprites, settings and the event stream.

pub mod accounts;
pub mod account_router;
pub mod activity;
pub mod briefing;
pub mod chat;
pub mod connectors;
pub mod events;
pub mod folders;
pub mod images;
pub mod log;
pub mod look;
pub mod mailwatch;
pub mod media;
pub mod memory;
pub mod niko;
pub mod notch;
pub mod orchestrator;
pub mod parley;
mod odds;
pub mod paths;
pub mod pet;
pub mod pixel;
pub mod providers;
pub mod router;
pub mod skills;
pub mod spotify;
pub mod sessions;
pub mod store;
pub mod telegram;
mod telegram_account;
pub mod tools;
pub mod usage;
pub mod voice;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub use chat::{ChatEngine, QueuedMessage};
pub use events::{Event, EventBus};
pub use folders::AuthorizedFolder;
pub use orchestrator::Agent;
pub use providers::{ProviderId, ProviderStatus};
pub use store::{ChatMessage, ChatSummary, SourceLink, TokenReport, TokenDay};
pub use pet::{PetBrain, PetContext, PetPlan, PetRect, clamp_to_area};
pub use pixel::{FaceRect, Sprite, SpriteState};
pub use look::{AgentLook, LookOption, LookOptions};
pub use briefing::BriefingItem;
pub use media::NowPlayingInfo;
pub use skills::Skill;
pub use tools::{FocusStatus, Shortcut};
pub use notch::{NotchTools, ShelfFile, SavedClip};
pub use notch::calendar::Appointment;
pub use router::{ModelOption, RouterConfig, Tier, TierChoice};
pub use usage::{ProviderUsage, UsageWindow};
pub use sessions::{HookPreview, HookStatusInfo, SessionHub, SessionInfo};
pub use telegram::TelegramStatus;

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
    usage: Arc<usage::Usage>,
    briefing: Arc<briefing::Briefing>,
    sessions: Arc<SessionHub>,
    spotify: Arc<spotify::Spotify>,
    telegram: Arc<telegram::Telegram>,
    telegram_account: Arc<telegram_account::Account>,
    odds: Arc<odds::Odds>,
    niko: Arc<niko::Niko>,
    mail: Arc<mailwatch::MailWatch>,
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
        let providers: Vec<Arc<dyn providers::Provider>> = vec![
            Arc::new(providers::claude::Claude::new()),
            Arc::new(providers::codex::Codex::new()),
            Arc::new(providers::gemini::Gemini::new()),
        ];
        let core = Self::with_providers(data_dir, store, providers)?;
        // Telegram listens from launch when a bot is connected and a chat paired (never in tests: fresh stores).
        core.telegram.start_if_configured();
        // Niko's mail review runs only where the user switched it on.
        core.niko.start();
        // The mail watch runs only when the user connected a Gmail address.
        core.mail.start();
        Ok(core)
    }

    /// The core with the given providers (tests use scripted ones).
    pub fn with_providers(
        data_dir: PathBuf,
        store: store::Store,
        providers: Vec<Arc<dyn providers::Provider>>,
    ) -> Result<Self, CoreError> {
        let store = Arc::new(Mutex::new(store));
        let bus = Arc::new(EventBus::default());
        let usage = Arc::new(usage::Usage::new(store.clone(), bus.clone()));
        let sessions = Arc::new(SessionHub::new(data_dir.clone(), bus.clone()));
        let briefing = Arc::new(briefing::Briefing::new(store.clone(), bus.clone(), Box::new(briefing::ClaudeSource)));
        let chat = Arc::new(
            ChatEngine::new(data_dir.clone(), store.clone(), bus.clone(), providers.clone())
                .with_usage(usage.clone())
                .with_gate(sessions.clone()),
        );
        let spotify = Arc::new(spotify::Spotify::default());
        // Spotify's search for the agents (the relay's `spotify_search`): runs on the hub's connection thread.
        let (s, st) = (spotify.clone(), store.clone());
        sessions.set_tool_handler(Box::new(move |request, payload| {
            let id = || spotify::client_id(&st.lock().unwrap_or_else(|p| p.into_inner())).ok().flatten();
            let text = |key: &str| payload[key].as_str().unwrap_or("").to_string();
            match request {
                "spotify_search" => Some(s.search(id(), &spotify::SystemSecrets, &text("query"), &text("kind"), payload["new"] == true)),
                // A playlist in the user's account (they allowed it in Settings).
                "spotify_playlist" => {
                    let tracks: Vec<String> = payload["tracks"].as_array().into_iter().flatten().filter_map(|t| t.as_str().map(String::from)).collect();
                    Some(s.create_playlist(id(), &text("name"), &text("description"), &tracks))
                }
                _ => None,
            }
        }));
        // Telegram: each message of the paired chat is a restricted PARLEY turn in «Telegram · PARLEY»; money notes
        // («gasté 45 en almuerzo», «Niko, …») go to Niko in «Telegram · Niko».
        let engine = Arc::downgrade(&chat);
        let telegram = Arc::new(telegram::Telegram::new(
            store.clone(),
            bus.clone(),
            Arc::new(telegram::HttpApi),
            Box::new(telegram::TokenSecret),
            Box::new(move |text, files| {
                let engine = engine.upgrade().ok_or("Buddy se está cerrando.")?;
                let (chat_id, title, agent) = if niko::is_finance_message(text) {
                    ("telegram-niko", "Telegram · Niko", niko::AGENT)
                } else {
                    (telegram::CHAT_ID, telegram::CHAT_TITLE, telegram::AGENT)
                };
                engine.run_direct_with_attachments(chat_id, title, agent, text, true, files).map_err(|e| match e {
                    CoreError::Store(m) | CoreError::Hooks(m) => m,
                    other => other.to_string(),
                })
            }),
        ).with_media_dir(data_dir.join("telegram-bot-media")));
        let telegram_account = Arc::new(telegram_account::Account::new(store.clone(), data_dir.clone()));
        let odds = Arc::new(odds::Odds::new(store.clone()));
        chat.set_parley_source(Arc::new(parley::Sources { account: telegram_account.clone(), bot: telegram.clone(), odds: odds.clone(), store: store.clone() }));
        let niko = Arc::new(niko::Niko::new(data_dir.clone(), store.clone(), bus.clone(), Box::new(niko::RoutedSource { data_dir: data_dir.clone(), store: store.clone(), usage: usage.clone(), providers })));
        let tg = telegram.clone();
        niko.set_notifier(Box::new(move |text| {
            let tg = tg.clone();
            std::thread::spawn(move || {
                if let Err(e) = tg.send(&text) {
                    log::line(format!("niko: Telegram no recibió el aviso: {e}"));
                }
            });
        }));
        let mail = Arc::new(mailwatch::MailWatch::new(store.clone(), bus.clone()));
        let reader = niko.clone();
        mail.set_handler(Box::new(move |mails| {
            // Off the watch's thread: the connection keeps listening while Niko works.
            let (reader, mails) = (reader.clone(), mails.to_vec());
            std::thread::spawn(move || {
                let _ = reader.mail_arrived(&mails);
            });
        }));
        Ok(Self { telegram_account, odds, data_dir, store, bus, chat, sessions, focus: tools::Focus::default(), usage, briefing, spotify, telegram, niko, mail })
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

    /// What the team remembers about the user (Settings shows it and can delete a note).
    pub fn memory_notes(&self) -> Vec<String> {
        self.with_store(|s| Ok(memory::notes(s))).unwrap_or_default()
    }

    pub fn forget_memory(&self, note: String) {
        let _ = self.with_store(|s| {
            memory::forget(s, &note);
            Ok(())
        });
    }

    /// What the user asks often, to offer it in the composer (see `Store::frequent_questions`).
    pub fn chat_suggestions(&self) -> Vec<String> {
        self.with_store(|s| s.frequent_questions(3)).unwrap_or_default()
    }

    /// Dictated text as Spanish writes it (see `voice::tidy`).
    pub fn voice_tidy(&self, text: String) -> String {
        voice::tidy(&text)
    }

    /// Names a recogniser should expect (see `voice::VOCABULARY`), plus the agents' own.
    pub fn voice_vocabulary(&self) -> Vec<String> {
        let mut words: Vec<String> = voice::VOCABULARY.iter().map(|w| w.to_string()).collect();
        for agent in self.agents() {
            if !words.contains(&agent.name) {
                words.push(agent.name);
            }
        }
        words
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

    pub fn queued_messages(&self, chat_id: String) -> Vec<QueuedMessage> {
        self.chat.queued_messages(&chat_id)
    }

    pub fn remove_queued(&self, chat_id: String, message_id: String) {
        self.chat.remove_queued(&chat_id, &message_id);
    }

    pub fn queued_thumbnail(&self, chat_id: String, message_id: String) -> Option<String> {
        self.chat.queued_thumbnail(&chat_id, &message_id)
    }

    pub fn redirect_queued(&self, chat_id: String, message_id: String) -> Result<(), CoreError> {
        self.chat.redirect_queued(&chat_id, &message_id)
    }

    pub fn take_queued(&self, chat_id: String, message_id: String) -> Result<QueuedMessage, CoreError> {
        self.chat.take_queued(&chat_id, &message_id)
    }

    pub fn resume_queue(&self, chat_id: String) -> Result<(), CoreError> {
        self.chat.resume_queue(&chat_id)
    }

    /// Call when the composer opens: Buddy's provider gets ready so the first words come sooner.
    pub fn prewarm(&self) {
        self.chat.prewarm();
    }

    /// Stops the answer being written in that chat and clears its pending messages.
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

    /// What an agent can be allowed, with the words Settings shows.
    pub fn agent_permission_catalog(&self) -> Vec<PermissionInfo> {
        orchestrator::PERMISSIONS
            .iter()
            .map(|(id, name, detail)| PermissionInfo { id: (*id).into(), name: (*name).into(), detail: (*detail).into() })
            .collect()
    }

    /// Saves which permissions an agent holds (unknown ones are dropped).
    pub fn set_agent_permissions(&self, agent_id: String, permissions: Vec<String>) -> Result<(), CoreError> {
        let list = orchestrator::clean_permissions(&permissions).join(",");
        self.with_store(|s| s.set_setting(&orchestrator::permissions_key(&agent_id), &list))
    }

    /// An agent's face (`cara`): agent.md's, or the one chosen in Settings.
    pub fn agent_look(&self, agent_id: String) -> AgentLook {
        self.with_store(|s| Ok(look::resolve(&self.data_dir, s, &agent_id))).unwrap_or_else(|_| AgentLook::buddy())
    }

    /// Saves a face chosen in Settings (agent.md's own clears the setting); announces it as a setting change.
    pub fn set_agent_look(&self, agent_id: String, look: AgentLook) -> Result<(), CoreError> {
        self.with_store(|s| look::save(&self.data_dir, s, &agent_id, &look))?;
        self.bus.publish(Event::SettingChanged { key: look::look_key(&agent_id) });
        Ok(())
    }

    /// Back to the face its agent.md gives.
    pub fn reset_agent_look(&self, agent_id: String) -> Result<(), CoreError> {
        self.with_store(|s| s.set_setting(&look::look_key(&agent_id), ""))?;
        self.bus.publish(Event::SettingChanged { key: look::look_key(&agent_id) });
        Ok(())
    }

    /// The agent wearing its face, ready to paint (same `Sprite` as buddy-base, with its avatar square).
    pub fn agent_sprite(&self, agent_id: String) -> Result<Sprite, CoreError> {
        let name = self.chat.agents().into_iter().find(|a| a.id == agent_id).map(|a| a.name).unwrap_or_default();
        look::sprite(&self.agent_look(agent_id.clone()), &agent_id, &name)
    }

    /// Only idle frame 0 of a face that is not saved yet (previews in the editor).
    pub fn look_preview(&self, look: AgentLook) -> Result<Sprite, CoreError> {
        look.validate()?;
        look::preview(&look)
    }

    /// The colours, accessories and eyes a face can have, with the words Settings shows.
    pub fn look_options(&self) -> LookOptions {
        look::options()
    }

    /// An agent's model: "auto" (the router), a model id from `router_config().models`, or "" for its agent.md.
    pub fn set_agent_model(&self, agent_id: String, model: String) -> Result<(), CoreError> {
        let known = model.is_empty() || model == "auto" || router::MODELS.iter().any(|m| m.0 == model);
        if !known {
            return Err(CoreError::Hooks(format!("Modelo desconocido: {model}")));
        }
        self.with_store(|s| s.set_setting(&orchestrator::model_key(&agent_id), &model))
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
    /// «Permitir siempre»: allows this one and every later one with the same program and subcommand.
    pub fn answer_approval_always(&self, request_id: String) {
        self.sessions.answer_approval_always(&request_id);
    }

    /// The commands allowed for good (Settings lists them).
    pub fn always_rules(&self) -> Vec<sessions::always::AlwaysRule> {
        self.sessions.always_rules()
    }

    pub fn remove_always_rule(&self, agent: String, prefix: String) {
        self.sessions.remove_always_rule(&agent, &prefix);
    }

    pub fn answer_approval(&self, request_id: String, allow: bool) {
        self.sessions.answer_approval(&request_id, allow);
    }

    /// Claude Code / Codex sessions Buddy heard from, most recent first.
    pub fn sessions(&self) -> Vec<SessionInfo> {
        self.sessions.sessions()
    }

    /// The token meter: what each feature spent over the last `days` days.
    pub fn token_report(&self, days: u32) -> Result<Vec<TokenReport>, CoreError> {
        self.with_store(|s| s.token_report(days))
    }

    pub fn token_activity(&self, days: u32) -> Result<Vec<TokenDay>, CoreError> {
        self.store.lock().unwrap_or_else(|p| p.into_inner()).token_activity(days)
    }

    /// The briefing lines of the last day («mensajitos»), newest first.
    pub fn briefing(&self) -> Vec<BriefingItem> {
        self.briefing.latest()
    }

    /// Runs the briefing when one of today's slots (8, 16, 19 h) is due. Call at launch and once a minute.
    pub fn briefing_tick(&self) {
        let (hour, today) = briefing::local_now();
        self.briefing.tick(hour, &today);
    }

    /// The app finished the screenshot the core asked for (`Event::ScreenshotRequest`); `ok` false on failure.
    pub fn screenshot_taken(&self, path: String, ok: bool) {
        self.sessions.screenshot_taken(&path, ok);
    }

    /// The app tells what its player plays (on every change), for the agents' `now_playing` tool.
    pub fn set_now_playing(&self, now: Option<NowPlayingInfo>) {
        self.sessions.set_now_playing(now);
    }

    /// Connects Spotify's search with the user's own app: checks the pair with Spotify, then keeps the Client ID in
    /// the settings and the Client Secret only in the Keychain / Credential Manager.
    pub fn spotify_connect(&self, client_id: String, client_secret: String) -> Result<(), CoreError> {
        self.spotify.connect(&client_id, &client_secret, &spotify::SystemSecrets).map_err(CoreError::Hooks)?;
        self.with_store(|s| s.set_setting(spotify::CLIENT_ID_KEY, client_id.trim()))
    }

    pub fn spotify_disconnect(&self) -> Result<(), CoreError> {
        self.spotify.forget(&spotify::SystemSecrets);
        self.with_store(|s| s.set_setting(spotify::CLIENT_ID_KEY, ""))
    }

    /// «Permitir crear playlists»: the page to open in the browser. When the user answers there, `SpotifyChanged`
    /// … arrives as `NikoChanged`-style news: read `spotify_can_create_playlists` again.
    pub fn spotify_authorize(&self) -> Result<String, CoreError> {
        let id = self.spotify_client_id();
        let bus = self.bus.clone();
        self.spotify
            .authorize(&id, Box::new(move |result| {
                if let Err(e) = &result {
                    log::line(format!("spotify: el permiso de playlists no se dio: {e}"));
                }
                bus.publish(Event::UsageChanged);
            }))
            .map_err(CoreError::Hooks)
    }

    pub fn spotify_can_create_playlists(&self) -> bool {
        self.spotify.can_write()
    }

    pub fn spotify_forget_playlists(&self) {
        self.spotify.forget_user();
    }

    /// The Client ID when Spotify is connected (empty otherwise). Never reads the secret back.
    pub fn spotify_client_id(&self) -> String {
        self.with_store(spotify::client_id).ok().flatten().unwrap_or_default()
    }

    /// Personal account actions from Settings. Secrets only enter the OS vault, never the history.
    pub fn telegram_account_request(&self, action: String, value: String) -> Result<String, CoreError> {
        if action == "analyze" {
            let answer = self.chat.run_direct("telegram-groups", "Telegram · Grupos", "parley", "Revisa los mensajes de hoy de todos mis grupos seleccionados. Dime qué picks siguen pendientes por hora, cuáles están en vivo o terminaron y propón parlays solo con los pendientes verificados.", true)?;
            return Ok(serde_json::json!({"analysis":answer}).to_string());
        }
        self.telegram_account.request(&action, &value).map_err(CoreError::Hooks)
    }

    /// PARLEY's API key is configured outside chats, and is never read back to the UI.
    pub fn parley_odds_request(&self, action: String, value: String) -> Result<String, CoreError> {
        match action.as_str() {
            "status" => {},
            "configure" => self.odds.configure(&value).map_err(CoreError::Hooks)?,
            _ => return Err(CoreError::Hooks("Acción de cuotas no válida.".into())),
        }
        Ok(self.odds.status().to_string())
    }

    /// Telegram for Settings: connected bot, paired chat and pairing code. While connected and not
    /// paired it makes a code (if none is valid) and starts listening for its `/start`.
    pub fn telegram_status(&self) -> TelegramStatus {
        self.telegram.status()
    }

    /// Checks the bot token with Telegram (`getMe`), then keeps it only in the Keychain / Credential Manager. Blocks
    /// on the network: call it off the main thread.
    pub fn telegram_connect(&self, token: String) -> Result<(), CoreError> {
        self.telegram.connect(&token).map_err(CoreError::Hooks)
    }

    /// Forgets the token, the bot and the paired chat; stops listening.
    pub fn telegram_disconnect(&self) -> Result<(), CoreError> {
        self.telegram.disconnect().map_err(CoreError::Hooks)
    }

    /// Unpairs the current chat and returns a fresh 6-digit code (`/start <code>` pairs a chat again).
    pub fn telegram_new_pairing_code(&self) -> Result<String, CoreError> {
        self.telegram.new_pairing_code().map_err(CoreError::Hooks)
    }

    /// Sends `text` (Markdown becomes plain text; long text is split) to the paired chat. Blocks on the network.
    pub fn telegram_send(&self, text: String) -> Result<(), CoreError> {
        self.telegram.send(&text).map_err(CoreError::Hooks)
    }

    /// Settings › Conectores (MCP): the built-in remote servers, each with its switch and whether a key is saved
    /// (never the key).
    pub fn connectors(&self) -> Vec<connectors::ConnectorInfo> {
        connectors::infos(&self.store.lock().unwrap_or_else(|p| p.into_inner()))
    }

    /// Turns a connector on or off for the agents with the web (from their next turn).
    pub fn set_connector_enabled(&self, id: String, on: bool) -> Result<(), CoreError> {
        self.with_store(|s| connectors::set_enabled(s, &id, on))?;
        self.bus.publish(Event::SettingChanged { key: connectors::enabled_key(&id) });
        Ok(())
    }

    /// Keeps a connector's optional key only in the Keychain / Credential Manager; empty removes it.
    pub fn set_connector_key(&self, id: String, key: String) -> Result<(), CoreError> {
        self.with_store(|s| connectors::set_key(s, &connectors::SystemKeys, &id, &key))?;
        self.bus.publish(Event::SettingChanged { key: connectors::enabled_key(&id) });
        Ok(())
    }

    /// The router's Settings: mode ("auto" or a model id), each tier's model and effort, the models to pick from.
    pub fn router_config(&self) -> router::RouterConfig {
        router::config(&self.store.lock().unwrap_or_else(|p| p.into_inner()))
    }

    /// "auto" (by tiers) or one model id for every turn.
    pub fn set_router_mode(&self, mode: String) -> Result<(), CoreError> {
        self.with_store(|s| router::set_mode(s, &mode).map_err(CoreError::Hooks))
    }

    pub fn set_router_tier(&self, tier: router::Tier, model: String, effort: String) -> Result<(), CoreError> {
        self.with_store(|s| router::set_tier(s, tier, &model, &effort).map_err(CoreError::Hooks))
    }

    /// What the router would do with `text` («A fondo → Opus 5.5 (high) · pide análisis»), to try it in Settings.
    pub fn router_preview(&self, text: String) -> String {
        let route = router::route(&self.store.lock().unwrap_or_else(|p| p.into_inner()), &text, &[]);
        format!("{} → {} ({}) · {}", route.tier.label(), route.model_name, effort_label(&route.effort), route.reason)
    }

    /// Buddy's skills (`<data>/skills/<name>/SKILL.md`), seeding the built-in ones.
    pub fn skills(&self) -> Vec<Skill> {
        skills::list(&self.data_dir)
    }

    /// The folder where the user can drop skills (shown in Settings).
    pub fn skills_dir(&self) -> String {
        skills::dir(&self.data_dir).to_string_lossy().into()
    }

    /// What the briefing looks for (the defaults until the user writes their own).
    pub fn briefing_topics(&self) -> String {
        self.with_store(|s| s.setting(briefing::TOPICS_KEY))
            .ok()
            .flatten()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| briefing::DEFAULT_TOPICS.into())
    }

    /// Saves the topics; an empty text goes back to the defaults.
    pub fn set_briefing_topics(&self, topics: String) -> Result<(), CoreError> {
        let topics = topics.trim();
        let value = if topics.is_empty() || topics == briefing::DEFAULT_TOPICS { "" } else { topics };
        self.with_store(|s| s.set_setting(briefing::TOPICS_KEY, value))
    }

    /// One briefing run now, whatever the time (from Settings); off the caller's thread.
    pub fn briefing_now(&self) {
        let b = self.briefing.clone();
        std::thread::spawn(move || {
            let _ = b.run_now();
        });
    }

    /// What is used of each plan (Claude, Codex), as last seen.
    pub fn usage(&self) -> Vec<ProviderUsage> {
        self.usage.snapshot()
    }

    /// Asks for fresh plan figures if the last ones are old (call when the user looks at them).
    pub fn refresh_usage(&self) {
        self.usage.refresh();
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

    pub fn notch_tools(&self) -> Result<NotchTools, CoreError> { self.with_store(notch::load) }
    pub fn notch_add_files(&self, paths: Vec<String>) -> Result<NotchTools, CoreError> { self.with_store(|s| notch::add_files(s, &paths)) }
    pub fn notch_remove_file(&self, path: String) -> Result<NotchTools, CoreError> { self.with_store(|s| notch::remove_file(s, &path)) }
    pub fn notch_save_clip(&self, text: String) -> Result<NotchTools, CoreError> { self.with_store(|s| notch::add_clip(s, &text)) }
    pub fn notch_remove_clip(&self, id: String) -> Result<NotchTools, CoreError> { self.with_store(|s| notch::remove_clip(s, &id)) }
    pub fn notch_widget(&self, widget: String, enabled: bool) -> Result<NotchTools, CoreError> { self.with_store(|s| notch::widget(s, &widget, enabled)) }
    pub fn notch_set_calendar(&self, path: Option<String>) -> Result<NotchTools, CoreError> { self.with_store(|s| notch::set_calendar(s, path)) }
    pub fn notch_calendar(&self) -> Result<Option<Appointment>, CoreError> {
        let state = self.notch_tools()?;
        match state.calendar_path { Some(path) => notch::calendar::read(PathBuf::from(path).as_path(), notch::calendar::now()), None => Ok(None) }
    }
    pub fn notch_appointment_file(&self) -> Result<String, CoreError> {
        let event = self.notch_calendar()?.ok_or_else(|| CoreError::Io("No hay una próxima cita en la agenda.".into()))?;
        notch::calendar::export(&event, &self.data_dir.join("notch"))
    }

    /// The folders the agents may use (read, or read and edit).
    pub fn folders(&self) -> Result<Vec<AuthorizedFolder>, CoreError> {
        self.with_store(folders::list)
    }

    pub fn add_folder(&self, path: String, can_edit: bool) -> Result<Vec<AuthorizedFolder>, CoreError> {
        self.with_store(|s| folders::add(s, &path, can_edit))
    }

    pub fn remove_folder(&self, path: String) -> Result<Vec<AuthorizedFolder>, CoreError> {
        self.with_store(|s| folders::remove(s, &path))
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

impl Drop for BuddyCore {
    /// The Telegram poller stops with the core (a request in flight ends within its timeout).
    fn drop(&mut self) {
        self.telegram.shutdown();
        self.niko.shutdown();
        self.mail.shutdown();
    }
}

pub fn hello() -> String {
    format!("¡Hola! Soy Buddy (núcleo {}).", env!("CARGO_PKG_VERSION"))
}

/// A permission an agent can hold.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PermissionInfo {
    pub id: String,
    pub name: String,
    pub detail: String,
}

/// Effort in the words Settings uses.
pub fn effort_label(effort: &str) -> &'static str {
    match effort {
        "low" => "bajo",
        "high" => "alto",
        _ => "medio",
    }
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

    #[test]
    fn briefing_topics_fall_back_to_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let core = BuddyCore::open(dir.path()).unwrap();
        assert_eq!(core.briefing_topics(), briefing::DEFAULT_TOPICS);
        core.set_briefing_topics("  fórmula 1  ".into()).unwrap();
        assert_eq!(core.briefing_topics(), "fórmula 1");
        core.set_briefing_topics(" ".into()).unwrap();
        assert_eq!(core.briefing_topics(), briefing::DEFAULT_TOPICS);
    }
}
