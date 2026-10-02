//! Telegram — the user's own account, read through the `mika-telegram` helper (apps/windows/telegram). The helper is
//! a separate process: MIKA's release build aborts on panic, and nothing in the Telegram stack may take the island
//! down. It is started on the first need and quits by itself after 15 idle minutes.
//!
//! What goes where:
//! * api_id and api_hash (from my.telegram.org): Credential Manager, typed by the user in the settings window.
//! * The session (the authorisation key the login creates): Credential Manager, written only from here, never
//!   reachable from a webview (`secrets::is_user_key`).
//! * The picked chats: settings.json (`telegram_chats`).
//! * The posts and their images: `agents\<id>\workspace\telegram\` of every agent whose `integration` is
//!   `telegram` (PARLEY), for 3 days. The last message id read per chat: `%LOCALAPPDATA%\MIKA\telegram\state.json`.
//!
//! Telegram's terms forbid using Telegram data for AI, and the settings window says so before the user logs in.
//! MIKA only reads: it never sends, deletes or marks anything as read.
//!
//! What channels post is untrusted. The helper downloads Telegram photos only and checks each one; here every name it
//! returns is checked again (`<digits>_<digits>.jpg`, nothing else reaches a path), each file gets the Mark of the Web
//! (Windows treats it as downloaded from the internet), the media folder is capped at 200 MB, and MIKA never opens or
//! shows those files itself: only the agent's model reads them.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout};

use super::{emit, IntegrationEvent, IntegrationUpdate};
use crate::services::{log, named_agents, secrets, settings};

pub const ID: &str = "integration_telegram";
pub const KEY_API_ID: &str = "telegram-api-id";
pub const KEY_API_HASH: &str = "telegram-api-hash";
const KEY_SESSION: &str = "telegram-session";
const HELPER_EXE: &str = "mika-telegram.exe";
/// The integration name an agent puts in its `agent.md` to receive the posts.
pub const AGENT_INTEGRATION: &str = "telegram";
/// How long posts and their images stay for the agents.
const KEEP_SECS: i64 = 3 * 24 * 3600;
/// Most the media folder of one agent may hold; the oldest files go first.
const MAX_MEDIA_BYTES: u64 = 200 * 1024 * 1024;

// ── The helper process ────────────────────────────────────────────────────────

struct Helper {
    _child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    next_id: u64,
}

enum Failure { Said(String), Timeout, Gone }

static HELPER: LazyLock<tokio::sync::Mutex<Option<Helper>>> = LazyLock::new(|| tokio::sync::Mutex::new(None));
/// New credentials or a sign-out: the next request starts a fresh helper.
static RESET: AtomicBool = AtomicBool::new(false);

/// Drops the running helper (it is killed with it), or asks the next request to do so if one is running now.
pub fn reset() {
    match HELPER.try_lock() {
        Ok(mut helper) => *helper = None,
        Err(_) => RESET.store(true, Ordering::SeqCst),
    }
}

impl Helper {
    async fn request(&mut self, mut body: Value, wait: Duration) -> Result<Value, Failure> {
        let id = self.next_id;
        self.next_id += 1;
        body["id"] = json!(id);
        let line = format!("{body}\n");
        if self.stdin.write_all(line.as_bytes()).await.is_err() || self.stdin.flush().await.is_err() { return Err(Failure::Gone); }
        let deadline = Instant::now() + wait;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = match tokio::time::timeout(left, self.lines.next_line()).await {
                Err(_) => return Err(Failure::Timeout),
                Ok(Ok(Some(line))) => line,
                Ok(_) => return Err(Failure::Gone),
            };
            let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
            if message["event"] == "session" {
                store_session(message["value"].as_str());
                continue;
            }
            if message["id"].as_u64() != Some(id) { continue; }
            return if message["ok"] == true { Ok(message["result"].clone()) } else {
                Err(Failure::Said(message["error"].as_str().unwrap_or("Telegram no respondió.").to_string()))
            };
        }
    }
}

/// The session goes straight to the Credential Manager; a value that does not fit is logged (never its content).
fn store_session(value: Option<&str>) {
    let result = match value { Some(v) => secrets::set(KEY_SESSION, v), None => secrets::clear(KEY_SESSION) };
    if let Err(err) = result { log::line(format!("telegram: could not store the session: {err}")); }
}

/// In an install it is a resource next to mika.exe; in `tauri dev`, in the workspace's target folder.
fn helper_path(app: &AppHandle) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(p) = app.path().resolve(HELPER_EXE, tauri::path::BaseDirectory::Resource) { candidates.push(p); }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
        candidates.push(dir.join(HELPER_EXE));
        candidates.push(dir.join("../release").join(HELPER_EXE));
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn credentials() -> Option<(i64, String)> {
    let id = secrets::get(KEY_API_ID)?.trim().parse::<i64>().ok().filter(|v| *v > 0)?;
    Some((id, secrets::get(KEY_API_HASH)?.trim().to_string()))
}

async fn start(app: &AppHandle) -> Result<Helper, String> {
    let (api_id, api_hash) = credentials().ok_or("Guarda primero tu api_id y api_hash de Telegram.")?;
    let path = helper_path(app).ok_or("No se encontró mika-telegram.exe. Reinstala MIKA.")?;
    let mut cmd = tokio::process::Command::new(path);
    cmd.creation_flags(0x0800_0000).kill_on_drop(true)
        .stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    let mut child = cmd.spawn().map_err(|_| "No se pudo iniciar el lector de Telegram.")?;
    let stdin = child.stdin.take().ok_or("No se pudo iniciar el lector de Telegram.")?;
    let stdout = child.stdout.take().ok_or("No se pudo iniciar el lector de Telegram.")?;
    let mut helper = Helper { _child: child, stdin, lines: BufReader::new(stdout).lines(), next_id: 1 };
    let connect = json!({ "cmd": "connect", "apiId": api_id, "apiHash": api_hash, "session": secrets::get(KEY_SESSION) });
    match helper.request(connect, Duration::from_secs(10)).await {
        Ok(_) => Ok(helper),
        Err(Failure::Said(error)) => Err(error),
        Err(_) => Err("El lector de Telegram no arrancó.".into()),
    }
}

/// One request to the helper, starting it when needed. A helper that hangs or dies is dropped; the next call starts
/// a new one.
async fn call(app: &AppHandle, body: Value, wait: Duration) -> Result<Value, String> {
    let mut guard = HELPER.lock().await;
    if RESET.swap(false, Ordering::SeqCst) { *guard = None; }
    if guard.is_none() { *guard = Some(start(app).await?); }
    let Some(helper) = guard.as_mut() else { return Err("El lector de Telegram no arrancó.".into()) };
    match helper.request(body, wait).await {
        Ok(value) => Ok(value),
        Err(Failure::Said(error)) => Err(error),
        Err(Failure::Timeout) => { *guard = None; Err("Telegram tardó demasiado en responder.".into()) }
        Err(Failure::Gone) => { *guard = None; Err("El lector de Telegram se cerró. Intenta de nuevo.".into()) }
    }
}

// ── Login and chats (the settings window) ─────────────────────────────────────

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// api_id and api_hash are stored.
    pub configured: bool,
    pub authorized: bool,
    /// "idle", "code" or "password".
    pub step: String,
    pub hint: Option<String>,
    pub name: Option<String>,
    pub error: Option<String>,
}

fn status_from(value: Value) -> Status {
    Status {
        configured: true,
        authorized: value["authorized"] == true,
        step: value["step"].as_str().unwrap_or("idle").to_string(),
        hint: value["hint"].as_str().map(str::to_string),
        name: value["name"].as_str().map(str::to_string),
        error: None,
    }
}

pub async fn status(app: &AppHandle) -> Status {
    if credentials().is_none() { return Status { step: "idle".into(), ..Default::default() }; }
    match call(app, json!({ "cmd": "status" }), Duration::from_secs(30)).await {
        Ok(value) => status_from(value),
        Err(error) => Status { configured: true, step: "idle".into(), error: Some(error), ..Default::default() },
    }
}

pub async fn send_code(app: &AppHandle, phone: &str) -> Result<Status, String> {
    call(app, json!({ "cmd": "sendCode", "phone": phone }), Duration::from_secs(60)).await.map(status_from)
}

pub async fn sign_in(app: &AppHandle, code: &str) -> Result<Status, String> {
    call(app, json!({ "cmd": "signIn", "code": code }), Duration::from_secs(60)).await.map(status_from)
}

pub async fn password(app: &AppHandle, password: &str) -> Result<Status, String> {
    call(app, json!({ "cmd": "password", "password": password }), Duration::from_secs(60)).await.map(status_from)
}

/// Closes the session on Telegram's side too, then forgets it here whatever Telegram said.
pub async fn sign_out(app: &AppHandle) {
    if credentials().is_some() { let _ = call(app, json!({ "cmd": "signOut" }), Duration::from_secs(20)).await; }
    store_session(None);
    reset();
}

/// A chat the user can pick. `hash` is a string: JavaScript would round a 64-bit number.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatOption { pub id: i64, pub hash: String, pub title: String, pub kind: String }

pub async fn chats(app: &AppHandle) -> Result<Vec<ChatOption>, String> {
    let value = call(app, json!({ "cmd": "chats", "limit": 300 }), Duration::from_secs(60)).await?;
    Ok(value.as_array().into_iter().flatten().filter_map(|c| Some(ChatOption {
        id: c["id"].as_i64()?,
        hash: c["hash"].as_i64()?.to_string(),
        title: c["title"].as_str().unwrap_or("").chars().take(120).collect(),
        kind: c["kind"].as_str().unwrap_or("channel").to_string(),
    })).collect())
}

// ── Posts ─────────────────────────────────────────────────────────────────────

/// One post, as the helper sends it and as `posts.jsonl` keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Post {
    pub chat_id: i64,
    pub chat: String,
    pub id: i32,
    /// Unix seconds.
    pub date: i64,
    #[serde(default)]
    pub text: String,
    /// Who wrote it when that is not the chat itself (a member of a group, a channel's signature).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sender: String,
    #[serde(default)]
    pub links: Vec<String>,
    /// File names inside `telegram\media\`.
    #[serde(default)]
    pub photos: Vec<String>,
    /// A video, an audio, a document… the message carries besides text and photos (named, never downloaded).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub media: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<i64>,
}

/// `telegram\` in the workspace of every agent that takes Telegram posts (PARLEY) and of the main agent, which can use
/// them for anything else.
pub fn agent_dirs() -> Vec<PathBuf> {
    named_agents::load_all().into_iter()
        .filter(|a| a.integration.as_deref() == Some(AGENT_INTEGRATION) || a.id == named_agents::ORCHESTRATOR)
        .map(|a| inbox_dir(&a.id))
        .collect()
}

pub fn inbox_dir(agent: &str) -> PathBuf { named_agents::workspace(agent).join("telegram") }

fn state_path() -> PathBuf { settings::local_dir().join("telegram").join("state.json") }

/// The newest message id read per chat.
fn load_state() -> HashMap<i64, i32> {
    std::fs::read(state_path()).ok().and_then(|b| serde_json::from_slice::<HashMap<String, i32>>(&b).ok())
        .map(|m| m.into_iter().filter_map(|(k, v)| Some((k.parse().ok()?, v))).collect())
        .unwrap_or_default()
}

fn save_state(state: &HashMap<i64, i32>) {
    let path = state_path();
    let map: HashMap<String, i32> = state.iter().map(|(k, v)| (k.to_string(), *v)).collect();
    if let (Some(dir), Ok(bytes)) = (path.parent(), serde_json::to_vec(&map)) {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(path, bytes);
    }
}

fn now_secs() -> i64 { (crate::services::chat_store::now_ms() / 1000) as i64 }

/// The only file names the helper may hand back: `<chat>_<post>.jpg`, digits only. Anything else could point
/// outside the media folder, so it is dropped before it gets anywhere near a path.
pub fn safe_media_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".jpg") else { return false };
    let Some((chat, post)) = stem.split_once('_') else { return false };
    [chat, post].iter().all(|p| !p.is_empty() && p.len() <= 20 && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Mark of the Web: SmartScreen, Defender and Office treat the file as downloaded from the internet if anyone ever
/// opens it outside MIKA.
fn mark_from_internet(path: &Path) {
    let mut stream = path.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    let _ = std::fs::write(stream, "[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://telegram.org/\r\n");
}

/// Appends the posts to every agent's `posts.jsonl` (images are downloaded into the first one and copied over), and
/// drops what is older than [`KEEP_SECS`].
fn store_posts(dirs: &[PathBuf], posts: &[Post]) {
    let Some(first) = dirs.first() else { return };
    for dir in dirs {
        let _ = std::fs::create_dir_all(dir.join("media"));
        for name in posts.iter().flat_map(|p| p.photos.iter()) {
            let target = dir.join("media").join(name);
            if dir != first { let _ = std::fs::copy(first.join("media").join(name), &target); }
            if target.is_file() { mark_from_internet(&target); }
        }
        if !posts.is_empty() {
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("posts.jsonl")) {
                for post in posts {
                    if let Ok(line) = serde_json::to_string(post) { let _ = writeln!(file, "{line}"); }
                }
            }
        }
        prune(dir);
    }
}

fn prune(dir: &Path) {
    let cutoff = now_secs() - KEEP_SECS;
    let file = dir.join("posts.jsonl");
    let all = read_posts(&file);
    if all.iter().any(|p| p.date < cutoff) {
        let kept: Vec<String> = all.iter().filter(|p| p.date >= cutoff).filter_map(|p| serde_json::to_string(p).ok()).collect();
        let tmp = file.with_extension("jsonl.tmp");
        if std::fs::write(&tmp, kept.join("\n") + if kept.is_empty() { "" } else { "\n" }).is_ok() { let _ = std::fs::rename(&tmp, &file); }
    }
    let Ok(entries) = std::fs::read_dir(dir.join("media")) else { return };
    let max_age = Duration::from_secs(KEEP_SECS as u64);
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        let name = entry.file_name();
        // Old files, half-downloaded ones and anything that isn't one of ours go.
        if modified.elapsed().is_ok_and(|age| age > max_age) || !name.to_str().is_some_and(safe_media_name) {
            let _ = std::fs::remove_file(entry.path());
        } else {
            files.push((modified, meta.len(), entry.path()));
        }
    }
    files.sort_by_key(|(modified, _, _)| *modified);
    let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
    for (_, len, path) in files {
        if total <= MAX_MEDIA_BYTES { break; }
        if std::fs::remove_file(&path).is_ok() { total -= len; }
    }
}

fn read_posts(file: &Path) -> Vec<Post> {
    std::fs::read_to_string(file).map(|text| text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()).unwrap_or_default()
}

/// An agent's posts newer than `since` (Unix seconds), oldest first.
pub fn posts_since(agent: &str, since: i64) -> Vec<Post> {
    let mut posts: Vec<Post> = read_posts(&inbox_dir(agent).join("posts.jsonl")).into_iter().filter(|p| p.date > since).collect();
    posts.sort_by_key(|p| (p.date, p.id));
    posts
}

// ── The poller ────────────────────────────────────────────────────────────────

pub async fn poll(app: AppHandle) {
    if credentials().is_none() { return emit_card(&app, Some("Configura Telegram en Ajustes.".into()), None); }
    if secrets::get(KEY_SESSION).is_none() { return emit_card(&app, Some("Inicia sesión en Telegram desde Ajustes.".into()), None); }
    let chats = app.try_state::<crate::Shared>().map(|s| s.settings.lock().unwrap().telegram_chats.clone()).unwrap_or_default();
    if chats.is_empty() { return emit_card(&app, Some("Elige tus canales en Ajustes.".into()), None); }
    let dirs = agent_dirs();
    let Some(first) = dirs.first() else { return };

    let mut state = load_state();
    // The chats read before this poll: a chat's first read is its backlog, not news.
    let known_before = state.clone();
    let request = json!({
        "cmd": "fetch",
        "chats": chats.iter().map(|c| json!({
            "chat": { "id": c.id, "hash": c.hash.parse::<i64>().unwrap_or(0), "title": c.title },
            "after": state.get(&c.id).copied().unwrap_or(0),
        })).collect::<Vec<_>>(),
        "limit": 20,
        "mediaDir": first.join("media"),
    });
    let result = match call(&app, request, Duration::from_secs(180)).await {
        Ok(result) => result,
        Err(error) => return emit_card(&app, Some(error), None),
    };
    let mut posts: Vec<Post> = serde_json::from_value(result["posts"].clone()).unwrap_or_default();
    for post in &mut posts { post.photos.retain(|name| safe_media_name(name)); }
    for entry in result["last"].as_array().into_iter().flatten() {
        if let (Some(chat), Some(last)) = (entry["chatId"].as_i64(), entry["last"].as_i64()) {
            let last = i32::try_from(last).unwrap_or(0);
            if last > state.get(&chat).copied().unwrap_or(0) { state.insert(chat, last); }
        }
    }
    save_state(&state);
    store_posts(&dirs, &posts);
    if !posts.is_empty() { crate::services::picks::note_posts(&posts); }
    let now = now_secs();
    let fresh: Vec<super::telegram_notice::NoticePost> = posts.iter()
        .filter(|p| super::telegram_notice::is_fresh(known_before.get(&p.chat_id).copied().unwrap_or(0) > 0, p.date, now))
        .map(|p| super::telegram_notice::NoticePost { chat_id: p.chat_id, chat: &p.chat, id: p.id, date: p.date, text: &p.text, sender: &p.sender, photos: p.photos.len() })
        .collect();
    let notices = super::telegram_notice::summaries(&fresh);
    // One chat that can't be read (left, banned) doesn't hide the others: it is a warning on the card.
    let warning = result["errors"].as_array().and_then(|e| e.first()).map(|e| {
        let title = chats.iter().find(|c| Some(c.id) == e["chatId"].as_i64()).map(|c| c.title.clone()).unwrap_or_default();
        format!("{title}: {}", e["error"].as_str().unwrap_or(""))
    });
    emit_card_with(&app, None, None, warning, notices);
}

// ── The card ──────────────────────────────────────────────────────────────────

/// Sends the island what the Telegram card shows: the latest posts and PARLEY's latest picks.
pub fn emit_card(app: &AppHandle, error: Option<String>, event: Option<IntegrationEvent>) {
    emit_card_with(app, error, event, None, Vec::new());
}

fn emit_card_with(app: &AppHandle, error: Option<String>, event: Option<IntegrationEvent>, warning: Option<String>, notices: Vec<super::telegram_notice::Notice>) {
    let mut data = card_data();
    if let Some(warning) = warning { data["warning"] = json!(warning); }
    emit(app, IntegrationUpdate { id: ID, data, error, event, notices });
}

fn card_data() -> Value {
    let agent = crate::services::picks::AGENT;
    let posts: Vec<Value> = posts_since(agent, now_secs() - 24 * 3600).iter().rev().take(4).map(|p| json!({
        "chat": p.chat,
        "text": p.text.chars().take(160).collect::<String>(),
        "date": p.date * 1000,
        "photos": p.photos.len(),
    })).collect();
    let scan = crate::services::picks::last_scan();
    json!({ "posts": posts, "picks": scan.picks, "scanAt": scan.at, "scanNote": scan.note, "unit": scan.unit })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_our_media_names_reach_a_path() {
        assert!(safe_media_name("1001234567890_42.jpg"));
        for bad in ["../x.jpg", "1_2.png", "1_2.jpg.exe", "a_2.jpg", "1_2", "_2.jpg", "1_.jpg", "1\\..\\2_3.jpg", "C:1_2.jpg", "1_2.JPG"] {
            assert!(!safe_media_name(bad), "{bad:?}");
        }
    }
}
