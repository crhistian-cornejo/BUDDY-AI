//! mika-telegram — MIKA's read-only window on the user's own Telegram account, through MTProto (grammers).
//!
//! It runs as its own small process (`src/main.rs`) next to MIKA on both platforms: MIKA's release builds abort on
//! panic, and a crash in the Telegram stack must never take the island down with it. The macOS app runs the very same
//! helper.
//!
//! What it does: log in with the user's phone (code, then the 2FA password if there is one), list the channels and
//! groups the user is in, and fetch the new posts of the ones the user picked, with their photos. It never sends,
//! deletes, marks as read, reacts or joins anything.
//!
//! Everything a channel posts is untrusted. Only Telegram *photos* are downloaded — Telegram re-encodes those itself
//! as JPEG — never documents, archives, videos or anything a sender uploads byte for byte. Each one is checked after
//! the download (size, JPEG signature) and deleted if it is not what it claims. File names are MIKA's own. Links are
//! kept as text; nothing here opens or fetches them.
//!
//! It connects on Telegram's standard port only: if a network (a company firewall) blocks Telegram, MIKA says so
//! and does not try to get around it.
//!
//! The session (the authorisation key of the user's home datacenter) never touches the disk here: MIKA passes it in
//! from the OS vault and gets every new value back to store it there ([`Telegram::session`]).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use grammers_client::client::{LoginToken, PasswordToken};
use grammers_client::media::Media;
use grammers_client::message::Message;
use grammers_client::peer::Peer;
use grammers_client::sender::{ConnectionParams, InvocationError, SenderPool};
use grammers_client::session::types::{ChannelState, DcOption, PeerAuth, PeerId, PeerInfo, PeerRef, UpdateState, UpdatesState};
use grammers_client::session::{BoxFuture, Session, SessionData};
use grammers_client::tl;
use grammers_client::{Client, SignInError};
use serde::{Deserialize, Serialize};

/// Photos bigger than this are skipped (Telegram's own photos stay well under it; a bet slip is a few hundred KB).
const MAX_IMAGE_BYTES: usize = 5_000_000;
/// Every JPEG starts with these bytes (SOI marker + the first marker's prefix).
const JPEG_MAGIC: [u8; 3] = [0xFF, 0xD8, 0xFF];
/// The longest text kept per post. A pick fits in far less.
const MAX_TEXT_CHARS: usize = 4_000;
const MAX_SENDER_CHARS: usize = 80;

// ── Session ───────────────────────────────────────────────────────────────────

/// The session grammers works with: everything in memory. What has to survive a restart is exported with
/// [`Store::export`] and comes back through [`Store::import`].
#[derive(Default)]
pub struct Store(Mutex<SessionData>);

#[derive(Debug)]
pub struct StoreError;

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("session lock poisoned") }
}

impl std::error::Error for StoreError {}

/// Prefix of an exported session, so a value from some other tool is never mistaken for one.
const SESSION_TAG: &str = "mtg1";
/// Windows' Credential Manager holds 1280 UTF-16 characters per value; the home key alone is ~350.
const MAX_SESSION_LEN: usize = 1_200;

impl Store {
    fn data(&self) -> Result<MutexGuard<'_, SessionData>, StoreError> { self.0.lock().map_err(|_| StoreError) }

    /// `mtg1|<home dc>|<dc>:<base64 key>|…` — the home datacenter's key first. Addresses are not kept: the
    /// statically known ones are enough, and the library migrates if they changed. None while there is no key yet.
    pub fn export(&self) -> Option<String> {
        let data = self.data().ok()?;
        let mut keyed: Vec<&DcOption> = data.dc_options.values().filter(|dc| dc.auth_key.is_some()).collect();
        keyed.sort_by_key(|dc| (dc.id != data.home_dc, dc.id));
        if keyed.first().map(|dc| dc.id) != Some(data.home_dc) { return None; }
        let mut out = format!("{SESSION_TAG}|{}", data.home_dc);
        for dc in keyed {
            let part = format!("|{}:{}", dc.id, base64_encode(dc.auth_key.as_ref()?));
            if out.len() + part.len() > MAX_SESSION_LEN { break; }
            out.push_str(&part);
        }
        Some(out)
    }

    /// The opposite of [`Store::export`]. Anything malformed gives a fresh session (the user logs in again)
    /// rather than an error: a damaged vault entry must not keep the feature broken.
    pub fn import(text: &str) -> Store {
        let mut data = SessionData::default();
        let mut parts = text.trim().split('|');
        if parts.next() != Some(SESSION_TAG) { return Store::default(); }
        let Some(home) = parts.next().and_then(|h| h.parse::<i32>().ok()) else { return Store::default() };
        for part in parts {
            let Some((id, key)) = part.split_once(':') else { return Store::default() };
            let (Ok(id), Some(bytes)) = (id.parse::<i32>(), base64_decode(key)) else { return Store::default() };
            let (Ok(key), Some(dc)) = (<[u8; 256]>::try_from(bytes.as_slice()), data.dc_options.get_mut(&id)) else { return Store::default() };
            dc.auth_key = Some(key);
        }
        if !data.dc_options.contains_key(&home) { return Store::default(); }
        data.home_dc = home;
        Store(Mutex::new(data))
    }
}

// Same as grammers' MemorySession, which keeps its data private.
impl Session for Store {
    type Error = StoreError;

    fn home_dc_id(&self) -> Result<i32, StoreError> { Ok(self.data()?.home_dc) }

    fn set_home_dc_id(&self, dc_id: i32) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move { self.data()?.home_dc = dc_id; Ok(()) })
    }

    fn dc_option(&self, dc_id: i32) -> Result<Option<DcOption>, StoreError> { Ok(self.data()?.dc_options.get(&dc_id).cloned()) }

    fn set_dc_option(&self, dc_option: &DcOption) -> BoxFuture<'_, Result<(), StoreError>> {
        let dc_option = dc_option.clone();
        Box::pin(async move { self.data()?.dc_options.insert(dc_option.id, dc_option); Ok(()) })
    }

    fn peer(&self, peer: PeerId) -> BoxFuture<'_, Result<Option<PeerInfo>, StoreError>> {
        Box::pin(async move { Ok(self.data()?.peer_infos.get(&peer).cloned()) })
    }

    fn cache_peer(&self, peer: &PeerInfo) -> BoxFuture<'_, Result<(), StoreError>> {
        let peer = peer.clone();
        Box::pin(async move {
            self.data()?.peer_infos.entry(peer.id()).or_insert_with(|| peer.clone()).extend_info(&peer);
            Ok(())
        })
    }

    fn updates_state(&self) -> BoxFuture<'_, Result<UpdatesState, StoreError>> {
        Box::pin(async move { Ok(self.data()?.updates_state.clone()) })
    }

    fn set_update_state(&self, update: UpdateState) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            let mut data = self.data()?;
            match update {
                UpdateState::All(state) => data.updates_state = state,
                UpdateState::Primary { pts, date, seq } => {
                    data.updates_state.pts = pts;
                    data.updates_state.date = date;
                    data.updates_state.seq = seq;
                }
                UpdateState::Secondary { qts } => data.updates_state.qts = qts,
                UpdateState::Channel { id, pts } => {
                    data.updates_state.channels.retain(|c| c.id != id);
                    data.updates_state.channels.push(ChannelState { id, pts });
                }
            }
            Ok(())
        })
    }
}

// ── What MIKA sees ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub authorized: bool,
    /// "idle", "code" (a code was sent) or "password" (the account has 2FA).
    pub step: &'static str,
    /// The 2FA password hint, when Telegram has one.
    pub hint: Option<String>,
    /// Who is logged in, to show "Conectado como …".
    pub name: Option<String>,
}

/// A channel or group the user is in. `hash` is the access hash Telegram asks for to read it from this account.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRef {
    pub id: i64,
    pub hash: i64,
    pub title: String,
    /// "channel" or "group".
    #[serde(default)]
    pub kind: String,
}

/// One message of a picked chat, as MIKA stores it for the agents.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Post {
    pub chat_id: i64,
    pub chat: String,
    pub id: i32,
    /// Unix seconds.
    pub date: i64,
    pub text: String,
    /// Who wrote it when that is not the chat itself: a member of a group, or the signature of a channel post. Empty
    /// otherwise.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub sender: String,
    /// Every link in the post: the plain ones and the ones behind a word ("LINK").
    pub links: Vec<String>,
    /// File names, inside the media folder MIKA gave.
    pub photos: Vec<String>,
    /// What else the message carries when it is not a photo: "video", "audio", "documento", "sticker", "encuesta"…
    /// Never downloaded: only named, so a post that is only a video still shows up.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub media: String,
    /// Posts of the same album share it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<i64>,
}

/// A word for what a message carries besides text and photos. Looks at the kind only; nothing is downloaded.
fn media_kind(media: &Media) -> &'static str {
    match media {
        Media::Photo(_) | Media::WebPage(_) => "",
        Media::Sticker(_) => "sticker",
        Media::Poll(_) => "encuesta",
        Media::Contact(_) => "contacto",
        Media::Geo(_) | Media::GeoLive(_) | Media::Venue(_) => "ubicación",
        Media::Dice(_) => "dado",
        Media::Document(document) => match document.mime_type().unwrap_or("") {
            m if m.starts_with("video/") => "video",
            m if m.starts_with("audio/") => "audio",
            m if m.starts_with("image/") => "imagen",
            _ => "documento",
        },
        _ => "archivo",
    }
}

/// Where a login stands. The password token is boxed: it carries the account's whole SRP parameters.
enum Login { Idle, Code(LoginToken), Password(Box<PasswordToken>) }

pub struct Telegram {
    client: Client,
    store: Arc<Store>,
    api_hash: String,
    login: tokio::sync::Mutex<Login>,
    runner: tokio::task::JoinHandle<()>,
}

impl Drop for Telegram {
    fn drop(&mut self) {
        self.client.disconnect();
        self.runner.abort();
    }
}

impl Telegram {
    /// Opens the connection pool. Nothing goes over the network until the first request.
    pub fn connect(api_id: i32, api_hash: &str, session: Option<&str>) -> Telegram {
        let store = Arc::new(session.map(Store::import).unwrap_or_default());
        let params = ConnectionParams {
            device_model: format!("MIKA ({})", std::env::consts::OS),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            ..Default::default()
        };
        // The updates receiver is dropped on purpose: MIKA reads history on demand and never listens.
        let SenderPool { runner, handle, .. } = SenderPool::with_configuration(Arc::clone(&store), api_id, params);
        let client = Client::new(handle);
        let runner = tokio::spawn(runner.run());
        Telegram { client, store, api_hash: api_hash.to_string(), login: tokio::sync::Mutex::new(Login::Idle), runner }
    }

    /// The value to keep in the vault, when there is one.
    pub fn session(&self) -> Option<String> { self.store.export() }

    pub async fn status(&self) -> Result<Status, String> {
        let step = match &*self.login.lock().await {
            Login::Idle => "idle",
            Login::Code(_) => "code",
            Login::Password(_) => "password",
        };
        let hint = match &*self.login.lock().await { Login::Password(t) => t.hint().map(str::to_string), _ => None };
        // No key at all means nobody ever logged in here: no need to ask Telegram.
        if self.store.export().is_none() { return Ok(Status { authorized: false, step, hint, name: None }); }
        if !self.client.is_authorized().await.map_err(|e| error_text(&e))? {
            return Ok(Status { authorized: false, step, hint, name: None });
        }
        let name = self.client.get_me().await.ok().map(|me| {
            let full = [me.first_name(), me.last_name()].into_iter().flatten().collect::<Vec<_>>().join(" ");
            if full.trim().is_empty() { me.username().unwrap_or("").to_string() } else { full }
        });
        Ok(Status { authorized: true, step: "idle", hint: None, name })
    }

    /// Step 1: Telegram sends a login code to the user's Telegram app (or by SMS).
    pub async fn send_code(&self, phone: &str) -> Result<Status, String> {
        let phone: String = phone.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect();
        if phone.len() < 6 { return Err("Escribe tu número con el código de país, por ejemplo +51 999 999 999.".into()); }
        let token = self.client.request_login_code(&phone, &self.api_hash).await.map_err(|e| error_text(&e))?;
        *self.login.lock().await = Login::Code(token);
        self.status().await
    }

    /// Step 2: the code. Accounts with 2FA go on to [`Telegram::password`].
    pub async fn sign_in(&self, code: &str) -> Result<Status, String> {
        let mut login = self.login.lock().await;
        let Login::Code(token) = &*login else { return Err("Primero pide el código.".into()) };
        let code: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
        match self.client.sign_in(token, &code).await {
            Ok(_) => *login = Login::Idle,
            Err(SignInError::PasswordRequired(password)) => *login = Login::Password(Box::new(password)),
            Err(SignInError::InvalidCode) => return Err("Ese código no es válido. Revisa el último que te llegó.".into()),
            Err(SignInError::SignUpRequired) => return Err("Ese número no tiene cuenta de Telegram.".into()),
            Err(SignInError::InvalidPassword(password)) => *login = Login::Password(Box::new(password)),
            Err(SignInError::Other(e)) => return Err(error_text(&e)),
        }
        drop(login);
        self.status().await
    }

    /// Step 3, only with 2FA: the account password. MIKA never stores it.
    pub async fn password(&self, password: &str) -> Result<Status, String> {
        let mut login = self.login.lock().await;
        let Login::Password(_) = &*login else { return Err("Telegram no pidió contraseña.".into()) };
        let Login::Password(token) = std::mem::replace(&mut *login, Login::Idle) else { unreachable!() };
        match self.client.check_password(*token, password.trim()).await {
            Ok(_) => {}
            Err(SignInError::InvalidPassword(token)) => {
                *login = Login::Password(Box::new(token));
                return Err("Contraseña incorrecta.".into());
            }
            Err(SignInError::Other(e)) => return Err(error_text(&e)),
            Err(e) => return Err(format!("No se pudo entrar: {e}")),
        }
        drop(login);
        self.status().await
    }

    /// Ends this session on Telegram's side too (it disappears from "Dispositivos" in the user's app).
    pub async fn sign_out(&self) -> Result<(), String> {
        *self.login.lock().await = Login::Idle;
        self.client.sign_out().await.map(|_| ()).map_err(|e| error_text(&e))
    }

    /// The channels and groups the user is in, most recent first. Private chats and bots are left out: they are
    /// not where picks are published, and MIKA has no business reading them.
    pub async fn chats(&self, limit: usize) -> Result<Vec<ChatRef>, String> {
        let mut dialogs = self.client.iter_dialogs();
        let mut out = Vec::new();
        while let Some(dialog) = dialogs.next().await.map_err(|e| error_text(&e))? {
            let peer = dialog.peer();
            let (kind, title) = match peer {
                Peer::Channel(channel) => ("channel", channel.title().to_string()),
                Peer::Group(group) => ("group", group.title().unwrap_or("").to_string()),
                Peer::User(_) => continue,
            };
            let Ok(Some(reference)) = peer.to_ref().await else { continue };
            let Some(id) = reference.id.bot_api_dialog_id() else { continue };
            out.push(ChatRef { id, hash: reference.auth.hash(), title, kind: kind.into() });
            if out.len() >= limit { break; }
        }
        Ok(out)
    }

    /// The posts of `chat` newer than `after` (oldest first), at most `limit`, and the newest message id seen (even
    /// one that was not a post worth keeping), which is the next `after`. Photos and image files are saved in
    /// `media_dir` as `<chat>_<post>.jpg`.
    pub async fn fetch(&self, chat: &ChatRef, after: i32, limit: usize, media_dir: &Path) -> Result<(Vec<Post>, i32), String> {
        let id = PeerId::from_bot_api_dialog_id(chat.id).ok_or("Chat no válido.")?;
        let peer = PeerRef { id, auth: PeerAuth::from_hash(chat.hash) };
        let mut messages = self.client.iter_messages(peer).limit(limit);
        let mut posts = Vec::new();
        let mut last = after;
        while let Some(message) = messages.next().await.map_err(|e| error_text(&e))? {
            if message.id() <= after { break; }
            last = last.max(message.id());
            if message.action().is_some() { continue; } // "X joined", pinned notices…
            let post = self.post(chat, &message, media_dir).await;
            if !post.text.is_empty() || !post.photos.is_empty() || !post.links.is_empty() || !post.media.is_empty() { posts.push(post); }
        }
        posts.reverse();
        Ok((posts, last))
    }

    async fn post(&self, chat: &ChatRef, message: &Message, media_dir: &Path) -> Post {
        let text: String = message.text().chars().take(MAX_TEXT_CHARS).collect();
        let mut links = text_links(message.fmt_entities());
        for url in plain_links(message.text()) { if !links.contains(&url) { links.push(url); } }
        let mut photos = Vec::new();
        let mut kind = String::new();
        if let Some(media) = message.media() {
            kind = media_kind(&media).to_string();
            if let Some(name) = self.save_image(&media, &format!("{}_{}", chat.id.unsigned_abs(), message.id()), media_dir).await {
                photos.push(name);
            }
        }
        let sender: String = message.sender().and_then(|peer| peer.name()).or_else(|| message.post_author())
            .filter(|name| *name != chat.title).unwrap_or("").chars().take(MAX_SENDER_CHARS).collect();
        Post {
            chat_id: chat.id, chat: chat.title.clone(), id: message.id(), date: message.date().timestamp(),
            text, sender, links, photos, media: kind, album: message.grouped_id(),
        }
    }

    /// Telegram photos only (re-encoded by Telegram as JPEG). Documents — even ones that say they are images — are
    /// never downloaded: their bytes are whatever the sender uploaded. The file is checked once it is on disk and
    /// deleted unless it is a JPEG of the expected size.
    async fn save_image(&self, media: &Media, stem: &str, dir: &Path) -> Option<String> {
        let Media::Photo(photo) = media else { return None };
        if photo.size().is_none_or(|s| s > MAX_IMAGE_BYTES) { return None; }
        let name = format!("{stem}.jpg");
        let path: PathBuf = dir.join(&name);
        if path.is_file() { return Some(name); }
        tokio::fs::create_dir_all(dir).await.ok()?;
        let partial = dir.join(format!("{stem}.part"));
        let downloaded = self.client.download_media(media, &partial).await.is_ok();
        let bytes = if downloaded { tokio::fs::read(&partial).await.ok() } else { None };
        if !bytes.as_deref().is_some_and(is_safe_jpeg) {
            let _ = tokio::fs::remove_file(&partial).await;
            return None;
        }
        tokio::fs::rename(&partial, &path).await.ok()?;
        Some(name)
    }
}

/// A JPEG of a sane size: the SOI signature at the start and the EOI marker at the end, followed by nothing but zero
/// padding. Data appended after the image (a "polyglot" file hiding an archive or a script) is refused.
pub fn is_safe_jpeg(bytes: &[u8]) -> bool {
    if bytes.len() < 128 || bytes.len() > MAX_IMAGE_BYTES || !bytes.starts_with(&JPEG_MAGIC) { return false; }
    let tail = &bytes[bytes.len() - 64..];
    let Some(eoi) = tail.windows(2).rposition(|w| w == [0xFF, 0xD9]) else { return false };
    tail[eoi + 2..].iter().all(|b| *b == 0)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// The links hidden behind words ("LINK" → https://www.betano.pe/bookingcode/…).
fn text_links(entities: Option<&Vec<tl::enums::MessageEntity>>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for entity in entities.into_iter().flatten() {
        if let tl::enums::MessageEntity::TextUrl(link) = entity {
            if is_web_link(&link.url) && !out.contains(&link.url) { out.push(link.url.clone()); }
        }
    }
    out
}

/// Links written out in the text.
pub fn plain_links(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in text.split(|c: char| c.is_whitespace() || c == '<' || c == '>' || c == '"') {
        let Some(start) = word.find("http://").or_else(|| word.find("https://")) else { continue };
        let url = word[start..].trim_end_matches(['.', ',', ';', ')', ']', '!', '?']);
        if is_web_link(url) && !out.iter().any(|u| u == url) { out.push(url.to_string()); }
    }
    out
}

fn is_web_link(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("http://")) && url.len() > 10 && url.len() < 2_000
}

/// Telegram's error names in the user's words. Unknown ones keep their name: it is what a search would need.
pub fn error_text(error: &InvocationError) -> String {
    match error {
        InvocationError::Rpc(rpc) => match rpc.name.as_str() {
            "PHONE_NUMBER_INVALID" => "Ese número no es válido. Usa el formato internacional, por ejemplo +51 999 999 999.".into(),
            "PHONE_NUMBER_BANNED" => "Telegram bloqueó ese número.".into(),
            "PHONE_CODE_EXPIRED" => "El código expiró. Pide uno nuevo.".into(),
            "PHONE_CODE_INVALID" => "Ese código no es válido.".into(),
            "API_ID_INVALID" | "API_ID_PUBLISHED_FLOOD" => "El api_id o el api_hash no son válidos. Cópialos otra vez de my.telegram.org.".into(),
            "AUTH_KEY_UNREGISTERED" | "SESSION_REVOKED" | "SESSION_EXPIRED" | "USER_DEACTIVATED" => "La sesión se cerró. Vuelve a entrar.".into(),
            "CHANNEL_PRIVATE" | "CHANNEL_INVALID" | "CHAT_FORBIDDEN" => "Ya no tienes acceso a ese canal.".into(),
            "FLOOD_WAIT" | "FLOOD_PREMIUM_WAIT" => format!("Telegram pide esperar {} s antes de seguir.", rpc.value.unwrap_or(60)),
            name => format!("Telegram respondió {name}."),
        },
        InvocationError::Io(_) | InvocationError::Dropped => "No hay conexión con Telegram. Si estás en la red de una empresa, puede que la bloquee a propósito: usa MIKA con Telegram desde otra red.".into(),
        _ => "Telegram no respondió como se esperaba.".into(),
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let text = text.trim_end_matches('=').as_bytes();
    let value = |c: u8| B64.iter().position(|&x| x == c).map(|p| p as u32);
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    for chunk in text.chunks(4) {
        if chunk.len() == 1 { return None; }
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() { n |= value(c)? << (18 - 6 * i); }
        out.push((n >> 16) as u8);
        if chunk.len() > 2 { out.push((n >> 8) as u8); }
        if chunk.len() > 3 { out.push(n as u8); }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips() {
        for len in 0..40usize {
            let bytes: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(base64_decode(&base64_encode(&bytes)).unwrap(), bytes);
        }
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert!(base64_decode("a").is_none());
        assert!(base64_decode("ab$d").is_none());
    }

    #[test]
    fn a_session_survives_export_and_import() {
        let mut data = SessionData { home_dc: 4, ..Default::default() };
        data.dc_options.get_mut(&4).unwrap().auth_key = Some([7u8; 256]);
        data.dc_options.get_mut(&2).unwrap().auth_key = Some([9u8; 256]);
        let store = Store(Mutex::new(data));
        let text = store.export().unwrap();
        assert!(text.starts_with("mtg1|4|4:"), "home key first: {text}");
        assert!(text.len() <= MAX_SESSION_LEN);
        let back = Store::import(&text);
        let data = back.data().unwrap();
        assert_eq!(data.home_dc, 4);
        assert_eq!(data.dc_options[&4].auth_key, Some([7u8; 256]));
        assert_eq!(data.dc_options[&2].auth_key, Some([9u8; 256]));
    }

    #[test]
    fn only_a_whole_jpeg_is_kept() {
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
        jpeg.resize(4_000, 0x11);
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        assert!(is_safe_jpeg(&jpeg));
        let mut polyglot = jpeg.clone();
        polyglot.extend_from_slice(b"PK\x03\x04 a zip hiding after the image");
        assert!(!is_safe_jpeg(&polyglot), "bytes after the end of the image");
        let mut padded = jpeg.clone();
        padded.extend_from_slice(&[0; 8]);
        assert!(is_safe_jpeg(&padded), "zero padding after EOI is fine");
        let mut exe = b"MZ".to_vec();
        exe.resize(4_000, 0);
        assert!(!is_safe_jpeg(&exe));
        assert!(!is_safe_jpeg(&jpeg[..64]), "too small to be a photo");
        let mut huge = jpeg.clone();
        huge.resize(MAX_IMAGE_BYTES + 1, 0);
        assert!(!is_safe_jpeg(&huge));
    }

    #[test]
    fn nothing_to_export_before_a_login() {
        assert!(Store::default().export().is_none());
    }

    #[test]
    fn a_damaged_session_starts_fresh() {
        for bad in ["", "hello", "mtg1|x", "mtg1|2|2:%%%", "mtg1|2|2:AAAA", "mtg1|99", "mtg1|2|77:AAAA"] {
            let store = Store::import(bad);
            assert!(store.export().is_none(), "{bad:?} must not give a session");
        }
    }

    #[test]
    fn links_are_found_in_the_text() {
        let text = "Usen el LINK: https://www.betano.pe/bookingcode/54GPDF75/?pid=x&utm=y. Y también (https://t.me/canal)!";
        assert_eq!(plain_links(text), vec![
            "https://www.betano.pe/bookingcode/54GPDF75/?pid=x&utm=y".to_string(),
            "https://t.me/canal".to_string(),
        ]);
        assert!(plain_links("sin enlaces, solo 1.70 @ 5%").is_empty());
        assert!(plain_links("javascript:alert(1) ftp://x").is_empty());
    }

    #[test]
    fn hidden_links_are_kept_once() {
        let link = |url: &str| tl::enums::MessageEntity::TextUrl(tl::types::MessageEntityTextUrl { offset: 0, length: 4, url: url.into() });
        let entities = vec![link("https://www.betano.pe/bookingcode/L6C7TW3D/"), link("https://www.betano.pe/bookingcode/L6C7TW3D/"), link("tg://resolve")];
        assert_eq!(text_links(Some(&entities)), vec!["https://www.betano.pe/bookingcode/L6C7TW3D/".to_string()]);
    }
}
