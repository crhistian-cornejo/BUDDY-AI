//! Gemini through the user's Google AI Pro subscription: the Antigravity CLI (`agy`), Google's successor to the
//! Gemini CLI (whose Google-account sign-in stopped serving AI Pro on 2026-06-18). Warm processes, like Claude's:
//! `agy --input-format stream-json --output-format stream-json` reads one user message per line and answers each as
//! NDJSON (`init`, `step_update`…, `result`), keeping the conversation. The CLI's start-up (≈6 s) happens while the
//! user types (prewarm) or once per conversation; a cold one resumes with `--conversation <id>`.
//!
//! The CLI does nothing until its first message (measured, agy 1.2.16: start ≈5 s, plus each remote MCP server's
//! handshake, DeepWiki's alone ≈16 s; later turns ≈3 s). So the prewarm opens the conversation itself: it sends
//! Buddy's instructions as the first message, and the user's first message is already a warm turn.
//!
//! Permissions live in the agent workspace's `.agents/` folder (project-level settings the CLI reads): shell commands
//! are always denied (the CLI cannot route an approval to Buddy's card, and headless mode would soft-deny them
//! anyway), the web is allowed or denied by the agent's permission, read-only folders get a `write_file` deny, and
//! Buddy's own MCP server (`.agents/mcp_config.json`) carries the Office, music, screen and skills tools.
//!
//! Everything that depends on the CLI's spelling (flags, file names, rule syntax, tool names) is in this file, so a
//! change in `agy` (or going back to another Gemini CLI) is a change here only.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::process;
use super::{Cancel, Failure, FailureKind, Provider, ProviderId, TokenCount, TurnEvent, TurnRequest};

/// The CLI's command name.
pub const EXE: &str = "agy";
/// How long one turn may run before the CLI gives up (its default is 5 minutes, short for research turns).
const PRINT_TIMEOUT: &str = "20m";
/// The folder inside the workspace with the CLI's project-level settings.
const CONFIG_DIR: &str = ".agents";

/// Warm processes kept at most; one unused this long is closed.
const MAX_LIVE: usize = 3;
const IDLE: Duration = Duration::from_secs(10 * 60);

pub struct Gemini {
    exe: Option<PathBuf>,
    /// Warm `agy` processes: one per conversation, plus a spare made ready while the user types.
    pool: Arc<Mutex<Vec<Live>>>,
    reaper: Arc<AtomicBool>,
    /// Signatures of the spares being opened right now: a turn that arrives meanwhile waits for its spare.
    opening: Arc<Mutex<Vec<String>>>,
}

/// How long the opening message may take, and how long a turn waits for a spare that is being opened.
const OPENING: Duration = Duration::from_secs(60);

/// One running `agy` in stream-json mode, waiting for its next message.
struct Live {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    lines: Receiver<String>,
    stderr: Arc<Mutex<String>>,
    /// Everything that fixes its behaviour (arguments minus the conversation, folder).
    signature: String,
    /// The conversation it holds (None: a fresh spare).
    session: Option<String>,
    /// A spare whose conversation is already open: the instructions it was given and the conversation's id.
    opened: Option<(String, String)>,
    used: Instant,
}

impl Live {
    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Gemini {
    pub fn new() -> Self {
        Self { exe: locate(), pool: Arc::default(), reaper: Arc::default(), opening: Arc::default() }
    }

    /// The command line. The prompt never goes here (it travels on stdin), nor do secrets.
    pub fn arguments(request: &TurnRequest) -> Vec<String> {
        let mut args: Vec<String> = ["--input-format", "stream-json", "--output-format", "stream-json", "--print-timeout", PRINT_TIMEOUT]
            .iter()
            .map(|s| s.to_string())
            .collect();
        for dir in workspace_dirs(request) {
            args.extend(["--add-dir".into(), dir]);
        }
        // agy names each thinking level as its own model: gemini-3.8-flash-high, gemini-3.1-pro-low…
        if let Some(model) = request.model.as_ref().filter(|m| !m.trim().is_empty()) {
            args.extend(["--model".into(), model_id(model, effort(request))]);
        }
        if let Some(id) = request.resume.as_ref().filter(|r| !r.is_empty()) {
            args.extend(["--conversation".into(), id.clone()]);
        }
        args
    }
}

impl Default for Gemini {
    fn default() -> Self {
        Self::new()
    }
}

/// Finds `agy`, skipping the Antigravity IDE's old launcher of the same name (it opens the editor instead).
fn locate() -> Option<PathBuf> {
    let usable = |p: &PathBuf| !is_ide_launcher(p);
    process::locate(EXE).filter(usable).or_else(|| {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)?;
        let name = if cfg!(windows) { format!("{EXE}.exe") } else { EXE.to_string() };
        Some(home.join(".local/bin").join(name)).filter(|p| p.is_file() && usable(p))
    })
}

/// The IDE's `agy` lives inside the editor (`Antigravity.app/Contents/Resources/app/bin`, `~/.antigravity/…/bin`).
pub fn is_ide_launcher(path: &Path) -> bool {
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    [path, real.as_path()].iter().any(|p| {
        let text = p.to_string_lossy().replace('\\', "/").to_lowercase();
        text.contains("/resources/app/bin/") || text.contains("/.antigravity/antigravity/bin/")
    })
}

/// The folders this turn works in: its own workspace, the attachments' folders and the authorized folders.
fn workspace_dirs(request: &TurnRequest) -> Vec<String> {
    let mut dirs: Vec<String> = request
        .attachments
        .iter()
        .filter_map(|p| p.parent().map(|d| d.to_string_lossy().into_owned()))
        .chain(request.folders.iter().map(|f| f.path.clone()))
        .filter(|d| !d.is_empty())
        .collect();
    dirs.sort();
    dirs.dedup();
    let own = request.workspace.to_string_lossy().into_owned();
    if !own.is_empty() {
        dirs.retain(|d| *d != own);
        dirs.insert(0, own);
    }
    dirs
}

/// The model id agy knows: the family plus its thinking level (`-low`, `-medium`, `-high`), unless the name
/// already carries one. Without an effort, the highest the family has.
pub fn model_id(model: &str, effort: Option<&str>) -> String {
    let model = model.trim();
    if ["-low", "-medium", "-high"].iter().any(|s| model.ends_with(s)) || !model.starts_with("gemini-") {
        return model.to_string();
    }
    format!("{model}-{}", effort.unwrap_or("high"))
}

/// The router's effort, as the CLI takes it. Gemini Pro thinks only low or high.
fn effort(request: &TurnRequest) -> Option<&'static str> {
    let pro = request.model.as_deref().is_some_and(|m| m.contains("-pro"));
    match request.effort.as_deref()? {
        "low" => Some("low"),
        "medium" if pro => Some("high"),
        "medium" => Some("medium"),
        "high" => Some("high"),
        _ => None,
    }
}

/// A rule's folder: the path with one trailing separator (the CLI matches by prefix).
fn rule_dir(dir: &str) -> String {
    format!("{}/", dir.trim_end_matches(['/', '\\']))
}

/// `.agents/settings.json`: what the CLI may do without asking (deny wins over allow).
pub fn settings(request: &TurnRequest) -> Value {
    // Never a shell: Buddy cannot approve it from here (the CLI has no approval channel back to the app).
    let mut deny = vec!["command(*)".to_string()];
    let mut allow = Vec::new();
    if request.no_web {
        deny.extend(["search_web(*)", "read_url(*)", "execute_url(*)"].map(String::from));
    } else {
        allow.extend(["search_web(*)", "read_url(*)"].map(String::from));
    }
    // Attachments and read-only folders are readable, never writable.
    let mut read_only: Vec<String> =
        request.attachments.iter().filter_map(|p| p.parent().map(|d| d.to_string_lossy().into_owned())).collect();
    read_only.extend(request.folders.iter().filter(|f| !f.can_edit).map(|f| f.path.clone()));
    read_only.sort();
    read_only.dedup();
    deny.extend(read_only.iter().filter(|d| !d.is_empty()).map(|d| format!("write_file({})", rule_dir(d))));
    allow.extend(request.folders.iter().filter(|f| f.can_edit).map(|f| format!("write_file({})", rule_dir(&f.path))));
    if request.office.is_some() {
        allow.push("mcp(buddy/*)".into());
    }
    // Each connector's tools by name too: in live checks `mcp(context7/*)` did not cover `resolve-library-id`.
    for connector in request.remote() {
        allow.push(format!("mcp({}/*)", connector.id));
        allow.extend(connector.tools().iter().map(|tool| format!("mcp({}/{tool})", connector.id)));
    }
    json!({ "permissions": { "allow": allow, "deny": deny } })
}

/// `.agents/mcp_config.json`: only Buddy's own server. Its secret (the music tools' token) travels in the CLI's
/// environment, never in this file; the data folder is not secret.
/// The connectors go by URL only: this CLI reads no header from its environment, and a key never goes in a file,
/// so here they run keyless (their free limits).
pub fn mcp_config(request: &TurnRequest) -> Option<Value> {
    let mut servers = serde_json::Map::new();
    if let Some(office) = &request.office {
        let mut server = office.server();
        if let Some(link) = &office.link {
            server["env"] = json!({ "BUDDY_DATA_DIR": link.data_dir });
        }
        servers.insert("buddy".into(), server);
    }
    for connector in request.remote() {
        servers.insert(connector.id.clone(), json!({ "url": connector.url }));
    }
    (!servers.is_empty()).then(|| json!({ "mcpServers": servers }))
}

/// Writes this turn's `.agents/` files into the workspace (and removes a stale MCP config).
fn prepare(request: &TurnRequest) -> std::io::Result<()> {
    let dir = request.workspace.join(CONFIG_DIR);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("settings.json"), serde_json::to_vec_pretty(&settings(request))?)?;
    // Live checks (agy, Oct 2026): an MCP allow rule never took effect from `settings.json` alone and sometimes did
    // from `.agents/config.json` (the name of the shared `~/.gemini/config/config.json`), so the same rules go in both.
    std::fs::write(dir.join("config.json"), serde_json::to_vec_pretty(&settings(request))?)?;
    let mcp = dir.join("mcp_config.json");
    match mcp_config(request) {
        Some(config) => std::fs::write(mcp, serde_json::to_vec_pretty(&config)?)?,
        None => {
            if mcp.exists() {
                std::fs::remove_file(mcp)?;
            }
        }
    }
    Ok(())
}

/// The text of the user message. The CLI has no system-prompt flag, so Buddy's instructions open a new
/// conversation; a resumed one already holds them.
pub fn prompt(request: &TurnRequest) -> String {
    if request.system.trim().is_empty() || request.resume.as_ref().is_some_and(|r| !r.is_empty()) {
        return request.prompt.clone();
    }
    format!(
        "[Instrucciones de Buddy para esta conversación]\n{}\n\nUsa solo tus herramientas de búsqueda web y las del servidor \
MCP «buddy»; no uses otros servidores MCP (no están permitidos aquí). Responde siempre con texto.\n\n[Mensaje del usuario]\n{}",
        request.system.trim(),
        request.prompt
    )
}

/// The stdin line for one turn (text blocks only: the CLI's stream-json input takes no images).
pub fn user_line(request: &TurnRequest) -> String {
    message_line(&prompt(request))
}

fn message_line(text: &str) -> String {
    json!({ "event": "user", "message": { "content": text } }).to_string()
}

/// The message that opens a spare's conversation: the instructions, and nothing to answer yet.
pub fn opening_line(request: &TurnRequest) -> String {
    let waiting = TurnRequest {
        prompt: "(Todavía no ha escrito nada. No uses herramientas; responde solo «ok» y espera su mensaje.)".into(),
        ..request.clone()
    };
    user_line(&waiting)
}

/// Sends the instructions to a fresh spare and waits for its answer, so the start-up and the MCP handshakes are
/// done before the user's message. A spare that fails to open stays a plain one (or dies, and is dropped).
fn open_conversation(live: &mut Live, request: &TurnRequest) {
    if request.system.trim().is_empty() {
        return;
    }
    if writeln!(live.stdin, "{}", opening_line(request)).and_then(|_| live.stdin.flush()).is_err() {
        return;
    }
    let mut parser = StreamParser::default();
    let started = Instant::now();
    while started.elapsed() < OPENING {
        match live.lines.recv_timeout(Duration::from_millis(200)) {
            Ok(line) => {
                for event in parser.feed(&line) {
                    match event {
                        TurnEvent::Done => {
                            live.opened = parser.conversation.clone().map(|id| (request.system.clone(), id));
                            return;
                        }
                        TurnEvent::Failed(_) => {
                            let _ = live.child.kill();
                            return;
                        }
                        _ => {}
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
    // Still busy after all this time: not a spare worth keeping.
    let _ = live.child.kill();
}

/// A failure from the CLI's words: quota and credits are a limit (the next provider takes the turn), sign-in
/// problems are auth, the rest as usual.
pub fn failure(text: &str) -> Failure {
    let t = text.to_lowercase();
    let kind = if ["resource_exhausted", "quota", "out of credits", "ai credits", "insufficient credits"].iter().any(|k| t.contains(k)) {
        FailureKind::Limit
    } else if ["keyring", "not signed in", "signed out", "unauthenticated", "credentials", "oauth", "login", "/logout"]
        .iter()
        .any(|k| t.contains(k))
    {
        FailureKind::Auth
    } else if ["model_capacity_exhausted", "unavailable"].iter().any(|k| t.contains(k)) {
        FailureKind::Other
    } else {
        super::classify(text)
    };
    Failure { kind, message: text.trim().to_string() }
}

impl Provider for Gemini {
    fn id(&self) -> ProviderId {
        ProviderId::Antigravity
    }

    fn installed(&self) -> bool {
        self.exe.is_some()
    }

    /// It opens an attached picture with its own file tool (the path rides in the message; live check, agy 1.2.16).
    fn sees_images(&self) -> bool {
        true
    }

    fn prewarm(&self, request: &TurnRequest) {
        let Some(exe) = &self.exe else { return };
        let fresh = TurnRequest { resume: None, ..request.clone() };
        let signature = signature(&fresh);
        if self.pool.lock().unwrap().iter().any(|l| l.session.is_none() && l.signature == signature) {
            return;
        }
        {
            let mut opening = self.opening.lock().unwrap();
            if opening.contains(&signature) {
                return;
            }
            opening.push(signature.clone());
        }
        if let Ok(mut live) = spawn_live(exe, &fresh) {
            open_conversation(&mut live, &fresh);
            if live.alive() {
                live.used = Instant::now();
                self.put(live);
            }
        }
        self.opening.lock().unwrap().retain(|s| *s != signature);
    }

    fn run(&self, request: &TurnRequest, cancel: &Cancel, emit: &mut dyn FnMut(TurnEvent)) {
        let Some(exe) = &self.exe else {
            emit(TurnEvent::Failed(Failure { kind: FailureKind::Missing, message: format!("{EXE} no está instalado") }));
            return;
        };
        // A spare is being opened for this very request: waiting for it is shorter than starting another.
        let signature = signature(request);
        let started = Instant::now();
        while request.resume.as_ref().is_none_or(|r| r.is_empty())
            && self.opening.lock().unwrap().contains(&signature)
            && started.elapsed() < OPENING
            && !cancel.is_cancelled()
        {
            std::thread::sleep(Duration::from_millis(100));
        }
        // A warm process for this conversation (or a spare), else a new one; a dead one is replaced once.
        let mut live = None;
        for attempt in 0..2 {
            let mut candidate = match (attempt, self.take(request)) {
                (0, Some(l)) => l,
                _ => match spawn_live(exe, request) {
                    Ok(l) => l,
                    Err(e) => return emit(TurnEvent::Failed(Failure::new(e))),
                },
            };
            // An opened spare already holds these instructions: only the message goes.
            let message = match candidate.opened.take() {
                Some((system, id)) if system == request.system => {
                    candidate.session = Some(id.clone());
                    emit(TurnEvent::Session(id));
                    message_line(&request.prompt)
                }
                _ => user_line(request),
            };
            if writeln!(candidate.stdin, "{message}").and_then(|_| candidate.stdin.flush()).is_ok() {
                live = Some(candidate);
                break;
            }
        }
        let Some(mut live) = live else {
            return emit(TurnEvent::Failed(Failure::new("No se pudo hablar con Gemini.")));
        };

        let mut parser = StreamParser { conversation: live.session.clone(), ..StreamParser::default() };
        let mut outcome = None;
        loop {
            if cancel.is_cancelled() {
                // Stopped by the user: ask the CLI to stop, and make sure it goes away.
                process::interrupt(&mut live.child);
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(3));
                    drop(live);
                });
                return emit(TurnEvent::Done);
            }
            match live.lines.recv_timeout(Duration::from_millis(100)) {
                Ok(line) => {
                    for event in parser.feed(&line) {
                        if let TurnEvent::Session(id) = &event {
                            live.session = Some(id.clone());
                        }
                        if matches!(event, TurnEvent::Done | TurnEvent::Failed(_)) {
                            outcome = Some(matches!(event, TurnEvent::Done));
                        }
                        emit(event);
                    }
                    if outcome.is_some() {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        match outcome {
            Some(true) => {
                live.used = Instant::now();
                self.put(live);
            }
            Some(false) => {}
            None => {
                // The CLI ended without a result: say what it printed, if anything.
                let status = live.child.wait().ok();
                std::thread::sleep(Duration::from_millis(50));
                let detail = live.stderr.lock().unwrap().trim().to_string();
                emit(match (status.is_some_and(|s| s.success()), detail.is_empty()) {
                    (_, false) => TurnEvent::Failed(failure(&detail)),
                    (true, true) if parser.wrote_text => TurnEvent::Done,
                    _ => TurnEvent::Failed(Failure::new("Gemini terminó sin responder.")),
                });
            }
        }
    }
}

impl Gemini {
    /// The warm process for this request: its conversation's, or a spare when it starts a new one.
    fn take(&self, request: &TurnRequest) -> Option<Live> {
        let signature = signature(request);
        let wanted = request.resume.clone().filter(|r| !r.is_empty());
        let mut pool = self.pool.lock().unwrap();
        pool.retain_mut(Live::alive);
        let index = pool.iter().position(|l| l.signature == signature && l.session == wanted)?;
        Some(pool.remove(index))
    }

    fn put(&self, live: Live) {
        let mut pool = self.pool.lock().unwrap();
        pool.push(live);
        if pool.len() > MAX_LIVE {
            pool.sort_by_key(|l| std::cmp::Reverse(l.used));
            pool.truncate(MAX_LIVE);
        }
        drop(pool);
        self.ensure_reaper();
    }

    /// Closes idle processes; runs only while there are some.
    fn ensure_reaper(&self) {
        if self.reaper.swap(true, Ordering::SeqCst) {
            return;
        }
        let (pool, running) = (self.pool.clone(), self.reaper.clone());
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(60));
            let mut p = pool.lock().unwrap();
            p.retain_mut(|l| l.used.elapsed() < IDLE && l.alive());
            if p.is_empty() {
                running.store(false, Ordering::SeqCst);
                return;
            }
        });
    }
}

/// What makes two requests answerable by the same process: everything but the conversation and the message.
fn signature(request: &TurnRequest) -> String {
    let fresh = TurnRequest { resume: None, prompt: String::new(), ..request.clone() };
    let env: Vec<String> = request.office.iter().flat_map(|o| o.env()).map(|(k, v)| format!("{k}={v}")).collect();
    // The connectors live in the workspace files, not the arguments: a change needs a new process.
    let connectors: Vec<&str> = request.remote().iter().map(|c| c.url.as_str()).collect();
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}",
        Gemini::arguments(&fresh).join("\u{1f}"),
        request.workspace.display(),
        env.join("\u{1f}"),
        connectors.join(",")
    )
}

/// Starts `agy` reading messages as stream-json (its project settings written first), with readers for its output.
fn spawn_live(exe: &Path, request: &TurnRequest) -> Result<Live, String> {
    prepare(request).map_err(|e| format!("No se pudo preparar Gemini: {e}"))?;
    let mut cmd = process::command(exe);
    cmd.args(Gemini::arguments(request)).current_dir(&request.workspace);
    cmd.env(super::OWN_RUN_ENV, "1");
    for (key, value) in request.office.iter().flat_map(|o| o.env()) {
        cmd.env(key, value);
    }
    let mut child = cmd.spawn().map_err(|e| format!("No se pudo iniciar Gemini: {e}"))?;
    let stdin = child.stdin.take().ok_or("sin stdin")?;
    let stdout = child.stdout.take().ok_or("sin stdout")?;
    let stderr = child.stderr.take().ok_or("sin stderr")?;
    let (tx, lines) = channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let err_text = Arc::new(Mutex::new(String::new()));
    let sink = err_text.clone();
    std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = stderr.take(32_768).read_to_string(&mut buf);
        *sink.lock().unwrap() = buf;
    });
    Ok(Live {
        child,
        stdin,
        lines,
        stderr: err_text,
        signature: signature(request),
        session: request.resume.clone().filter(|r| !r.is_empty()),
        opened: None,
        used: Instant::now(),
    })
}

/// Turns the CLI's `--output-format stream-json` lines into turn events. Thinking and tool arguments are never
/// shown; a subagent's steps (another conversation id) are not the answer.
#[derive(Default)]
pub struct StreamParser {
    conversation: Option<String>,
    model: Option<String>,
    wrote_text: bool,
    /// A tool ran after some text: the next text starts a new paragraph.
    break_before_text: bool,
    /// Tool steps already announced, by step index.
    announced: HashSet<i64>,
}

impl StreamParser {
    pub fn feed(&mut self, line: &str) -> Vec<TurnEvent> {
        let Ok(obj) = serde_json::from_str::<Value>(line) else {
            return vec![];
        };
        match obj["event"].as_str() {
            Some("init") => {
                self.model = obj["init"]["model"].as_str().filter(|m| !m.is_empty()).map(str::to_string);
                match obj["conversation_id"].as_str().filter(|id| !id.is_empty()) {
                    Some(id) => {
                        self.conversation = Some(id.to_string());
                        vec![TurnEvent::Session(id.into())]
                    }
                    None => vec![],
                }
            }
            Some("step_update") => self.step(&obj["step_update"]),
            Some("result") => self.result(&obj["result"]),
            _ => match obj["error"].as_str().or(obj["error"]["message"].as_str()) {
                Some(message) => vec![TurnEvent::Failed(failure(message))],
                None => vec![],
            },
        }
    }

    fn step(&mut self, step: &Value) -> Vec<TurnEvent> {
        if let (Some(mine), Some(theirs)) = (&self.conversation, step["conversation_id"].as_str())
            && mine != theirs
        {
            return vec![];
        }
        match step["step_type"].as_str() {
            Some("agent_response") => match step["text_delta"].as_str() {
                Some(text) if !text.is_empty() => {
                    let text = if std::mem::take(&mut self.break_before_text) && self.wrote_text {
                        format!("\n\n{text}")
                    } else {
                        text.to_string()
                    };
                    self.wrote_text = true;
                    vec![TurnEvent::Delta(text)]
                }
                _ => vec![],
            },
            Some("tool") => {
                let info = &step["tool_info"];
                let raw = step["tool_name"].as_str().or(info["name"].as_str()).unwrap_or("");
                let params = &info["parameters"];
                let mut events = vec![];
                let index = step["step_index"].as_i64().unwrap_or(-1);
                if self.announced.insert(index) || index < 0 {
                    self.break_before_text = true;
                    let (name, summary) = tool_status(raw, params);
                    events.push(TurnEvent::Tool { name, summary });
                }
                if step["state"] == "DONE" {
                    events.extend(sources(raw, params, &info["output"]).into_iter().map(|(title, url)| TurnEvent::Source { title, url }));
                }
                events
            }
            _ => vec![],
        }
    }

    fn result(&mut self, result: &Value) -> Vec<TurnEvent> {
        let mut events = vec![];
        if let Some(usage) = result.get("usage").filter(|u| u.is_object()) {
            let n = |k: &str| usage[k].as_i64().unwrap_or(0);
            let cached = n("cache_read_tokens");
            events.push(TurnEvent::Tokens(TokenCount {
                input: (n("input_tokens") - cached).max(0),
                output: n("output_tokens"),
                cached,
                cost_usd: None,
                model: self.model.clone(),
            }));
        }
        let response = result["response"].as_str().unwrap_or("");
        match result["status"].as_str() {
            // A finished turn with no text at all (e.g. it tried one of the user's own MCP servers, which headless
            // mode denies): the next provider answers instead of an empty bubble.
            Some("SUCCESS") if !self.wrote_text && response.trim().is_empty() => {
                events.push(TurnEvent::Failed(Failure { kind: FailureKind::Missing, message: "Gemini terminó sin responder.".into() }));
            }
            Some("SUCCESS" | "CANCELED" | "INTERRUPTED" | "WAITING") => {
                if !self.wrote_text && !response.trim().is_empty() {
                    self.wrote_text = true;
                    events.push(TurnEvent::Delta(response.to_string()));
                }
                events.push(TurnEvent::Done);
            }
            _ => {
                let message = result["error"].as_str().filter(|e| !e.is_empty()).unwrap_or("Gemini no pudo completar la respuesta.");
                events.push(TurnEvent::Failed(failure(message)));
            }
        }
        events
    }
}

/// The CLI's tool as Buddy's apps show it («Buscando: …», «Leyendo una página…», «Leyendo el archivo…»).
fn tool_status(raw: &str, params: &Value) -> (String, String) {
    let detail = |keys: &[&str]| -> String {
        keys.iter().find_map(|k| params[*k].as_str()).unwrap_or("").chars().take(160).collect()
    };
    match raw {
        "search_web" => ("WebSearch".into(), detail(&["query", "Query", "SearchQuery"])),
        "read_url" | "read_url_content" | "execute_url" => ("WebFetch".into(), detail(&["url", "Url", "URL"])),
        "read_file" | "view_file" | "list_dir" | "grep_search" | "find_by_name" => ("Read".into(), String::new()),
        other => (other.to_string(), String::new()),
    }
}

/// Pages a finished tool step read or found: the URL it read, or the links a search returned.
fn sources(raw: &str, params: &Value, output: &Value) -> Vec<(String, String)> {
    let web = |u: &str| u.starts_with("https://") || u.starts_with("http://");
    match raw {
        "read_url" | "read_url_content" => {
            ["url", "Url", "URL"].iter().find_map(|k| params[*k].as_str()).filter(|u| web(u)).map(|u| vec![(String::new(), u.to_string())]).unwrap_or_default()
        }
        "search_web" => {
            let text = match output {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            };
            links(&text)
        }
        _ => vec![],
    }
}

/// Up to 8 web links in a tool's text: Markdown `[title](url)` first, else bare URLs.
pub fn links(text: &str) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    let mut found = vec![];
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close) = after.find("](") else { break };
        let title = &after[..close];
        let tail = &after[close + 2..];
        let Some(end) = tail.find(')') else { break };
        let url = tail[..end].trim();
        if (url.starts_with("https://") || url.starts_with("http://")) && !title.contains('[') && seen.insert(url.to_string()) {
            found.push((title.trim().to_string(), url.to_string()));
        }
        rest = &tail[end..];
    }
    if found.is_empty() {
        for word in text.split(|c: char| c.is_whitespace() || c == '"' || c == '<' || c == '>') {
            let url = word.trim_end_matches(['.', ',', ';', ')', ']', '}']);
            if (url.starts_with("https://") || url.starts_with("http://")) && url.len() > 10 && seen.insert(url.to_string()) {
                found.push((String::new(), url.to_string()));
            }
        }
    }
    found.truncate(8);
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::folders::AuthorizedFolder;

    fn feed_all(lines: &[&str]) -> Vec<TurnEvent> {
        let mut p = StreamParser::default();
        lines.iter().flat_map(|l| p.feed(l)).collect()
    }

    /// A turn as the CLI's headless docs show it: init, the user's step, the answer in pieces, the result.
    const TURN: [&str; 6] = [
        r#"{"event":"init","conversation_id":"c-1","init":{"cwd":"/d/agentes/buddy","tools":["search_web","read_url"],"permission_mode":"request-review","model":"gemini-3.8-flash"}}"#,
        r#"{"event":"step_update","step_update":{"conversation_id":"c-1","step_index":0,"state":"DONE","step_type":"user_input"}}"#,
        r#"{"event":"step_update","step_update":{"conversation_id":"c-1","step_index":1,"state":"ACTIVE","step_type":"agent_response","text_delta":"¡Hola"}}"#,
        r#"{"event":"step_update","step_update":{"conversation_id":"c-1","step_index":1,"state":"ACTIVE","step_type":"agent_response","text_delta":"! ¿Qué tal?"}}"#,
        r#"{"event":"step_update","step_update":{"conversation_id":"c-1","step_index":1,"state":"DONE","step_type":"agent_response","duration_seconds":1.5}}"#,
        r#"{"event":"result","result":{"conversation_id":"c-1","status":"SUCCESS","response":"¡Hola! ¿Qué tal?","duration_seconds":6.88,"num_turns":1,"usage":{"input_tokens":10415,"output_tokens":657,"thinking_tokens":616,"cache_read_tokens":8113,"total_tokens":11072}}}"#,
    ];

    #[test]
    fn streams_the_answer_with_its_conversation_and_tokens() {
        let events = feed_all(&TURN);
        assert_eq!(events[0], TurnEvent::Session("c-1".into()));
        assert_eq!(events[1], TurnEvent::Delta("¡Hola".into()));
        assert_eq!(events[2], TurnEvent::Delta("! ¿Qué tal?".into()));
        assert_eq!(
            events[3],
            TurnEvent::Tokens(TokenCount { input: 2302, output: 657, cached: 8113, cost_usd: None, model: Some("gemini-3.8-flash".into()) })
        );
        assert_eq!(events[4], TurnEvent::Done);
        assert_eq!(events.len(), 5, "the result's full text is not repeated");
    }

    #[test]
    fn a_result_without_deltas_brings_the_whole_text() {
        let events = feed_all(&[r#"{"event":"result","result":{"status":"SUCCESS","response":"Entero"}}"#]);
        assert_eq!(events, vec![TurnEvent::Delta("Entero".into()), TurnEvent::Done]);
    }

    #[test]
    fn web_tools_show_as_buddys_and_bring_sources() {
        let events = feed_all(&[
            TURN[0],
            r#"{"event":"step_update","step_update":{"conversation_id":"c-1","step_index":1,"state":"ACTIVE","step_type":"agent_response","text_delta":"Busco."}}"#,
            r#"{"event":"step_update","step_update":{"conversation_id":"c-1","step_index":2,"state":"ACTIVE","step_type":"tool","tool_name":"search_web","tool_info":{"name":"search_web","parameters":{"query":"clima Lima"}}}}"#,
            r#"{"event":"step_update","step_update":{"conversation_id":"c-1","step_index":2,"state":"DONE","step_type":"tool","tool_name":"search_web","tool_info":{"name":"search_web","parameters":{"query":"clima Lima"},"output":"1. [SENAMHI](https://senamhi.gob.pe) pronóstico\n2. [x](javascript:alert(1))"}}}"#,
            r#"{"event":"step_update","step_update":{"conversation_id":"c-1","step_index":3,"state":"DONE","step_type":"tool","tool_name":"read_url","tool_info":{"name":"read_url","parameters":{"Url":"https://weather.com/lima"},"output":"..."}}}"#,
            r#"{"event":"step_update","step_update":{"conversation_id":"c-1","step_index":4,"state":"ACTIVE","step_type":"agent_response","text_delta":"Listo"}}"#,
        ]);
        assert_eq!(events[2], TurnEvent::Tool { name: "WebSearch".into(), summary: "clima Lima".into() });
        assert_eq!(events[3], TurnEvent::Source { title: "SENAMHI".into(), url: "https://senamhi.gob.pe".into() });
        assert_eq!(events[4], TurnEvent::Tool { name: "WebFetch".into(), summary: "https://weather.com/lima".into() });
        assert_eq!(events[5], TurnEvent::Source { title: String::new(), url: "https://weather.com/lima".into() });
        assert_eq!(events[6], TurnEvent::Delta("\n\nListo".into()), "text after a tool starts a new paragraph");
        assert_eq!(events.len(), 7, "a tool is announced once and a non-web link is dropped");
    }

    #[test]
    fn a_subagents_steps_are_not_the_answer() {
        let events = feed_all(&[
            TURN[0],
            r#"{"event":"step_update","step_update":{"conversation_id":"sub-9","step_index":0,"state":"ACTIVE","step_type":"agent_response","text_delta":"notas internas"}}"#,
        ]);
        assert_eq!(events, vec![TurnEvent::Session("c-1".into())]);
    }

    #[test]
    fn failures_are_classified_and_quota_moves_to_the_next_provider() {
        let events = feed_all(&[r#"{"event":"result","result":{"status":"ERROR","error":"RESOURCE_EXHAUSTED: Individual quota reached. Resets in 3h."}}"#]);
        assert!(matches!(&events[0], TurnEvent::Failed(f) if f.kind == FailureKind::Limit && f.is_no_usage()), "{events:?}");
        assert_eq!(failure("You are not signed in. Run agy to sign in.").kind, FailureKind::Auth);
        assert_eq!(failure("keyring is locked").kind, FailureKind::Auth);
        let capacity = failure("MODEL_CAPACITY_EXHAUSTED: try again later");
        assert_eq!(capacity.kind, FailureKind::Other);
        assert!(!capacity.is_no_usage(), "busy servers are not an empty plan");
        assert_eq!(failure("Your AI credits ran out").kind, FailureKind::Limit);
        let events = feed_all(&[r#"{"event":"result","result":{"status":"INVALID"}}"#]);
        assert!(matches!(&events[0], TurnEvent::Failed(f) if f.kind == FailureKind::Other));
        assert_eq!(feed_all(&["no es json", r#"{"event":"result","result":{"status":"CANCELED"}}"#]), vec![TurnEvent::Done]);
    }

    #[test]
    fn arguments_keep_the_prompt_out_and_resume() {
        let args = Gemini::arguments(&TurnRequest {
            prompt: "hola".into(),
            system: "Eres Buddy".into(),
            workspace: "/d/agentes/buddy".into(),
            resume: Some("c-1".into()),
            model: Some("gemini-3.1-pro".into()),
            effort: Some("medium".into()),
            ..Default::default()
        });
        let joined = args.join(" ");
        assert!(joined.starts_with("--input-format stream-json --output-format stream-json"), "{joined}");
        assert!(joined.contains("--add-dir /d/agentes/buddy") && joined.contains("--conversation c-1"));
        assert!(joined.contains("--model gemini-3.1-pro-high"), "Pro has no medium: {joined}");
        assert!(!joined.contains("--effort"));
        assert!(!joined.contains("hola") && !joined.contains("Eres Buddy"), "prompt and instructions go on stdin");
        assert!(!joined.contains("--dangerously-skip-permissions") && !joined.contains(" -p"), "{joined}");
        let plain = Gemini::arguments(&TurnRequest::default()).join(" ");
        assert!(!plain.contains("--model") && !plain.contains("--conversation") && !plain.contains("--effort"));
        assert_eq!(model_id("gemini-3.8-flash", Some("medium")), "gemini-3.8-flash-medium");
        assert_eq!(model_id("gemini-3.8-flash", None), "gemini-3.8-flash-high");
        assert_eq!(model_id("gemini-3.1-pro-low", Some("high")), "gemini-3.1-pro-low", "an explicit level wins");
        assert_eq!(model_id("claude-sonnet-4-6", Some("high")), "claude-sonnet-4-6");
    }

    #[test]
    fn a_spare_is_opened_with_the_instructions_and_nothing_to_answer() {
        let request = TurnRequest { system: "Eres Buddy.".into(), prompt: "hola".into(), ..Default::default() };
        let line: Value = serde_json::from_str(&opening_line(&request)).unwrap();
        let text = line["message"]["content"].as_str().unwrap();
        assert!(text.contains("Eres Buddy.") && text.contains("responde solo «ok»"), "{text}");
        assert!(!text.contains("hola"), "{text}");
        // The user's message then travels alone.
        let line: Value = serde_json::from_str(&message_line("hola")).unwrap();
        assert_eq!(line["message"]["content"], "hola");
    }

    #[test]
    fn instructions_open_a_new_conversation_only() {
        let fresh = TurnRequest { prompt: "hola".into(), system: "Eres Buddy".into(), ..Default::default() };
        let line: Value = serde_json::from_str(&user_line(&fresh)).unwrap();
        assert_eq!(line["event"], "user");
        let text = line["message"]["content"].as_str().unwrap();
        assert!(text.starts_with("[Instrucciones de Buddy") && text.contains("Eres Buddy") && text.ends_with("hola"));
        let resumed = TurnRequest { resume: Some("c-1".into()), ..fresh };
        assert_eq!(prompt(&resumed), "hola");
    }

    #[test]
    fn permissions_map_to_project_rules() {
        let request = TurnRequest {
            workspace: "/d/agentes/buddy".into(),
            attachments: vec!["/d/adjuntos/c1/foto.png".into(), "/d/adjuntos/c1/notas.txt".into()],
            folders: vec![
                AuthorizedFolder { path: "/u/docs".into(), can_edit: false },
                AuthorizedFolder { path: "/u/proyecto/".into(), can_edit: true },
            ],
            ..Default::default()
        };
        let s = settings(&request);
        let list = |k: &str| s["permissions"][k].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect::<Vec<_>>();
        let (allow, deny) = (list("allow"), list("deny"));
        assert!(deny.contains(&"command(*)".into()), "never a shell");
        assert!(allow.contains(&"search_web(*)".into()) && allow.contains(&"read_url(*)".into()));
        assert!(deny.contains(&"write_file(/d/adjuntos/c1/)".into()) && deny.contains(&"write_file(/u/docs/)".into()));
        assert!(allow.contains(&"write_file(/u/proyecto/)".into()) && !deny.iter().any(|d| d.contains("proyecto")));
        assert!(!allow.iter().any(|a| a.starts_with("mcp(")), "no MCP without Buddy's tools");
        let args = Gemini::arguments(&request).join(" ");
        assert!(args.contains("--add-dir /d/agentes/buddy --add-dir /d/adjuntos/c1 --add-dir /u/docs --add-dir /u/proyecto/"), "{args}");

        let offline = settings(&TurnRequest { no_web: true, ..Default::default() });
        let deny = offline["permissions"]["deny"].to_string();
        assert!(deny.contains("search_web(*)") && deny.contains("read_url(*)") && deny.contains("command(*)"));
        assert!(!offline["permissions"]["allow"].to_string().contains("search_web"));
    }

    #[test]
    fn office_tools_come_from_buddys_own_mcp_server_without_the_secret() {
        let request = TurnRequest {
            office: Some(super::super::Office {
                relay: "/d/bin/buddy-hook".into(),
                dir: "/d/documentos".into(),
                skills: "/d/skills".into(),
                link: Some(super::super::Link { token: "secreto".into(), data_dir: "/d".into() }),
                read: vec!["/d/adjuntos".into()],
            }),
            ..Default::default()
        };
        let config = mcp_config(&request).unwrap().to_string();
        assert!(config.contains("\"buddy\"") && config.contains("--mcp") && config.contains("/d/documentos"), "{config}");
        assert!(config.contains("BUDDY_DATA_DIR") && !config.contains("secreto"), "the token travels in the environment");
        assert!(settings(&request)["permissions"]["allow"].to_string().contains("mcp(buddy/*)"));
        assert!(mcp_config(&TurnRequest::default()).is_none());
    }

    #[test]
    fn connectors_go_by_url_without_their_key() {
        let request = TurnRequest {
            connectors: vec![
                crate::connectors::Connector::new("context7", "https://mcp.context7.com/mcp", Some("ctx7sk-secreto".into())),
                crate::connectors::Connector::new("deepwiki", "https://mcp.deepwiki.com/mcp", None),
            ],
            ..Default::default()
        };
        let config = mcp_config(&request).unwrap();
        assert_eq!(
            config,
            json!({ "mcpServers": { "context7": { "url": "https://mcp.context7.com/mcp" }, "deepwiki": { "url": "https://mcp.deepwiki.com/mcp" } } })
        );
        let allow = settings(&request)["permissions"]["allow"].to_string();
        assert!(allow.contains("mcp(context7/*)") && allow.contains("mcp(deepwiki/*)") && !allow.contains("mcp(buddy/*)"), "{allow}");
        assert!(allow.contains("mcp(context7/resolve-library-id)") && allow.contains("mcp(context7/query-docs)"), "{allow}");
        assert!(!signature(&request).contains("secreto"));
        let offline = TurnRequest { no_web: true, ..request };
        assert!(mcp_config(&offline).is_none() && !settings(&offline).to_string().contains("context7"), "no web, no connectors");

        let dir = tempfile::tempdir().unwrap();
        let on_disk = TurnRequest { workspace: dir.path().join("buddy"), no_web: false, ..offline };
        prepare(&on_disk).unwrap();
        let written = std::fs::read_to_string(on_disk.workspace.join(".agents/mcp_config.json")).unwrap();
        assert!(written.contains("https://mcp.context7.com/mcp") && !written.contains("secreto"), "{written}");
    }

    #[test]
    fn prepare_writes_and_clears_the_workspace_config() {
        let dir = tempfile::tempdir().unwrap();
        let office = super::super::Office {
            relay: "/d/bin/buddy-hook".into(),
            dir: "/d/documentos".into(),
            skills: "/d/skills".into(),
            link: None,
            read: vec![],
        };
        let mut request = TurnRequest { workspace: dir.path().join("buddy"), office: Some(office), ..Default::default() };
        prepare(&request).unwrap();
        let agents = request.workspace.join(".agents");
        assert!(agents.join("settings.json").is_file() && agents.join("mcp_config.json").is_file());
        assert_eq!(std::fs::read(agents.join("config.json")).unwrap(), std::fs::read(agents.join("settings.json")).unwrap());
        request.office = None;
        prepare(&request).unwrap();
        assert!(!agents.join("mcp_config.json").exists(), "no stale tools");
    }

    #[test]
    fn the_ides_launcher_is_not_the_cli() {
        assert!(is_ide_launcher(Path::new("/Applications/Antigravity.app/Contents/Resources/app/bin/antigravity")));
        assert!(is_ide_launcher(Path::new("/Users/u/.antigravity/antigravity/bin/agy")));
        assert!(!is_ide_launcher(Path::new("/Users/u/.local/bin/agy")));
    }

    #[test]
    fn bare_links_are_found_too() {
        let found = links("Fuentes: https://a.pe/x, https://b.pe/y. y https://a.pe/x otra vez");
        assert_eq!(found.iter().map(|l| l.1.as_str()).collect::<Vec<_>>(), vec!["https://a.pe/x", "https://b.pe/y"]);
    }
}
