//! Codex through the user's ChatGPT subscription: one `codex app-server` per conversation (JSON-RPC, one message
//! per line on stdio). It streams the answer word by word (`item/agentMessage/delta`), stops a turn with
//! `turn/interrupt` and keeps the thread between turns. Ported from MIKA's codex_server.rs (threads, not tokio).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use super::process;
use super::{Cancel, Failure, FailureKind, Provider, ProviderId, TokenCount, TurnEvent, TurnRequest};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_MODEL: &str = "gpt-6.1-sol";

/// How a Codex answer is laid out so the chat renders it like a Claude one.
const FORMAT: &str = "Formato: Markdown. Párrafos cortos; títulos ## solo si hay varias partes; listas con «- »; \
**negrita** para lo importante; tablas Markdown para comparar; código en bloques ``` con el lenguaje; \
enlaces [título](url). Sin HTML ni emojis decorativos.";

type Waiters = Arc<Mutex<HashMap<u64, Sender<Result<Value, String>>>>>;

pub struct Codex {
    exe: Option<PathBuf>,
    /// Live conversations by thread id.
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    /// Puts each command Codex wants to run in front of the user (only turns with the gate ask at all).
    approver: std::sync::OnceLock<super::Approver>,
}

impl Codex {
    pub fn new() -> Self {
        Self { exe: process::locate("codex"), sessions: Mutex::new(HashMap::new()), approver: std::sync::OnceLock::new() }
    }

    fn session(&self, exe: &std::path::Path, request: &TurnRequest) -> Result<Arc<Session>, String> {
        let connectors = crate::connectors::fingerprint(request.remote());
        if let Some(id) = &request.resume {
            // A live app-server holds the connectors (and keys) it started with: a change resumes in a new one.
            if let Some(s) = self.sessions.lock().unwrap().get(id).filter(|s| s.alive() && s.connectors == connectors) {
                return Ok(s.clone());
            }
        }
        let mut session = Session::spawn(exe, &request.workspace, self.approver.get().cloned(), &request.remote_env())?;
        session.connectors = connectors;
        session
            .request("initialize", json!({ "clientInfo": { "name": "buddy", "title": "Buddy", "version": env!("CARGO_PKG_VERSION") } }))?;
        session.notify("initialized");
        let resumed = request
            .resume
            .as_ref()
            .and_then(|id| {
                let mut params = thread_params(request);
                params["threadId"] = json!(id);
                session.request("thread/resume", params).ok()
            })
            .and_then(|r| r["thread"]["id"].as_str().map(str::to_string));
        session.thread_id = match resumed {
            Some(id) => id,
            None => session
                .request("thread/start", thread_params(request))?
                .get("thread")
                .and_then(|t| t["id"].as_str())
                .map(str::to_string)
                .ok_or("Codex no abrió la conversación.")?,
        };
        let session = Arc::new(session);
        self.sessions.lock().unwrap().insert(session.thread_id.clone(), session.clone());
        Ok(session)
    }
}

impl Default for Codex {
    fn default() -> Self {
        Self::new()
    }
}

/// Read-only sandbox and no approvals: Codex can think and search, but runs nothing and edits nothing (phase 4
/// opens that behind Buddy's approval gate).
pub fn thread_params(request: &TurnRequest) -> Value {
    let mut config = json!({ "web_search": if request.no_web { "disabled" } else { "live" } });
    if let Some(office) = &request.office {
        let mut server = office.server();
        // Codex hands MCP servers a bare environment: the music tools' variables go in the config (over stdin).
        let env: serde_json::Map<String, Value> = office.env().into_iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
        if !env.is_empty() {
            server["env"] = Value::Object(env);
        }
        config["mcp_servers"] = json!({ "buddy": server });
    }
    // The connectors: remote (streamable HTTP) servers. A key goes as `bearer_token_env_var`, read by Codex from its
    // own environment (`Session::spawn` sets it), never written in the config.
    for connector in request.remote() {
        let mut server = json!({ "url": connector.url });
        if connector.has_key() {
            server["bearer_token_env_var"] = json!(connector.env_var());
        }
        if !config["mcp_servers"].is_object() {
            config["mcp_servers"] = json!({});
        }
        config["mcp_servers"][connector.id.as_str()] = server;
    }
    if let Some(effort) = &request.effort {
        config["model_reasoning_effort"] = json!(effort);
    }
    let mut params = json!({ "cwd": request.workspace });
    if cfg!(target_os = "macos") {
        // A permission profile: Codex's commands read only what this agent may (attachments, its folders) plus the
        // system's minimum and Codex itself, write only in its workspace and editable folders, and have no network.
        // (The legacy read-only sandbox let them read the whole disk.)
        config["default_permissions"] = json!("buddy");
        config["permissions"] = json!({ "buddy": { "filesystem": filesystem_rules(request), "network": { "enabled": false } } });
    } else {
        // Windows: the legacy sandbox until permission profiles are checked there.
        let writable: Vec<&str> = request.folders.iter().filter(|f| f.can_edit).map(|f| f.path.as_str()).collect();
        if !writable.is_empty() {
            config["sandbox_workspace_write"] = json!({ "writable_roots": writable, "network_access": false });
        }
        params["sandbox"] = json!(if writable.is_empty() { "read-only" } else { "workspace-write" });
    }
    let extra = json!({
        // With the gate (the agent may run commands) Codex asks before anything not plainly read-only, and each
        // request becomes a card with Allow / Deny; without it, it never asks and runs nothing.
        "approvalPolicy": if request.gate.is_some() { "untrusted" } else { "never" },
        "developerInstructions": format!("{}\n\n{FORMAT}", request.system),
        "config": config,
    });
    for (key, value) in extra.as_object().into_iter().flatten() {
        params[key] = value.clone();
    }
    params["model"] = json!(request.model.as_deref().unwrap_or(DEFAULT_MODEL));
    params
}

/// The profile's filesystem table: path → "read" | "write".
pub fn filesystem_rules(request: &TurnRequest) -> Value {
    let mut rules = serde_json::Map::new();
    rules.insert(":minimal".into(), json!("read"));
    for dir in codex_install_dirs() {
        rules.insert(dir.to_string_lossy().into(), json!("read"));
    }
    for file in &request.attachments {
        if let Some(dir) = file.parent() {
            rules.insert(dir.to_string_lossy().into(), json!("read"));
        }
    }
    for folder in &request.folders {
        rules.insert(folder.path.clone(), json!(if folder.can_edit { "write" } else { "read" }));
    }
    rules.insert(request.workspace.to_string_lossy().into(), json!("write"));
    Value::Object(rules)
}

/// Where Codex itself lives (its sandbox helper re-runs the executable): the folder of the command on PATH and of
/// the real file behind it, and `~/.codex/packages` for the standalone installer's versions.
fn codex_install_dirs() -> Vec<PathBuf> {
    static DIRS: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();
    DIRS.get_or_init(|| {
        let Some(exe) = process::locate("codex") else { return Vec::new() };
        let mut dirs: Vec<PathBuf> = exe.parent().map(|d| vec![d.to_path_buf()]).unwrap_or_default();
        if let Ok(real) = std::fs::canonicalize(&exe) {
            let text = real.to_string_lossy().to_string();
            match text.find("/.codex/packages/") {
                Some(i) => dirs.push(PathBuf::from(&text[..i + "/.codex/packages".len()])),
                None => dirs.extend(real.parent().and_then(|bin| bin.parent()).map(|d| d.to_path_buf())),
            }
        }
        dirs.dedup();
        dirs
    })
    .clone()
}

impl Provider for Codex {
    fn id(&self) -> ProviderId {
        ProviderId::Codex
    }

    fn set_approver(&self, approver: super::Approver) {
        let _ = self.approver.set(approver);
    }

    fn sees_images(&self) -> bool {
        true
    }

    fn installed(&self) -> bool {
        self.exe.is_some()
    }

    fn run(&self, request: &TurnRequest, cancel: &Cancel, emit: &mut dyn FnMut(TurnEvent)) {
        let Some(exe) = &self.exe else {
            emit(TurnEvent::Failed(Failure { kind: FailureKind::Missing, message: "codex no está instalado".into() }));
            return;
        };
        let session = match self.session(exe, request) {
            Ok(s) => s,
            Err(e) => return emit(TurnEvent::Failed(Failure::new(e))),
        };
        emit(TurnEvent::Session(session.thread_id.clone()));
        session.drain();
        let turn = session.request(
            "turn/start",
            json!({ "threadId": session.thread_id, "input": turn_input(request),
                "model": request.model.as_deref().unwrap_or(DEFAULT_MODEL), "effort": request.effort }),
        );
        let turn_id = match turn.ok().and_then(|t| t["turn"]["id"].as_str().map(str::to_string)) {
            Some(id) => id,
            None => {
                return emit(TurnEvent::Failed(Failure::new("Codex no empezó la respuesta.")));
            }
        };
        let mut interrupted = false;
        loop {
            if cancel.is_cancelled() && !interrupted {
                interrupted = true;
                let _ = session.request("turn/interrupt", json!({ "threadId": session.thread_id, "turnId": turn_id }));
            }
            let (method, params) = match session.notes.lock().unwrap().recv_timeout(Duration::from_millis(100)) {
                Ok(note) => note,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    self.sessions.lock().unwrap().remove(&session.thread_id);
                    return emit(if interrupted { TurnEvent::Done } else { TurnEvent::Failed(Failure::new("Codex se cerró.")) });
                }
            };
            for event in turn_events(&method, &params, &turn_id) {
                let end = matches!(event, TurnEvent::Done | TurnEvent::Failed(_));
                emit(event);
                if end {
                    return;
                }
            }
        }
    }
}

/// The text, plus attached images as `localImage` items (other files are named in the text and read from disk).
pub fn turn_input(request: &TurnRequest) -> Value {
    let mut input = vec![json!({ "type": "text", "text": request.prompt, "text_elements": [] })];
    for path in request.images() {
        input.push(json!({ "type": "localImage", "path": path }));
    }
    Value::Array(input)
}

/// What one app-server notification means for the turn `turn_id`.
pub fn turn_events(method: &str, params: &Value, turn_id: &str) -> Vec<TurnEvent> {
    match method {
        "item/agentMessage/delta" => match params["delta"].as_str() {
            Some(d) if !d.is_empty() => vec![TurnEvent::Delta(d.into())],
            _ => vec![],
        },
        "item/started" => {
            let item = &params["item"];
            match item["type"].as_str() {
                Some("webSearch") => vec![TurnEvent::Tool {
                    name: "WebSearch".into(),
                    summary: item["query"].as_str().unwrap_or("").chars().take(160).collect(),
                }],
                Some("commandExecution") => vec![TurnEvent::Tool { name: "Bash".into(), summary: String::new() }],
                Some("fileChange") => vec![TurnEvent::Tool { name: "Edit".into(), summary: String::new() }],
                Some("mcpToolCall") => vec![TurnEvent::Tool {
                    name: format!("mcp__{}__{}", item["server"].as_str().unwrap_or(""), item["tool"].as_str().unwrap_or("")),
                    summary: String::new(),
                }],
                _ => vec![],
            }
        }
        "turn/completed" if params["turn"]["id"].as_str() == Some(turn_id) => match params["turn"]["status"].as_str() {
            Some("failed") => {
                let message = params["turn"]["error"]["message"].as_str().unwrap_or("Codex no pudo responder.");
                vec![TurnEvent::Failed(Failure::new(message))]
            }
            _ => vec![TurnEvent::Done],
        },
        "account/rateLimits/updated" => vec![TurnEvent::Usage(params.clone())],
        "thread/tokenUsage/updated" if params["turnId"].as_str() == Some(turn_id) => {
            let last = &params["tokenUsage"]["last"];
            let n = |k: &str| last[k].as_i64().unwrap_or(0);
            vec![TurnEvent::Tokens(TokenCount {
                input: n("inputTokens") - n("cachedInputTokens"),
                output: n("outputTokens") + n("reasoningOutputTokens"),
                cached: n("cachedInputTokens"),
                cost_usd: None,
                model: None,
            })]
        }
        "error" if params["willRetry"] != true => {
            let message = params["error"]["message"].as_str().unwrap_or("Codex tuvo un error.");
            vec![TurnEvent::Failed(Failure::new(message))]
        }
        _ => vec![],
    }
}

/// One app-server process and its thread.
struct Session {
    child: Arc<Mutex<Child>>,
    stdin: Arc<Mutex<ChildStdin>>,
    waiters: Waiters,
    next_id: AtomicU64,
    notes: Mutex<Receiver<(String, Value)>>,
    thread_id: String,
    /// The connectors it started with (`connectors::fingerprint`).
    connectors: String,
}

impl Session {
    fn spawn(exe: &std::path::Path, cwd: &std::path::Path, approver: Option<super::Approver>, env: &[(String, String)]) -> Result<Self, String> {
        let _ = std::fs::create_dir_all(cwd);
        let mut cmd = process::command(exe);
        cmd.env(super::OWN_RUN_ENV, "1");
        // The connectors' keys (`bearer_token_env_var`).
        for (key, value) in env {
            cmd.env(key, value);
        }
        cmd.arg("app-server").current_dir(cwd);
        let mut child = cmd.spawn().map_err(|e| format!("No se pudo iniciar Codex: {e}"))?;
        let stdin = child.stdin.take().ok_or("sin stdin")?;
        let stdout = child.stdout.take().ok_or("sin stdout")?;
        let stderr = child.stderr.take().ok_or("sin stderr")?;
        std::thread::spawn(move || for _ in BufReader::new(stderr).lines() {});

        let waiters: Waiters = Arc::default();
        let (notes_tx, notes_rx) = channel();
        let child = Arc::new(Mutex::new(child));
        let stdin = Arc::new(Mutex::new(stdin));
        let session = Self {
            child: child.clone(),
            stdin: stdin.clone(),
            waiters: waiters.clone(),
            next_id: AtomicU64::new(1),
            notes: Mutex::new(notes_rx),
            thread_id: String::new(),
            connectors: String::new(),
        };
        // The reader answers the server's own requests itself, through the shared stdin.
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                match (msg.get("id"), msg["method"].as_str()) {
                    // A command to approve: the user decides on their own thread (the card waits for a click),
                    // so this reader keeps delivering everything else meanwhile.
                    (Some(id), Some("item/commandExecution/requestApproval")) if approver.is_some() => {
                        let (approver, stdin, id) = (approver.clone().unwrap(), stdin.clone(), id.clone());
                        let (command, folder) = approval_subject(&msg["params"]);
                        std::thread::spawn(move || {
                            let decision = if approver(&command, &folder) { "accept" } else { "decline" };
                            let mut out = stdin.lock().unwrap();
                            let _ = writeln!(out, "{}", json!({ "id": id, "result": { "decision": decision } })).and_then(|_| out.flush());
                        });
                    }
                    (Some(id), Some(method)) => {
                        let mut out = stdin.lock().unwrap();
                        let _ = writeln!(out, "{}", server_request_reply(id.clone(), method)).and_then(|_| out.flush());
                    }
                    (Some(id), None) => {
                        if let Some(waiter) = id.as_u64().and_then(|id| waiters.lock().unwrap().remove(&id)) {
                            let result = if msg["error"].is_null() {
                                Ok(msg["result"].clone())
                            } else {
                                Err(msg["error"]["message"].as_str().unwrap_or("Codex rechazó la petición.").to_string())
                            };
                            let _ = waiter.send(result);
                        }
                    }
                    (None, Some(method)) => {
                        let _ = notes_tx.send((method.to_string(), msg["params"].clone()));
                    }
                    _ => {}
                }
            }
            waiters.lock().unwrap().clear();
            let _ = child.lock().unwrap().kill();
        });
        Ok(session)
    }

    fn write(&self, value: Value) -> Result<(), String> {
        let mut stdin = self.stdin.lock().unwrap();
        writeln!(stdin, "{value}").and_then(|_| stdin.flush()).map_err(|_| "Codex se cerró.".to_string())
    }

    fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = channel();
        self.waiters.lock().unwrap().insert(id, tx);
        self.write(json!({ "id": id, "method": method, "params": params }))?;
        match rx.recv_timeout(REQUEST_TIMEOUT) {
            Ok(result) => result,
            Err(RecvTimeoutError::Disconnected) => Err("Codex se cerró.".into()),
            Err(RecvTimeoutError::Timeout) => {
                self.waiters.lock().unwrap().remove(&id);
                Err("Codex tardó demasiado en responder.".into())
            }
        }
    }

    fn notify(&self, method: &str) {
        let _ = self.write(json!({ "method": method }));
    }

    /// Leftovers of an earlier, interrupted turn must not leak into the next one.
    fn drain(&self) {
        while self.notes.lock().unwrap().try_recv().is_ok() {}
    }

    fn alive(&self) -> bool {
        matches!(self.child.lock().unwrap().try_wait(), Ok(None))
    }
}

/// The command line and folder of an approval request (`command` as text or as argv).
pub fn approval_subject(params: &Value) -> (String, String) {
    let command = match &params["command"] {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "),
        _ => params["reason"].as_str().unwrap_or("un comando").to_string(),
    };
    (command, params["cwd"].as_str().unwrap_or("").to_string())
}

/// Deny by default: approvals are declined (the sandbox is read-only, so none should come) and anything else the
/// server asks gets an error. Never `acceptForSession`.
pub fn server_request_reply(id: Value, method: &str) -> Value {
    match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            json!({ "id": id, "result": { "decision": "decline" } })
        }
        "item/permissions/requestApproval" => {
            json!({ "id": id, "result": { "permissions": {}, "scope": "turn" } })
        }
        _ => {
            json!({ "id": id, "error": { "code": -32601, "message": "Buddy no atiende esta petición." } })
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.lock().unwrap().kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buddys_mcp_server_gets_its_variables_in_the_config() {
        let request = TurnRequest {
            office: Some(crate::providers::Office {
                relay: "/d/buddy-hook".into(),
                dir: "/d/documentos".into(),
                skills: "/d/skills".into(),
                link: Some(crate::providers::Link { token: "secreto".into(), data_dir: "/d".into() }),
                read: vec![],
            }),
            ..Default::default()
        };
        let server = &thread_params(&request)["config"]["mcp_servers"]["buddy"];
        assert_eq!(server["env"]["BUDDY_GATE_TOKEN"], "secreto");
        assert_eq!(server["env"]["BUDDY_DATA_DIR"], "/d");
        assert!(server["args"].to_string().contains("/d/skills"));
    }

    #[test]
    fn connectors_are_remote_servers_with_the_key_in_the_environment() {
        let request = TurnRequest {
            connectors: vec![
                crate::connectors::Connector::new("context7", "https://mcp.context7.com/mcp", Some("ctx7sk-secreto".into())),
                crate::connectors::Connector::new("deepwiki", "https://mcp.deepwiki.com/mcp", None),
            ],
            ..Default::default()
        };
        let params = thread_params(&request);
        let servers = &params["config"]["mcp_servers"];
        assert_eq!(servers["context7"], json!({ "url": "https://mcp.context7.com/mcp", "bearer_token_env_var": "BUDDY_CONNECTOR_CONTEXT7_KEY" }));
        assert_eq!(servers["deepwiki"], json!({ "url": "https://mcp.deepwiki.com/mcp" }));
        assert!(!params.to_string().contains("secreto"), "the key is read from Codex's environment");
        let offline = thread_params(&TurnRequest { no_web: true, ..request });
        assert!(offline["config"].get("mcp_servers").is_none(), "no web, no connectors");
    }

    #[test]
    fn notifications_become_turn_events() {
        assert_eq!(turn_events("item/agentMessage/delta", &json!({ "delta": "Hola" }), "t"), vec![TurnEvent::Delta("Hola".into())]);
        assert_eq!(
            turn_events("item/started", &json!({ "item": { "type": "webSearch", "query": "clima" } }), "t"),
            vec![TurnEvent::Tool { name: "WebSearch".into(), summary: "clima".into() }]
        );
        assert_eq!(turn_events("turn/completed", &json!({ "turn": { "id": "t", "status": "completed" } }), "t"), vec![TurnEvent::Done]);
        assert!(turn_events("turn/completed", &json!({ "turn": { "id": "other", "status": "completed" } }), "t").is_empty());
        assert!(matches!(
            &turn_events("turn/completed", &json!({ "turn": { "id": "t", "status": "failed", "error": { "message": "usage limit" } } }), "t")[0],
            TurnEvent::Failed(f) if f.kind == FailureKind::Limit
        ));
        assert!(turn_events("error", &json!({ "willRetry": true, "error": { "message": "x" } }), "t").is_empty());
    }

    #[test]
    fn images_go_as_local_images() {
        let input = turn_input(&TurnRequest {
            prompt: "mira".into(),
            attachments: vec!["/a/foto.PNG".into(), "/a/notas.txt".into()],
            ..Default::default()
        });
        assert_eq!(input.as_array().unwrap().len(), 2);
        assert_eq!(input[1]["type"], "localImage");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn commands_read_only_what_the_agent_may() {
        let request = TurnRequest {
            workspace: "/d/agents/buddy/workspace".into(),
            attachments: vec!["/d/adjuntos/c1/foto.png".into()],
            folders: vec![
                crate::folders::AuthorizedFolder { path: "/u/docs".into(), can_edit: false },
                crate::folders::AuthorizedFolder { path: "/u/proyecto".into(), can_edit: true },
            ],
            ..Default::default()
        };
        let p = thread_params(&request);
        assert!(p.get("sandbox").is_none(), "a profile never mixes with the legacy sandbox");
        assert_eq!(p["config"]["default_permissions"], "buddy");
        let fs = &p["config"]["permissions"]["buddy"]["filesystem"];
        assert_eq!(fs[":minimal"], "read");
        assert_eq!(fs["/d/adjuntos/c1"], "read");
        assert_eq!(fs["/u/docs"], "read");
        assert_eq!(fs["/u/proyecto"], "write");
        assert_eq!(fs["/d/agents/buddy/workspace"], "write");
        assert!(fs.get("/u").is_none() && fs.get("/").is_none());
        assert_eq!(p["config"]["permissions"]["buddy"]["network"]["enabled"], false);
    }

    #[test]
    fn approvals_name_the_command_and_follow_the_gate() {
        assert_eq!(approval_subject(&json!({ "command": "ls -la", "cwd": "/u" })), ("ls -la".into(), "/u".into()));
        assert_eq!(approval_subject(&json!({ "command": ["git", "status"] })).0, "git status");
        let gated = TurnRequest {
            gate: Some(crate::providers::Gate { relay: "/r".into(), token: "t".into(), data_dir: "/d".into() }),
            ..Default::default()
        };
        assert_eq!(thread_params(&gated)["approvalPolicy"], "untrusted");
        assert_eq!(thread_params(&TurnRequest::default())["approvalPolicy"], "never");
    }

    #[test]
    fn the_server_never_gets_an_approval() {
        assert_eq!(server_request_reply(json!(1), "item/commandExecution/requestApproval")["result"]["decision"], "decline");
        assert_eq!(server_request_reply(json!(2), "item/fileChange/requestApproval")["result"]["decision"], "decline");
        assert!(server_request_reply(json!(3), "account/chatgptAuthTokens/refresh")["error"].is_object());
    }

    #[test]
    fn threads_are_read_only_with_buddys_instructions() {
        let p = thread_params(&TurnRequest { system: "Eres Buddy".into(), effort: Some("low".into()), ..Default::default() });
        if cfg!(target_os = "macos") {
            assert_eq!(p["config"]["default_permissions"], "buddy", "a profile instead of the legacy sandbox");
        } else {
            assert_eq!(p["sandbox"], "read-only");
        }
        assert_eq!(p["approvalPolicy"], "never");
        assert!(p["developerInstructions"].as_str().unwrap().starts_with("Eres Buddy"));
        assert_eq!(p["config"]["model_reasoning_effort"], "low");
        assert_eq!(p["model"], "gpt-6.1-sol", "Buddy explicitly chooses the requested fallback model");
    }
}
