//! The phone link: Buddy on the user's iPhone, through a relay that only forwards encrypted bytes
//! (docs/phases/10-iphone.md).
//!
//! `Remote` is the service the apps see: where the relay is, pairing a phone by QR, forgetting it, the switch for
//! approving from the phone, and the log of what the phone did. `session` decides what is said, `link` carries it,
//! `rpc` lists what the phone may ask, `push` what it is told while away.
//!
//! Nothing runs until the user pairs a phone: no thread, no socket. Keys live only in the Keychain / Credential
//! Manager; settings hold the relay's address, the room's name and the phone's name and public key.

pub mod link;
pub mod push;
pub mod rpc;
pub mod session;

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use buddy_remote::pairing::Offer;
use buddy_remote::{Keys, decode, encode};
use tokio::sync::mpsc::UnboundedSender;

use crate::events::{Event, EventBus};
use crate::store::Store;
use crate::{CoreError, log};
use link::Command;
use session::{Options, Out, Session};

const KEYCHAIN_SERVICE: &str = "io.github.crhistian-cornejo.buddy";
/// The relay's owner key: creates rooms.
const OWNER_KEY: &str = "remote-owner-key";
/// This machine's room key for the relay.
const ROOM_KEY: &str = "remote-room-key";
/// This machine's private key.
const DEVICE_KEY: &str = "remote-device-key";
/// The key the phone's notices are sealed with.
const PUSH_KEY: &str = "remote-push-key";

/// The relay's address (`https://…`).
pub const RELAY: &str = "remote.relay";
pub const ROOM: &str = "remote.room";
/// The paired phone's name and public key (empty: none).
pub const PHONE: &str = "remote.phone";
pub const PHONE_KEY: &str = "remote.phoneKey";
/// «Aprobar permisos desde el iPhone» ("false": the phone can only deny).
pub const APPROVALS: &str = "remote.approvals";
const LOG: &str = "remote.log";
const LOG_MAX: usize = 200;

/// Settings only their own code may write: the generic setter refuses them, whoever calls it.
pub fn reserved_setting(key: &str) -> bool {
    key.starts_with("remote.") || matches!(key, "folders.authorized" | "telegram.chat" | "telegram.bot" | "telegram.offset")
}

/// The Keychain (Mac) or Credential Manager (Windows), by entry name.
pub trait Vault: Send + Sync {
    fn get(&self, name: &str) -> Option<String>;
    fn set(&self, name: &str, value: &str) -> Result<(), String>;
    fn delete(&self, name: &str);
}

pub struct SystemVault;

impl Vault for SystemVault {
    fn get(&self, name: &str) -> Option<String> {
        keyring::Entry::new(KEYCHAIN_SERVICE, name).ok()?.get_password().ok().filter(|s| !s.is_empty())
    }
    fn set(&self, name: &str, value: &str) -> Result<(), String> {
        keyring::Entry::new(KEYCHAIN_SERVICE, name).and_then(|e| e.set_password(value)).map_err(|e| format!("No se pudo guardar en el llavero: {e}"))
    }
    fn delete(&self, name: &str) {
        if let Ok(entry) = keyring::Entry::new(KEYCHAIN_SERVICE, name) {
            let _ = entry.delete_credential();
        }
    }
}

/// Creating and deleting this machine's room at the relay.
pub trait Rooms: Send + Sync {
    /// The new room's name and key.
    fn create(&self, relay: &str, owner_key: &str) -> Result<(String, String), String>;
    fn delete(&self, relay: &str, room: &str, room_key: &str) -> Result<(), String>;
}

pub struct HttpRooms;

impl Rooms for HttpRooms {
    fn create(&self, relay: &str, owner_key: &str) -> Result<(String, String), String> {
        let url = format!("{}/rooms", relay.trim_end_matches('/'));
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(20))).http_status_as_error(false).build().into();
        let mut response = agent.post(&url).header("Authorization", &format!("Bearer {owner_key}")).send_empty().map_err(|_| "No se pudo conectar con el relé.".to_string())?;
        match response.status().as_u16() {
            201 => {}
            401 => return Err("El relé no reconoce esa clave de dueño.".into()),
            code => return Err(format!("El relé respondió {code}.")),
        }
        let text = response.body_mut().with_config().limit(16 * 1024).read_to_string().map_err(|_| "El relé respondió algo que no se entiende.".to_string())?;
        let body: serde_json::Value = serde_json::from_str(&text).map_err(|_| "El relé respondió algo que no se entiende.".to_string())?;
        match (body["room"].as_str().filter(|r| valid_name(r, 32)), body["key"].as_str().filter(|k| valid_name(k, 64))) {
            (Some(room), Some(key)) => Ok((room.to_string(), key.to_string())),
            _ => Err("El relé respondió algo que no se entiende.".into()),
        }
    }

    fn delete(&self, relay: &str, room: &str, room_key: &str) -> Result<(), String> {
        let url = format!("{}/rooms/{room}", relay.trim_end_matches('/'));
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(20))).http_status_as_error(false).build().into();
        agent.delete(&url).header("Authorization", &format!("Bearer {room_key}")).call().map(|_| ()).map_err(|_| "No se pudo conectar con el relé.".to_string())
    }
}

/// A room's name or key as the relay makes them: hex of that length.
fn valid_name(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// What Settings shows.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct RemoteStatus {
    /// The relay's address (empty: not set).
    pub relay: String,
    pub has_owner_key: bool,
    pub paired: bool,
    /// The paired phone's name.
    pub phone: String,
    /// This machine reaches the relay.
    pub connected: bool,
    /// The phone is connected right now.
    pub online: bool,
    pub approvals: bool,
    /// A pairing code is on screen.
    pub pairing: bool,
    /// Why the relay could not be reached, in words (empty: no problem).
    pub error: String,
}

/// The pairing code to draw: `cells` is the QR, `size` by `size`, row by row (true: dark).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct PairOffer {
    pub uri: String,
    pub size: u32,
    pub cells: Vec<bool>,
    /// Unix seconds.
    pub expires_at: i64,
}

/// One line of the log of what the phone did.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct RemoteAction {
    /// Unix seconds.
    pub at: i64,
    pub text: String,
}

#[derive(Default)]
struct Live {
    tx: Option<UnboundedSender<Command>>,
    connected: bool,
    online: bool,
    pairing: bool,
    error: String,
}

/// What the service and its link thread share.
struct Shared {
    store: Arc<Mutex<Store>>,
    bus: Arc<EventBus>,
    vault: Box<dyn Vault>,
    live: Mutex<Live>,
}

impl Shared {
    fn store(&self) -> MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn live(&self) -> MutexGuard<'_, Live> {
        self.live.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn setting(&self, key: &str) -> String {
        self.store().setting(key).ok().flatten().unwrap_or_default()
    }

    fn changed(&self) {
        self.bus.publish(Event::RemoteChanged);
    }

    fn log(&self, text: &str) {
        let mut all = self.actions();
        all.push(RemoteAction { at: now(), text: text.chars().take(200).collect() });
        let from = all.len().saturating_sub(LOG_MAX);
        let _ = self.store().set_setting(LOG, &serde_json::to_string(&all[from..]).unwrap_or_else(|_| "[]".into()));
    }

    fn actions(&self) -> Vec<RemoteAction> {
        serde_json::from_str(&self.setting(LOG)).unwrap_or_default()
    }
}

impl link::Report for Shared {
    fn connected(&self, on: bool, error: &str) {
        {
            let mut live = self.live();
            live.connected = on;
            live.error = error.to_string();
            if !on {
                live.online = false;
            }
        }
        self.changed();
    }

    fn out(&self, out: Out) {
        match out {
            Out::Log(text) => {
                log::line(format!("remoto: {text}"));
                self.log(&text);
            }
            Out::Paired { peer, push_key } => {
                // The keys first: a phone is only called paired once its key is kept.
                if let Err(e) = self.vault.set(PUSH_KEY, &encode(&push_key)) {
                    log::line(format!("remoto: {e}"));
                    return;
                }
                let store = self.store();
                let _ = store.set_setting(PHONE_KEY, &encode(&peer.public));
                let _ = store.set_setting(PHONE, &peer.name);
                drop(store);
                log::line("remoto: iPhone emparejado");
                self.log(&format!("{} emparejado", peer.name));
            }
            Out::PairingEnded => self.live().pairing = false,
            Out::Online(on) => self.live().online = on,
            Out::Send(_) => {}
        }
        self.changed();
    }
}

pub struct Remote {
    shared: Arc<Shared>,
    rooms: Box<dyn Rooms>,
    host: Arc<dyn rpc::Host>,
}

impl Remote {
    pub fn new(store: Arc<Mutex<Store>>, bus: Arc<EventBus>, vault: Box<dyn Vault>, rooms: Box<dyn Rooms>, host: Arc<dyn rpc::Host>) -> Self {
        Self { shared: Arc::new(Shared { store, bus, vault, live: Mutex::default() }), rooms, host }
    }

    pub fn status(&self) -> RemoteStatus {
        let live = self.shared.live();
        let phone = self.shared.setting(PHONE);
        RemoteStatus {
            relay: self.shared.setting(RELAY),
            has_owner_key: self.shared.vault.get(OWNER_KEY).is_some(),
            paired: !self.shared.setting(PHONE_KEY).is_empty(),
            phone,
            connected: live.connected,
            online: live.online,
            approvals: self.approvals(),
            pairing: live.pairing,
            error: live.error.clone(),
        }
    }

    /// Where the relay is and the key that creates rooms in it. Refused while a phone is paired through another
    /// relay: forget it first.
    pub fn set_relay(&self, url: &str, owner_key: &str) -> Result<(), CoreError> {
        let url = url.trim().trim_end_matches('/').to_string();
        link::ws_url(&url, "x").map_err(CoreError::Io)?;
        if !self.shared.setting(ROOM).is_empty() && self.shared.setting(RELAY) != url {
            return Err(CoreError::Io("Olvida el iPhone antes de cambiar de relé.".into()));
        }
        let owner_key = owner_key.trim();
        if !owner_key.is_empty() {
            self.shared.vault.set(OWNER_KEY, owner_key).map_err(CoreError::Io)?;
        }
        self.shared.store().set_setting(RELAY, &url)?;
        self.shared.changed();
        Ok(())
    }

    /// Shows a pairing code (good for five minutes, once). Creates this machine's room at the relay the first time.
    pub fn pair(&self) -> Result<PairOffer, CoreError> {
        let relay = self.shared.setting(RELAY);
        if relay.is_empty() {
            return Err(CoreError::Io("Escribe primero la dirección del relé.".into()));
        }
        let keys = self.device_keys()?;
        let (room, room_key) = self.room(&relay)?;
        let offer = Offer::new(&relay, &room, &room_key, &keys.public, now());
        let uri = offer.uri();
        let code = qrcode::QrCode::new(uri.as_bytes()).map_err(|_| CoreError::Io("No se pudo dibujar el código.".into()))?;
        let cells = code.to_colors().into_iter().map(|c| c == qrcode::Color::Dark).collect();
        let expires_at = offer.exp;
        self.start();
        let sent = self.shared.live().tx.as_ref().is_some_and(|tx| tx.send(Command::Pair(offer)).is_ok());
        if !sent {
            return Err(CoreError::Io("La conexión con el relé no está en marcha.".into()));
        }
        self.shared.live().pairing = true;
        self.shared.changed();
        Ok(PairOffer { uri, size: code.width() as u32, cells, expires_at })
    }

    /// Forgets the phone: stops the link, asks the relay to delete the room, and deletes this machine's keys for it.
    pub fn forget(&self) {
        self.shutdown();
        let (relay, room) = (self.shared.setting(RELAY), self.shared.setting(ROOM));
        if let (false, Some(key)) = (room.is_empty(), self.shared.vault.get(ROOM_KEY))
            && let Err(e) = self.rooms.delete(&relay, &room, &key)
        {
            // The room without its keys is useless to anyone; the relay drops it when asked again or never used.
            log::line(format!("remoto: el relé no borró la sala: {e}"));
        }
        for name in [ROOM_KEY, DEVICE_KEY, PUSH_KEY] {
            self.shared.vault.delete(name);
        }
        {
            let store = self.shared.store();
            for key in [ROOM, PHONE, PHONE_KEY] {
                let _ = store.set_setting(key, "");
            }
        }
        *self.shared.live() = Live::default();
        self.shared.log("iPhone olvidado");
        self.shared.changed();
    }

    pub fn set_approvals(&self, on: bool) -> Result<(), CoreError> {
        self.shared.store().set_setting(APPROVALS, if on { "true" } else { "false" })?;
        if let Some(tx) = &self.shared.live().tx {
            let _ = tx.send(Command::Approvals(on));
        }
        self.shared.changed();
        Ok(())
    }

    /// What the phone did, newest first.
    pub fn actions(&self) -> Vec<RemoteAction> {
        let mut all = self.shared.actions();
        all.reverse();
        all
    }

    /// Starts the link when a phone is paired (at launch).
    pub fn start_if_configured(&self) {
        if !self.shared.setting(PHONE_KEY).is_empty() {
            self.start();
        }
    }

    pub fn shutdown(&self) {
        if let Some(tx) = self.shared.live().tx.take() {
            let _ = tx.send(Command::Stop);
        }
    }

    fn approvals(&self) -> bool {
        self.shared.setting(APPROVALS) != "false"
    }

    /// This machine's key pair, made the first time.
    fn device_keys(&self) -> Result<Keys, CoreError> {
        if let Some(private) = self.shared.vault.get(DEVICE_KEY).and_then(|k| decode(&k).ok()).and_then(|k| <[u8; 32]>::try_from(k).ok()) {
            return Ok(Keys::from_private(private));
        }
        let keys = Keys::generate();
        self.shared.vault.set(DEVICE_KEY, &encode(&keys.private)).map_err(CoreError::Io)?;
        Ok(keys)
    }

    /// This machine's room, created at the relay the first time.
    fn room(&self, relay: &str) -> Result<(String, String), CoreError> {
        let room = self.shared.setting(ROOM);
        if let (false, Some(key)) = (room.is_empty(), self.shared.vault.get(ROOM_KEY)) {
            return Ok((room, key));
        }
        let owner = self.shared.vault.get(OWNER_KEY).ok_or_else(|| CoreError::Io("Falta la clave del relé.".into()))?;
        let (room, key) = self.rooms.create(relay, &owner).map_err(CoreError::Io)?;
        self.shared.vault.set(ROOM_KEY, &key).map_err(CoreError::Io)?;
        self.shared.store().set_setting(ROOM, &room)?;
        Ok((room, key))
    }

    /// Starts the link thread and the one that hands it the core's events, unless they run already.
    fn start(&self) {
        let mut live = self.shared.live();
        if live.tx.is_some() {
            return;
        }
        let (room, relay) = (self.shared.setting(ROOM), self.shared.setting(RELAY));
        let (Some(room_key), Ok(keys)) = (self.shared.vault.get(ROOM_KEY), self.device_keys()) else { return };
        if room.is_empty() {
            return;
        }
        let mut session = Session::new(self.host.clone(), keys, &room, Options { approvals: self.approvals() });
        let key32 = |text: String| decode(&text).ok().and_then(|k| <[u8; 32]>::try_from(k).ok());
        if let (Some(public), Some(push_key)) = (key32(self.shared.setting(PHONE_KEY)), self.shared.vault.get(PUSH_KEY).and_then(key32)) {
            session.set_peer(public, push_key);
        }
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        live.tx = Some(tx.clone());
        drop(live);
        let report: Arc<dyn link::Report> = self.shared.clone();
        std::thread::Builder::new()
            .name("buddy-remote".into())
            .spawn(move || link::run(link::Config { relay, room, room_key }, session, rx, report))
            .ok();
        // The core's events, handed to the link. Blocked on the bus it costs nothing; it ends with the link.
        let events = self.shared.bus.subscribe();
        std::thread::Builder::new()
            .name("buddy-remote-events".into())
            .spawn(move || {
                for event in events {
                    // Its own news would only wake the link for nothing.
                    if event != Event::RemoteChanged && tx.send(Command::Event(event)).is_err() {
                        break;
                    }
                }
            })
            .ok();
    }
}

/// This machine's name, as the phone lists it («MacBook de Juan»).
pub fn machine_name() -> String {
    #[cfg(unix)]
    let name = {
        let mut buffer = [0u8; 256];
        // SAFETY: the buffer is valid for its length; gethostname writes at most that many bytes.
        let ok = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) } == 0;
        let end = buffer.iter().position(|b| *b == 0).unwrap_or(buffer.len());
        if ok { String::from_utf8_lossy(&buffer[..end]).trim_end_matches(".local").to_string() } else { String::new() }
    };
    #[cfg(not(unix))]
    let name = std::env::var("COMPUTERNAME").unwrap_or_default();
    let name: String = name.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(60).collect();
    if name.is_empty() { "Este equipo".into() } else { name }
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
