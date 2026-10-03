//! Telegram: the user talks to PARLEY from their own bot, and PARLEY's picks can be sent there.
//!
//! The user makes a bot with @BotFather and pastes its token in Settings; it is checked with `getMe` and then kept
//! only in the Keychain / Credential Manager. Pairing: Settings shows a 6-digit code and the user sends
//! `/start <code>` to the bot; the first private chat that sends the right code becomes the only chat Buddy listens
//! to (five wrong codes and that code is gone). Anything from any other chat is ignored: a log line, no reply.
//!
//! One long-polling thread (`getUpdates`, 50 s) runs only while the bot is connected and paired (or waiting for its
//! code): blocked in the request it costs no CPU. It stops on disconnect, when the code expires unpaired, when
//! Telegram refuses the token, and when the core goes; network errors wait 2 s, 4 s, 8 s… up to 5 min.
//!
//! A message from the paired chat is one PARLEY turn in Buddy's chat «Telegram · PARLEY» (it shows in the history),
//! always restricted: no commands, no folder edits, no screen. Telegram text is data, never instructions for Buddy.
//! Answers go back as plain text (Markdown removed), split under Telegram's 4096-character limit.
//!
//! MIKA's Telegram (`reuse/mika/apps/windows/telegram`) reads the user's own account over MTProto: nothing of it
//! fits a bot, so this is new code.

use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::events::{Event, EventBus};
use crate::spotify::Secrets;
use crate::store::Store;

const KEYCHAIN_SERVICE: &str = "io.github.crhistian-cornejo.buddy";
const KEYCHAIN_ACCOUNT: &str = "telegram-bot-token";
/// `@username` of the connected bot (empty: not connected). Not a secret.
pub const BOT_KEY: &str = "telegram.bot";
/// The one chat Buddy answers (empty: not paired yet).
pub const CHAT_KEY: &str = "telegram.chat";
/// The next `getUpdates` offset, so nothing is answered twice after a restart.
pub const OFFSET_KEY: &str = "telegram.offset";
/// Buddy's chat where the Telegram conversation is kept.
pub const CHAT_ID: &str = "telegram-parley";
pub const CHAT_TITLE: &str = "Telegram · PARLEY";
/// The agent that answers Telegram.
pub const AGENT: &str = "parley";

const POLL_SECONDS: u64 = 50;
/// Telegram allows 4096 characters per message; a margin for safety.
pub const MAX_MESSAGE: usize = 4000;
/// The longest incoming text passed to PARLEY.
const MAX_INCOMING: usize = 4000;
const CODE_TTL: Duration = Duration::from_secs(15 * 60);
const MAX_WRONG_CODES: u32 = 5;
const MAX_BACKOFF: Duration = Duration::from_secs(300);

/// What Settings shows.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct TelegramStatus {
    /// A bot token is stored and was accepted by Telegram.
    pub connected: bool,
    /// A chat is paired: Buddy listens to it.
    pub paired: bool,
    /// `@username` of the bot (empty when not connected).
    pub bot_name: String,
    /// The code to send as `/start <code>` while connected and not paired (empty otherwise).
    pub pairing_code: String,
    /// The last problem worth showing (token refused, another app reading the bot…); empty when fine.
    pub error: String,
}

/// The bot token in the Keychain (Mac) or Credential Manager (Windows).
pub struct TokenSecret;

impl TokenSecret {
    fn entry() -> Option<keyring::Entry> {
        keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).ok()
    }
}

impl Secrets for TokenSecret {
    fn get(&self) -> Option<String> {
        Self::entry()?.get_password().ok().filter(|s| !s.is_empty())
    }
    fn set(&self, secret: &str) -> Result<(), String> {
        Self::entry().ok_or("No hay llavero disponible.")?.set_password(secret).map_err(|e| e.to_string())
    }
    fn delete(&self) {
        if let Some(entry) = Self::entry() {
            let _ = entry.delete_credential();
        }
    }
}

// ── The Bot API ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// No answer (offline, timeout, DNS, a 5xx): try again later.
    Network(String),
    /// The token is not (or no longer) valid.
    Unauthorized,
    /// Someone else reads this bot's updates (a webhook or another program).
    Conflict,
    /// Too many requests: wait this many seconds.
    RetryAfter(u64),
    /// Telegram refused this request (its description).
    Refused(String),
}

impl ApiError {
    pub fn text(&self) -> String {
        match self {
            ApiError::Network(e) => format!("No hay conexión con Telegram ({e})."),
            ApiError::Unauthorized => "Telegram no acepta el token del bot. Cópialo otra vez de @BotFather.".into(),
            ApiError::Conflict => {
                "Otro programa está leyendo este bot (un webhook u otra app). Usa un bot solo para Buddy.".into()
            }
            ApiError::RetryAfter(s) => format!("Telegram pide esperar {s} s."),
            ApiError::Refused(e) => format!("Telegram respondió: {e}"),
        }
    }
}

/// One Bot API call: `method` with a JSON body; the `result` when Telegram says ok.
pub trait Api: Send + Sync {
    fn call(&self, token: &str, method: &str, body: &Value, timeout: Duration) -> Result<Value, ApiError>;
}

/// The real one, over HTTPS to api.telegram.org.
pub struct HttpApi;

impl Api for HttpApi {
    fn call(&self, token: &str, method: &str, body: &Value, timeout: Duration) -> Result<Value, ApiError> {
        let agent: ureq::Agent =
            ureq::Agent::config_builder().timeout_global(Some(timeout)).http_status_as_error(false).build().into();
        let url = format!("https://api.telegram.org/bot{token}/{method}");
        // The token is part of the URL: it never reaches an error text or the log.
        let mut response = agent
            .post(&url)
            .header("Content-Type", "application/json")
            .send(body.to_string())
            .map_err(|e| ApiError::Network(e.to_string().replace(token, "…")))?;
        let status = response.status().as_u16();
        let text = response.body_mut().read_to_string().unwrap_or_default();
        match serde_json::from_str::<Value>(&text) {
            Ok(reply) => api_result(status, reply),
            Err(_) if status >= 500 || status == 0 => Err(ApiError::Network(format!("Telegram respondió {status}"))),
            Err(_) => Err(ApiError::Refused(format!("respuesta extraña ({status})"))),
        }
    }
}

/// Telegram's `{ok, result, error_code, description, parameters}` envelope.
fn api_result(status: u16, reply: Value) -> Result<Value, ApiError> {
    if reply["ok"] == true {
        return Ok(reply["result"].clone());
    }
    let code = reply["error_code"].as_u64().unwrap_or(status as u64);
    let description: String = reply["description"].as_str().unwrap_or("error").chars().take(200).collect();
    Err(match code {
        401 | 404 => ApiError::Unauthorized,
        409 => ApiError::Conflict,
        429 => ApiError::RetryAfter(reply["parameters"]["retry_after"].as_u64().unwrap_or(5)),
        c if c >= 500 => ApiError::Network(description),
        _ => ApiError::Refused(description),
    })
}

/// `123456789:AA…`: digits, a colon, then the secret part.
pub fn token_looks_valid(token: &str) -> bool {
    let Some((id, secret)) = token.split_once(':') else { return false };
    !id.is_empty()
        && id.len() <= 20
        && id.chars().all(|c| c.is_ascii_digit())
        && (30..=100).contains(&secret.len())
        && secret.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

// ── Updates and pairing (pure, tested) ────────────────────────────────────────

/// One update, as far as Buddy cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub update_id: i64,
    /// The chat of a `message` update (None for anything else).
    pub chat_id: Option<i64>,
    pub private: bool,
    /// The text, when the message has one.
    pub text: Option<String>,
}

/// The `result` of `getUpdates`. Entries without an `update_id` are dropped.
pub fn parse_updates(result: &Value) -> Vec<Update> {
    result
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|u| {
            let update_id = u["update_id"].as_i64()?;
            let message = &u["message"];
            Some(Update {
                update_id,
                chat_id: message["chat"]["id"].as_i64(),
                private: message["chat"]["type"] == "private",
                text: message["text"].as_str().map(str::to_string),
            })
        })
        .collect()
}

/// A pairing code waiting for its `/start`.
#[derive(Debug, Clone)]
pub struct Pending {
    pub code: String,
    pub until: Instant,
    pub wrong: u32,
}

impl Pending {
    pub fn new(now: Instant) -> Self {
        Pending { code: new_code(), until: now + CODE_TTL, wrong: 0 }
    }
    fn valid(&self, now: Instant) -> bool {
        now < self.until && self.wrong < MAX_WRONG_CODES
    }
}

/// Six digits from the process' random hasher seed and the clock (no extra crate; the code lives 15 minutes and
/// allows five tries).
fn new_code() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    hasher.write_u128(nanos);
    format!("{:06}", hasher.finish() % 1_000_000)
}

/// What to do with one update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The right code from a private chat: it becomes the paired chat.
    Pair(i64),
    /// Text from the paired chat for PARLEY.
    Message(i64, String),
    /// A short fixed answer to the paired chat.
    Reply(i64, &'static str),
    /// Not for Buddy (why, for the log).
    Ignore(&'static str),
}

pub const PAIRED_TEXT: &str = "¡Listo! Este chat quedó vinculado con Buddy. Escribe aquí lo que quieras preguntarle a PARLEY.";
const ALREADY_TEXT: &str = "Ya estamos conectados. Escríbeme lo que quieras preguntarle a PARLEY.";
const TEXT_ONLY: &str = "Por ahora solo leo mensajes de texto.";

/// The pairing rules. `pending` loses a wrong try, and goes once used up or used.
pub fn decide(paired: Option<i64>, pending: &mut Option<Pending>, now: Instant, update: &Update) -> Decision {
    let Some(chat) = update.chat_id else { return Decision::Ignore("no es un mensaje") };
    if let Some(own) = paired {
        if chat != own {
            return Decision::Ignore("otro chat");
        }
        return match update.text.as_deref().map(str::trim) {
            None | Some("") => Decision::Reply(chat, TEXT_ONLY),
            Some(t) if command(t) == Some("/start") => Decision::Reply(chat, ALREADY_TEXT),
            Some(t) => Decision::Message(chat, t.chars().take(MAX_INCOMING).collect()),
        };
    }
    if !update.private {
        return Decision::Ignore("no es un chat privado");
    }
    let Some(code) = pending.as_mut().filter(|p| p.valid(now)) else {
        return Decision::Ignore("sin código de vinculación");
    };
    let text = update.text.as_deref().unwrap_or("").trim();
    if command(text) != Some("/start") {
        return Decision::Ignore("sin vincular");
    }
    let given: String = text.split_whitespace().nth(1).unwrap_or("").chars().filter(|c| c.is_ascii_digit()).collect();
    if given == code.code {
        *pending = None;
        return Decision::Pair(chat);
    }
    code.wrong += 1;
    if code.wrong >= MAX_WRONG_CODES {
        *pending = None;
    }
    Decision::Ignore("código incorrecto")
}

/// `/start` from `/start 123` or `/start@mi_bot 123`.
fn command(text: &str) -> Option<&str> {
    let first = text.split_whitespace().next()?;
    first.starts_with('/').then(|| first.split('@').next().unwrap_or(first))
}

/// 2 s, 4 s, 8 s… after each failure in a row, never more than 5 minutes.
pub fn backoff(failures: u32) -> Duration {
    let exp = failures.saturating_sub(1).min(16);
    Duration::from_secs(2u64.saturating_mul(1 << exp)).min(MAX_BACKOFF)
}

// ── Text out (pure, tested) ───────────────────────────────────────────────────

/// Markdown as plain text Telegram shows as written: no `**`, `#`, backticks or table rulers; links as
/// `text (url)`; bullets as `•`; table rows as cells joined by ` · `.
pub fn to_plain(markdown: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_code = false;
    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            out.push(line.to_string());
            continue;
        }
        if is_table_ruler(trimmed) {
            continue;
        }
        if is_rule(trimmed) {
            out.push(String::new());
            continue;
        }
        let mut text = trimmed.to_string();
        let hashes = text.chars().take_while(|c| *c == '#').count();
        if hashes > 0 && text[hashes..].starts_with(' ') {
            text = text[hashes..].trim().to_string();
        }
        if let Some(rest) = text.strip_prefix("- ").or_else(|| text.strip_prefix("* ")).or_else(|| text.strip_prefix("+ ")) {
            text = format!("• {rest}");
        }
        if let Some(rest) = text.strip_prefix("> ") {
            text = rest.to_string();
        }
        if text.starts_with('|') {
            let cells: Vec<&str> = text.trim_matches('|').split('|').map(str::trim).collect();
            text = cells.join(" · ");
        }
        text = links_plain(&text);
        for mark in ["**", "__", "~~", "`"] {
            text = text.replace(mark, "");
        }
        out.push(text);
    }
    let mut joined = out.join("\n");
    while joined.contains("\n\n\n") {
        joined = joined.replace("\n\n\n", "\n\n");
    }
    joined.trim().to_string()
}

/// `---`, `***` or `___`: a blank line.
fn is_rule(line: &str) -> bool {
    let body: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    body.len() >= 3 && ['-', '*', '_'].iter().any(|m| body.chars().all(|c| c == *m))
}

/// A table's `|---|:--:|` line: dropped.
fn is_table_ruler(line: &str) -> bool {
    let body: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    body.starts_with('|') && body.contains('-') && body.chars().all(|c| matches!(c, '|' | '-' | ':'))
}

/// `[text](url)` → `text (url)` (just `url` when both are the same).
fn links_plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let parsed = after.find("](").and_then(|close| {
            let url_part = &after[close + 2..];
            url_part.find(')').map(|end| (&after[..close], &url_part[..end], &url_part[end + 1..]))
        });
        match parsed {
            Some((label, url, tail)) if !label.contains('[') => {
                out.push_str(&rest[..open]);
                if label.trim() == url.trim() || label.trim().is_empty() {
                    out.push_str(url);
                } else {
                    out.push_str(&format!("{label} ({url})"));
                }
                rest = tail;
            }
            _ => {
                out.push_str(&rest[..open + 1]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Pieces of at most `max` characters, cut at a blank line, a line end or a space when there is one in the second
/// half of the piece; otherwise exactly at `max`.
pub fn split_message(text: &str, max: usize) -> Vec<String> {
    let max = max.max(1);
    let mut out = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        if rest.chars().count() <= max {
            out.push(rest.to_string());
            break;
        }
        let limit = rest.char_indices().nth(max).map(|(i, _)| i).unwrap_or(rest.len());
        let window = &rest[..limit];
        let half = rest.char_indices().nth(max / 2).map(|(i, _)| i).unwrap_or(0);
        let cut = [window.rfind("\n\n"), window.rfind('\n'), window.rfind(' ')]
            .into_iter()
            .flatten()
            .find(|&i| i >= half && i > 0)
            .unwrap_or(limit);
        let piece = rest[..cut].trim_end();
        if !piece.is_empty() {
            out.push(piece.to_string());
        }
        rest = rest[cut..].trim_start();
    }
    out
}

// ── The service ───────────────────────────────────────────────────────────────

/// Answers one message from the paired chat (PARLEY's turn); Err is shown to the user as it is.
pub type Handler = Box<dyn Fn(&str) -> Result<String, String> + Send + Sync>;

#[derive(Default)]
struct State {
    /// The poller should keep going.
    want: bool,
    /// The poller thread exists.
    alive: bool,
    /// In memory while connected (read from the vault once).
    token: Option<String>,
    pending: Option<Pending>,
    error: String,
}

pub struct Telegram {
    store: Arc<Mutex<Store>>,
    bus: Arc<EventBus>,
    api: Arc<dyn Api>,
    secrets: Box<dyn Secrets>,
    handler: Handler,
    state: Mutex<State>,
    wake: Condvar,
    /// False in tests that drive `poll_once` by hand.
    threads: bool,
}

impl Telegram {
    pub fn new(store: Arc<Mutex<Store>>, bus: Arc<EventBus>, api: Arc<dyn Api>, secrets: Box<dyn Secrets>, handler: Handler) -> Self {
        Self { store, bus, api, secrets, handler, state: Mutex::default(), wake: Condvar::new(), threads: true }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn setting(&self, key: &str) -> String {
        self.store.lock().unwrap_or_else(|p| p.into_inner()).setting(key).ok().flatten().unwrap_or_default()
    }

    fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        self.store.lock().unwrap_or_else(|p| p.into_inner()).set_setting(key, value).map_err(|e| e.to_string())
    }

    fn bot(&self) -> String {
        self.setting(BOT_KEY)
    }

    fn paired_chat(&self) -> Option<i64> {
        self.setting(CHAT_KEY).trim().parse().ok()
    }

    fn changed(&self) {
        self.bus.publish(Event::TelegramChanged);
    }

    /// Where things stand. Connected and not paired: a code is made (if there is no valid one) and the poller
    /// started, so the `/start` is heard.
    pub fn status(self: &Arc<Self>) -> TelegramStatus {
        let bot = self.bot();
        let paired = self.paired_chat().is_some();
        let connected = !bot.is_empty();
        let mut code = String::new();
        if connected && !paired {
            let now = Instant::now();
            let mut state = self.state();
            if !state.pending.as_ref().is_some_and(|p| p.valid(now)) {
                state.pending = Some(Pending::new(now));
            }
            code = state.pending.as_ref().map(|p| p.code.clone()).unwrap_or_default();
            drop(state);
            self.ensure_running();
        }
        let error = self.state().error.clone();
        TelegramStatus { connected, paired, bot_name: bot, pairing_code: code, error }
    }

    /// Checks the token with `getMe`, then keeps it in the vault and the bot's name in the settings. A new bot
    /// starts unpaired, with a fresh code.
    pub fn connect(self: &Arc<Self>, token: &str) -> Result<(), String> {
        let token = token.trim();
        if !token_looks_valid(token) {
            return Err("Ese no parece un token de bot. Cópialo completo de @BotFather (números, dos puntos y letras).".into());
        }
        let me = self.api.call(token, "getMe", &json!({}), Duration::from_secs(15)).map_err(|e| e.text())?;
        let username = me["username"].as_str().filter(|u| !u.is_empty()).ok_or("Telegram no devolvió el nombre del bot.")?;
        if me["is_bot"] != true {
            return Err("Ese token no es de un bot.".into());
        }
        self.secrets.set(token)?;
        self.set_setting(BOT_KEY, &format!("@{username}"))?;
        self.set_setting(CHAT_KEY, "")?;
        self.set_setting(OFFSET_KEY, "")?;
        {
            let mut state = self.state();
            state.token = Some(token.to_string());
            state.pending = Some(Pending::new(Instant::now()));
            state.error.clear();
        }
        crate::log::line(format!("telegram: conectado con @{username}"));
        self.ensure_running();
        self.changed();
        Ok(())
    }

    /// Forgets the token (vault), the bot, the paired chat and the offset; the poller stops.
    pub fn disconnect(&self) -> Result<(), String> {
        self.secrets.delete();
        for key in [BOT_KEY, CHAT_KEY, OFFSET_KEY] {
            self.set_setting(key, "")?;
        }
        {
            let mut state = self.state();
            state.want = false;
            state.token = None;
            state.pending = None;
            state.error.clear();
        }
        self.wake.notify_all();
        crate::log::line("telegram: desconectado");
        self.changed();
        Ok(())
    }

    /// Unpairs the current chat (if any) and makes a fresh code, to pair again or move to another chat.
    pub fn new_pairing_code(self: &Arc<Self>) -> Result<String, String> {
        if self.bot().is_empty() {
            return Err("Primero conecta tu bot.".into());
        }
        self.set_setting(CHAT_KEY, "")?;
        let code = {
            let mut state = self.state();
            if state.token.is_none() {
                state.token = self.secrets.get();
            }
            let pending = Pending::new(Instant::now());
            let code = pending.code.clone();
            state.pending = Some(pending);
            code
        };
        self.ensure_running();
        self.changed();
        Ok(code)
    }

    /// Sends `text` (Markdown is fine: it goes as plain text) to the paired chat, in as many messages as needed.
    pub fn send(&self, text: &str) -> Result<(), String> {
        let chat = self.paired_chat().ok_or("Telegram no está vinculado. Conéctalo en Ajustes › Conexiones.")?;
        let token = self.token().ok_or("Falta el token del bot. Conéctalo otra vez en Ajustes › Conexiones.")?;
        let pieces = split_message(&to_plain(text), MAX_MESSAGE);
        if pieces.is_empty() {
            return Err("No hay nada que enviar.".into());
        }
        for piece in pieces {
            self.send_text(&token, chat, &piece).map_err(|e| e.text())?;
        }
        Ok(())
    }

    /// At launch: the poller starts only when a bot is connected and a chat paired.
    pub fn start_if_configured(self: &Arc<Self>) {
        if self.bot().is_empty() || self.paired_chat().is_none() {
            return;
        }
        if self.token().is_none() {
            self.state().error = "Falta el token del bot en el llavero. Conéctalo otra vez.".into();
            return;
        }
        self.ensure_running();
    }

    /// Stops the poller (the core is going away). A request in flight ends within its timeout.
    pub fn shutdown(&self) {
        self.state().want = false;
        self.wake.notify_all();
    }

    /// The token in memory, else from the vault (once).
    fn token(&self) -> Option<String> {
        let mut state = self.state();
        if state.token.is_none() {
            state.token = self.secrets.get();
        }
        state.token.clone()
    }

    fn send_text(&self, token: &str, chat: i64, text: &str) -> Result<Value, ApiError> {
        let body = json!({ "chat_id": chat, "text": text, "link_preview_options": { "is_disabled": true } });
        self.api.call(token, "sendMessage", &body, Duration::from_secs(20))
    }

    fn ensure_running(self: &Arc<Self>) {
        let mut state = self.state();
        state.want = true;
        if !state.alive && self.threads {
            state.alive = true;
            let me = self.clone();
            let spawned = std::thread::Builder::new().name("buddy-telegram".into()).spawn(move || me.run());
            if spawned.is_err() {
                state.alive = false;
            }
        }
        drop(state);
        self.wake.notify_all();
    }

    /// The token to poll with, while there is a reason to poll; None ends the thread.
    fn should_poll(&self) -> Option<String> {
        let paired = self.paired_chat().is_some();
        let mut state = self.state();
        let waiting_code = state.pending.as_ref().is_some_and(|p| p.valid(Instant::now()));
        let token = state.token.clone().filter(|_| state.want && (paired || waiting_code));
        if token.is_none() {
            state.alive = false;
            state.want = false;
        }
        token
    }

    fn run(self: Arc<Self>) {
        crate::log::line("telegram: escuchando");
        let mut failures = 0u32;
        while let Some(token) = self.should_poll() {
            let started = Instant::now();
            match self.poll_once(&token) {
                Ok(count) => {
                    failures = 0;
                    let mut state = self.state();
                    if !state.error.is_empty() {
                        state.error.clear();
                        drop(state);
                        self.changed();
                    }
                    // A long poll that came back at once with nothing: never spin.
                    if count == 0 && started.elapsed() < Duration::from_secs(1) {
                        self.sleep(Duration::from_secs(1));
                    }
                }
                Err(ApiError::Unauthorized) => {
                    crate::log::line("telegram: el token ya no vale; dejo de escuchar");
                    {
                        let mut state = self.state();
                        state.error = ApiError::Unauthorized.text();
                        state.want = false;
                    }
                    self.changed();
                }
                Err(e) => {
                    failures += 1;
                    let wait = match e {
                        ApiError::RetryAfter(s) => Duration::from_secs(s).min(MAX_BACKOFF),
                        _ => backoff(failures),
                    };
                    crate::log::line(format!("telegram: {} — reintento en {} s", e.text(), wait.as_secs()));
                    if matches!(e, ApiError::Conflict) || failures == 3 {
                        self.state().error = e.text();
                        self.changed();
                    }
                    self.sleep(wait);
                }
            }
        }
        crate::log::line("telegram: dejo de escuchar");
    }

    /// Waits up to `wait`, waking early when told to stop (or for a new code).
    fn sleep(&self, wait: Duration) {
        let until = Instant::now() + wait;
        let mut state = self.state();
        while state.want {
            let now = Instant::now();
            if now >= until {
                break;
            }
            let (next, timeout) = self.wake.wait_timeout(state, until - now).unwrap_or_else(|p| p.into_inner());
            state = next;
            if !timeout.timed_out() {
                break;
            }
        }
    }

    /// One `getUpdates` and what it brought (how many updates). The offset is saved before each update is
    /// handled: a turn is never run twice, even if Buddy closes in the middle.
    pub fn poll_once(&self, token: &str) -> Result<usize, ApiError> {
        let offset: i64 = self.setting(OFFSET_KEY).parse().unwrap_or(0);
        let body = json!({ "offset": offset, "timeout": POLL_SECONDS, "allowed_updates": ["message"] });
        let result = self.api.call(token, "getUpdates", &body, Duration::from_secs(POLL_SECONDS + 15))?;
        let updates = parse_updates(&result);
        for update in &updates {
            if !self.state().want {
                break;
            }
            let _ = self.set_setting(OFFSET_KEY, &(update.update_id + 1).to_string());
            self.handle(token, update);
        }
        Ok(updates.len())
    }

    fn handle(&self, token: &str, update: &Update) {
        let paired = self.paired_chat();
        let decision = decide(paired, &mut self.state().pending, Instant::now(), update);
        match decision {
            Decision::Ignore(why) => crate::log::line(format!("telegram: mensaje ignorado ({why})")),
            Decision::Pair(chat) => {
                if self.set_setting(CHAT_KEY, &chat.to_string()).is_ok() {
                    crate::log::line("telegram: chat vinculado");
                    let _ = self.send_text(token, chat, PAIRED_TEXT);
                    self.changed();
                }
            }
            Decision::Reply(chat, text) => {
                let _ = self.send_text(token, chat, text);
            }
            Decision::Message(chat, text) => {
                let typing = self.typing(token, chat);
                let answer = (self.handler)(&text);
                drop(typing);
                // Disconnected while PARLEY worked: the answer stays in the history only.
                if !self.state().want {
                    return;
                }
                let text = match answer {
                    Ok(answer) if !answer.trim().is_empty() => answer,
                    Ok(_) => "PARLEY no devolvió texto.".into(),
                    Err(e) => format!("No pude responder: {e}"),
                };
                for piece in split_message(&to_plain(&text), MAX_MESSAGE) {
                    if let Err(e) = self.send_text(token, chat, &piece) {
                        crate::log::line(format!("telegram: no se pudo enviar la respuesta ({})", e.text()));
                        break;
                    }
                }
            }
        }
    }

    /// «escribiendo…» every 4 s until the returned sender is dropped (Telegram shows it for ~5 s).
    fn typing(&self, token: &str, chat: i64) -> std::sync::mpsc::Sender<()> {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let (api, token) = (self.api.clone(), token.to_string());
        let _ = std::thread::Builder::new().name("buddy-telegram-typing".into()).spawn(move || {
            let body = json!({ "chat_id": chat, "action": "typing" });
            loop {
                let _ = api.call(&token, "sendChatAction", &body, Duration::from_secs(10));
                if !matches!(rx.recv_timeout(Duration::from_secs(4)), Err(std::sync::mpsc::RecvTimeoutError::Timeout)) {
                    break;
                }
            }
        });
        tx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "123456789:AAFakeFakeFakeFakeFakeFakeFakeFake_-x";

    fn msg(update_id: i64, chat: i64, text: &str) -> Update {
        Update { update_id, chat_id: Some(chat), private: true, text: Some(text.into()) }
    }

    #[test]
    fn updates_are_parsed_and_odd_ones_kept_harmless() {
        let result = json!([
            { "update_id": 10, "message": { "message_id": 1, "chat": { "id": 42, "type": "private" }, "text": "/start 123456" } },
            { "update_id": 11, "message": { "chat": { "id": -100, "type": "group" }, "photo": [] } },
            { "update_id": 12, "edited_message": { "chat": { "id": 42 } } },
            { "message": { "chat": { "id": 1 } } },
            "basura"
        ]);
        let updates = parse_updates(&result);
        assert_eq!(updates.len(), 3);
        assert_eq!(updates[0], msg(10, 42, "/start 123456"));
        assert_eq!(updates[1], Update { update_id: 11, chat_id: Some(-100), private: false, text: None });
        assert_eq!(updates[2].chat_id, None);
        assert!(parse_updates(&json!({})).is_empty());
    }

    #[test]
    fn the_api_envelope_maps_to_errors() {
        assert_eq!(api_result(200, json!({"ok": true, "result": {"username": "b"}})).unwrap()["username"], "b");
        assert_eq!(api_result(401, json!({"ok": false, "error_code": 401})), Err(ApiError::Unauthorized));
        assert_eq!(api_result(404, json!({"ok": false, "error_code": 404})), Err(ApiError::Unauthorized));
        assert_eq!(api_result(409, json!({"ok": false, "error_code": 409})), Err(ApiError::Conflict));
        let flood = json!({"ok": false, "error_code": 429, "parameters": {"retry_after": 17}});
        assert_eq!(api_result(429, flood), Err(ApiError::RetryAfter(17)));
        assert!(matches!(api_result(502, json!({"ok": false})), Err(ApiError::Network(_))));
        assert!(matches!(api_result(400, json!({"ok": false, "description": "chat not found"})), Err(ApiError::Refused(d)) if d == "chat not found"));
    }

    #[test]
    fn tokens_are_checked_for_shape() {
        assert!(token_looks_valid(TOKEN));
        for bad in ["", "123", "abc:AAFakeFakeFakeFakeFakeFakeFakeFake", "123:short", "123:AAFake Fake Fake Fake Fake Fake Fake"] {
            assert!(!token_looks_valid(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_wrong_code_is_ignored_and_the_right_one_pairs_once() {
        let now = Instant::now();
        let mut pending = Some(Pending { code: "123456".into(), until: now + CODE_TTL, wrong: 0 });
        assert_eq!(decide(None, &mut pending, now, &msg(1, 7, "/start 000000")), Decision::Ignore("código incorrecto"));
        assert_eq!(decide(None, &mut pending, now, &msg(2, 7, "hola")), Decision::Ignore("sin vincular"));
        assert_eq!(pending.as_ref().unwrap().wrong, 1);
        assert_eq!(decide(None, &mut pending, now, &msg(3, 7, "/start@mi_bot 123456")), Decision::Pair(7));
        assert!(pending.is_none(), "a code works once");
        assert_eq!(decide(None, &mut pending, now, &msg(4, 8, "/start 123456")), Decision::Ignore("sin código de vinculación"));
    }

    #[test]
    fn groups_expired_codes_and_too_many_tries_never_pair() {
        let now = Instant::now();
        let mut pending = Some(Pending { code: "123456".into(), until: now + CODE_TTL, wrong: 0 });
        let group = Update { private: false, ..msg(1, -5, "/start 123456") };
        assert_eq!(decide(None, &mut pending, now, &group), Decision::Ignore("no es un chat privado"));
        let later = now + CODE_TTL + Duration::from_secs(1);
        assert_eq!(decide(None, &mut pending, later, &msg(2, 7, "/start 123456")), Decision::Ignore("sin código de vinculación"));
        for _ in 0..MAX_WRONG_CODES {
            decide(None, &mut pending, now, &msg(3, 9, "/start 999999"));
        }
        assert!(pending.is_none(), "five wrong tries burn the code");
        let mut pending = Some(Pending { code: "123456".into(), until: now + CODE_TTL, wrong: 0 });
        assert_eq!(decide(None, &mut pending, now, &Update { text: None, ..msg(4, 7, "") }), Decision::Ignore("sin vincular"));
    }

    #[test]
    fn once_paired_only_that_chat_is_heard() {
        let now = Instant::now();
        let mut none = None;
        assert_eq!(decide(Some(7), &mut none, now, &msg(1, 8, "hola")), Decision::Ignore("otro chat"));
        assert_eq!(decide(Some(7), &mut none, now, &msg(2, 8, "/start 123456")), Decision::Ignore("otro chat"));
        assert_eq!(decide(Some(7), &mut none, now, &msg(3, 7, "  ¿Quién gana hoy?  ")), Decision::Message(7, "¿Quién gana hoy?".into()));
        assert_eq!(decide(Some(7), &mut none, now, &msg(4, 7, "/start")), Decision::Reply(7, ALREADY_TEXT));
        assert_eq!(decide(Some(7), &mut none, now, &Update { text: None, ..msg(5, 7, "") }), Decision::Reply(7, TEXT_ONLY));
        let long = "a".repeat(MAX_INCOMING + 50);
        assert!(matches!(decide(Some(7), &mut none, now, &msg(6, 7, &long)), Decision::Message(_, t) if t.chars().count() == MAX_INCOMING));
    }

    #[test]
    fn codes_are_six_digits() {
        for _ in 0..20 {
            let code = new_code();
            assert_eq!(code.len(), 6);
            assert!(code.chars().all(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn backoff_doubles_up_to_five_minutes() {
        let secs: Vec<u64> = (1..=11).map(|n| backoff(n).as_secs()).collect();
        assert_eq!(secs, [2, 4, 8, 16, 32, 64, 128, 256, 300, 300, 300]);
        assert_eq!(backoff(0).as_secs(), 2);
        assert_eq!(backoff(u32::MAX).as_secs(), 300);
    }

    #[test]
    fn long_text_is_split_at_natural_places() {
        assert!(split_message("   ", 10).is_empty());
        assert_eq!(split_message("hola", 10), ["hola"]);
        assert_eq!(split_message("uno dos tres cuatro", 12), ["uno dos", "tres cuatro"]);
        assert_eq!(split_message("párrafo uno\n\npárrafo dos", 16), ["párrafo uno", "párrafo dos"]);
        assert_eq!(split_message("abcdefghijkl", 5), ["abcde", "fghij", "kl"]);
        // Characters, not bytes: accents and emoji never break.
        let accents = "ñ".repeat(9000);
        let pieces = split_message(&accents, MAX_MESSAGE);
        assert_eq!(pieces.iter().map(|p| p.chars().count()).collect::<Vec<_>>(), [4000, 4000, 1000]);
        let long: String = (0..2000).map(|i| format!("palabra{i} ")).collect();
        for piece in split_message(&long, MAX_MESSAGE) {
            assert!(piece.chars().count() <= MAX_MESSAGE && !piece.ends_with(' ') && !piece.starts_with(' '));
        }
    }

    #[test]
    fn markdown_becomes_plain_text() {
        let md = "## Picks de hoy\n\n**Real Madrid** gana (confianza *media*).\n\n- Over 2.5 @ 1.80\n* `BTTS`\n\n\
| Partido | Pick |\n|---|:--:|\n| RMA-BAR | 1 |\n\n---\n> ojo\n\nFuente: [Marca](https://marca.com) y [https://x.y](https://x.y)\n\
```\ncódigo **literal**\n```";
        let plain = to_plain(md);
        assert_eq!(
            plain,
            "Picks de hoy\n\nReal Madrid gana (confianza *media*).\n\n• Over 2.5 @ 1.80\n• BTTS\n\nPartido · Pick\nRMA-BAR · 1\n\n\
ojo\n\nFuente: Marca (https://marca.com) y https://x.y\ncódigo **literal**"
        );
        assert_eq!(to_plain("#hashtag y [corchetes] sueltos ["), "#hashtag y [corchetes] sueltos [");
        assert_eq!(to_plain("x\n\n\n\n\ny"), "x\n\ny");
    }

    // ── The service, with a scripted Bot API ──

    #[derive(Default)]
    struct FakeApi {
        replies: Mutex<Vec<Result<Value, ApiError>>>,
        calls: Mutex<Vec<(String, Value)>>,
    }

    impl Api for FakeApi {
        fn call(&self, token: &str, method: &str, body: &Value, _: Duration) -> Result<Value, ApiError> {
            assert_eq!(token, TOKEN);
            self.calls.lock().unwrap().push((method.into(), body.clone()));
            match method {
                "getMe" => Ok(json!({ "id": 1, "is_bot": true, "username": "buddy_test_bot" })),
                "getUpdates" => {
                    let mut replies = self.replies.lock().unwrap();
                    if replies.is_empty() { Ok(json!([])) } else { replies.remove(0) }
                }
                _ => Ok(json!(true)),
            }
        }
    }

    impl FakeApi {
        fn sent(&self) -> Vec<(i64, String)> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(m, _)| m == "sendMessage")
                .map(|(_, b)| (b["chat_id"].as_i64().unwrap(), b["text"].as_str().unwrap().to_string()))
                .collect()
        }
    }

    struct Vault(Mutex<Option<String>>);
    impl Secrets for Vault {
        fn get(&self) -> Option<String> {
            self.0.lock().unwrap().clone()
        }
        fn set(&self, s: &str) -> Result<(), String> {
            *self.0.lock().unwrap() = Some(s.into());
            Ok(())
        }
        fn delete(&self) {
            *self.0.lock().unwrap() = None;
        }
    }

    fn service(threads: bool, handler: Handler) -> (Arc<Telegram>, Arc<FakeApi>, std::sync::mpsc::Receiver<Event>) {
        let api = Arc::new(FakeApi::default());
        let bus = Arc::new(EventBus::default());
        let rx = bus.subscribe();
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let mut telegram = Telegram::new(store, bus, api.clone(), Box::new(Vault(Mutex::new(None))), handler);
        telegram.threads = threads;
        (Arc::new(telegram), api, rx)
    }

    fn updates(list: &[(i64, i64, &str)]) -> Result<Value, ApiError> {
        Ok(Value::Array(
            list.iter()
                .map(|(id, chat, text)| json!({ "update_id": id, "message": { "chat": { "id": chat, "type": "private" }, "text": text } }))
                .collect(),
        ))
    }

    #[test]
    fn connect_pair_talk_and_disconnect_without_a_network() {
        let asked = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen = asked.clone();
        let (telegram, api, rx) = service(false, Box::new(move |text| {
            seen.lock().unwrap().push(text.to_string());
            Ok("**Pick:** over 2.5".into())
        }));
        assert!(telegram.connect("no-es-un-token").is_err());
        assert!(api.calls.lock().unwrap().is_empty(), "a malformed token never goes out");
        assert!(!telegram.status().connected);

        telegram.connect(TOKEN).unwrap();
        assert_eq!(rx.try_recv().unwrap(), Event::TelegramChanged);
        let status = telegram.status();
        assert!(status.connected && !status.paired);
        assert_eq!(status.bot_name, "@buddy_test_bot");
        assert_eq!(telegram.secrets.get().as_deref(), Some(TOKEN), "the token lives in the vault");
        let code = status.pairing_code.clone();
        assert_eq!(code.len(), 6);

        let wrong = if code == "000000" { "111111" } else { "000000" };
        api.replies.lock().unwrap().push(updates(&[(5, 77, &format!("/start {wrong}")), (6, 66, "hola")]));
        telegram.poll_once(TOKEN).unwrap();
        assert!(!telegram.status().paired);
        assert!(api.sent().is_empty(), "strangers get no reply");

        api.replies.lock().unwrap().push(updates(&[(7, 77, &format!("/start {code}")), (8, 66, &format!("/start {code}"))]));
        telegram.poll_once(TOKEN).unwrap();
        let status = telegram.status();
        assert!(status.paired && status.pairing_code.is_empty());
        assert_eq!(api.sent(), [(77, PAIRED_TEXT.to_string())]);
        assert_eq!(telegram.setting(OFFSET_KEY), "9");

        api.replies.lock().unwrap().push(updates(&[(9, 66, "dame un pick"), (10, 77, "dame un pick")]));
        telegram.poll_once(TOKEN).unwrap();
        assert_eq!(*asked.lock().unwrap(), ["dame un pick"], "only the paired chat reaches PARLEY");
        assert_eq!(api.sent().last().unwrap(), &(77, "Pick: over 2.5".to_string()));
        let offsets: Vec<i64> = api
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == "getUpdates")
            .map(|(_, b)| b["offset"].as_i64().unwrap())
            .collect();
        assert_eq!(offsets, [0, 7, 9]);

        telegram.send(&"línea\n".repeat(1500)).unwrap();
        assert!(api.sent().len() >= 4, "a long send is split");

        telegram.disconnect().unwrap();
        let status = telegram.status();
        assert!(!status.connected && !status.paired && status.pairing_code.is_empty());
        assert!(telegram.secrets.get().is_none());
        assert!(telegram.send("hola").is_err());
    }

    #[test]
    fn a_failed_turn_is_told_and_a_revoked_token_stops_the_poller() {
        let (telegram, api, _rx) = service(true, Box::new(|_| Err("Claude no tiene uso.".into())));
        telegram.set_setting(BOT_KEY, "@b").unwrap();
        telegram.set_setting(CHAT_KEY, "77").unwrap();
        telegram.secrets.set(TOKEN).unwrap();
        api.replies.lock().unwrap().push(updates(&[(1, 77, "hola")]));
        telegram.state().want = true;
        telegram.poll_once(TOKEN).unwrap();
        assert_eq!(api.sent(), [(77, "No pude responder: Claude no tiene uso.".to_string())]);

        api.replies.lock().unwrap().push(Err(ApiError::Unauthorized));
        telegram.start_if_configured();
        for _ in 0..200 {
            if !telegram.state().alive {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let state = telegram.state();
        assert!(!state.alive && !state.want, "the thread ended");
        assert!(state.error.contains("token"));
    }

    /// Reaches Telegram over HTTPS with a made-up token: `cargo test -p buddy-core telegram -- --ignored`.
    #[test]
    #[ignore]
    fn live_telegram_refuses_a_made_up_token() {
        let err = HttpApi.call("123456:AAinventadoinventadoinventadoinvent", "getMe", &json!({}), Duration::from_secs(15));
        assert_eq!(err, Err(ApiError::Unauthorized));
    }
}
