//! One `codex app-server` per agent conversation (stdio, one JSON-RPC message per line). It streams the answer
//! word by word (`item/agentMessage/delta`), reports images (`imageGeneration`), stops a turn with
//! `turn/interrupt` and keeps the thread between turns. Twin of CodexClient.swift on macOS.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use super::agent_gate::CommandRun;
use super::agent_tools::{self, CodexPlan};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

type Waiters = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

pub struct CodexServer {
    child: Mutex<Option<tokio::process::Child>>,
    out: mpsc::UnboundedSender<String>,
    waiters: Waiters,
    next_id: AtomicU64,
    /// Notifications for whoever runs the current turn (one at a time per conversation).
    notes: tokio::sync::Mutex<mpsc::UnboundedReceiver<(String, Value)>>,
    pub thread_id: String,
    /// The Stop flag of the turn that is running (a card waiting for a click is taken down when it is set).
    cancel: Arc<Mutex<Arc<AtomicBool>>>,
}

/// How a Codex answer is laid out so the chat renders it like a Claude one (twin of `codexFormat` on macOS).
const CODEX_FORMAT: &str = "Formato de tus respuestas (la app las muestra con Markdown): usa párrafos cortos separados por una línea en blanco; títulos con ## o ### solo si la respuesta tiene varias partes; listas con «- » o «1. » (un elemento por línea, nunca viñetas «•» ni listas en una sola línea); **negrita** para lo importante; tablas Markdown con fila de encabezado y separador |---| cuando comparas datos; bloques de código con ``` y el lenguaje; enlaces como [título](url). No uses HTML, ni emojis decorativos, ni líneas de adornos. Esto prevalece sobre cualquier indicación anterior de «texto plano».";

/// What a conversation is started with. The capabilities of the agent decide the sandbox, the web search and the
/// MIKA tools (see `agent_tools::codex_plan`); the shell is never on.
pub struct ThreadOptions<'a> {
    pub cwd: &'a str,
    pub model: &'a str,
    pub effort: &'a str,
    pub instructions: &'a str,
    pub plan: &'a CodexPlan,
    /// For an agent that may run commands: where the question "may this run?" goes.
    pub gate: Option<GateCtx>,
}

/// What a `run_command` call returns: the text for the model, whether it counts as a success, and the run to show
/// in the chat (None when nothing ran).
pub type CommandOutcome = (String, bool, Option<CommandRun>);
pub type CommandFuture = std::pin::Pin<Box<dyn std::future::Future<Output = CommandOutcome> + Send>>;

/// How a `run_command` call is served: asks the user, and only on Allow runs it. A closure (built where the app
/// handle lives, subscription.rs) so this file knows nothing about windows; it gets the arguments of the call and
/// the Stop flag of the turn.
#[derive(Clone)]
pub struct GateCtx {
    pub run: Arc<dyn Fn(Value, Arc<AtomicBool>) -> CommandFuture + Send + Sync>,
}

/// How MIKA answers a request the server sends us.
#[derive(Debug, PartialEq)]
pub enum Handling {
    /// An immediate JSON-RPC result (a refusal, for the approval requests).
    Result(Value),
    /// A file tool of ours (`list_files` / `read_file`) or PARLEY's odds reader: computed at once.
    File { tool: String, args: Value },
    /// `run_command`: asks the user first.
    RunCommand(Value),
    /// Not something MIKA handles: an error answer.
    Unsupported,
}

/// What each kind of server request gets. Deny by default: every approval request of Codex is declined (the shell
/// is off, so none should come; an edit outside the workspace or a permission grant is refused all the same), and
/// a tool call is served only when the agent's plan has that tool. Never `acceptForSession`.
pub fn classify(method: &str, params: &Value, tools: &[&str], odds_reader: bool) -> Handling {
    match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => Handling::Result(json!({ "decision": "decline" })),
        // The older request names, still accepted by some versions.
        "execCommandApproval" | "applyPatchApproval" => Handling::Result(json!({ "decision": "denied" })),
        "item/permissions/requestApproval" => Handling::Result(json!({ "permissions": {}, "scope": "turn" })),
        "mcpServer/elicitation/request" => Handling::Result(json!({ "action": "decline" })),
        "item/tool/call" => {
            let tool = params["tool"].as_str().unwrap_or("");
            let args = params["arguments"].clone();
            if tool == "read_parley_odds" && odds_reader { return Handling::File { tool: tool.into(), args }; }
            if !tools.contains(&tool) { return Handling::Unsupported; }
            match tool {
                "run_command" => Handling::RunCommand(args),
                "list_files" | "read_file" => Handling::File { tool: tool.into(), args },
                _ => Handling::Unsupported,
            }
        }
        _ => Handling::Unsupported,
    }
}

fn tool_reply(id: &Value, success: bool, text: &str) -> String {
    json!({ "id": id, "result": { "success": success, "contentItems": [{ "type": "inputText", "text": text }] } }).to_string()
}

impl CodexServer {
    /// Starts the server, does the handshake and opens a thread. `cmd` is the `codex` command with a clean
    /// environment; this adds the arguments.
    pub async fn start(mut cmd: tokio::process::Command, options: ThreadOptions<'_>) -> Result<Arc<Self>, String> {
        // MIKA's own fixed settings: web search as the capabilities say, ChatGPT login only, no shell, no MCP
        // servers or apps from the user's Codex configuration.
        cmd.args(agent_tools::codex_server_args(options.plan.web_search));
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        let mut child = cmd.spawn().map_err(|_| "No se pudo iniciar Codex.")?;
        let mut stdin = child.stdin.take().ok_or("No se pudo iniciar Codex.")?;
        let stdout = child.stdout.take().ok_or("No se pudo iniciar Codex.")?;

        let (out, mut out_rx) = mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            while let Some(line) = out_rx.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err() || stdin.write_all(b"\n").await.is_err() { break; }
                let _ = stdin.flush().await;
            }
        });

        let waiters: Waiters = Arc::default();
        let (notes_tx, notes_rx) = mpsc::unbounded_channel();
        let reader_waiters = waiters.clone();
        let replies = out.clone();
        let workspace = std::path::PathBuf::from(options.cwd);
        let tools: Vec<&'static str> = options.plan.tools.clone();
        let odds_reader = options.plan.odds_reader;
        let gate = options.gate.clone();
        let cancel: Arc<Mutex<Arc<AtomicBool>>> = Arc::new(Mutex::new(Arc::new(AtomicBool::new(false))));
        let reader_cancel = cancel.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
                // A request from the server (JSON-RPC IDs may be strings as well as numbers).
                if !message["id"].is_null() && message["method"].is_string() {
                    let id = message["id"].clone();
                    let method = message["method"].as_str().unwrap_or("");
                    match classify(method, &message["params"], &tools, odds_reader) {
                        Handling::Result(result) => { let _ = replies.send(json!({ "id": id, "result": result }).to_string()); }
                        Handling::Unsupported => { let _ = replies.send(json!({ "id": id, "error": { "code": -32601, "message": "MIKA no soporta esta petición" } }).to_string()); }
                        Handling::File { tool, args } => {
                            let result = match tool.as_str() {
                                "list_files" => agent_tools::list_files(&workspace, &args),
                                "read_file" => agent_tools::read_file(&workspace, &args),
                                _ => super::odds_reader::read_page(&workspace, &args),
                            };
                            let success = result.is_ok();
                            let text = result.map(|v| v.to_string()).unwrap_or_else(|e| e);
                            let _ = replies.send(tool_reply(&id, success, &text));
                        }
                        Handling::RunCommand(args) => {
                            // The question to the user takes as long as the user takes: the reader keeps reading.
                            let (replies, notes) = (replies.clone(), notes_tx.clone());
                            let Some(ctx) = gate.clone() else { let _ = replies.send(tool_reply(&id, false, "Este agente no puede ejecutar comandos.")); continue };
                            let current = message["params"]["turnId"].as_str().map(str::to_string);
                            let cancel = reader_cancel.lock().unwrap().clone();
                            tokio::spawn(async move {
                                if current.is_none() {
                                    let _ = replies.send(tool_reply(&id, false, "La petición no trae un turno: el comando no se ejecutó."));
                                    return;
                                }
                                let (text, success, run) = (ctx.run)(args, cancel).await;
                                let _ = replies.send(tool_reply(&id, success, &text));
                                if let Some(run) = run {
                                    let _ = notes.send(("mika/commandOutput".into(), json!({ "turnId": current, "command": run.command, "output": run.output, "code": run.code })));
                                }
                            });
                        }
                    }
                    continue;
                }
                let id = message["id"].as_u64();
                match (id, message["method"].as_str()) {
                    (Some(id), None) => {
                        if let Some(waiter) = reader_waiters.lock().unwrap().remove(&id) {
                            let result = if message["error"].is_null() { Ok(message["result"].clone()) } else {
                                Err(message["error"]["message"].as_str().unwrap_or("Codex rechazó la petición.").to_string())
                            };
                            let _ = waiter.send(result);
                        }
                    }
                    (None, Some(method)) => { let _ = notes_tx.send((method.to_string(), message["params"].clone())); }
                    _ => {}
                }
            }
            // The process ended: every open request fails now instead of waiting for its timeout.
            reader_waiters.lock().unwrap().clear();
        });

        let mut server = CodexServer {
            child: Mutex::new(Some(child)), out, waiters, next_id: AtomicU64::new(1),
            notes: tokio::sync::Mutex::new(notes_rx), thread_id: String::new(), cancel,
        };
        let experimental = options.plan.odds_reader || !options.plan.tools.is_empty();
        server.request("initialize", json!({ "clientInfo": { "name": "mika", "title": "MIKA", "version": env!("CARGO_PKG_VERSION") }, "capabilities":{"experimentalApi":experimental} })).await?;
        let _ = server.out.send(json!({ "method": "initialized" }).to_string());
        let params = agent_tools::codex_thread_params(options.cwd, options.model, options.effort, &format!("{}

{}", options.instructions, CODEX_FORMAT), options.plan);
        let thread = server.request("thread/start", params).await?;
        server.thread_id = thread["thread"]["id"].as_str().ok_or("Codex no abrió la conversación.")?.to_string();
        Ok(Arc::new(server))
    }

    pub async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.waiters.lock().unwrap().insert(id, tx);
        if self.out.send(json!({ "id": id, "method": method, "params": params }).to_string()).is_err() {
            self.waiters.lock().unwrap().remove(&id);
            return Err("Codex se cerró.".into());
        }
        match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("Codex se cerró.".into()),
            Err(_) => { self.waiters.lock().unwrap().remove(&id); Err("Codex tardó demasiado en responder.".into()) }
        }
    }

    /// `images` go with the text as `localImage` items (PARLEY's Telegram screenshots).
    pub async fn start_turn(&self, text: &str, images: &[std::path::PathBuf], effort: &str, cancel: Arc<AtomicBool>) -> Result<String, String> {
        // Leftovers of an earlier, interrupted turn must not leak into this one.
        { let mut notes = self.notes.lock().await; while notes.try_recv().is_ok() {} }
        *self.cancel.lock().unwrap() = cancel;
        let mut input = vec![json!({ "type": "text", "text": text, "text_elements": [] })];
        input.extend(images.iter().map(|path| json!({ "type": "localImage", "path": path })));
        let turn = self.request("turn/start", json!({
            "threadId": self.thread_id,
            "input": input,
            "effort": effort,
        })).await?;
        turn["turn"]["id"].as_str().map(str::to_string).ok_or_else(|| "Codex no empezó la respuesta.".into())
    }

    /// The next notification, or None after `wait` (or when the server is gone and nothing is left).
    pub async fn next_note(&self, wait: Duration) -> Option<Option<(String, Value)>> {
        let mut notes = self.notes.lock().await;
        match tokio::time::timeout(wait, notes.recv()).await {
            Ok(Some(note)) => Some(Some(note)),
            Ok(None) => Some(None),
            Err(_) => None,
        }
    }

    pub async fn interrupt(&self, turn_id: &str) {
        let _ = self.request("turn/interrupt", json!({ "threadId": self.thread_id, "turnId": turn_id })).await;
    }

    pub fn shutdown(&self) {
        if let Some(mut child) = self.child.lock().unwrap().take() { let _ = child.start_kill(); }
    }
}

impl Drop for CodexServer {
    fn drop(&mut self) { self.shutdown(); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    const TOOLS: [&str; 3] = ["list_files", "read_file", "run_command"];
    fn call(tool: &str) -> Value { json!({ "threadId": "t", "turnId": "u", "callId": "c", "tool": tool, "arguments": { "command": "dir", "path": "a.txt" } }) }

    #[test]
    fn every_approval_request_of_codex_is_declined() {
        let declined = |method: &str| classify(method, &json!({ "itemId": "i", "threadId": "t", "turnId": "u", "command": "rm -rf /" }), &TOOLS, false);
        assert_eq!(declined("item/commandExecution/requestApproval"), Handling::Result(json!({ "decision": "decline" })));
        assert_eq!(declined("item/fileChange/requestApproval"), Handling::Result(json!({ "decision": "decline" })));
        assert_eq!(declined("execCommandApproval"), Handling::Result(json!({ "decision": "denied" })));
        assert_eq!(declined("applyPatchApproval"), Handling::Result(json!({ "decision": "denied" })));
        assert_eq!(declined("item/permissions/requestApproval"), Handling::Result(json!({ "permissions": {}, "scope": "turn" })));
        assert_eq!(declined("mcpServer/elicitation/request"), Handling::Result(json!({ "action": "decline" })));
        // Nothing ever answers with an acceptance.
        for method in ["item/commandExecution/requestApproval", "item/fileChange/requestApproval", "execCommandApproval", "applyPatchApproval", "item/permissions/requestApproval", "mcpServer/elicitation/request"] {
            let text = match declined(method) { Handling::Result(v) => v.to_string(), other => panic!("{other:?}") };
            assert!(!text.contains("accept") && !text.contains("approved"), "{method}: {text}");
        }
    }

    #[test]
    fn unknown_requests_are_not_supported_and_tools_only_when_the_plan_has_them() {
        for method in ["item/tool/requestUserInput", "account/chatgptAuthTokens/refresh", "attestation/generate", "something/new", ""] {
            assert_eq!(classify(method, &json!({}), &TOOLS, true), Handling::Unsupported, "{method}");
        }
        assert_eq!(classify("item/tool/call", &call("run_command"), &TOOLS, false), Handling::RunCommand(json!({ "command": "dir", "path": "a.txt" })));
        assert!(matches!(classify("item/tool/call", &call("read_file"), &TOOLS, false), Handling::File { .. }));
        // An agent without `run` has no run_command, even if the model invents the call.
        assert_eq!(classify("item/tool/call", &call("run_command"), &["list_files", "read_file"], false), Handling::Unsupported);
        assert_eq!(classify("item/tool/call", &call("read_file"), &[], false), Handling::Unsupported);
        assert_eq!(classify("item/tool/call", &call("shell"), &TOOLS, false), Handling::Unsupported);
        // PARLEY's odds reader only for the agent that has it.
        assert_eq!(classify("item/tool/call", &call("read_parley_odds"), &TOOLS, false), Handling::Unsupported);
        assert!(matches!(classify("item/tool/call", &call("read_parley_odds"), &[], true), Handling::File { .. }));
        assert_eq!(classify("item/tool/call", &json!({}), &TOOLS, true), Handling::Unsupported);
    }

    #[test]
    fn a_tool_reply_is_a_plain_result() {
        let reply: Value = serde_json::from_str(&tool_reply(&json!("abc"), false, "no")).unwrap();
        assert_eq!((reply["id"].as_str(), reply["result"]["success"].as_bool(), reply["result"]["contentItems"][0]["text"].as_str()), (Some("abc"), Some(false), Some("no")));
    }
    #[test]
    #[ignore = "uses the user's Codex subscription; no OddsPapi request"]
    fn real_odds_reader_round_trip() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let workspace = crate::services::named_agents::workspace("parley");
            let csv = workspace.join("odds/latest-odds.csv");
            assert!(csv.exists(), "requires a saved odds query");
            let agent = crate::services::named_agents::find("parley").unwrap();
            let model = crate::services::named_agents::model_for(&agent, "codex");
            let mut cmd = tokio::process::Command::new(crate::services::subscription::cli_path("codex").unwrap());
            cmd.creation_flags(0x0800_0000).kill_on_drop(true);
            for key in ["OPENAI_API_KEY", "CODEX_API_KEY", "OPENAI_BASE_URL", "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_BASE_URL", "CLAUDE_CODE_OAUTH_TOKEN"] { cmd.env_remove(key); }
            let cwd = workspace.to_string_lossy();
            let plan = crate::services::agent_tools::codex_plan(&agent, &crate::services::agent_tools::Caps::of(&agent));
            let server = CodexServer::start(cmd, ThreadOptions { cwd: &cwd, model: &model.id, effort: model.effort.unwrap(), plan: &plan, gate: None,
                instructions: "Prueba de lectura. Usa read_parley_odds una vez y devuelve el primer nombre de columna leído. Sin búsqueda web, sin apuestas." }).await.unwrap();
            let turn = server.start_turn("Lee latest-odds.csv, offset 0, limit 1, con read_parley_odds. Devuelve el primer nombre de columna.", &[], model.effort.unwrap(), Arc::new(AtomicBool::new(false))).await.unwrap();
            let deadline = Instant::now();
            let mut read_ok = false;
            let mut answer = String::new();
            while deadline.elapsed() < Duration::from_secs(120) {
                if let Some(Some((method, params))) = server.next_note(Duration::from_millis(250)).await {
                    if method == "item/completed" && params["item"]["type"] == "dynamicToolCall" {
                        read_ok |= params["item"]["success"] == true;
                    }
                    if method == "item/agentMessage/delta" { answer.push_str(params["delta"].as_str().unwrap_or("")); }
                    if method == "turn/completed" && params["turn"]["id"].as_str() == Some(&turn) { break; }
                }
            }
            server.shutdown();
            println!("tool success={read_ok}; answer={answer}");
            assert!(read_ok && answer.contains("consultaUTC"), "the real client must read the saved CSV through the registered tool");
        });
    }

    /// A real Codex turn: the agent calls `run_command`; the first command is allowed (the closure stands in for the
    /// island's click), the second is denied. The allowed one must run in the workspace and show up as a command
    /// output note; the denied one must never run.
    #[test]
    #[ignore = "uses the user's Codex subscription"]
    fn real_run_command_asks_first_and_only_an_allow_runs_it() {
        use crate::services::agent_gate::{ask_with, execute, GateTurn, Verdict};
        use crate::services::agent_tools::{codex_plan, Caps};
        use crate::agents::claude_code::pipe::IslandAnswer;
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let workspace = std::env::temp_dir().join(format!("mika-codex-run-{}", std::process::id()));
            std::fs::create_dir_all(&workspace).unwrap();
            let agent = crate::services::named_agents::parse(crate::services::named_agents::BUILT_INS.iter().find(|(id, _)| *id == "maki").unwrap().1).unwrap();
            let model = crate::services::named_agents::model_for(&agent, "codex");
            let plan = codex_plan(&agent, &Caps::of(&agent));
            let asked = Arc::new(Mutex::new(Vec::<String>::new()));
            let answers = Arc::new(Mutex::new(vec![IslandAnswer::Deny, IslandAnswer::Allow]));
            let (log, script, ws) = (asked.clone(), answers.clone(), workspace.clone());
            let gate = GateCtx { run: Arc::new(move |args, cancel| {
                let (log, script, ws) = (log.clone(), script.clone(), ws.clone());
                Box::pin(async move {
                    let command = args["command"].as_str().unwrap_or("").to_string();
                    log.lock().unwrap().push(command.clone());
                    let turn = GateTurn { agent_id: "maki".into(), agent_name: "MAKI".into(), workspace: ws.clone(), cancel: cancel.clone() };
                    let next = script.lock().unwrap().remove(0);
                    match ask_with(&turn, "PowerShell", &command, None, move |_| std::future::ready(next)).await {
                        Verdict::Denied(why) => (format!("{why} No lo intentes de otra forma."), false, None),
                        Verdict::Allowed => {
                            let run = execute(&command, &ws, Duration::from_secs(30), &cancel).await;
                            (format!("Código de salida: {:?}\n{}", run.code, run.output), run.code == Some(0), Some(run))
                        }
                    }
                })
            }) };
            let mut cmd = tokio::process::Command::new(crate::services::subscription::cli_path("codex").unwrap());
            cmd.creation_flags(0x0800_0000).kill_on_drop(true);
            for key in ["OPENAI_API_KEY", "CODEX_API_KEY", "OPENAI_BASE_URL", "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_BASE_URL", "CLAUDE_CODE_OAUTH_TOKEN"] { cmd.env_remove(key); }
            let cwd = workspace.to_string_lossy().to_string();
            let instructions = crate::services::agent_skills::system_prompt(&agent);
            let server = CodexServer::start(cmd, ThreadOptions { cwd: &cwd, model: &model.id, effort: "low", plan: &plan, gate: Some(gate), instructions: &instructions }).await.unwrap();
            let prompt = "Usa run_command dos veces, una tras otra, sin explicar: primero `echo PRIMERO > primero.txt` y después `echo SEGUNDO > segundo.txt`. Si te deniegan uno, igual intenta el otro. Al final di en una frase qué pasó.";
            let turn = server.start_turn(prompt, &[], "low", Arc::new(AtomicBool::new(false))).await.unwrap();
            let started = Instant::now();
            let (mut outputs, mut answer) = (Vec::<Value>::new(), String::new());
            while started.elapsed() < Duration::from_secs(150) {
                if let Some(Some((method, params))) = server.next_note(Duration::from_millis(250)).await {
                    if method == "mika/commandOutput" { outputs.push(params.clone()); }
                    if method == "item/agentMessage/delta" { answer.push_str(params["delta"].as_str().unwrap_or("")); }
                    if method == "turn/completed" && params["turn"]["id"].as_str() == Some(&turn) { break; }
                }
            }
            server.shutdown();
            println!("asked={:?}\noutputs={outputs:?}\nanswer={answer}", asked.lock().unwrap());
            let asked = asked.lock().unwrap().clone();
            assert!(asked.len() >= 2 && asked[0].contains("primero.txt") && asked[1].contains("segundo.txt"), "{asked:?}");
            assert!(!workspace.join("primero.txt").exists(), "the denied command must not run");
            assert!(workspace.join("segundo.txt").exists(), "the allowed command ran in the workspace");
            assert_eq!(outputs.len(), 1, "only the allowed command produces an output note: {outputs:?}");
            let _ = std::fs::remove_dir_all(&workspace);
        });
    }
}
