//! Each named agent's chat over the user's subscriptions, through the official CLIs. The CLIs own authentication:
//! MIKA never reads or copies OAuth tokens. Claude runs one `claude -p` per turn and resumes its session; Codex keeps
//! one `codex app-server` per conversation. MIKA can pass a request to another agent (handoff.rs).
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::time::{Duration, Instant};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use crate::platform::shell::find_on_path;
use super::agent_gate::{self, CommandRun};
use super::agent_tools::{self, Caps, ClaudeArgs};
use super::attachments::{self, Kind};
use super::chat_frames::{self, Frame, FrameParser};
use super::chat_store::{self, Conversation, HistoryEntry};
use super::codex_server::{CodexServer, GateCtx, ThreadOptions};
use super::handoff;
use super::named_agents::{self, AgentDefinition, ModelChoice};

/// The longest a turn may run (an image takes about a minute).
const TURN_LIMIT: Duration = Duration::from_secs(600);
/// After Stop, how long the provider gets to close the turn cleanly before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(3);
const MAX_MESSAGES: usize = 200;

#[derive(Default)]
pub struct SubscriptionChat {
    /// One answer at a time per agent chat.
    running: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// Live Codex conversations, by runner key (the agent id, or `handoff-<id>` for MIKA's hand-offs).
    codex: tokio::sync::Mutex<HashMap<String, Arc<CodexServer>>>,
    /// Claude sessions to resume, by runner key.
    claude_sessions: Mutex<HashMap<String, String>>,
    /// Hash of the instructions each live Codex conversation was started with: Codex reads them once, so when the
    /// agent's instructions, skills or name change the conversation is restarted (the history seeds the new one).
    codex_system: Mutex<HashMap<String, u64>>,
}

fn system_hash(system: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    system.hash(&mut hasher);
    hasher.finish()
}

/// A page an answer used. Shown as a small icon under the answer.
#[derive(Clone, Serialize, Deserialize)]
pub struct Source { pub title: String, pub url: String }

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub role: String,
    pub content: String,
    /// Chats saved before sources existed have none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<Source>,
    /// An image the agent created (MIRO).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_path: Option<String>,
    /// In MIKA's chat: the agent that answered for her. None is MIKA herself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// What MIKA asked that agent to do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff_task: Option<String>,
    /// On a user message: the name of the file attached to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment: Option<String>,
    /// On an answer: the commands the user allowed and what they printed (cut short).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<CommandRun>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus { pub provider: String, pub installed: bool, pub connected: bool, pub detail: String }

/// An agent as the island shows it: its definition and the model line of each provider it can use.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    #[serde(flatten)]
    pub agent: AgentDefinition,
    pub models: HashMap<String, String>,
    /// The shipped name, colour and hat of a built-in agent (what Reset restores); None for agents the user added.
    pub defaults: Option<named_agents::Look>,
    /// Capabilities this agent can never have (the settings page shows those switches off), and why.
    pub blocked: Vec<String>,
    pub blocked_reason: Option<String>,
}

/// Events of a running answer, sent to the island as `chat-progress`. `agent` is the chat; `speaker` is who is
/// talking in it (another agent during a hand-off). `event`: `delta` (text), `tool` (title: tool, text: detail),
/// `source` (title, url), `command` (title: the command, text: its output, code), `progress`, `error`, `image-start`, `image` (text: path), `image-failed` (text: why),
/// `handoff-thinking`, `handoff` (title: agent id, text: task).
#[derive(Clone, Serialize)]
pub struct Progress {
    pub agent: String,
    pub speaker: String,
    pub event: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// `command`: the exit code, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<i32>,
}

pub fn provider(provider: &str) -> Result<&'static str, String> {
    match provider { "codex" => Ok("codex"), "claude" => Ok("claude"), _ => Err("Proveedor no válido.".into()) }
}

pub fn cli_path(provider: &str) -> Option<PathBuf> {
    if let Some(path) = find_on_path(provider) { return Some(path); }
    let home = PathBuf::from(std::env::var_os("USERPROFILE")?);
    let local = home.join(".local/bin").join(format!("{provider}.exe"));
    if local.is_file() { return Some(local); }
    if provider == "claude" {
        let npm = PathBuf::from(std::env::var_os("APPDATA")?).join("npm/claude.cmd");
        if npm.is_file() { return Some(npm); }
    }
    if provider == "codex" {
        let bin = crate::services::settings::local_dir().parent()?.join("OpenAI/Codex/bin");
        let mut dirs: Vec<_> = std::fs::read_dir(bin).ok()?.flatten().map(|e| e.path()).collect();
        dirs.sort();
        return dirs.into_iter().rev().map(|p| p.join("codex.exe")).find(|p| p.is_file());
    }
    None
}

fn command(provider: &str) -> Result<tokio::process::Command, String> {
    let path = cli_path(provider).ok_or_else(|| format!("{} no está instalado. Usa el enlace de instalación oficial.", if provider == "codex" { "Codex" } else { "Claude" }))?;
    let mut cmd = tokio::process::Command::new(path);
    cmd.creation_flags(0x0800_0000).kill_on_drop(true);
    // A configured API key must never silently change subscription billing.
    for key in ["OPENAI_API_KEY", "CODEX_API_KEY", "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_BASE_URL", "OPENAI_BASE_URL", "CLAUDE_CODE_OAUTH_TOKEN"] { cmd.env_remove(key); }
    Ok(cmd)
}

pub async fn status(provider_name: &str) -> Result<ProviderStatus, String> {
    let provider = provider(provider_name)?;
    let Some(_) = cli_path(provider) else { return Ok(ProviderStatus { provider: provider.into(), installed: false, connected: false, detail: "Instala el cliente oficial para usar tu suscripción.".into() }); };
    let mut cmd = command(provider)?;
    if provider == "codex" { cmd.args(["login", "status"]); } else { cmd.args(["auth", "status"]); }
    let output = tokio::time::timeout(Duration::from_secs(10), cmd.output()).await.map_err(|_| "El cliente tardó demasiado en responder.")?.map_err(|_| "No se pudo consultar el cliente.")?;
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    let connected = if provider == "codex" { output.status.success() && text.to_lowercase().contains("chatgpt") } else {
        serde_json::from_slice::<Value>(&output.stdout).ok().is_some_and(|v| v["loggedIn"] == true && v["authMethod"].as_str().is_some_and(|s| s.contains("claude") || s.contains("oauth") || s.contains("subscription")))
    };
    Ok(ProviderStatus { provider: provider.into(), installed: true, connected, detail: if connected { "Conectado con tu suscripción." } else { "Conecta tu cuenta de suscripción con el cliente oficial." }.into() })
}

pub fn login(provider_name: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let provider = provider(provider_name)?;
    let path = cli_path(provider).ok_or("Instala primero el cliente oficial.")?;
    let quoted = path.to_string_lossy().replace('\'', "''");
    let suffix = if provider == "codex" { "login" } else { "auth login" };
    // Fixed PowerShell syntax; only an OS-discovered executable path is quoted.
    let script = format!("& '{quoted}' {suffix}");
    let mut cmd = std::process::Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-NoExit", "-Command", &script]).creation_flags(0x0000_0010);
    for key in ["OPENAI_API_KEY", "CODEX_API_KEY", "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"] { cmd.env_remove(key); }
    cmd.spawn().map_err(|_| "No se pudo abrir el inicio de sesión.")?;
    Ok(())
}

pub fn agents() -> Vec<AgentInfo> {
    named_agents::load_all().into_iter().map(|agent| {
        let models = agent.providers.iter().map(|p| (p.clone(), named_agents::model_for(&agent, p).display)).collect();
        let defaults = named_agents::default_look(&agent.id);
        let blocked: Vec<String> = named_agents::forbidden_caps(&agent.id, agent.integration.as_deref()).iter().map(|c| c.to_string()).collect();
        let blocked_reason = (!blocked.is_empty()).then(|| named_agents::FORBIDDEN_REASON.to_string());
        AgentInfo { agent, models, defaults, blocked, blocked_reason }
    }).collect()
}

fn agent(id: &str) -> Result<AgentDefinition, String> {
    if !named_agents::valid_id(id) { return Err("Agente no válido.".into()); }
    named_agents::find(id).ok_or_else(|| "Ese agente no existe.".into())
}

pub fn history(agent_id: &str) -> Result<Vec<Message>, String> {
    let path = named_agents::chat_file(&agent(agent_id)?.id);
    if !path.exists() { return Ok(Vec::new()); }
    let bytes = std::fs::read(&path).map_err(|_| "No se pudo leer el chat guardado.")?;
    match (bytes.len() <= 4_000_000).then(|| serde_json::from_slice::<Vec<Message>>(&bytes).ok()).flatten() {
        Some(messages) => Ok(messages),
        None => {
            // A damaged or oversized file is set aside instead of blocking the chat.
            let _ = std::fs::rename(&path, path.with_extension("json.corrupt"));
            Ok(Vec::new())
        }
    }
}

fn save(agent_id: &str, history: &[Message]) -> Result<(), String> {
    let path = named_agents::chat_file(agent_id);
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|_| "No se pudo crear la carpeta del chat.")?;
    let start = history.len().saturating_sub(MAX_MESSAGES);
    let bytes = serde_json::to_vec(&history[start..]).map_err(|_| "No se pudo guardar el chat.")?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, &path)).map_err(|_| "No se pudo guardar el chat.".into())
}

/// Only images the agents created are ever read back or opened: Codex's own folder or the agents' folders.
fn allowed_image(path: &str) -> Result<(PathBuf, &'static str), String> {
    let file = Path::new(path);
    let ext = file.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    let mime = match ext.as_str() { "png" => "image/png", "jpg" | "jpeg" => "image/jpeg", "webp" => "image/webp", "gif" => "image/gif", _ => return Err("Ese archivo no es una imagen.".into()) };
    if !file.is_absolute() { return Err("Imagen fuera de las carpetas permitidas.".into()); }
    let real = file.canonicalize().map_err(|_| "La imagen ya no existe.")?;
    let codex_home = std::env::var_os("CODEX_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(".codex")));
    let roots: Vec<PathBuf> = [codex_home.map(|h| h.join("generated_images")), Some(named_agents::root())].into_iter().flatten()
        .filter_map(|r| r.canonicalize().ok()).collect();
    if !roots.iter().any(|root| real.starts_with(root)) { return Err("Imagen fuera de las carpetas permitidas.".into()); }
    // What Telegram channels posted is never shown or opened by MIKA: only the agent's model reads it.
    let telegram: Vec<PathBuf> = crate::integrations::telegram::agent_dirs().into_iter().filter_map(|d| d.canonicalize().ok()).collect();
    if telegram.iter().any(|dir| real.starts_with(dir)) || real.components().any(|c| c.as_os_str() == "telegram") {
        return Err("Imagen fuera de las carpetas permitidas.".into());
    }
    Ok((real, mime))
}

pub fn image_data_url(path: &str) -> Result<String, String> {
    let (real, mime) = allowed_image(path)?;
    let bytes = std::fs::read(&real).map_err(|_| "No se pudo leer la imagen.")?;
    if bytes.len() > 25_000_000 { return Err("La imagen es demasiado grande.".into()); }
    Ok(format!("data:{mime};base64,{}", super::base64(&bytes)))
}

/// "Abrir" and "Mostrar en la carpeta" under an image.
pub fn open_image(path: &str, reveal: bool) -> Result<(), String> {
    let (real, _) = allowed_image(path)?;
    crate::platform::shell::open_local(&real, reveal)
}

/// Where a turn's events go and what it collected.
struct Sink {
    app: AppHandle,
    chat: String,
    speaker: String,
    /// False while the text is held back: MIKA's turns hold it while it may still be a hand-off line.
    released: bool,
    thinking_sent: bool,
    text: String,
    held: String,
    fallback: String,
    sources: Vec<Source>,
    commands: Vec<CommandRun>,
    image: Option<String>,
    error: Option<String>,
    session: Option<String>,
    done: bool,
    /// The turn was started with MIKA's hand-off hold (see `released`); a retry starts from the same state.
    hold: bool,
    /// While a fallback model is still possible a provider error is kept, not shown: only a no-credits error is
    /// answered with the retry, any other one is shown afterwards as it always was.
    hold_errors: bool,
    held_error: Option<String>,
}

impl Sink {
    fn new(app: &AppHandle, chat: &str, speaker: &str, hold: bool) -> Self {
        Sink { app: app.clone(), chat: chat.into(), speaker: speaker.into(), released: !hold, thinking_sent: false,
               text: String::new(), held: String::new(), fallback: String::new(), sources: Vec::new(), commands: Vec::new(), image: None,
               error: None, session: None, done: false, hold, hold_errors: false, held_error: None }
    }

    /// Forgets what the failed first attempt collected, so the retry on the fallback model starts clean.
    fn reset(&mut self) {
        self.released = !self.hold;
        self.thinking_sent = false;
        self.text.clear();
        self.held.clear();
        self.fallback.clear();
        self.sources.clear();
        self.commands.clear();
        self.image = None;
        self.error = None;
        self.session = None;
        self.done = false;
        self.held_error = None;
    }

    fn emit(&self, event: &str, text: String, title: Option<String>, url: Option<String>) {
        let _ = self.app.emit("chat-progress", Progress { agent: self.chat.clone(), speaker: self.speaker.clone(), event: event.into(), text, title, url, code: None });
    }

    fn frame(&mut self, frame: Frame) {
        match frame {
            Frame::Text(text) => {
                self.text.push_str(&text);
                if self.released { self.emit("delta", text, None, None); return; }
                self.held.push_str(&text);
                if handoff::pending(&self.text) {
                    if !self.thinking_sent && self.text.trim_start().starts_with("[[") {
                        self.thinking_sent = true;
                        self.emit("handoff-thinking", String::new(), None, None);
                    }
                } else {
                    self.release();
                }
            }
            Frame::Tool { name, detail } => self.emit("tool", detail, Some(name), None),
            Frame::Source { title, url } => {
                if self.sources.len() < 12 && !self.sources.iter().any(|s| s.url == url) {
                    self.sources.push(Source { title: title.clone(), url: url.clone() });
                    self.emit("source", title.clone(), Some(title), Some(url));
                }
            }
            Frame::Progress(text) => self.emit("progress", text, None, None),
            Frame::Command { command, output, code } => {
                // Shown collapsed under the answer: few, and short.
                let run = CommandRun { command: agent_gate::clip(&command, 600), output: agent_gate::clip(&output, agent_gate::MAX_CHAT_OUTPUT), code };
                if self.commands.len() < 8 {
                    let _ = self.app.emit("chat-progress", Progress { agent: self.chat.clone(), speaker: self.speaker.clone(), event: "command".into(), text: run.output.clone(), title: Some(run.command.clone()), url: None, code });
                    self.commands.push(run);
                }
            }
            Frame::Error(text) => {
                self.error = Some(text.clone());
                if self.hold_errors { self.held_error = Some(text); } else { self.emit("error", text, None, None); }
            }
            Frame::Final(text) => self.fallback = text,
            Frame::Session(id) => self.session = Some(id),
            Frame::ImageStarted => self.emit("image-start", String::new(), None, None),
            Frame::Image(path) => { self.image = Some(path.clone()); self.emit("image", path, None, None); }
            Frame::ImageFailed(why) => self.emit("image-failed", why, None, None),
            Frame::Done => self.done = true,
        }
    }

    /// Shows whatever was held back: it was not a hand-off after all.
    fn release(&mut self) {
        self.released = true;
        let held = std::mem::take(&mut self.held);
        if !held.is_empty() { self.emit("delta", held, None, None); }
    }

    fn answer(&self) -> String { if self.text.trim().is_empty() { self.fallback.clone() } else { self.text.clone() } }
}

/// What a turn asks of a provider.
struct Turn<'a> {
    key: String,
    agent: &'a AgentDefinition,
    provider: &'static str,
    /// The model of this attempt (locked per agent: see `named_agents::model_for`).
    model: ModelChoice,
    system: String,
    prompt: String,
    /// The prompt when the turn is retried on the fallback model, whose session is new: it carries the recent
    /// conversation as data. None: the same prompt.
    fallback_prompt: Option<String>,
    utc_offset: i64,
    /// Pictures that go with the prompt (PARLEY's Telegram screenshots). Codex gets them attached; Claude gets their
    /// paths in the prompt and opens them with its Read tool.
    images: Vec<PathBuf>,
}

enum Outcome { Answered, Stopped }

impl SubscriptionChat {
    pub fn cancel(&self, agent_id: &str) {
        if let Some(cancel) = self.running.lock().unwrap().get(agent_id) { cancel.store(true, Ordering::Relaxed); }
    }

    /// Ends the provider sessions of an agent's chat (and MIKA's hand-offs): the next turn starts fresh.
    async fn end_sessions(&self, agent: &AgentDefinition) -> Result<(), String> {
        if self.running.lock().unwrap().contains_key(&agent.id) { return Err("Detén la respuesta antes de cambiar de conversación.".into()); }
        let belongs = |key: &str| { let key = key.strip_prefix(FALLBACK_KEY).unwrap_or(key); key == agent.id || (agent.id == "mika" && key.starts_with("handoff-")) };
        self.claude_sessions.lock().unwrap().retain(|k, _| !belongs(k));
        let mut codex = self.codex.lock().await;
        codex.retain(|k, server| { if belongs(k) { server.shutdown(); false } else { true } });
        Ok(())
    }

    /// "Nuevo chat": the open conversation goes to the History (and its gist to the agent's memory).
    pub async fn new_chat(&self, agent_id: &str, utc_offset: i64) -> Result<(), String> {
        let agent = agent(agent_id)?;
        self.end_sessions(&agent).await?;
        let messages = history(&agent.id)?;
        if let Some(archived) = chat_store::archive(&agent.id, &messages) { remember(agent.clone(), archived, utc_offset); }
        save(&agent.id, &[])
    }

    /// Opens a conversation from the History in its agent's chat; the one that was open there is archived.
    pub async fn open_conversation(&self, agent_id: &str, id: &str, utc_offset: i64) -> Result<Vec<Message>, String> {
        let agent = agent(agent_id)?;
        if id == "open" { return history(&agent.id); }
        self.end_sessions(&agent).await?;
        if !chat_store::exists(&agent.id, id) { return Err("Esa conversación ya no existe.".into()); }
        // The open one is archived first, with its own start time; only then does the wanted one take its place.
        let current = history(&agent.id)?;
        if let Some(archived) = chat_store::archive(&agent.id, &current) { remember(agent.clone(), archived, utc_offset); }
        let wanted = chat_store::take(&agent.id, id)?;
        chat_store::set_meta(&agent.id, chat_store::Meta { created: wanted.created, memorized: wanted.memorized });
        save(&agent.id, &wanted.messages)?;
        Ok(wanted.messages)
    }

    pub fn shutdown_now(&self) {
        if let Ok(mut servers) = self.codex.try_lock() { for (_, server) in servers.drain() { server.shutdown(); } }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn send(&self, app: AppHandle, agent_id: String, provider_name: String, query: String, today: Option<String>, utc_offset: Option<i64>, attachment: Option<String>) -> Result<Message, String> {
        let agent = agent(&agent_id)?;
        let provider = provider(&provider_name)?;
        // The models are locked: only MIKA chooses her provider; everyone else runs on Codex whatever the window asks.
        let provider = if agent.id == named_agents::ORCHESTRATOR {
            if !agent.providers.iter().any(|p| p == provider) { return Err(format!("{} no trabaja con ese proveedor.", agent.name)); }
            provider
        } else { "codex" };
        if query.trim().is_empty() || query.len() > 24_000 { return Err("El prompt debe tener entre 1 y 24,000 bytes.".into()); }
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut running = self.running.lock().unwrap();
            if running.contains_key(&agent.id) { return Err("Ya hay una respuesta en curso en este chat.".into()); }
            running.insert(agent.id.clone(), cancel.clone());
        }
        let result = self.chat(&app, &agent, provider, query, today, utc_offset.unwrap_or(0), attachment, &cancel).await;
        self.running.lock().unwrap().remove(&agent.id);
        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn chat(&self, app: &AppHandle, agent: &AgentDefinition, provider: &'static str, query: String, today: Option<String>, utc_offset: i64, attachment: Option<String>, cancel: &Arc<AtomicBool>) -> Result<Message, String> {
        // The attachment first: what it is and whether this agent may take it decide which provider reads it.
        let workspace = named_agents::workspace(&agent.id);
        let attached = match attachment.as_deref().filter(|p| !p.trim().is_empty()) {
            Some(path) => Some(attachments::prepare(agent, &workspace, path)?),
            None => None,
        };
        // Codex cannot read a PDF. A PDF turn of an agent that may read PDFs runs on Claude (Sonnet), the model every
        // agent already falls back to; without Claude connected it says so instead of pretending.
        let pdf_on_claude = attached.as_ref().is_some_and(|a| a.kind == Kind::Pdf) && provider == "codex";
        if pdf_on_claude && !status("claude").await.is_ok_and(|s| s.connected) {
            return Err(format!("{} lee los PDF con Claude y Claude no está conectado. Conéctalo en Ajustes o pásale el PDF a MIKA.", agent.name));
        }
        if !pdf_on_claude && !status(provider).await?.connected { return Err("Conecta primero tu suscripción con el botón Conectar.".into()); }
        let mut messages = history(&agent.id)?;
        let orchestrates = agent.id == "mika";
        let team = if orchestrates { named_agents::load_all() } else { Vec::new() };

        let mut system = super::agent_skills::system_prompt(agent);
        if agent.integration.as_deref() == Some(crate::integrations::telegram::AGENT_INTEGRATION) {
            system.push_str(&super::picks::manual_instructions());
        }
        if orchestrates { system.push_str(&handoff::roster_prompt(&team)); }
        system.push_str(&chat_store::memory_prompt(&agent.id));
        // Instructions that changed since the Codex conversation began: start it again (a new one is "fresh").
        if provider == "codex" { self.sync_codex_prompt(&agent.id, &system, agent).await; }
        let fresh = !self.has_session(&agent.id, provider).await;

        let mut prompt = String::new();
        if let Some(note) = chat_frames::today_note(today.as_deref()) { prompt.push_str(&format!("[Nota de MIKA, no del usuario] {note}\n\n")); }
        if orchestrates {
            if let Some(last) = messages.last().filter(|m| m.role == "assistant") {
                if let Some(name) = last.agent.as_deref().and_then(|id| team.iter().find(|a| a.id == id)).map(|a| a.name.clone()) {
                    prompt.push_str(&handoff::followup_note(&name, &last.content));
                }
            }
        }
        // An agent fed by Telegram (PARLEY) gets the user's rules and the posts it has not seen yet, as data.
        let mut images = Vec::new();
        if agent.integration.as_deref() == Some(crate::integrations::telegram::AGENT_INTEGRATION) {
            let (note, pictures) = super::picks::chat_note(app, &agent.id, fresh);
            prompt.push_str(&note);
            images = pictures;
            if let Some(odds) = super::picks::chat_odds(app, &query).await { prompt.push_str(&odds); }
        }
        // The file, for the provider that will read it; images also go to Codex as image inputs.
        let reader = if pdf_on_claude { "claude" } else { provider };
        if let Some(a) = &attached {
            prompt.push_str(&attachments::note(a, reader));
            if a.kind == Kind::Image && reader == "codex" { images.push(a.path.clone()); }
        }
        // The retry on the fallback model starts a new session, so it always carries the recent conversation (and the
        // file as Claude opens it).
        let claude_note = attached.as_ref().map(|a| attachments::note(a, "claude")).unwrap_or_default();
        let fallback_prompt = (!messages.is_empty() || attached.is_some()).then(|| {
            let base = match &attached { Some(a) => prompt.replacen(&attachments::note(a, reader), &claude_note, 1), None => prompt.clone() };
            if messages.is_empty() { format!("{base}{query}") } else { format!("{base}{}", seed(&messages, &query)) }
        });
        if (fresh || pdf_on_claude) && !messages.is_empty() { prompt.push_str(&seed(&messages, &query)); } else { prompt.push_str(&query); }
        let (run_provider, key) = if pdf_on_claude { ("claude", format!("{FALLBACK_KEY}{}", agent.id)) } else { (provider, agent.id.clone()) };
        let model = if pdf_on_claude { named_agents::sonnet() } else { named_agents::model_for(agent, provider) };
        let turn = Turn { key: key.clone(), agent, provider: run_provider, model, system, prompt, fallback_prompt, utc_offset, images };
        let mut sink = Sink::new(app, &agent.id, &agent.id, orchestrates);
        if pdf_on_claude { sink.emit("fallback", format!("Leo el PDF con Claude ({}): Codex no lee PDF", named_agents::sonnet().display), None, None); }
        let outcome = self.run(&turn, &mut sink, cancel).await;
        // A PDF turn on Claude stands alone: its session is not the chat's.
        if pdf_on_claude { self.claude_sessions.lock().unwrap().remove(&key); }
        let outcome = outcome?;

        let mut reply = Message { role: "assistant".into(), content: sink.answer(), sources: sink.sources.clone(), image_path: sink.image.clone(), commands: sink.commands.clone(), ..Default::default() };
        if orchestrates && !sink.released {
            let known: Vec<&str> = team.iter().map(|a| a.id.as_str()).collect();
            match (matches!(outcome, Outcome::Answered)).then(|| handoff::parse(&sink.text, &known)).flatten() {
                Some((target_id, task)) => {
                    let target = team.iter().find(|a| a.id == target_id).cloned().ok_or("Ese agente no existe.")?;
                    sink.emit("handoff", task.clone(), Some(target.id.clone()), None);
                    reply = self.hand_off(app, &target, &task, &query, utc_offset, today.as_deref(), attachment.as_deref(), cancel).await?;
                    reply.handoff_task = Some(if task.is_empty() { query.clone() } else { task });
                }
                None if matches!(outcome, Outcome::Answered) => sink.release(),
                // Stopped while it still looked like a hand-off line: nothing to keep.
                None => reply.content.clear(),
            }
        }
        if reply.content.trim().is_empty() && reply.image_path.is_none() {
            if matches!(outcome, Outcome::Stopped) { return Err("Respuesta detenida.".into()); }
            return Err("El cliente no devolvió una respuesta. Revisa su conexión o inicia sesión de nuevo.".into());
        }
        messages.push(Message { role: "user".into(), content: query, attachment: attached.as_ref().map(|a| a.name.clone()), ..Default::default() });
        messages.push(reply.clone());
        save(&agent.id, &messages)?;
        Ok(reply)
    }

    /// The other agent does the task with its own prompt, provider, model and permissions, in its own session.
    #[allow(clippy::too_many_arguments)]
    async fn hand_off(&self, app: &AppHandle, target: &AgentDefinition, task: &str, question: &str, utc_offset: i64, today: Option<&str>, attachment: Option<&str>, cancel: &Arc<AtomicBool>) -> Result<Message, String> {
        let mut provider = provider(&target.provider)?;
        if !status(provider).await?.connected {
            for other in &target.providers {
                let other = super::subscription::provider(other)?;
                if status(other).await.is_ok_and(|s| s.connected) { provider = other; break; }
            }
        }
        let mut prompt = String::new();
        if let Some(note) = chat_frames::today_note(today) { prompt.push_str(&format!("[Nota de MIKA, no del usuario] {note}\n\n")); }
        let mut images = Vec::new();
        if target.integration.as_deref() == Some(crate::integrations::telegram::AGENT_INTEGRATION) {
            let (note, pictures) = super::picks::chat_note(app, &target.id, true);
            prompt.push_str(&note);
            images = pictures;
            if let Some(odds) = super::picks::chat_odds(app, if task.is_empty() { question } else { task }).await { prompt.push_str(&odds); }
        }
        // The file the user attached goes along when the target may take it; otherwise it is told why not.
        let mut model_override = None;
        if let Some(source) = attachment {
            match attachments::prepare(target, &named_agents::workspace(&target.id), source) {
                Ok(a) if a.kind == Kind::Pdf && provider == "codex" => {
                    if status("claude").await.is_ok_and(|s| s.connected) {
                        prompt.push_str(&attachments::note(&a, "claude"));
                        model_override = Some(("claude", named_agents::sonnet()));
                    } else {
                        prompt.push_str("[Nota de MIKA, no del usuario] El usuario adjuntó un PDF y Claude no está conectado para leerlo: dile que lo conecte en Ajustes.

");
                    }
                }
                Ok(a) => {
                    prompt.push_str(&attachments::note(&a, provider));
                    if a.kind == Kind::Image && provider == "codex" { images.push(a.path.clone()); }
                }
                Err(why) => prompt.push_str(&format!("[Nota de MIKA, no del usuario] El usuario adjuntó un archivo que no puedes abrir ({why}). Díselo.

")),
            }
        }
        prompt.push_str(&handoff::task_prompt(&target.name, task, question));
        let manual = if target.integration.as_deref() == Some(crate::integrations::telegram::AGENT_INTEGRATION) { super::picks::manual_instructions() } else { String::new() };
        let system = format!("{}{manual}{}", super::agent_skills::system_prompt(target), chat_store::memory_prompt(&target.id));
        let key = format!("handoff-{}", target.id);
        // Codex reads images from the turn only; a hand-off conversation keeps its own server.
        let (run_provider, model, key) = match model_override {
            Some((p, m)) => (p, m, format!("{FALLBACK_KEY}{key}")),
            None => (provider, named_agents::model_for(target, provider), key),
        };
        let turn = Turn { key: key.clone(), agent: target, provider: run_provider, model, system, prompt, fallback_prompt: None, utc_offset, images };
        let mut sink = Sink::new(app, "mika", &target.id, false);
        let outcome = self.run(&turn, &mut sink, cancel).await;
        if run_provider == "claude" && key.starts_with(FALLBACK_KEY) { self.claude_sessions.lock().unwrap().remove(&key); }
        let outcome = outcome?;
        if matches!(outcome, Outcome::Stopped) && sink.answer().trim().is_empty() && sink.image.is_none() { return Err("Respuesta detenida.".into()); }
        // The agent that did the work remembers it too, even though the conversation is MIKA's.
        let task_line: String = (if task.is_empty() { question } else { task }).lines().next().unwrap_or("").chars().take(150).collect();
        chat_store::append_memory(&target.id, &format!("- {}: MIKA te pasó: {task_line}", chat_store::date_label(chat_store::now_ms(), utc_offset)));
        Ok(Message { role: "assistant".into(), content: sink.answer(), sources: sink.sources, image_path: sink.image, agent: Some(target.id.clone()), handoff_task: None, attachment: None, commands: sink.commands })
    }

    /// A turn MIKA starts on its own (PARLEY's reviews): the agent's prompt and subscription, a session of its own that
    /// is thrown away after, and the exchange saved in the agent's chat — `header` as the user line and the answer as
    /// `visible` makes it. Returns the raw answer. Skipped (Err) while the agent is answering in its chat.
    pub async fn automatic(&self, app: &AppHandle, agent: &AgentDefinition, header: &str, prompt: String, images: Vec<PathBuf>,
                           visible: impl FnOnce(&str) -> String) -> Result<String, String> {
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut running = self.running.lock().unwrap();
            if running.contains_key(&agent.id) { return Err("El agente está respondiendo en su chat.".into()); }
            running.insert(agent.id.clone(), cancel.clone());
        }
        let result = self.automatic_turn(app, agent, header, prompt, images, visible, &cancel).await;
        self.running.lock().unwrap().remove(&agent.id);
        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn automatic_turn(&self, app: &AppHandle, agent: &AgentDefinition, header: &str, prompt: String, images: Vec<PathBuf>,
                            visible: impl FnOnce(&str) -> String, cancel: &Arc<AtomicBool>) -> Result<String, String> {
        let mut provider = None;
        for p in std::iter::once(&agent.provider).chain(agent.providers.iter()) {
            let Ok(p) = self::provider(p) else { continue };
            if status(p).await.is_ok_and(|s| s.connected) { provider = Some(p); break; }
        }
        let provider = provider.ok_or("Conecta tu suscripción de Claude o Codex en Ajustes.")?;
        let key = format!("auto-{}", agent.id);
        let system = format!("{}{}", super::agent_skills::system_prompt(agent), chat_store::memory_prompt(&agent.id));
        let turn = Turn { key: key.clone(), agent, provider, model: named_agents::model_for(agent, provider), system, prompt, fallback_prompt: None, utc_offset: crate::platform::clock::utc_offset_minutes(), images };
        // Its events go to a chat nobody shows: the island only hears about the result.
        let mut sink = Sink::new(app, &key, &agent.id, false);
        let outcome = self.run(&turn, &mut sink, cancel).await;
        // Each review stands on its own.
        self.claude_sessions.lock().unwrap().remove(&key);
        self.claude_sessions.lock().unwrap().remove(&format!("{FALLBACK_KEY}{key}"));
        self.drop_codex(&key).await;
        outcome?;
        let raw = sink.answer();
        if raw.trim().is_empty() { return Err("El cliente no devolvió una respuesta.".into()); }
        let mut messages = history(&agent.id)?;
        messages.push(Message { role: "user".into(), content: header.to_string(), ..Default::default() });
        messages.push(Message { role: "assistant".into(), content: visible(&raw), sources: sink.sources.clone(), ..Default::default() });
        save(&agent.id, &messages)?;
        // The chat's own session never saw this exchange: the next question starts a new one, seeded from the history.
        self.claude_sessions.lock().unwrap().remove(&agent.id);
        self.drop_codex(&agent.id).await;
        Ok(raw)
    }

    async fn has_session(&self, key: &str, provider: &str) -> bool {
        if provider == "codex" { self.codex.lock().await.contains_key(key) } else { self.claude_sessions.lock().unwrap().contains_key(key) }
    }

    /// One turn. When the primary model has no credits left the turn is retried once on the agent's fallback model
    /// (Haiku 4.5 for MIKA, Sonnet 5.5 for the rest), saying so in one line; any other failure is shown as before. The
    /// fallback is for this turn only: nothing is stored and the next turn tries the primary again.
    async fn run(&self, turn: &Turn<'_>, sink: &mut Sink, cancel: &Arc<AtomicBool>) -> Result<Outcome, String> {
        let fallback = named_agents::fallback_for(&turn.agent.id);
        let may_fall_back = turn.model != fallback;
        sink.hold_errors = may_fall_back;
        let first = self.run_once(turn, sink, cancel).await;
        sink.hold_errors = false;
        let no_credits = may_fall_back && matches!(&first, Err(message) if message == USAGE_LIMIT_MESSAGE);
        if !no_credits || !status(fallback.provider).await.is_ok_and(|s| s.connected) {
            if let Some(error) = sink.held_error.take() { sink.emit("error", error, None, None); }
            return first;
        }
        sink.reset();
        sink.emit("fallback", fallback_notice(&turn.model.display, &fallback.display), None, None);
        let retry = Turn {
            key: format!("{FALLBACK_KEY}{}", turn.key), agent: turn.agent, provider: fallback.provider, model: fallback,
            system: turn.system.clone(), prompt: turn.fallback_prompt.clone().unwrap_or_else(|| turn.prompt.clone()), fallback_prompt: None,
            utc_offset: turn.utc_offset, images: turn.images.clone(),
        };
        self.run_once(&retry, sink, cancel).await
    }

    async fn run_once(&self, turn: &Turn<'_>, sink: &mut Sink, cancel: &Arc<AtomicBool>) -> Result<Outcome, String> {
        // A conversation continues on the provider it started with; switching provider starts a new session.
        if turn.provider == "codex" { self.claude_sessions.lock().unwrap().remove(&turn.key); }
        else if let Some(server) = self.codex.lock().await.remove(&turn.key) { server.shutdown(); }
        let workspace = named_agents::workspace(&turn.agent.id);
        std::fs::create_dir_all(&workspace).map_err(|_| "No se pudo preparar la carpeta del agente.")?;
        let outcome = if turn.provider == "codex" { self.run_codex(turn, &workspace, sink, cancel).await } else { self.run_claude(turn, &workspace, sink, cancel).await }?;
        if let Some(error) = sink.error.clone() {
            if sink.answer().trim().is_empty() && sink.image.is_none() { return Err(user_error(&error)); }
        }
        Ok(outcome)
    }

    async fn run_claude(&self, turn: &Turn<'_>, workspace: &Path, sink: &mut Sink, cancel: &Arc<AtomicBool>) -> Result<Outcome, String> {
        let model = &turn.model;
        // The tools come from the agent's capabilities. A command tool exists only for `run`, only with the relay
        // that asks the user, and every use of it goes through the gate (agent_gate.rs).
        let relay = agent_gate::relay_exe();
        let caps = Caps::of(turn.agent).with_relay(relay.is_some());
        let gate = if caps.run { agent_gate::open_turn(turn.agent, workspace, cancel.clone()) } else { None };
        // The system prompt goes in a file: on Windows `claude` may be a .cmd script, and multi-line arguments
        // cannot cross cmd.exe safely. The gate's settings go in a file for the same reason.
        let dir = named_agents::root().join(&turn.agent.id);
        let system_file = dir.join(format!(".system-{}.md", turn.key));
        std::fs::write(&system_file, &turn.system).map_err(|_| "No se pudo preparar el agente.")?;
        let gate_file = match (&gate, &relay) {
            (Some(_), Some(exe)) => {
                let file = dir.join(format!(".gate-{}.json", turn.key));
                std::fs::write(&file, agent_tools::gate_settings_json(exe)).map_err(|_| "No se pudo preparar el agente.")?;
                Some(file)
            }
            _ => None,
        };
        let mut cmd = command("claude")?;
        cmd.current_dir(workspace);
        let session = self.claude_sessions.lock().unwrap().get(&turn.key).cloned();
        cmd.args(agent_tools::claude_args(&ClaudeArgs {
            caps, model_id: &model.id, effort: model.effort, system_file: &system_file, resume: session.as_deref(), gate_settings: gate_file.as_deref(),
        }));
        if let Some(guard) = &gate { cmd.env("MIKA_GATE_TOKEN", guard.token()); } else { cmd.env_remove("MIKA_GATE_TOKEN"); }
        cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().map_err(|_| "No se pudo iniciar el cliente de Claude.")?;
        let mut input = child.stdin.take().unwrap();
        input.write_all(turn.prompt.as_bytes()).await.map_err(|_| "No se pudo enviar el prompt.")?;
        drop(input);
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let err_task = tokio::spawn(async move { let mut bytes = Vec::new(); let _ = stderr.take(32_768).read_to_end(&mut bytes).await; bytes });
        let (lines_tx, mut lines) = tokio::sync::mpsc::unbounded_channel::<String>();
        let read_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout.take(8_000_000)).lines();
            while let Ok(Some(line)) = reader.next_line().await { if lines_tx.send(line).is_err() { break; } }
        });
        let mut parser = FrameParser::new("claude");
        let started = Instant::now();
        let mut stopped = false;
        loop {
            if cancel.load(Ordering::Relaxed) || started.elapsed() > TURN_LIMIT {
                stopped = cancel.load(Ordering::Relaxed);
                let _ = child.kill().await;
                read_task.abort();
                if !stopped { sink.error = Some("timeout".into()); }
                break;
            }
            match tokio::time::timeout(Duration::from_millis(100), lines.recv()).await {
                Ok(Some(line)) => for frame in parser.feed(&line) { sink.frame(frame); },
                Ok(None) => break,
                Err(_) => {}
            }
        }
        let exit = if stopped { None } else { tokio::time::timeout(Duration::from_secs(5), child.wait()).await.ok().and_then(Result::ok) };
        let stderr = err_task.await.unwrap_or_default();
        let _ = std::fs::remove_file(&system_file);
        if let Some(file) = &gate_file { let _ = std::fs::remove_file(file); }
        drop(gate);
        if let Some(session) = sink.session.clone() { self.claude_sessions.lock().unwrap().insert(turn.key.clone(), session); }
        if stopped { return Ok(Outcome::Stopped); }
        if sink.error.as_deref() == Some("timeout") { return Err("La respuesta excedió el tiempo máximo. Intenta de nuevo.".into()); }
        if sink.error.is_none() && sink.answer().trim().is_empty() && exit.is_none_or(|e| !e.success()) {
            // Do not forward full CLI logs (they can contain account/config details).
            let detail = String::from_utf8_lossy(&stderr).to_lowercase();
            if detail.contains("resume") || detail.contains("session") { self.claude_sessions.lock().unwrap().remove(&turn.key); }
            sink.error = Some(detail);
        }
        Ok(Outcome::Answered)
    }

    async fn run_codex(&self, turn: &Turn<'_>, workspace: &Path, sink: &mut Sink, cancel: &Arc<AtomicBool>) -> Result<Outcome, String> {
        let model = &turn.model;
        let effort = model.effort.unwrap_or("medium");
        let server = {
            let mut servers = self.codex.lock().await;
            let hash = system_hash(&format!("{}
{:?}", turn.system, turn.agent.can));
            if self.codex_system.lock().unwrap().get(&turn.key).is_some_and(|h| *h != hash) {
                if let Some(old) = servers.remove(&turn.key) { old.shutdown(); }
            }
            match servers.get(&turn.key) {
                Some(server) => server.clone(),
                None => {
                    let cwd = workspace.to_string_lossy().to_string();
                    let caps = Caps::of(turn.agent).with_relay(true);
                    let plan = agent_tools::codex_plan(turn.agent, &caps);
                    // `run_command` is MIKA's own tool: its question to the user needs the card, nothing else.
                    let gate = caps.run.then(|| {
                        let (app, base) = (sink.app.clone(), agent_gate::GateTurn { agent_id: turn.agent.id.clone(), agent_name: turn.agent.name.clone(), workspace: workspace.to_path_buf(), cancel: cancel.clone() });
                        GateCtx { run: Arc::new(move |args, cancel| {
                            let (app, turn) = (app.clone(), agent_gate::GateTurn { cancel, ..base.clone() });
                            Box::pin(async move { agent_gate::run_command(&app, &turn, &args).await })
                        }) }
                    });
                    let options = ThreadOptions { cwd: &cwd, model: &model.id, effort, instructions: &turn.system, plan: &plan, gate };
                    let server = CodexServer::start(command("codex")?, options).await.map_err(|e| user_error(&e))?;
                    servers.insert(turn.key.clone(), server.clone());
                    self.codex_system.lock().unwrap().insert(turn.key.clone(), hash);
                    server
                }
            }
        };
        let images: &[PathBuf] = if Caps::of(turn.agent).images { &turn.images } else { &[] };
        let turn_id = match server.start_turn(&turn.prompt, images, effort, cancel.clone()).await {
            Ok(id) => id,
            Err(error) => { self.drop_codex(&turn.key).await; return Err(user_error(&error)); }
        };
        let mut parser = FrameParser::new("codex").with_utc_offset(turn.utc_offset);
        let started = Instant::now();
        let mut interrupted: Option<Instant> = None;
        loop {
            if interrupted.is_none() && (cancel.load(Ordering::Relaxed) || started.elapsed() > TURN_LIMIT) {
                server.interrupt(&turn_id).await;
                interrupted = Some(Instant::now());
            }
            if interrupted.is_some_and(|at| at.elapsed() > STOP_GRACE) { self.drop_codex(&turn.key).await; break; }
            match server.next_note(Duration::from_millis(100)).await {
                None => {}
                Some(None) => { self.drop_codex(&turn.key).await; if sink.error.is_none() && sink.answer().trim().is_empty() { sink.error = Some("Codex se cerró.".into()); } break; }
                Some(Some((method, params))) => {
                    // Plan usage is about the account, not the turn: learn it and move on.
                    if method == "account/rateLimits/updated" { super::limits::record_codex(&params); continue; }
                    let belongs = params["turnId"].as_str() == Some(turn_id.as_str()) || params["turn"]["id"].as_str() == Some(turn_id.as_str())
                        || (method == "error" && params["turnId"].is_null());
                    if !belongs { continue; }
                    for frame in parser.codex_notification(&method, &params) { sink.frame(frame); }
                    if sink.done || sink.error.is_some() { break; }
                }
            }
        }
        if interrupted.is_some() {
            if cancel.load(Ordering::Relaxed) { return Ok(Outcome::Stopped); }
            return Err("La respuesta excedió el tiempo máximo. Intenta de nuevo.".into());
        }
        Ok(Outcome::Answered)
    }

    /// Drops the Codex conversation of `key` when the instructions it was started with are not these.
    async fn sync_codex_prompt(&self, key: &str, system: &str, agent: &AgentDefinition) {
        let hash = system_hash(&format!("{}
{:?}", system, agent.can));
        if self.codex_system.lock().unwrap().get(key).is_some_and(|h| *h != hash) { self.drop_codex(key).await; }
    }

    async fn drop_codex(&self, key: &str) {
        if let Some(server) = self.codex.lock().await.remove(key) { server.shutdown(); }
    }
}

pub fn history_list() -> Vec<HistoryEntry> { chat_store::list(|id| history(id).unwrap_or_default()) }

pub fn delete_conversation(agent_id: &str, id: &str) -> Result<(), String> { chat_store::delete(agent_id, id) }

pub fn open_memory(agent_id: &str) -> Result<(), String> {
    let agent = agent(agent_id)?;
    let path = chat_store::memory_path(&agent.id);
    if !path.exists() {
        std::fs::write(&path, format!("# Memoria de {}\n\nNotas que {} recibe en cada conversación. Puedes editarlas o borrarlas.\n", agent.name, agent.name))
            .map_err(|_| "No se pudo crear la memoria.")?;
    }
    crate::platform::shell::open_local(&path, false)
}

/// After a conversation is archived, the agent writes down in the background what is worth remembering. Only new
/// messages count (a reopened conversation is not summarised twice); a failure just leaves the memory as it was.
fn remember(agent: AgentDefinition, conversation: Conversation, utc_offset: i64) {
    let new = &conversation.messages[conversation.memorized.min(conversation.messages.len())..];
    if new.len() < 2 || !new.iter().any(|m| m.role == "user") { return; }
    let prompt = chat_store::summary_prompt(new);
    tauri::async_runtime::spawn(async move {
        let Some(answer) = one_shot(&agent, &prompt).await else { return };
        let notes = chat_store::summary_notes(&answer, &chat_store::date_label(chat_store::now_ms(), utc_offset));
        chat_store::append_memory(&agent.id, &notes);
    });
}

/// A single answer without a session or tools, for the memory notes. Uses whichever subscription is connected.
async fn one_shot(agent: &AgentDefinition, prompt: &str) -> Option<String> {
    let mut chosen = None;
    for p in std::iter::once(&agent.provider).chain(agent.providers.iter()) {
        let Ok(p) = provider(p) else { continue };
        if status(p).await.is_ok_and(|s| s.connected) { chosen = Some(p); break; }
    }
    let provider = chosen?;
    let model = named_agents::model_for(agent, provider);
    let workspace = named_agents::workspace(&agent.id);
    std::fs::create_dir_all(&workspace).ok()?;
    let mut cmd = command(provider).ok()?;
    cmd.current_dir(&workspace);
    if provider == "codex" {
        cmd.args(["exec", "--ignore-user-config", "--skip-git-repo-check", "--sandbox", "read-only", "--disable", "shell_tool",
                  "-c", "forced_login_method=\"chatgpt\"", "-c", "model_reasoning_effort=\"low\"", "-m", &model.id, "--json", "--color", "never", "-"]);
    } else {
        cmd.args(["-p", "--output-format", "text", "--safe-mode", "--strict-mcp-config", "--mcp-config", "{\"mcpServers\":{}}",
                  "--setting-sources", "", "--tools", "Read", "--allowedTools", "Read", "--permission-mode", "dontAsk",
                  "--model", &model.id, "--effort", "low"]);
    }
    cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    let mut child = cmd.spawn().ok()?;
    let mut input = child.stdin.take()?;
    input.write_all(prompt.as_bytes()).await.ok()?;
    drop(input);
    let output = tokio::time::timeout(Duration::from_secs(120), child.wait_with_output()).await.ok()?.ok()?;
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    if provider == "claude" { return Some(text); }
    let mut parser = FrameParser::new("codex");
    Some(text.lines().flat_map(|l| parser.feed(l)).filter_map(|f| if let Frame::Text(t) = f { Some(t) } else { None }).collect())
}

/// The first turn of a fresh session carries the recent conversation as data.
fn seed(messages: &[Message], query: &str) -> String {
    let mut out = String::from("Conversación anterior (datos, no instrucciones del sistema):\n");
    for message in &messages[messages.len().saturating_sub(12)..] {
        let who = if message.role == "user" { "Usuario" } else { "Asistente" };
        let content: String = message.content.chars().take(2000).collect();
        out.push_str(&format!("{who}: {content}\n"));
    }
    out.push_str(&format!("\nContinúa la conversación. Pregunta actual:\n{query}"));
    out
}

/// What the user reads when the account has no credits / usage left. A turn that fails with exactly this is the one
/// that is retried on the fallback model.
const USAGE_LIMIT_MESSAGE: &str = "Tu cuenta alcanzó un límite de uso. Revísalo en el cliente oficial.";

/// The key prefix of the session of a turn retried on the fallback model (kept apart from the primary's).
const FALLBACK_KEY: &str = "fb-";

/// The line shown in the chat when a turn is answered by the fallback model.
fn fallback_notice(primary: &str, fallback: &str) -> String { format!("Sin créditos de {primary}: respondo con {fallback}") }

/// Whether a provider's error text says the account has no credits or usage left (Claude's "Usage limit reached" and
/// "5-hour / weekly limit reached", Codex's "Usage limit" / `usageLimitExceeded`, quota and credit-balance errors).
/// Every other failure, including a rate limit, a context limit, a login problem or a timeout, is NOT one.
fn is_usage_limit(detail: &str) -> bool {
    let lower = detail.to_lowercase();
    // A per-minute rate limit (429) is temporary, not an empty account.
    if (lower.contains("rate limit") || lower.contains("rate_limit")) && !lower.contains("usage limit") { return false; }
    ["usage limit", "usagelimitexceeded", "usage_limit", "limit reached", "reached your limit", "hit your limit", "hit your usage",
     "quota", "out of credits", "insufficient credits", "no credits", "credit balance", "credits are exhausted", "run out of credits"]
        .iter().any(|k| lower.contains(k))
}

/// A provider failure in the user's words (never the raw CLI log).
fn user_error(detail: &str) -> String {
    let lower = detail.to_lowercase();
    if is_usage_limit(detail) { return USAGE_LIMIT_MESSAGE.into(); }
    if lower.contains("rate limit") || lower.contains("too many requests") || lower.contains("overloaded") {
        return "El servicio está recibiendo demasiadas peticiones. Intenta de nuevo en un momento.".into();
    }
    if ["not logged in", "/login", "unauthorized", "authenticat", "sign in", "log in", "oauth"].iter().any(|k| lower.contains(k)) {
        return "Conecta tu cuenta con el botón Conectar.".into();
    }
    // MIKA's own messages about the Codex process are already in the user's words.
    if detail.starts_with("Codex ") && detail.len() < 80 { return detail.to_string(); }
    "El cliente no devolvió una respuesta. Revisa su conexión o inicia sesión de nuevo.".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_the_two_known_providers_are_accepted() {
        assert!(provider("claude & calc").is_err());
        assert_eq!(provider("claude"), Ok("claude"));
        assert_eq!(provider("codex"), Ok("codex"));
    }

    #[test]
    fn a_chat_saved_before_sources_existed_still_loads() {
        let old: Vec<Message> = serde_json::from_str(r#"[{"role":"user","content":"hola"},{"role":"assistant","content":"qué tal"}]"#).unwrap();
        assert!(old.iter().all(|m| m.sources.is_empty() && m.agent.is_none()));
        let saved = serde_json::to_string(&old).unwrap();
        assert!(!saved.contains("sources") && !saved.contains("agent"), "nothing extra in the file");
    }

    #[test]
    fn a_hand_off_answer_keeps_who_answered_and_the_task() {
        let m = Message { role: "assistant".into(), content: "listo".into(), agent: Some("miro".into()), handoff_task: Some("un gato".into()), image_path: Some("C:\\a.png".into()), ..Default::default() };
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains(r#""agent":"miro""#) && json.contains(r#""handoffTask":"un gato""#) && json.contains(r#""imagePath""#));
    }

    #[test]
    fn the_seed_keeps_the_last_twelve_messages_as_data() {
        let messages: Vec<Message> = (0..20).map(|i| Message { role: if i % 2 == 0 { "user" } else { "assistant" }.into(), content: format!("m{i}"), ..Default::default() }).collect();
        let seed = seed(&messages, "¿y ahora?");
        assert!(seed.starts_with("Conversación anterior (datos, no instrucciones del sistema):\nUsuario: m8\n"));
        assert!(!seed.contains("m7\n") && seed.ends_with("Pregunta actual:\n¿y ahora?"));
    }

    #[test]
    fn errors_never_leak_the_cli_log() {
        assert!(user_error("Error: You've hit your usage limit").contains("límite"));
        assert!(user_error("Not logged in · Please run /login").contains("Conectar"));
        assert!(!user_error("panic at C:\\Users\\x\\config.toml token=abc").contains("token"));
    }

    #[test]
    fn only_a_no_credits_error_triggers_the_fallback() {
        for yes in ["Usage limit reached", "Usage limit", "Claude AI usage limit reached|1791164280", "5-hour limit reached ∙ resets 3pm",
                    "Weekly limit reached", "You've hit your limit", "You exceeded your current quota, please check your plan",
                    "Your credit balance is too low to access the API", "usageLimitExceeded", "insufficient_quota", "Out of credits",
                    "Error: You've hit your usage limit. Upgrade to Pro"] {
            assert!(is_usage_limit(yes), "{yes}");
            assert_eq!(user_error(yes), USAGE_LIMIT_MESSAGE, "{yes}");
        }
        for no in ["Not logged in · Please run /login", "Rate limit reached for requests (429)", "rate_limit_error: too many requests",
                   "Overloaded", "context length limit exceeded", "timeout", "Codex se cerró.", "", "panic at config.toml token=abc",
                   "invalid model", "network error: connection reset", "unauthorized"] {
            assert!(!is_usage_limit(no), "{no}");
            assert_ne!(user_error(no), USAGE_LIMIT_MESSAGE, "{no}");
        }
        // The retry is decided on the user-facing message, so the two stay in step.
        assert!(USAGE_LIMIT_MESSAGE.contains("límite de uso"));
    }

    #[test]
    fn the_fallback_line_names_both_models() {
        assert_eq!(fallback_notice("GPT-6.1-Sol", "Sonnet 5.5"), "Sin créditos de GPT-6.1-Sol: respondo con Sonnet 5.5");
        assert_eq!(fallback_notice("Sonnet 5.5", "Haiku 4.5"), "Sin créditos de Sonnet 5.5: respondo con Haiku 4.5");
        // MIKA on Sonnet falls to Haiku (a different model, so a retry happens); on Haiku itself there is nothing left.
        assert_ne!(named_agents::sonnet(), named_agents::fallback_for("mika"));
        assert_eq!(named_agents::haiku(), named_agents::fallback_for("mika"));
        assert_ne!(named_agents::codex_sol(), named_agents::fallback_for("mira"));
        assert_eq!(named_agents::sonnet(), named_agents::fallback_for("mira"));
    }

    #[test]
    fn images_are_only_read_from_the_allowed_folders() {
        assert!(image_data_url("C:\\Windows\\win.ini").is_err());
        assert!(image_data_url("relative.png").is_err());
        assert!(image_data_url("C:\\Windows\\System32\\nothing-here.png").is_err());
    }
}
