//! Sessions: what Claude Code and Codex are doing outside Buddy, from their own hooks.
//!
//! The agents run `buddy-hook` (the `hook/` crate) on every hook event; the relay hands each event to the local
//! server here (`server`: a Unix socket on Mac, a named pipe on Windows). `SessionHub` turns events into
//! `SessionUpdate`s for the notch / top bar, and a `PermissionRequest` into an `ApprovalRequest` card whose answer
//! (`answer_approval`) goes back to the waiting agent. The hooks are written into the agents' config files only
//! after the user saw the diff and clicked (`hooks_preview` → `hooks_write`; see `hook_file`).

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
    /// `claude` or `codex`.
    pub agent: String,
    /// `Claude Code` or `Codex`.
    pub name: String,
    /// Buddy's hooks are in the agent's config file.
    pub installed: bool,
    /// The agent's config folder exists (`~/.claude`, `$CODEX_HOME` or `~/.codex`).
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
    /// `claude` or `codex`.
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

/// `claude` or `codex`, from the relay's `_agent` tag (anything else is Claude Code, the relay's default).
fn agent_of(payload: &Value) -> &'static str {
    match payload.get("_agent").and_then(Value::as_str) {
        Some("codex") => "codex",
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
}

#[derive(Default)]
struct Inner {
    /// Keyed by (agent, session id).
    sessions: HashMap<(String, String), SessionInfo>,
    pending: HashMap<String, Pending>,
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
}

impl SessionHub {
    pub fn new(data_dir: PathBuf, bus: Arc<EventBus>) -> Self {
        Self::with_timeout(data_dir, bus, DECISION_TIMEOUT)
    }

    /// Tests use a short decision timeout.
    pub(crate) fn with_timeout(data_dir: PathBuf, bus: Arc<EventBus>, decision_timeout: Duration) -> Self {
        Self {
            data_dir,
            bus,
            inner: Mutex::new(Inner::default()),
            started: AtomicBool::new(false),
            next_id: AtomicU64::new(1),
            decision_timeout,
            gate_token: random_token(),
        }
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
        ]
    }

    fn hook_file(&self, agent: &str) -> Result<hook_file::HookFile, CoreError> {
        let relay = self.relay_path();
        match agent {
            "claude" => claude_hooks::file(&relay),
            "codex" => codex_hooks::file(&relay),
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
                    summary: if state == "done" { place.summary.clone() } else { String::new() },
                })
            }
        };
        if let Some(event) = event {
            self.bus.publish(event);
        }
    }
}

impl server::Sink for SessionHub {
    fn event(&self, payload: Value) {
        let event = text(&payload, "hook_event_name");
        let agent = agent_of(&payload);
        // Never the payload itself: tool inputs may hold secrets.
        log::line(format!("gancho {event} ({agent})"));
        if let Some(state) = state_for(event) {
            self.update_session(agent, text(&payload, "session_id"), &project_of(text(&payload, "cwd")), state, &Place::of(&payload));
        }
    }

    fn permission(&self, payload: Value, closed: &dyn Fn() -> bool) -> Option<&'static str> {
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
}

impl SessionHub {
    /// Puts an approval card in front of the user and waits for the click (or the timeout, or the asker hanging up).
    fn ask(&self, payload: &Value, agent: &str, session_id: String, project: String, closed: &dyn Fn() -> bool) -> Option<&'static str> {
        let shown = format::approval_text(payload);
        let request_id = format!("{}-{}", std::process::id(), self.next_id.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = channel();
        self.lock().pending.insert(request_id.clone(), Pending { answer: tx, can_allow: shown.can_allow });
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

            let Event::ApprovalRequest { request_id, session_id, agent, project, title, summary, detail, can_allow } =
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
