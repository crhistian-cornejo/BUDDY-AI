//! Sessions: what Claude Code, Codex and the Antigravity CLI (Gemini) are doing outside Buddy, from their own hooks.
//!
//! The agents run `buddy-hook` (the `hook/` crate) on every hook event; the relay hands each event to the local
//! server here (`server`: a Unix socket on Mac, a named pipe on Windows). `SessionHub` turns events into
//! `SessionUpdate`s for the notch / top bar, and a `PermissionRequest` into an `ApprovalRequest` card whose answer
//! (`answer_approval`) goes back to the waiting agent. The hooks are written into the agents' config files only
//! after the user saw the diff and clicked (`hooks_preview` → `hooks_write`; see `hook_file`).

pub mod always;
pub mod antigravity_hooks;
pub mod claude_hooks;
pub mod codex_hooks;
pub mod format;
pub mod hook_file;
pub(crate) mod server;
#[cfg(windows)]
pub(crate) mod win_user;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

pub use hook_file::HookPreview;

use crate::events::{Event, EventBus};
use crate::{CoreError, log, paths};

/// How long a permission request is held for a click: under the relay's own 110 s, so we always answer (or let go)
/// first and the agent falls back to its terminal prompt cleanly.
const DECISION_TIMEOUT: Duration = Duration::from_secs(108);
/// How often a held request checks whether the relay is still there.
const PEER_POLL: Duration = Duration::from_millis(250);
/// A session nobody heard from in this long (the agent was killed, SessionEnd never came) is forgotten.
const STALE_AFTER_SECS: i64 = 24 * 3600;

/// One coding agent's hooks, as the settings screen shows them.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct HookStatusInfo {
    /// `claude`, `codex` or `antigravity`.
    pub agent: String,
    /// `Claude Code`, `Codex` or `Gemini (Antigravity)`.
    pub name: String,
    /// Buddy's hooks are in the agent's config file.
    pub installed: bool,
    /// The agent's config folder exists (`~/.claude`, `$CODEX_HOME` or `~/.codex`, `~/.gemini/config`).
    pub available: bool,
    /// The relay the hooks run (`<data_dir>/bin/buddy-hook`) is in place.
    pub relay_ready: bool,
}

/// A Claude Code / Codex session Buddy heard from.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SessionInfo {
    pub session_id: String,
    /// `claude`, `codex` or `antigravity`.
    pub agent: String,
    /// The folder name of the session's working directory.
    pub project: String,
    /// `working`, `waiting`, `done` or `error`.
    pub state: String,
    /// Unix seconds of the last event.
    pub updated_at: i64,
}

/// The session state an ordinary hook event means. `None`: the event changes nothing Buddy shows.
pub(crate) fn state_for(event: &str) -> Option<&'static str> {
    Some(match event {
        "SessionStart" | "UserPromptSubmit" | "PreToolUse" | "PostToolUse" | "PostToolUseFailure"
        | "SubagentStart" | "SubagentStop" => "working",
        // Antigravity: before each model call.
        "PreInvocation" => "working",
        // Claude Code's Notification: it needs the user (a question, the idle prompt).
        "Notification" | "PermissionRequest" => "waiting",
        "Stop" => "done",
        // Codex only: the user stopped the turn. Nothing failed; the turn is over.
        "Interrupt" => "done",
        "StopFailure" => "error",
        "SessionEnd" => "ended",
        _ => return None,
    })
}

/// The question an agent is asking the user, when this event is its question tool starting.
///
/// Codex has no event for a question: its `request_user_input` tools arrive as an ordinary PreToolUse, and the
/// `_async` one returns at once, so the agent goes on working while the question stays on its screen (live check,
/// Codex 0.160: the answer comes back later as a UserPromptSubmit). Its text and options are for the notice.
fn question_of(payload: &Value) -> Option<String> {
    if text(payload, "hook_event_name") != "PreToolUse" || !text(payload, "tool_name").to_lowercase().contains("request_user_input") {
        return None;
    }
    let input = &payload["tool_input"];
    let first = input["questions"].get(0).unwrap_or(input);
    let ask = ["title", "question", "header", "prompt"].iter().find_map(|k| first.get(*k).and_then(Value::as_str)).unwrap_or("");
    let options: Vec<&str> = first
        .get("options")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|o| o.as_str().or_else(|| o.get("label").and_then(Value::as_str)))
        .collect();
    let mut out = ask.split_whitespace().collect::<Vec<_>>().join(" ");
    if !out.is_empty() && !options.is_empty() {
        out.push_str(&format!(" — {}", options.join(" · ")));
    }
    Some(out.chars().take(240).collect())
}

/// True when this event is a question tool ending with its answer: the tool that waits for the user
/// (`request_user_input`). The `_async` one ends at once, long before the answer.
fn answers_question(payload: &Value) -> bool {
    let tool = text(payload, "tool_name").to_lowercase();
    text(payload, "hook_event_name") == "PostToolUse" && tool.contains("request_user_input") && !tool.contains("async")
}

/// `claude`, `codex` or `antigravity`, from the relay's `_agent` tag (anything else is Claude Code, the relay's
/// default).
fn agent_of(payload: &Value) -> &'static str {
    match payload.get("_agent").and_then(Value::as_str) {
        Some("codex") => "codex",
        Some("antigravity") => "antigravity",
        _ => "claude",
    }
}

/// The last folder of `cwd`, with either separator.
fn project_of(cwd: &str) -> String {
    cwd.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or("").to_string()
}

fn text<'a>(payload: &'a Value, key: &str) -> &'a str {
    payload.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Where a session runs and what it last said, from one hook payload.
#[derive(Default)]
struct Place {
    cwd: String,
    terminal: String,
    summary: String,
}

impl Place {
    fn of(payload: &Value) -> Self {
        let summary = text(payload, "last_assistant_message").trim();
        let summary: String = summary.chars().take(240).collect();
        Self {
            cwd: text(payload, "cwd").into(),
            terminal: text(payload, "bundle_id").into(),
            summary: summary.split_whitespace().collect::<Vec<_>>().join(" "),
        }
    }
}

fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// A permission request waiting for a click.
struct Pending {
    answer: Sender<bool>,
    can_allow: bool,
    /// Who asked and the rule «Permitir siempre» would save (None: not offered).
    always: Option<(String, String)>,
}

#[derive(Default)]
struct Inner {
    /// Keyed by (agent, session id).
    sessions: HashMap<(String, String), SessionInfo>,
    pending: HashMap<String, Pending>,
    /// Sessions whose agent asked the user a question that is still unanswered (same key): they stay `waiting`
    /// whatever the agent does meanwhile.
    asked: std::collections::HashSet<(String, String)>,
}

pub struct SessionHub {
    data_dir: PathBuf,
    bus: Arc<EventBus>,
    inner: Mutex<Inner>,
    started: AtomicBool,
    next_id: AtomicU64,
    decision_timeout: Duration,
    /// Secret of this run of the app: only `claude` processes Buddy starts get it (BUDDY_GATE_TOKEN), and only a
    /// gate request carrying it can ever be allowed.
    gate_token: String,
    /// What the player plays, as the app last told us (for the agents' `now_playing`).
    now_playing: Mutex<Option<crate::media::NowPlayingInfo>>,
    pub youtube: crate::youtube::YouTube,
    /// Screenshots the app is taking, by file: the request waits for the app's word.
    shots: Mutex<HashMap<String, Sender<bool>>>,
    /// Requests of Buddy's tools that live elsewhere in the core (Spotify's search): `None` means "not mine".
    tools: std::sync::OnceLock<ToolHandler>,
}

/// Keeps only the newest `keep` captures.
fn prune_captures(dir: &Path, keep: usize) {
    let mut files: Vec<_> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "png")).collect();
    files.sort();
    let extra = files.len().saturating_sub(keep);
    for old in files.into_iter().take(extra) {
        let _ = std::fs::remove_file(old);
    }
}

/// Answers one tool request by name (`request`), or `None` when it does not know it.
pub type ToolHandler = Box<dyn Fn(&str, &Value) -> Option<Result<String, String>> + Send + Sync>;

impl SessionHub {
    pub fn new(data_dir: PathBuf, bus: Arc<EventBus>) -> Self {
        Self::with_timeout(data_dir, bus, DECISION_TIMEOUT)
    }

    /// Tests use a short decision timeout.
    pub(crate) fn with_timeout(data_dir: PathBuf, bus: Arc<EventBus>, decision_timeout: Duration) -> Self {
        Self {
            data_dir,
            youtube: crate::youtube::YouTube::new(bus.clone()),
            bus,
            inner: Mutex::new(Inner::default()),
            started: AtomicBool::new(false),
            next_id: AtomicU64::new(1),
            decision_timeout,
            gate_token: random_token(),
            now_playing: Mutex::new(None),
            tools: std::sync::OnceLock::new(),
            shots: Mutex::new(HashMap::new()),
        }
    }

    /// The app finished (or failed) the screenshot it was asked for.
    pub fn screenshot_taken(&self, path: &str, ok: bool) {
        if let Some(waiter) = self.shots.lock().unwrap_or_else(|p| p.into_inner()).remove(path) {
            let _ = waiter.send(ok);
        }
    }

    /// One screenshot for an agent, only after the user's «Permitir»: the app captures, the core shrinks it like
    /// any attached image and keeps the last few in `<data>/capturas`. Returns the file.
    fn screenshot(&self, reason: &str) -> Result<PathBuf, String> {
        let card = serde_json::json!({ "tool_name": "Pantalla", "tool_input": { "description": if reason.trim().is_empty() { "Una captura para responderte" } else { reason } } });
        if self.ask(&card, "buddy", "buddy".into(), "Buddy".into(), &|| false) != Some("allow") {
            return Err("El usuario no quiso compartir la pantalla.".into());
        }
        let dir = self.data_dir.join("capturas");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        prune_captures(&dir, 9);
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let path = dir.join(format!("captura-{stamp}.png"));
        let key = path.to_string_lossy().to_string();
        let (tx, rx) = channel();
        self.shots.lock().unwrap_or_else(|p| p.into_inner()).insert(key.clone(), tx);
        self.bus.publish(Event::ScreenshotRequest { path: key.clone() });
        let taken = rx.recv_timeout(Duration::from_secs(20)).unwrap_or(false);
        self.shots.lock().unwrap_or_else(|p| p.into_inner()).remove(&key);
        if !taken {
            return Err("No se pudo capturar la pantalla (¿falta el permiso de Grabación de pantalla?).".into());
        }
        // A screen is opaque: without its alpha channel it goes out as a small JPEG.
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        let screen = image::load_from_memory(&bytes).map_err(|e| e.to_string())?.to_rgb8();
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(screen).write_to(&mut png, image::ImageFormat::Png).map_err(|e| e.to_string())?;
        let (small, ext) = crate::images::prepare(png.get_ref())?;
        let out = path.with_extension(ext);
        std::fs::write(&out, small).map_err(|e| e.to_string())?;
        if out != path {
            let _ = std::fs::remove_file(&path);
        }
        Ok(out)
    }

    /// Installs the handler for the tool requests the hub does not answer itself (set once, at start).
    pub fn set_tool_handler(&self, handler: ToolHandler) {
        let _ = self.tools.set(handler);
    }

    /// A command one of Buddy's agents wants to run through its provider's own protocol (Codex): the same card
    /// as the gate. True only on the user's «Permitir».
    pub fn approve_command(&self, command: &str, folder: &str) -> bool {
        let payload = serde_json::json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": { "command": command },
            "cwd": folder,
        });
        self.ask(&payload, "buddy", "buddy".into(), "Buddy".into(), &|| false) == Some("allow")
    }

    /// The app reports what plays (on each change of its player).
    pub fn set_now_playing(&self, now: Option<crate::media::NowPlayingInfo>) {
        *self.now_playing.lock().unwrap_or_else(|p| p.into_inner()) = now;
    }

    /// The secret Buddy's own agents carry to ask for a command (see `gate`).
    pub fn gate_token(&self) -> &str {
        &self.gate_token
    }

    /// Whether the server runs (so a gate request would reach someone).
    pub fn is_started(&self) -> bool {
        self.started.load(Ordering::SeqCst)
    }

    /// The stable copy of the relay the agents' config files point at.
    pub fn relay_path(&self) -> PathBuf {
        paths::relay_path(&self.data_dir)
    }

    /// Copies the app's bundled relay to `relay_path()` (empty = skip) and starts the server once. Never blocks:
    /// the server runs on its own thread.
    pub fn start(self: &Arc<Self>, bundled_relay: &str) -> Result<(), CoreError> {
        let copied = if bundled_relay.trim().is_empty() {
            Ok(())
        } else {
            install_relay(Path::new(bundled_relay), &self.relay_path())
                .map_err(|e| CoreError::Hooks(format!("No se pudo instalar buddy-hook: {e}")))
        };
        self.start_at(self.endpoint())?;
        copied
    }

    #[cfg(unix)]
    fn endpoint(&self) -> server::Endpoint {
        server::Endpoint::Socket(paths::hooks_socket_path(&self.data_dir))
    }

    #[cfg(windows)]
    fn endpoint(&self) -> server::Endpoint {
        server::Endpoint::Pipe(win_user::pipe_name())
    }

    /// Starts the server on `endpoint` unless it already runs (tests pass their own socket).
    pub(crate) fn start_at(self: &Arc<Self>, endpoint: server::Endpoint) -> Result<(), CoreError> {
        if self.started.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let sink: Arc<dyn server::Sink> = self.clone();
        match server::spawn(endpoint, sink) {
            Ok(()) => {
                log::line("servidor de ganchos listo");
                Ok(())
            }
            Err(e) => {
                self.started.store(false, Ordering::Release);
                log::line(format!("no se pudo abrir el servidor de ganchos: {e}"));
                Err(CoreError::Hooks(format!("No se pudo abrir el canal de ganchos: {e}")))
            }
        }
    }

    pub fn hooks_status(&self) -> Vec<HookStatusInfo> {
        let relay = self.relay_path();
        let relay_ready = relay.exists();
        let status = |agent: &str, name: &str, dir: Result<PathBuf, String>, file: Result<hook_file::HookFile, String>| {
            HookStatusInfo {
                agent: agent.into(),
                name: name.into(),
                installed: file.map(|f| f.installed()).unwrap_or(false),
                available: dir.map(|d| d.is_dir()).unwrap_or(false),
                relay_ready,
            }
        };
        vec![
            status("claude", "Claude Code", claude_hooks::config_dir(), claude_hooks::file(&relay)),
            status("codex", "Codex", codex_hooks::config_dir(), codex_hooks::file(&relay)),
            status("antigravity", "Gemini (Antigravity)", antigravity_hooks::config_dir(), antigravity_hooks::file(&relay)),
        ]
    }

    fn hook_file(&self, agent: &str) -> Result<hook_file::HookFile, CoreError> {
        let relay = self.relay_path();
        match agent {
            "claude" => claude_hooks::file(&relay),
            "codex" => codex_hooks::file(&relay),
            "antigravity" => antigravity_hooks::file(&relay),
            other => return Err(CoreError::Hooks(format!("Agente desconocido: {other}"))),
        }
        .map_err(CoreError::Hooks)
    }

    pub fn hooks_preview(&self, agent: &str, install: bool) -> Result<HookPreview, CoreError> {
        self.hook_file(agent)?.preview(install).map_err(CoreError::Hooks)
    }

    /// Applies the previewed change (only if the file still matches `fingerprint`). Returns the backup's path, or
    /// an empty string when there was no file to back up.
    pub fn hooks_write(&self, agent: &str, install: bool, fingerprint: &str) -> Result<String, CoreError> {
        let backup = self.hook_file(agent)?.write(install, fingerprint).map_err(CoreError::Hooks)?;
        log::line(format!("ganchos de {agent} {}", if install { "instalados" } else { "quitados" }));
        Ok(backup)
    }

    /// The user's click on the approval card. An "allow" on a request that arrived cut short is not passed on: the
    /// card is closed and the agent asks in its terminal, where the whole request can be read.
    pub fn answer_approval(&self, request_id: &str, allow: bool) {
        let Some(pending) = self.lock().pending.remove(request_id) else {
            log::line(format!("respuesta para {request_id}: ya no hay petición"));
            return;
        };
        if allow && !pending.can_allow {
            log::line(format!("aprobación {request_id}: recortada, la terminal decide"));
            return; // dropping the sender releases the request without a decision
        }
        log::line(format!("aprobación {request_id}: {}", if allow { "permitir" } else { "denegar" }));
        let _ = pending.answer.send(allow);
    }

    /// «Permitir siempre»: allows this request and saves its rule, so the next ones like it need no card.
    pub fn answer_approval_always(&self, request_id: &str) {
        let rule = self.lock().pending.get(request_id).and_then(|p| p.always.clone());
        if let Some((agent, prefix)) = rule {
            let mut rules = always::load(&self.data_dir);
            if !rules.iter().any(|r| r.agent == agent && r.prefix == prefix) {
                let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
                log::line(format!("permitir siempre ({agent}): {prefix}"));
                rules.push(always::AlwaysRule { agent, prefix, added_at: now });
                always::save(&self.data_dir, &rules);
            }
        }
        self.answer_approval(request_id, true);
    }

    /// The commands allowed for good, newest first.
    pub fn always_rules(&self) -> Vec<always::AlwaysRule> {
        let mut rules = always::load(&self.data_dir);
        rules.sort_by(|a, b| b.added_at.cmp(&a.added_at));
        rules
    }

    pub fn remove_always_rule(&self, agent: &str, prefix: &str) {
        let mut rules = always::load(&self.data_dir);
        rules.retain(|r| !(r.agent == agent && r.prefix == prefix));
        always::save(&self.data_dir, &rules);
    }

    /// Live sessions, most recent first.
    pub fn sessions(&self) -> Vec<SessionInfo> {
        let mut list: Vec<SessionInfo> = self.lock().sessions.values().cloned().collect();
        list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then_with(|| a.session_id.cmp(&b.session_id)));
        list
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Records the session's new state; emits `SessionUpdate` only when something shown changed.
    /// A hook from one of Buddy's own turns (its agents work in `<data>/agents/<id>/workspace`), not a session of
    /// the user's: the notch must not show it.
    /// Antigravity also lists every folder of the session (`workspace_paths`): Buddy's turns put the agent's own
    /// workspace first, and add the user's folders after it.
    fn is_own_run(&self, payload: &Value) -> bool {
        let agents = self.data_dir.join("agents");
        let real = std::fs::canonicalize(&agents).ok();
        let own = |dir: &str| {
            !dir.is_empty() && (Path::new(dir).starts_with(&agents) || real.as_ref().is_some_and(|r| Path::new(dir).starts_with(r)))
        };
        own(text(payload, "cwd"))
            || payload.get("workspace_paths").and_then(Value::as_array).is_some_and(|dirs| dirs.iter().filter_map(Value::as_str).any(own))
    }

    fn update_session(&self, agent: &str, session_id: &str, project: &str, state: &str, place: &Place) {
        let now = unix_now();
        let key = (agent.to_string(), session_id.to_string());
        let event = {
            let mut inner = self.lock();
            inner.sessions.retain(|_, s| now - s.updated_at < STALE_AFTER_SECS);
            if state == "ended" {
                let known = inner.sessions.remove(&key);
                let project = known.map(|s| s.project).filter(|_| project.is_empty()).unwrap_or_else(|| project.to_string());
                Some(Event::SessionUpdate {
                    session_id: session_id.into(),
                    agent: agent.into(),
                    project,
                    state: state.into(),
                    cwd: place.cwd.clone(),
                    terminal: place.terminal.clone(),
                    summary: String::new(),
                })
            } else {
                let entry = inner.sessions.entry(key).or_insert_with(|| SessionInfo {
                    session_id: session_id.into(),
                    agent: agent.into(),
                    project: String::new(),
                    state: String::new(),
                    updated_at: now,
                });
                entry.updated_at = now;
                let changed = entry.state != state || (!project.is_empty() && entry.project != project);
                if !project.is_empty() {
                    entry.project = project.into();
                }
                entry.state = state.into();
                changed.then(|| Event::SessionUpdate {
                    session_id: entry.session_id.clone(),
                    agent: entry.agent.clone(),
                    project: entry.project.clone(),
                    state: entry.state.clone(),
                    cwd: place.cwd.clone(),
                    terminal: place.terminal.clone(),
                    // What it said last (done), or the question it asks (waiting).
                    summary: if state == "done" || state == "waiting" { place.summary.clone() } else { String::new() },
                })
            }
        };
        if let Some(event) = event {
            self.bus.publish(event);
        }
    }
}

impl server::Sink for SessionHub {
    fn youtube(&self, payload: Value) -> String { self.youtube.receive(&payload) }
    fn event(&self, payload: Value) {
        if self.is_own_run(&payload) {
            log::line(format!("gancho de un turno de Buddy ignorado ({})", agent_of(&payload)));
            return;
        }
        let event = text(&payload, "hook_event_name");
        let agent = agent_of(&payload);
        // Never the payload itself: tool inputs may hold secrets.
        log::line(format!("gancho {event} ({agent}, {})", project_of(text(&payload, "cwd"))));
        let session = text(&payload, "session_id");
        let project = project_of(text(&payload, "cwd"));
        let key = (agent.to_string(), session.to_string());
        if let Some(question) = question_of(&payload) {
            // The agent asks the user something: the session waits for them, and the notice says what is asked.
            log::line(format!("pregunta ({agent}, {project})"));
            self.lock().asked.insert(key);
            self.update_session(agent, session, &project, "waiting", &Place { summary: question, ..Place::of(&payload) });
            return;
        }
        let unanswered = {
            let mut inner = self.lock();
            match event {
                // The answer comes as the user's next message; a stop or the session's end closes the question too.
                "UserPromptSubmit" | "Interrupt" | "SessionStart" | "SessionEnd" => {
                    inner.asked.remove(&key);
                    false
                }
                _ if answers_question(&payload) => {
                    inner.asked.remove(&key);
                    false
                }
                _ => inner.asked.contains(&key),
            }
        };
        // While its question is unanswered the session keeps waiting, whatever the agent does meanwhile.
        if unanswered {
            return;
        }
        if let Some(state) = state_for(event) {
            self.update_session(agent, session, &project, state, &Place::of(&payload));
        }
    }

    fn permission(&self, payload: Value, closed: &dyn Fn() -> bool) -> Option<&'static str> {
        // Buddy's own turns never ask through the user's hooks (Codex runs them with approvals off).
        if self.is_own_run(&payload) {
            return None;
        }
        let agent = agent_of(&payload);
        let session_id = text(&payload, "session_id").to_string();
        let project = project_of(text(&payload, "cwd"));
        self.update_session(agent, &session_id, &project, "waiting", &Place::of(&payload));
        self.ask(&payload, agent, session_id, project, closed)
    }

    fn gate(&self, payload: Value, closed: &dyn Fn() -> bool) -> &'static str {
        if payload.get("_gate").and_then(Value::as_str) != Some(self.gate_token.as_str()) {
            log::line("puerta: petición sin el secreto de esta sesión, rechazada");
            return "deny";
        }
        self.ask(&payload, "buddy", "buddy".into(), "Buddy".into(), closed).unwrap_or("deny")
    }

    /// The music player for Buddy's agents: no click needed (it only presses play/pause/next or opens a Spotify
    /// item), but only with this run's secret and only clean Spotify links.
    fn app(&self, payload: Value) -> String {
        let reply = |ok: bool, text: &str| serde_json::json!({ "ok": ok, "text": text }).to_string();
        if payload.get("_app").and_then(Value::as_str) != Some(self.gate_token.as_str()) {
            log::line("herramientas: petición sin el secreto de esta sesión, rechazada");
            return reply(false, "Petición rechazada.");
        }
        match text(&payload, "request") {
            "screenshot" => match self.screenshot(text(&payload, "reason")) {
                Ok(path) => serde_json::json!({ "ok": true, "text": "Captura de la pantalla del usuario.", "image": path }).to_string(),
                Err(e) => reply(false, &e),
            },
            "now_playing" => {
                let now = self.now_playing.lock().unwrap_or_else(|p| p.into_inner()).clone();
                reply(true, &crate::media::describe(now.as_ref()))
            }
            "media" => {
                let action = text(&payload, "action");
                let uri = if action == "open" {
                    match crate::media::spotify_uri(text(&payload, "uri")) {
                        Some(uri) => uri,
                        None => return reply(false, "Ese enlace no es de Spotify (usa https://open.spotify.com/… o spotify:…)."),
                    }
                } else if action == "search" {
                    match crate::media::search_uri(text(&payload, "query")) {
                        Some(uri) => uri,
                        None => return reply(false, "Falta qué buscar."),
                    }
                } else if crate::media::ACTIONS.contains(&action) {
                    String::new()
                } else {
                    return reply(false, "Acción desconocida.");
                };
                log::line(format!("música: {action} {uri}"));
                self.bus.publish(Event::MediaCommand { action: action.into(), uri });
                reply(true, crate::media::done_text(action))
            }
            other => match self.tools.get().and_then(|handle| handle(other, &payload)) {
                Some(Ok(text)) => reply(true, &text),
                Some(Err(text)) => reply(false, &text),
                None => reply(false, "Petición desconocida."),
            },
        }
    }
}

impl SessionHub {
    /// Puts an approval card in front of the user and waits for the click (or the timeout, or the asker hanging up).
    fn ask(&self, payload: &Value, agent: &str, session_id: String, project: String, closed: &dyn Fn() -> bool) -> Option<&'static str> {
        let shown = format::approval_text(payload);
        // A command the user allowed for good: no card. Only an agent's own shell tool: an MCP server's tool that
        // takes a `command` is another program's business, and always asks.
        let mcp = text(payload, "tool_name").starts_with("mcp__");
        let command = payload.get("tool_input").and_then(Value::as_object).and_then(format::command_of).filter(|_| !mcp);
        if let Some(command) = command.as_deref().filter(|_| shown.can_allow) {
            if always::load(&self.data_dir).iter().any(|r| r.agent == agent && always::covers(&r.prefix, command)) {
                log::line(format!("permiso ({agent}): permitido siempre"));
                return Some("allow");
            }
        }
        let always = command.as_deref().filter(|_| shown.can_allow).and_then(always::prefix_for);
        let request_id = format!("{}-{}", std::process::id(), self.next_id.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = channel();
        self.lock().pending.insert(
            request_id.clone(),
            Pending { answer: tx, can_allow: shown.can_allow, always: always.clone().map(|p| (agent.to_string(), p)) },
        );
        log::line(format!("permiso {request_id} ({agent}): {}", shown.title));
        self.bus.publish(Event::ApprovalRequest {
            request_id: request_id.clone(),
            session_id,
            agent: agent.into(),
            project,
            title: shown.title,
            summary: shown.summary,
            detail: shown.detail,
            can_allow: shown.can_allow,
            always: always.unwrap_or_default(),
        });
        self.bus.publish(Event::MascotState { state: "ask".into() });

        let deadline = Instant::now() + self.decision_timeout;
        let decision = loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                log::line(format!("permiso {request_id}: sin respuesta, la terminal decide"));
                break None;
            }
            match rx.recv_timeout(left.min(PEER_POLL)) {
                Ok(allow) => break Some(allow),
                Err(RecvTimeoutError::Timeout) => {
                    if closed() {
                        log::line(format!("permiso {request_id}: el agente ya no espera"));
                        break None;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break None,
            }
        };

        let none_left = {
            let mut inner = self.lock();
            inner.pending.remove(&request_id);
            inner.pending.is_empty()
        };
        self.bus.publish(Event::ApprovalClosed { request_id });
        if none_left {
            self.bus.publish(Event::MascotState { state: "idle".into() });
        }
        decision.map(|allow| if allow { "allow" } else { "deny" })
    }
}

/// 128 random bits as hex, from the OS (the clock or a counter would be guessable).
fn random_token() -> String {
    let mut bytes = [0u8; 16];
    let filled = std::fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes));
    if filled.is_err() {
        // Windows (no /dev/urandom): the OS-seeded hasher keys of std, mixed with time and pid.
        use std::hash::{BuildHasher, Hasher};
        for chunk in bytes.chunks_mut(8) {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
            h.write_u32(std::process::id());
            chunk.copy_from_slice(&h.finish().to_le_bytes());
        }
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Copies the relay to `dest` (mode 0755 on Unix) unless an identical copy is already there. Written beside the
/// target and renamed over it, so an agent running the old copy right now never sees half a file. On Windows a
/// running relay cannot be replaced; the old copy stays, which is fine (it is the same protocol).
fn install_relay(src: &Path, dest: &Path) -> std::io::Result<()> {
    let bytes = std::fs::read(src)?;
    if std::fs::read(dest).ok().as_deref() == Some(bytes.as_slice()) {
        return set_executable(dest);
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = dest.with_extension(format!("new-{}", std::process::id()));
    std::fs::write(&temp, &bytes)?;
    set_executable(&temp)?;
    if let Err(err) = std::fs::rename(&temp, dest) {
        let _ = std::fs::remove_file(&temp);
        if dest.exists() {
            log::line(format!("buddy-hook en uso, se queda la copia anterior: {err}"));
            return Ok(());
        }
        return Err(err);
    }
    log::line("buddy-hook instalado");
    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::server::Sink;
    use serde_json::json;
    use std::sync::mpsc::Receiver;

    #[test]
    fn a_screenshot_needs_the_click_and_the_app() {
        let (hub, rx, _dir) = hub(Duration::from_secs(5));
        let token = hub.gate_token().to_string();
        let asker = hub.clone();
        let t = token.clone();
        let denied = std::thread::spawn(move || Sink::app(&*asker, json!({ "_app": t, "request": "screenshot" })));
        let Ok(Event::ApprovalRequest { request_id, title, .. }) = rx.recv_timeout(Duration::from_secs(2)) else { panic!("no card") };
        assert_eq!(title, "Ver tu pantalla");
        hub.answer_approval(&request_id, false);
        assert!(denied.join().unwrap().contains("no quiso"));
        while rx.try_recv().is_ok() {}

        let asker = hub.clone();
        let taken = std::thread::spawn(move || Sink::app(&*asker, json!({ "_app": token, "request": "screenshot", "reason": "ver el error" })));
        let Ok(Event::ApprovalRequest { request_id, summary, .. }) = rx.recv_timeout(Duration::from_secs(2)) else { panic!("no card") };
        assert_eq!(summary, "ver el error");
        hub.answer_approval(&request_id, true);
        let path = loop {
            if let Ok(Event::ScreenshotRequest { path }) = rx.recv_timeout(Duration::from_secs(2)) {
                break path;
            }
        };
        image::RgbaImage::from_pixel(3000, 2000, image::Rgba([1, 2, 3, 255])).save(&path).unwrap();
        hub.screenshot_taken(&path, true);
        let reply: Value = serde_json::from_str(&taken.join().unwrap()).unwrap();
        let image = reply["image"].as_str().unwrap().to_string();
        assert_eq!((reply["ok"].clone(), image.as_str()), (json!(true), path.replace(".png", ".jpg").as_str()));
        assert!(!std::path::Path::new(&path).exists(), "only the small copy stays");
        let small = image::open(&image).unwrap();
        assert!(small.width().max(small.height()) < 3000, "shrunk like any attached image");
    }

    #[test]
    fn allow_always_saves_the_rule_and_skips_the_next_card() {
        let (hub, rx, _dir) = hub(Duration::from_secs(5));
        let asker = hub.clone();
        let first = std::thread::spawn(move || asker.approve_command("git status", "/u"));
        let Ok(Event::ApprovalRequest { request_id, always, .. }) = rx.recv_timeout(Duration::from_secs(2)) else { panic!("no card") };
        assert_eq!(always, "git status");
        hub.answer_approval_always(&request_id);
        assert!(first.join().unwrap());
        while rx.try_recv().is_ok() {}
        assert!(hub.approve_command("git status --short", "/u"), "covered: allowed with no card");
        assert!(!matches!(rx.try_recv(), Ok(Event::ApprovalRequest { .. })));
        assert_eq!(hub.always_rules()[0].prefix, "git status");
        // Another command still asks; a risky one is never offered «always».
        let asker = hub.clone();
        let other = std::thread::spawn(move || asker.approve_command("rm -rf build", "/u"));
        let Ok(Event::ApprovalRequest { request_id, always, .. }) = rx.recv_timeout(Duration::from_secs(2)) else { panic!("no card") };
        assert_eq!(always, "");
        hub.answer_approval(&request_id, false);
        assert!(!other.join().unwrap());
        hub.remove_always_rule("buddy", "git status");
        assert!(hub.always_rules().is_empty());
    }

    /// A rule is for a shell command: an MCP tool that happens to take a `command` argument always asks.
    #[test]
    fn an_always_rule_never_covers_an_mcp_tool() {
        let (hub, rx, _dir) = hub(Duration::from_secs(5));
        always::save(&hub.data_dir, &[always::AlwaysRule { agent: "claude".into(), prefix: "git status".into(), added_at: 1 }]);
        let payload = json!({
            "hook_event_name": "PermissionRequest", "session_id": "s1", "cwd": "/p/buddy",
            "tool_name": "mcp__otro__run", "tool_input": { "command": "git status" }
        });
        let asker = hub.clone();
        let asked = std::thread::spawn(move || asker.ask(&payload, "claude", "s1".into(), "buddy".into(), &|| false));
        let Ok(Event::ApprovalRequest { request_id, always, .. }) = rx.recv_timeout(Duration::from_secs(2)) else { panic!("no card: the rule covered an MCP tool") };
        assert_eq!(always, "", "and it is not offered for good");
        hub.answer_approval(&request_id, false);
        assert_eq!(asked.join().unwrap(), Some("deny"));
    }

    #[test]
    fn a_codex_command_waits_for_the_users_click() {
        let (hub, rx, _dir) = hub(Duration::from_secs(5));
        for allow in [true, false] {
            let asker = hub.clone();
            let waiting = std::thread::spawn(move || asker.approve_command("git status", "/u/proyecto"));
            let Ok(Event::ApprovalRequest { request_id, agent, summary, .. }) = rx.recv_timeout(Duration::from_secs(2)) else {
                panic!("no card")
            };
            assert_eq!((agent.as_str(), summary.as_str()), ("buddy", "git status"));
            hub.answer_approval(&request_id, allow);
            assert_eq!(waiting.join().unwrap(), allow);
            while rx.try_recv().is_ok() {}
        }
    }

    #[test]
    fn hooks_from_buddys_own_turns_never_reach_the_notch() {
        let (hub, rx, dir) = hub(Duration::from_secs(1));
        let own = dir.path().join("agents/buddy/workspace");
        Sink::event(&*hub, json!({ "hook_event_name": "UserPromptSubmit", "session_id": "b1", "_agent": "codex", "cwd": own }));
        assert!(rx.try_recv().is_err(), "Buddy's own Codex turn is not a session");
        assert!(Sink::permission(&*hub, json!({ "hook_event_name": "PermissionRequest", "session_id": "b1", "cwd": own }), &|| false).is_none());
        Sink::event(&*hub, json!({ "hook_event_name": "UserPromptSubmit", "session_id": "u1", "_agent": "codex", "cwd": "/Users/u/proyecto" }));
        assert!(matches!(rx.try_recv(), Ok(Event::SessionUpdate { .. })), "the user's own sessions still show");
    }

    #[test]
    fn the_music_player_needs_the_secret_and_a_clean_spotify_link() {
        let (hub, rx, _dir) = hub(Duration::from_secs(1));
        let token = hub.gate_token().to_string();
        let ask = |v: Value| serde_json::from_str::<Value>(&Sink::app(&*hub, v)).unwrap();
        assert_eq!(ask(json!({ "_app": "robado", "request": "media", "action": "next" }))["ok"], false);
        assert!(rx.try_recv().is_err(), "nothing announced without the secret");
        assert_eq!(ask(json!({ "_app": token, "request": "media", "action": "rm" }))["ok"], false);
        assert_eq!(ask(json!({ "_app": token, "request": "media", "action": "open", "uri": "https://evil.example/x" }))["ok"], false);
        assert_eq!(ask(json!({ "_app": token, "request": "media", "action": "next" }))["ok"], true);
        assert_eq!(rx.try_recv().unwrap(), Event::MediaCommand { action: "next".into(), uri: String::new() });
        let link = "https://open.spotify.com/track/7hQJA50XrCWABAu5v6QZ4i?si=1";
        assert_eq!(ask(json!({ "_app": token, "request": "media", "action": "open", "uri": link }))["ok"], true);
        assert_eq!(rx.try_recv().unwrap(), Event::MediaCommand { action: "open".into(), uri: "spotify:track:7hQJA50XrCWABAu5v6QZ4i".into() });
        assert_eq!(ask(json!({ "_app": token, "request": "media", "action": "search", "query": "Crisco" }))["ok"], true);
        assert_eq!(rx.try_recv().unwrap(), Event::MediaCommand { action: "search".into(), uri: "spotify:search:Crisco".into() });
        hub.set_now_playing(Some(crate::media::NowPlayingInfo { title: "Creep".into(), artist: "Radiohead".into(), app: "Spotify".into(), playing: true }));
        assert_eq!(ask(json!({ "_app": token, "request": "now_playing" }))["text"], "«Creep» de Radiohead en Spotify (sonando).");
    }

    fn hub(timeout: Duration) -> (Arc<SessionHub>, Receiver<Event>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let bus = Arc::new(EventBus::default());
        let rx = bus.subscribe();
        (Arc::new(SessionHub::with_timeout(dir.path().to_path_buf(), bus, timeout)), rx, dir)
    }

    fn drain(rx: &Receiver<Event>) -> Vec<Event> {
        rx.try_iter().map(bare).collect()
    }

    /// The event without where the session runs (the tests below compare what changed, not the place).
    fn bare(mut event: Event) -> Event {
        if let Event::SessionUpdate { cwd, terminal, summary, .. } = &mut event {
            cwd.clear();
            terminal.clear();
            summary.clear();
        }
        event
    }

    fn update(session: &str, agent: &str, project: &str, state: &str) -> Event {
        Event::SessionUpdate {
            session_id: session.into(),
            agent: agent.into(),
            project: project.into(),
            state: state.into(),
            cwd: String::new(),
            terminal: String::new(),
            summary: String::new(),
        }
    }

    #[test]
    fn hook_events_map_to_session_states() {
        for (event, state) in [
            ("SessionStart", Some("working")),
            ("UserPromptSubmit", Some("working")),
            ("PreToolUse", Some("working")),
            ("PostToolUse", Some("working")),
            ("Notification", Some("waiting")),
            ("Stop", Some("done")),
            ("StopFailure", Some("error")),
            ("SessionEnd", Some("ended")),
            ("PreCompact", None),
            ("", None),
        ] {
            assert_eq!(state_for(event), state, "{event}");
        }
        assert_eq!(project_of("/Users/a/code/buddy/"), "buddy");
        assert_eq!(project_of(r"C:\Users\a\code\mika"), "mika");
        assert_eq!(project_of(""), "");
    }

    #[test]
    fn a_question_from_codex_keeps_the_session_waiting_until_it_is_answered() {
        let (hub, rx, _dir) = hub(DECISION_TIMEOUT);
        let ev = |name: &str| json!({ "hook_event_name": name, "session_id": "s1", "cwd": "/p/buddy", "_agent": "codex", "tool_name": "exec" });
        let question = json!({
            "hook_event_name": "PreToolUse", "session_id": "s1", "cwd": "/p/buddy", "_agent": "codex",
            "tool_name": "request_user_input_async",
            "tool_input": { "questions": [{ "title": "¿Usamos  barras\no una tarjeta?", "options": ["Barras", "Tarjeta"] }] }
        });
        hub.event(ev("UserPromptSubmit"));
        hub.event(question.clone());
        // The tool returns at once and the agent goes on working, then its turn ends: the question still stands.
        for name in ["PostToolUse", "PreToolUse", "PostToolUse", "Stop"] {
            hub.event(ev(name));
        }
        let events: Vec<Event> = rx.try_iter().collect();
        assert_eq!(events.iter().cloned().map(bare).collect::<Vec<_>>(), vec![update("s1", "codex", "buddy", "working"), update("s1", "codex", "buddy", "waiting")]);
        let Event::SessionUpdate { summary, cwd, .. } = &events[1] else { panic!("a session update") };
        assert_eq!((summary.as_str(), cwd.as_str()), ("¿Usamos barras o una tarjeta? — Barras · Tarjeta", "/p/buddy"));
        assert_eq!(hub.sessions()[0].state, "waiting");
        // The answer is the user's next message: back to work, and the turn's end is an end again.
        hub.event(ev("UserPromptSubmit"));
        hub.event(ev("Stop"));
        assert_eq!(drain(&rx), vec![update("s1", "codex", "buddy", "working"), update("s1", "codex", "buddy", "done")]);
        // A stop by the user closes the question too; any other tool is not a question.
        hub.event(question);
        hub.event(ev("Interrupt"));
        assert_eq!(drain(&rx), vec![update("s1", "codex", "buddy", "waiting"), update("s1", "codex", "buddy", "done")]);
        // The tool that waits for the answer (plan mode) ends when the user has answered: back to work.
        let waits = |event: &str| json!({
            "hook_event_name": event, "session_id": "s1", "cwd": "/p/buddy", "_agent": "codex", "tool_name": "request_user_input",
            "tool_input": { "questions": [{ "id": "q", "header": "Formato", "question": "¿Barras o tarjeta?", "options": [{ "label": "Barras", "description": "…" }] }] }
        });
        hub.event(waits("PreToolUse"));
        hub.event(waits("PostToolUse"));
        let events: Vec<Event> = rx.try_iter().collect();
        assert!(matches!(&events[0], Event::SessionUpdate { state, summary, .. } if state == "waiting" && summary == "¿Barras o tarjeta? — Barras"), "{events:?}");
        assert!(matches!(&events[1], Event::SessionUpdate { state, .. } if state == "working"));
        assert_eq!(question_of(&ev("PreToolUse")), None);
        assert_eq!(question_of(&json!({ "hook_event_name": "PostToolUse", "tool_name": "request_user_input_async" })), None);
        assert_eq!(question_of(&json!({ "hook_event_name": "PreToolUse", "tool_name": "request_user_input" })).as_deref(), Some(""));
    }

    #[test]
    fn working_is_only_announced_when_the_state_changes() {
        let (hub, rx, _dir) = hub(DECISION_TIMEOUT);
        let ev = |name: &str, agent: &str| json!({ "hook_event_name": name, "session_id": "s1", "cwd": "/p/buddy", "_agent": agent });
        hub.event(ev("SessionStart", "claude"));
        hub.event(ev("UserPromptSubmit", "claude"));
        hub.event(ev("PreToolUse", "claude"));
        hub.event(ev("PostToolUse", "claude"));
        hub.event(ev("Notification", "claude"));
        hub.event(ev("PreToolUse", "claude"));
        hub.event(ev("Stop", "claude"));
        hub.event(ev("PreCompact", "claude"));
        // Same session id from the other agent is another session.
        hub.event(ev("PreToolUse", "codex"));
        assert_eq!(
            drain(&rx),
            vec![
                update("s1", "claude", "buddy", "working"),
                update("s1", "claude", "buddy", "waiting"),
                update("s1", "claude", "buddy", "working"),
                update("s1", "claude", "buddy", "done"),
                update("s1", "codex", "buddy", "working"),
            ]
        );
        assert_eq!(hub.sessions().len(), 2);

        hub.event(json!({ "hook_event_name": "SessionEnd", "session_id": "s1", "_agent": "claude" }));
        assert_eq!(drain(&rx), vec![update("s1", "claude", "buddy", "ended")]);
        let left = hub.sessions();
        assert_eq!(left.len(), 1);
        assert_eq!((left[0].agent.as_str(), left[0].state.as_str()), ("codex", "working"));
    }

    /// What `buddy-hook --antigravity` forwards for an `agy` session (see hook/src/main.rs `antigravity_payload`).
    #[test]
    fn an_antigravity_session_is_its_own_agent() {
        let (hub, rx, dir) = hub(DECISION_TIMEOUT);
        let ev = |name: &str| json!({
            "hook_event_name": name, "_agent": "antigravity", "session_id": "c-1",
            "cwd": "/Users/u/web", "workspace_paths": ["/Users/u/web"]
        });
        hub.event(ev("PreInvocation"));
        hub.event(ev("PostToolUse"));
        hub.event(ev("PreInvocation"));
        hub.event(ev("Stop"));
        hub.event(ev("PreInvocation"));
        hub.event(ev("StopFailure"));
        assert_eq!(
            drain(&rx),
            vec![
                update("c-1", "antigravity", "web", "working"),
                update("c-1", "antigravity", "web", "done"),
                update("c-1", "antigravity", "web", "working"),
                update("c-1", "antigravity", "web", "error"),
            ]
        );

        // Buddy's own Gemini turn: its workspace comes first, the user's folders after it.
        let own = dir.path().join("agents/investigador/workspace");
        hub.event(json!({
            "hook_event_name": "PreInvocation", "_agent": "antigravity", "session_id": "b-1",
            "cwd": own, "workspace_paths": [own, "/Users/u/web"]
        }));
        hub.event(json!({
            "hook_event_name": "Stop", "_agent": "antigravity", "session_id": "b-2",
            "cwd": "", "workspace_paths": [own]
        }));
        assert!(rx.try_recv().is_err(), "Buddy's own Gemini turns never reach the notch");
        assert_eq!(hub.sessions().len(), 1);
    }

    #[test]
    fn the_settings_list_gemini_next_to_claude_and_codex() {
        let (hub, _rx, _dir) = hub(DECISION_TIMEOUT);
        let status = hub.hooks_status();
        let agents: Vec<(&str, &str)> = status.iter().map(|s| (s.agent.as_str(), s.name.as_str())).collect();
        assert_eq!(agents, [("claude", "Claude Code"), ("codex", "Codex"), ("antigravity", "Gemini (Antigravity)")]);
        assert!(hub.hook_file("gemini-cli").is_err());
    }

    #[test]
    fn an_unanswered_request_times_out_and_the_mascot_goes_back_to_idle() {
        let (hub, rx, _dir) = hub(Duration::from_millis(300));
        let payload = json!({ "hook_event_name": "PermissionRequest", "session_id": "s", "cwd": "/p/x", "tool_name": "Bash", "tool_input": { "command": "ls" } });
        assert_eq!(hub.permission(payload, &|| false), None);
        let events = drain(&rx);
        assert!(matches!(&events[1], Event::ApprovalRequest { title, .. } if title == "Ejecutar un comando"));
        assert_eq!(events[2], Event::MascotState { state: "ask".into() });
        assert!(matches!(&events[3], Event::ApprovalClosed { .. }));
        assert_eq!(events[4], Event::MascotState { state: "idle".into() });
    }

    #[test]
    fn a_relay_that_hangs_up_closes_the_card() {
        let (hub, rx, _dir) = hub(DECISION_TIMEOUT);
        let started = Instant::now();
        let payload = json!({ "hook_event_name": "PermissionRequest", "session_id": "s", "tool_name": "Read" });
        assert_eq!(hub.permission(payload, &|| true), None);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(drain(&rx).iter().any(|e| matches!(e, Event::ApprovalClosed { .. })));
    }

    #[test]
    fn allow_is_not_passed_on_for_a_truncated_request() {
        let (hub, rx, _dir) = hub(DECISION_TIMEOUT);
        let worker = {
            let hub = hub.clone();
            std::thread::spawn(move || {
                let payload = json!({ "hook_event_name": "PermissionRequest", "session_id": "s", "tool_name": "Bash", "tool_input": { "command": "x…" }, "_truncated": true });
                hub.permission(payload, &|| false)
            })
        };
        let id = loop {
            if let Ok(Event::ApprovalRequest { request_id, can_allow, .. }) = rx.recv_timeout(Duration::from_secs(5)) {
                assert!(!can_allow);
                break request_id;
            }
        };
        hub.answer_approval(&id, true);
        assert_eq!(worker.join().unwrap(), None, "the terminal decides");
    }

    #[test]
    fn the_relay_is_copied_once_and_made_executable() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("bundled-buddy-hook");
        std::fs::write(&src, b"relay v1").unwrap();
        let dest = paths::relay_path(&dir.path().join("data"));
        install_relay(&src, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"relay v1");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777, 0o755);
        }
        let modified = std::fs::metadata(&dest).unwrap().modified().unwrap();
        install_relay(&src, &dest).unwrap();
        assert_eq!(std::fs::metadata(&dest).unwrap().modified().unwrap(), modified, "an identical copy is left alone");
        std::fs::write(&src, b"relay v2").unwrap();
        install_relay(&src, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"relay v2");
        assert!(install_relay(&dir.path().join("missing"), &dest).is_err());
    }

    /// End to end over a real socket: what buddy-hook does, played by a client thread.
    #[cfg(unix)]
    mod socket {
        use super::*;
        use std::io::{BufRead, BufReader, Write};
        use std::os::unix::net::UnixStream;

        fn started() -> (Arc<SessionHub>, Receiver<Event>, PathBuf, tempfile::TempDir) {
            let (hub, rx, dir) = hub(DECISION_TIMEOUT);
            let socket = dir.path().join("hooks.sock");
            hub.start_at(server::Endpoint::Socket(socket.clone())).unwrap();
            // A second start is a no-op.
            hub.start_at(server::Endpoint::Socket(socket.clone())).unwrap();
            (hub, rx, socket, dir)
        }

        fn next_where(rx: &Receiver<Event>, pred: impl Fn(&Event) -> bool) -> Event {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                let event = rx.recv_timeout(left).expect("the expected event never came");
                if pred(&event) {
                    return super::bare(event);
                }
            }
        }

        #[test]
        fn the_socket_is_private() {
            use std::os::unix::fs::PermissionsExt;
            let (_hub, _rx, socket, dir) = started();
            assert_eq!(std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777, 0o700);
        }

        #[test]
        fn the_gate_asks_with_the_secret_and_refuses_without_it() {
            let (hub, rx, socket, _dir) = started();
            let ask = |token: String| {
                let socket = socket.clone();
                std::thread::spawn(move || {
                    let mut stream = UnixStream::connect(&socket).unwrap();
                    let line = json!({
                        "hook_event_name": "PreToolUse", "_gate": token, "session_id": "s", "cwd": "/tmp/x",
                        "tool_name": "Bash", "tool_input": { "command": "ls -la" }
                    });
                    stream.write_all(format!("{line}\n").as_bytes()).unwrap();
                    let mut answer = String::new();
                    BufReader::new(stream).read_line(&mut answer).unwrap();
                    answer
                })
            };
            assert_eq!(ask("adivinado".into()).join().unwrap(), "deny\n", "a wrong secret is refused at once");

            let client = ask(hub.gate_token().to_string());
            let Event::ApprovalRequest { request_id, agent, summary, .. } =
                next_where(&rx, |e| matches!(e, Event::ApprovalRequest { .. }))
            else {
                unreachable!()
            };
            assert_eq!((agent.as_str(), summary.as_str()), ("buddy", "ls -la"));
            hub.answer_approval(&request_id, true);
            assert_eq!(client.join().unwrap(), "allow\n");
            assert!(hub.sessions().is_empty(), "Buddy's own commands are not an outside session");
        }

        #[test]
        fn a_permission_request_waits_for_the_click() {
            let (hub, rx, socket, _dir) = started();
            let client = std::thread::spawn(move || {
                let mut stream = UnixStream::connect(&socket).unwrap();
                let line = json!({
                    "hook_event_name": "PermissionRequest", "_agent": "codex", "session_id": "abc",
                    "cwd": "/Users/a/code/buddy", "tool_name": "Bash", "tool_input": { "command": "cargo test" }
                });
                stream.write_all(format!("{line}\n").as_bytes()).unwrap();
                let mut answer = String::new();
                BufReader::new(stream).read_line(&mut answer).unwrap();
                answer
            });

            let Event::ApprovalRequest { request_id, session_id, agent, project, title, summary, detail, can_allow, .. } =
                next_where(&rx, |e| matches!(e, Event::ApprovalRequest { .. }))
            else {
                unreachable!()
            };
            assert_eq!((session_id.as_str(), agent.as_str(), project.as_str()), ("abc", "codex", "buddy"));
            assert_eq!((title.as_str(), summary.as_str(), detail.as_str()), ("Ejecutar un comando", "cargo test", "cargo test"));
            assert!(can_allow);
            assert_eq!(next_where(&rx, |_| true), Event::MascotState { state: "ask".into() });

            hub.answer_approval(&request_id, true);
            assert_eq!(client.join().unwrap(), "allow\n");
            assert_eq!(next_where(&rx, |_| true), Event::ApprovalClosed { request_id });
            assert_eq!(next_where(&rx, |_| true), Event::MascotState { state: "idle".into() });
            assert_eq!(hub.sessions()[0].state, "waiting");
        }

        #[test]
        fn a_deny_reaches_the_relay_too() {
            let (hub, rx, socket, _dir) = started();
            let client = std::thread::spawn(move || {
                let mut stream = UnixStream::connect(&socket).unwrap();
                stream.write_all(b"{\"hook_event_name\":\"PermissionRequest\",\"session_id\":\"s\"}\n").unwrap();
                let mut answer = String::new();
                BufReader::new(stream).read_line(&mut answer).unwrap();
                answer
            });
            let Event::ApprovalRequest { request_id, .. } = next_where(&rx, |e| matches!(e, Event::ApprovalRequest { .. }))
            else {
                unreachable!()
            };
            hub.answer_approval(&request_id, false);
            assert_eq!(client.join().unwrap(), "deny\n");
        }

        #[test]
        fn a_relay_that_disconnects_closes_the_approval() {
            let (_hub, rx, socket, _dir) = started();
            let mut stream = UnixStream::connect(&socket).unwrap();
            stream.write_all(b"{\"hook_event_name\":\"PermissionRequest\",\"session_id\":\"s\"}\n").unwrap();
            let Event::ApprovalRequest { request_id, .. } = next_where(&rx, |e| matches!(e, Event::ApprovalRequest { .. }))
            else {
                unreachable!()
            };
            drop(stream);
            assert_eq!(
                next_where(&rx, |e| matches!(e, Event::ApprovalClosed { .. })),
                Event::ApprovalClosed { request_id }
            );
        }

        #[test]
        fn a_fire_and_forget_stop_marks_the_session_done() {
            let (hub, rx, socket, _dir) = started();
            for event in ["UserPromptSubmit", "Stop"] {
                let mut stream = UnixStream::connect(&socket).unwrap();
                let line = json!({ "hook_event_name": event, "_agent": "claude", "session_id": "s9", "cwd": "/w/notch" });
                stream.write_all(format!("{line}\n").as_bytes()).unwrap();
                // The server closes without answering.
                let mut rest = String::new();
                BufReader::new(stream).read_line(&mut rest).unwrap();
                assert_eq!(rest, "");
            }
            assert_eq!(next_where(&rx, |_| true), update("s9", "claude", "notch", "working"));
            assert_eq!(next_where(&rx, |_| true), update("s9", "claude", "notch", "done"));
            assert_eq!(hub.sessions()[0].state, "done");
        }

        #[test]
        fn junk_is_ignored_and_the_server_keeps_serving() {
            let (_hub, rx, socket, _dir) = started();
            for junk in [&b"not json\n"[..], b"[1,2]\n", b"\n"] {
                let mut stream = UnixStream::connect(&socket).unwrap();
                stream.write_all(junk).unwrap();
                let mut rest = String::new();
                BufReader::new(stream).read_line(&mut rest).unwrap();
            }
            let mut stream = UnixStream::connect(&socket).unwrap();
            stream.write_all(b"{\"hook_event_name\":\"StopFailure\",\"session_id\":\"e\",\"cwd\":\"/x\"}\n").unwrap();
            assert_eq!(next_where(&rx, |_| true), update("e", "claude", "x", "error"));
        }
    }
}

#[cfg(test)]
mod place_tests {
    use super::*;
    use serde_json::json;
    use server::Sink;

    #[test]
    fn a_stop_carries_where_it_ran_and_what_it_said() {
        let dir = tempfile::tempdir().unwrap();
        let bus = Arc::new(EventBus::default());
        let rx = bus.subscribe();
        let hub = SessionHub::new(dir.path().to_path_buf(), bus);
        hub.event(json!({
            "hook_event_name": "Stop", "session_id": "s1", "cwd": "/p/buddy", "bundle_id": "com.mitchellh.ghostty",
            "last_assistant_message": "Listo.\n\nArreglé   el scroll."
        }));
        match rx.try_recv().unwrap() {
            Event::SessionUpdate { cwd, terminal, summary, state, .. } => {
                assert_eq!((cwd.as_str(), terminal.as_str(), state.as_str()), ("/p/buddy", "com.mitchellh.ghostty", "done"));
                assert_eq!(summary, "Listo. Arreglé el scroll.");
            }
            other => panic!("{other:?}"),
        }
    }
}
