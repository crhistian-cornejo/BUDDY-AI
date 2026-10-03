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
}

impl Codex {
    pub fn new() -> Self {
        Self { exe: process::locate("codex"), sessions: Mutex::new(HashMap::new()) }
    }

    fn session(&self, exe: &std::path::Path, request: &TurnRequest) -> Result<Arc<Session>, String> {
        if let Some(id) = &request.resume {
            if let Some(s) = self.sessions.lock().unwrap().get(id).filter(|s| s.alive()) {
                return Ok(s.clone());
            }
        }
        let mut session = Session::spawn(exe, &request.workspace)?;
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
    if let Some(effort) = &request.effort {
        config["model_reasoning_effort"] = json!(effort);
    }
    // Editable authorized folders become the only writable roots; otherwise nothing is writable.
    let writable: Vec<&str> = request.folders.iter().filter(|f| f.can_edit).map(|f| f.path.as_str()).collect();
    if !writable.is_empty() {
        config["sandbox_workspace_write"] = json!({ "writable_roots": writable, "network_access": false });
    }
    let mut params = json!({
        "cwd": request.workspace,
        "sandbox": if writable.is_empty() { "read-only" } else { "workspace-write" },
        "approvalPolicy": "never",
        "developerInstructions": format!("{}\n\n{FORMAT}", request.system),
        "config": config,
    });
    params["model"] = json!(request.model.as_deref().unwrap_or(DEFAULT_MODEL));
    params
}

impl Provider for Codex {
    fn id(&self) -> ProviderId {
        ProviderId::Codex
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
}

impl Session {
    fn spawn(exe: &std::path::Path, cwd: &std::path::Path) -> Result<Self, String> {
        let _ = std::fs::create_dir_all(cwd);
        let mut cmd = process::command(exe);
        cmd.env(super::OWN_RUN_ENV, "1");
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
        };
        // The reader answers the server's own requests itself, through the shared stdin.
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(msg) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                match (msg.get("id"), msg["method"].as_str()) {
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
            }),
            ..Default::default()
        };
        let server = &thread_params(&request)["config"]["mcp_servers"]["buddy"];
        assert_eq!(server["env"]["BUDDY_GATE_TOKEN"], "secreto");
        assert_eq!(server["env"]["BUDDY_DATA_DIR"], "/d");
        assert!(server["args"].to_string().contains("/d/skills"));
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
    fn the_server_never_gets_an_approval() {
        assert_eq!(server_request_reply(json!(1), "item/commandExecution/requestApproval")["result"]["decision"], "decline");
        assert_eq!(server_request_reply(json!(2), "item/fileChange/requestApproval")["result"]["decision"], "decline");
        assert!(server_request_reply(json!(3), "account/chatgptAuthTokens/refresh")["error"].is_object());
    }

    #[test]
    fn threads_are_read_only_with_buddys_instructions() {
        let p = thread_params(&TurnRequest { system: "Eres Buddy".into(), effort: Some("low".into()), ..Default::default() });
        assert_eq!(p["sandbox"], "read-only");
        assert_eq!(p["approvalPolicy"], "never");
        assert!(p["developerInstructions"].as_str().unwrap().starts_with("Eres Buddy"));
        assert_eq!(p["config"]["model_reasoning_effort"], "low");
        assert_eq!(p["model"], "gpt-6.1-sol", "Buddy explicitly chooses the requested fallback model");
    }
}
