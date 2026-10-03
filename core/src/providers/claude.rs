//! Claude through the user's Claude Code subscription: `claude -p --output-format stream-json`, prompt on stdin,
//! one JSON object per line out. Ported from MIKA's ClaudeTurn.swift and ClaudeStreamParser.swift.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::process;
use super::{Cancel, Failure, FailureKind, Provider, ProviderId, TokenCount, TurnEvent, TurnRequest};

/// Chat-only tools: search and read the web. No shell, no file edits (phase 4 adds them behind the approval gate).
const TOOLS: &str = "WebSearch,WebFetch";

pub struct Claude {
    exe: Option<PathBuf>,
    /// Warm `claude` processes (stream-json in and out): one per conversation, plus a spare made ready while the
    /// user types. Each answers its next turn without the CLI's start-up time.
    pool: Arc<Mutex<Vec<Live>>>,
    reaper: Arc<AtomicBool>,
}

/// Keeping more than this many warm processes kills the least recently used.
const MAX_LIVE: usize = 4;
/// A warm process unused for this long is closed.
const IDLE: Duration = Duration::from_secs(10 * 60);

/// One running `claude -p --input-format stream-json`, waiting for its next message.
struct Live {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    lines: Receiver<String>,
    stderr: Arc<Mutex<String>>,
    /// Everything that fixes its behaviour (arguments minus the conversation to resume, folder).
    signature: String,
    /// The conversation it holds (None: a fresh spare).
    session: Option<String>,
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

impl Claude {
    pub fn new() -> Self {
        Self { exe: process::locate("claude"), pool: Arc::default(), reaper: Arc::default() }
    }

    /// `--safe-mode` ignores the user's hooks, plugins and CLAUDE.md (Buddy's own prompt rules); no MCP servers.
    pub fn arguments(request: &TurnRequest) -> Vec<String> {
        // Attachments and authorized folders add Read (and, for editable folders, Edit and Write), allowed only
        // inside those folders; anything else is refused by `dontAsk`.
        let mut reads: Vec<String> = request
            .attachments
            .iter()
            .filter_map(|p| p.parent().map(|d| d.to_string_lossy().into_owned()))
            .chain(request.folders.iter().map(|f| f.path.clone()))
            .collect();
        reads.sort();
        reads.dedup();
        let edits: Vec<&str> = request.folders.iter().filter(|f| f.can_edit).map(|f| f.path.as_str()).collect();
        let web = if request.no_web { "" } else { TOOLS };
        let mut tools: Vec<&str> = web.split(',').filter(|t| !t.is_empty()).collect();
        if !reads.is_empty() {
            tools.extend(["Read", "Glob", "Grep"]);
        }
        if !edits.is_empty() {
            tools.extend(["Edit", "Write"]);
        }
        // Bash is available only behind the gate, and never pre-allowed: the PreToolUse hook asks the user each time.
        if request.gate.is_some() {
            tools.push("Bash");
        }
        let tools = tools.join(",");
        let rule = |verb: &str, dir: &str| format!("{verb}(//{}/**)", dir.trim_start_matches('/'));
        let allowed = std::iter::once(web.to_string())
            .filter(|w| !w.is_empty())
            .chain(reads.iter().map(|d| rule("Read", d)))
            .chain(edits.iter().flat_map(|d| [rule("Edit", d), rule("Write", d)]))
            .collect::<Vec<_>>()
            .join(",");
        let dirs = reads;
        let mut args: Vec<String> = [
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            // `--safe-mode` turns every hook off, the gate's too; `--restricted` ignores the user's settings files
            // but keeps the `--settings` hook.
            // (and MCP servers, so Buddy's Office tools need it too).
            if request.gate.is_some() || request.office.is_some() || !request.remote().is_empty() || request.accounts {
                "--restricted"
            } else {
                "--safe-mode"
            },
            "--permission-mode",
            "dontAsk",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        // `--strict-mcp-config` hides the user's claude.ai accounts too: a turn with them goes without it (with
        // `--restricted` the user's own local servers and plugins still stay out).
        if !request.accounts {
            args.push("--strict-mcp-config".into());
        }
        // Only Buddy's own MCP server and its connectors, never the user's: the Office tools, pre-allowed (they write
        // only in their folder), and each connector's tools (read-only network lookups).
        let mcp = serde_json::json!({ "mcpServers": mcp_servers(request) }).to_string();
        let office_tools = request.office.as_ref().map(|_| super::Office::TOOLS.join(",")).unwrap_or_default();
        let connector_tools: Vec<String> = request.remote().iter().map(|c| format!("mcp__{}", c.id)).collect();
        let account_tools = if request.accounts { crate::accounts::allowed_tools().join(",") } else { String::new() };
        let allowed = [allowed, office_tools, connector_tools.join(","), account_tools]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(",");
        args.extend(["--mcp-config".into(), mcp]);
        args.extend(["--tools".into(), tools, "--allowedTools".into(), allowed]);
        // What those accounts offer beyond reading (sending, drafts, trash…) never reaches the model.
        if request.accounts {
            args.extend(["--disallowedTools".into(), crate::accounts::denied_tools().join(",")]);
        }
        for dir in &dirs {
            args.extend(["--add-dir".into(), dir.clone()]);
        }
        let extended_haiku = request.effort.as_deref() == Some("xhigh")
            && request.model.as_deref().is_some_and(|model| model == "haiku" || model.starts_with("claude-haiku-4-5"));
        let mut settings = serde_json::json!({});
        if extended_haiku {
            // Haiku 4.5 has manual extended thinking, not an effort parameter.
            // Keep this turn-local so Buddy and background finance reviews keep their own profiles.
            settings["alwaysThinkingEnabled"] = serde_json::json!(true);
            settings["env"] = serde_json::json!({
                "MAX_THINKING_TOKENS": "31999",
                "CLAUDE_CODE_MAX_OUTPUT_TOKENS": "64000",
                "CLAUDE_CODE_DISABLE_THINKING": "0",
                "CLAUDE_CODE_EFFORT_LEVEL": "auto"
            });
        }
        if let Some(gate) = &request.gate {
            let command = format!("\"{}\" --gate PreToolUse", gate.relay.to_string_lossy().replace('\\', "/"));
            settings["hooks"] = serde_json::json!({ "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": command, "timeout": 120 }] }] });
        }
        if settings.as_object().is_some_and(|settings| !settings.is_empty()) {
            args.extend(["--settings".into(), settings.to_string()]);
        }
        if let Some(model) = &request.model {
            args.extend(["--model".into(), model.clone()]);
        }
        if !extended_haiku && let Some(effort) = &request.effort {
            args.extend(["--effort".into(), effort.clone()]);
        }
        if !request.system.is_empty() {
            args.extend(["--append-system-prompt".into(), request.system.clone()]);
        }
        if let Some(resume) = request.resume.as_ref().filter(|r| !r.is_empty()) {
            args.extend(["--resume".into(), resume.clone()]);
        }
        args
    }
}

/// `mcpServers` for `--mcp-config`: Buddy's server and the turn's connectors. A connector's key is written as
/// `${VAR}`, which Claude Code expands from its own environment (`spawn_live` sets it): never in the arguments.
fn mcp_servers(request: &TurnRequest) -> serde_json::Value {
    let mut servers = serde_json::Map::new();
    if let Some(office) = &request.office {
        servers.insert("buddy".into(), office.server());
    }
    for connector in request.remote() {
        let mut server = serde_json::json!({ "type": "http", "url": connector.url });
        if connector.has_key() {
            server["headers"] = serde_json::json!({ "Authorization": format!("Bearer ${{{}}}", connector.env_var()) });
        }
        servers.insert(connector.id.clone(), server);
    }
    serde_json::Value::Object(servers)
}

impl Default for Claude {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for Claude {
    fn release_session(&self, id: &str) {
        self.pool.lock().unwrap().retain(|live| live.session.as_deref() != Some(id));
    }
    fn id(&self) -> ProviderId {
        ProviderId::Claude
    }

    fn installed(&self) -> bool {
        self.exe.is_some()
    }

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
        if let Ok(live) = spawn_live(exe, &fresh) {
            self.put(live);
        }
    }

    fn run(&self, request: &TurnRequest, cancel: &Cancel, emit: &mut dyn FnMut(TurnEvent)) {
        let Some(exe) = &self.exe else {
            emit(TurnEvent::Failed(Failure { kind: FailureKind::Missing, message: "claude no está instalado".into() }));
            return;
        };
        let message = serde_json::json!({
            "type": "user",
            "message": { "role": "user", "content": user_content(request) },
        })
        .to_string();
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
            if writeln!(candidate.stdin, "{message}").and_then(|_| candidate.stdin.flush()).is_ok() {
                live = Some(candidate);
                break;
            }
        }
        let Some(mut live) = live else {
            return emit(TurnEvent::Failed(Failure::new("No se pudo hablar con Claude.")));
        };

        let mut parser = StreamParser::default();
        let mut outcome = None;
        loop {
            if cancel.is_cancelled() {
                // Stopped by the user: the process (and what it was writing) goes away.
                return emit(TurnEvent::Done);
            }
            match live.lines.recv_timeout(Duration::from_millis(100)) {
                Ok(line) => {
                    for event in parser.feed(&line) {
                        if let TurnEvent::Session(id) = &event {
                            live.session = Some(id.clone());
                        }
                        let end = matches!(event, TurnEvent::Done | TurnEvent::Failed(_));
                        if end {
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
                // The process ended without a result: say what it printed, if anything.
                let _ = live.child.wait();
                let detail = live.stderr.lock().unwrap().trim().to_string();
                emit(if detail.is_empty() { TurnEvent::Done } else { TurnEvent::Failed(Failure::new(detail)) });
            }
        }
    }
}

impl Claude {
    /// Closes every warm process now (a one-off turn's owner, such as Niko's sync, does not keep them).
    pub fn close_all(&self) {
        self.pool.lock().unwrap().clear();
    }

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
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(60));
                let mut p = pool.lock().unwrap();
                p.retain_mut(|l| l.used.elapsed() < IDLE && l.alive());
                if p.is_empty() {
                    running.store(false, Ordering::SeqCst);
                    return;
                }
            }
        });
    }
}

/// What makes two requests answerable by the same process: everything but the conversation to resume.
/// The user message's content: the text and each image as a base64 `image` block, so Claude sees them
/// without having to decide to read a file.
fn user_content(request: &TurnRequest) -> serde_json::Value {
    use base64::Engine;
    let mut content = vec![serde_json::json!({ "type": "text", "text": request.prompt })];
    for path in request.images() {
        if let Ok(bytes) = std::fs::read(path) {
            content.push(serde_json::json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": crate::images::mime(path),
                    "data": base64::engine::general_purpose::STANDARD.encode(bytes),
                },
            }));
        }
    }
    serde_json::Value::Array(content)
}

fn signature(request: &TurnRequest) -> String {
    let fresh = TurnRequest { resume: None, prompt: String::new(), ..request.clone() };
    // The connectors' fingerprint: a new key means a new process (the arguments only name its variable).
    format!(
        "{}\u{1f}{}\u{1f}{}",
        Claude::arguments(&fresh).join("\u{1f}"),
        request.workspace.display(),
        crate::connectors::fingerprint(request.remote())
    )
}

/// Starts `claude` reading messages as stream-json, with readers for its output.
fn spawn_live(exe: &std::path::Path, request: &TurnRequest) -> Result<Live, String> {
    let _ = std::fs::create_dir_all(&request.workspace);
    let mut cmd = process::command(exe);
    cmd.args(Claude::arguments(request)).args(["--input-format", "stream-json"]).current_dir(&request.workspace);
    cmd.env(super::OWN_RUN_ENV, "1");
    if let Some(gate) = &request.gate {
        cmd.env("BUDDY_GATE_TOKEN", &gate.token).env("BUDDY_DATA_DIR", &gate.data_dir);
    }
    // Buddy's MCP server inherits these (the music tools); the same secret as the gate's.
    for (key, value) in request.office.iter().flat_map(|o| o.env()) {
        cmd.env(key, value);
    }
    // The connectors' keys, expanded by Claude Code into their `Authorization` headers.
    for (key, value) in request.remote_env() {
        cmd.env(key, value);
    }
    let mut child = cmd.spawn().map_err(|e| format!("No se pudo iniciar Claude: {e}"))?;
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
        used: Instant::now(),
    })
}

/// Turns the NDJSON of `claude -p --output-format stream-json --include-partial-messages` into turn events.
/// Thinking and tool arguments are never shown.
#[derive(Default)]
pub struct StreamParser {
    saw_delta: bool,
    /// Text was shown, then a tool ran: the next text starts a new paragraph.
    wrote_text: bool,
    break_before_text: bool,
    /// Tool calls seen so far, to know which page a WebFetch result belongs to.
    fetched: HashMap<String, String>,
}

impl StreamParser {
    pub fn feed(&mut self, line: &str) -> Vec<TurnEvent> {
        let Ok(obj) = serde_json::from_str::<Value>(line) else {
            return vec![];
        };
        match obj["type"].as_str() {
            Some("system") => {
                if obj["subtype"] == "init" {
                    self.saw_delta = false;
                    if let Some(id) = obj["session_id"].as_str() {
                        return vec![TurnEvent::Session(id.into())];
                    }
                }
                vec![]
            }
            Some("stream_event") => {
                let event = &obj["event"];
                match event["type"].as_str() {
                    Some("content_block_delta") if event["delta"]["type"] == "text_delta" => match event["delta"]["text"].as_str() {
                        Some(text) if !text.is_empty() => {
                            self.saw_delta = true;
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
                    Some("content_block_start") if event["content_block"]["type"] == "tool_use" => {
                        self.break_before_text = true;
                        let name = event["content_block"]["name"].as_str().unwrap_or("").to_string();
                        vec![TurnEvent::Tool { name, summary: String::new() }]
                    }
                    _ => vec![],
                }
            }
            Some("assistant") => {
                let blocks = obj["message"]["content"].as_array().cloned().unwrap_or_default();
                // Claude emits subscription exhaustion as a synthetic assistant message, sometimes followed
                // by a successful result. Treat the notice as a failure before it reaches the chat as text.
                let text = blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n");
                if obj["error"].as_str().is_some() || text.trim_start().starts_with("You've hit your ") && Failure::new(&text).is_no_usage()
                {
                    return vec![TurnEvent::Failed(Failure::new(text))];
                }
                let mut events = vec![];
                if !self.saw_delta {
                    let text: Vec<&str> =
                        blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect();
                    if !text.is_empty() {
                        events.push(TurnEvent::Delta(text.join("\n")));
                    }
                }
                for block in blocks.iter().filter(|b| b["type"] == "tool_use") {
                    let input = &block["input"];
                    let detail = input["query"].as_str().or(input["url"].as_str()).unwrap_or("");
                    if block["name"] == "WebFetch" {
                        if let (Some(id), Some(url)) = (block["id"].as_str(), input["url"].as_str()) {
                            self.fetched.insert(id.into(), url.into());
                        }
                    }
                    if let Some(name) = block["name"].as_str().filter(|_| !detail.is_empty()) {
                        events.push(TurnEvent::Tool { name: name.into(), summary: detail.chars().take(160).collect() });
                    }
                }
                events
            }
            Some("user") => {
                let blocks = obj["message"]["content"].as_array().cloned().unwrap_or_default();
                let mut events = vec![];
                for block in blocks.iter().filter(|b| b["type"] == "tool_result" && b["is_error"] != true) {
                    let links = search_links(&text_of(&block["content"]));
                    if !links.is_empty() {
                        events.extend(links.into_iter().map(|(title, url)| TurnEvent::Source { title, url }));
                    } else if let Some(url) = block["tool_use_id"].as_str().and_then(|id| self.fetched.get(id)) {
                        events.push(TurnEvent::Source { title: String::new(), url: url.clone() });
                    }
                }
                events
            }
            Some("rate_limit_event") => vec![TurnEvent::Usage(obj["rate_limit_info"].clone())],
            Some("result") => {
                let mut events: Vec<TurnEvent> = TokenCount::from_claude_result(&obj).map(TurnEvent::Tokens).into_iter().collect();
                if obj["is_error"] == true {
                    let message = obj["result"].as_str().unwrap_or("Claude no pudo completar la respuesta.");
                    events.push(TurnEvent::Failed(Failure::new(message)));
                } else {
                    events.push(TurnEvent::Done);
                }
                events
            }
            _ => vec![],
        }
    }
}

fn text_of(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// A web search answers with `Links: [{"title":…,"url":…}, …]` followed by a summary.
pub fn search_links(text: &str) -> Vec<(String, String)> {
    let Some(start) = text.find("Links: [").map(|i| i + "Links: ".len()) else {
        return vec![];
    };
    let mut stream = serde_json::Deserializer::from_str(&text[start..]).into_iter::<Value>();
    let Some(Ok(Value::Array(items))) = stream.next() else {
        return vec![];
    };
    let mut seen = std::collections::HashSet::new();
    items
        .iter()
        .filter_map(|item| {
            let url = item["url"].as_str()?;
            if !(url.starts_with("https://") || url.starts_with("http://")) || !seen.insert(url.to_string()) {
                return None;
            }
            Some((item["title"].as_str().unwrap_or("").to_string(), url.to_string()))
        })
        .take(10)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(lines: &[&str]) -> Vec<TurnEvent> {
        let mut p = StreamParser::default();
        lines.iter().flat_map(|l| p.feed(l)).collect()
    }

    #[test]
    fn streams_text_word_by_word_and_finishes() {
        let events = feed_all(&[
            r#"{"type":"system","subtype":"init","session_id":"abc"}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hola"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":" mundo"}}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hola mundo"}]}}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"result":"Hola mundo"}"#,
        ]);
        assert_eq!(
            events,
            vec![
                TurnEvent::Session("abc".into()),
                TurnEvent::Delta("Hola".into()),
                TurnEvent::Delta(" mundo".into()),
                TurnEvent::Done
            ]
        );
    }

    #[test]
    fn text_after_a_tool_starts_a_new_paragraph() {
        let delta = |t: &str| {
            format!(r#"{{"type":"stream_event","event":{{"type":"content_block_delta","delta":{{"type":"text_delta","text":"{t}"}}}}}}"#)
        };
        let tool =
            r#"{"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"tool_use","name":"WebSearch"}}}"#;
        let events = feed_all(&[tool, &delta("Busco."), tool, &delta("Listo"), &delta(" ya")]);
        let text: String = events.iter().filter_map(|e| if let TurnEvent::Delta(t) = e { Some(t.as_str()) } else { None }).collect();
        assert_eq!(text, "Busco.\n\nListo ya");
    }

    #[test]
    fn without_partial_messages_the_whole_text_arrives_once() {
        let events = feed_all(&[r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Entero"}]}}"#]);
        assert_eq!(events, vec![TurnEvent::Delta("Entero".into())]);
    }

    #[test]
    fn tools_and_sources() {
        let events = feed_all(&[
            r#"{"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"tool_use","name":"WebSearch"}}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"WebSearch","input":{"query":"clima Lima"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"Links: [{\"title\":\"SENAMHI\",\"url\":\"https://senamhi.gob.pe\"},{\"title\":\"x\",\"url\":\"javascript:alert(1)\"}] resumen"}]}}"#,
        ]);
        assert_eq!(events[0], TurnEvent::Tool { name: "WebSearch".into(), summary: String::new() });
        assert_eq!(events[1], TurnEvent::Tool { name: "WebSearch".into(), summary: "clima Lima".into() });
        assert_eq!(events[2], TurnEvent::Source { title: "SENAMHI".into(), url: "https://senamhi.gob.pe".into() });
        assert_eq!(events.len(), 3, "a non-web link is dropped");
    }

    #[test]
    fn errors_are_classified() {
        let events = feed_all(&[r#"{"type":"result","is_error":true,"result":"Claude AI usage limit reached"}"#]);
        assert!(matches!(&events[0], TurnEvent::Failed(f) if f.kind == FailureKind::Limit));
    }

    #[test]
    fn session_limit_notice_is_a_failure_even_with_a_successful_result() {
        let events = feed_all(&[
            r#"{"type":"assistant","error":"rate_limit","message":{"content":[{"type":"text","text":"You've hit your session limit · resets 8:10pm (America/Lima)"}]}}"#,
            r#"{"type":"result","is_error":false}"#,
        ]);
        assert!(matches!(&events[0], TurnEvent::Failed(f) if f.is_no_usage()));
        assert!(!events.iter().any(|e| matches!(e, TurnEvent::Delta(_))));
        let events = feed_all(&[
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"You've hit your session limit · resets 8:10pm (America/Lima)"}]}}"#,
        ]);
        assert!(matches!(&events[0], TurnEvent::Failed(f) if f.is_no_usage()));
    }

    #[test]
    fn images_travel_as_image_blocks_after_the_text() {
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("a.png");
        image::RgbaImage::new(2, 2).save(&img).unwrap();
        let request = TurnRequest { prompt: "mira".into(), attachments: vec![img, "/x/notas.txt".into()], ..Default::default() };
        let content = user_content(&request);
        assert_eq!(content.as_array().unwrap().len(), 2, "the text file is not an image block");
        assert_eq!(content[0]["text"], "mira");
        assert_eq!(content[1]["type"], "image");
        assert_eq!(content[1]["source"]["media_type"], "image/png");
        assert!(content[1]["source"]["data"].as_str().unwrap().starts_with("iVBOR"));
    }

    #[test]
    fn attachments_allow_reading_only_their_folder() {
        let args = Claude::arguments(&TurnRequest {
            attachments: vec!["/data/adjuntos/c1/foto.png".into(), "/data/adjuntos/c1/notas.txt".into()],
            ..Default::default()
        });
        let joined = args.join(" ");
        assert!(joined.contains("--tools WebSearch,WebFetch,Read"));
        assert!(joined.contains("--allowedTools WebSearch,WebFetch,Read(//data/adjuntos/c1/**)"), "{joined}");
        assert_eq!(joined.matches("--add-dir /data/adjuntos/c1").count(), 1);
        let plain = Claude::arguments(&TurnRequest::default()).join(" ");
        assert!(!plain.contains("Read") && !plain.contains("--add-dir"));
    }

    #[test]
    fn the_gate_adds_bash_never_preallowed_with_its_hook() {
        let args = Claude::arguments(&TurnRequest {
            gate: Some(super::super::Gate { relay: "/d/bin/buddy-hook".into(), token: "t".into(), data_dir: "/d".into() }),
            ..Default::default()
        });
        let joined = args.join(" ");
        assert!(joined.contains("--restricted") && !joined.contains("--safe-mode"));
        assert!(joined.contains("--tools WebSearch,WebFetch,Bash"));
        assert!(!joined.contains("--allowedTools WebSearch,WebFetch,Bash"), "Bash is never pre-allowed");
        let settings = &args[args.iter().position(|a| a == "--settings").unwrap() + 1];
        assert!(settings.contains("\\\"/d/bin/buddy-hook\\\" --gate PreToolUse") && settings.contains("\"matcher\":\"Bash\""), "{settings}");
        assert!(!joined.contains("\"t\""), "the secret travels in the environment, not the arguments");
    }

    #[test]
    fn without_the_web_permission_there_is_no_web_tool() {
        let args = Claude::arguments(&TurnRequest { no_web: true, ..Default::default() });
        let at = |flag: &str| args[args.iter().position(|a| a == flag).unwrap() + 1].clone();
        assert_eq!(at("--tools"), "");
        assert!(!at("--allowedTools").contains("WebSearch"));
        let web = Claude::arguments(&TurnRequest::default());
        assert!(web[web.iter().position(|a| a == "--tools").unwrap() + 1].starts_with("WebSearch,WebFetch"));
    }

    #[test]
    fn office_tools_come_from_buddys_own_mcp_server() {
        let args = Claude::arguments(&TurnRequest {
            office: Some(super::super::Office {
                relay: "/d/bin/buddy-hook".into(),
                dir: "/d/documentos".into(),
                skills: "/d/skills".into(),
                link: Some(super::super::Link { token: "secreto".into(), data_dir: "/d".into() }),
                read: vec!["/d/adjuntos".into(), "relativa".into()],
            }),
            ..Default::default()
        });
        let joined = args.join(" ");
        assert!(joined.contains("--restricted"));
        assert!(!joined.contains("secreto"), "the music tools' secret travels in the environment");
        assert!(joined.contains("mcp__buddy__media_play") && joined.contains("mcp__buddy__use_skill"));
        let mcp = &args[args.iter().position(|a| a == "--mcp-config").unwrap() + 1];
        assert!(mcp.contains("\"buddy\"") && mcp.contains("--mcp") && mcp.contains("/d/documentos") && mcp.contains("/d/skills"), "{mcp}");
        assert!(mcp.contains("\"--read\",\"/d/adjuntos\"") && !mcp.contains("relativa"), "only absolute read folders: {mcp}");
        assert!(joined.contains("mcp__buddy__read_document") && joined.contains("mcp__buddy__look_at_screen"));
        assert!(joined.contains("mcp__buddy__create_document,mcp__buddy__create_spreadsheet,mcp__buddy__create_presentation"));
        let plain = Claude::arguments(&TurnRequest::default());
        assert_eq!(plain[plain.iter().position(|a| a == "--mcp-config").unwrap() + 1], r#"{"mcpServers":{}}"#);
    }

    #[test]
    fn connectors_are_remote_servers_whose_key_never_reaches_the_arguments() {
        let request = TurnRequest {
            connectors: vec![
                crate::connectors::Connector::new("context7", "https://mcp.context7.com/mcp", Some("ctx7sk-secreto".into())),
                crate::connectors::Connector::new("deepwiki", "https://mcp.deepwiki.com/mcp", None),
            ],
            ..Default::default()
        };
        let args = Claude::arguments(&request);
        let joined = args.join(" ");
        assert!(!joined.contains("secreto"), "the key travels in the environment: {joined}");
        assert!(joined.contains("--restricted") && !joined.contains("--safe-mode"), "MCP servers need --restricted");
        let mcp: serde_json::Value = serde_json::from_str(&args[args.iter().position(|a| a == "--mcp-config").unwrap() + 1]).unwrap();
        assert_eq!(
            mcp["mcpServers"]["context7"],
            serde_json::json!({ "type": "http", "url": "https://mcp.context7.com/mcp",
                "headers": { "Authorization": "Bearer ${BUDDY_CONNECTOR_CONTEXT7_KEY}" } })
        );
        assert_eq!(mcp["mcpServers"]["deepwiki"], serde_json::json!({ "type": "http", "url": "https://mcp.deepwiki.com/mcp" }));
        let allowed = &args[args.iter().position(|a| a == "--allowedTools").unwrap() + 1];
        assert!(allowed.ends_with(",mcp__context7,mcp__deepwiki"), "{allowed}");
        assert_eq!(request.remote_env(), [("BUDDY_CONNECTOR_CONTEXT7_KEY".to_string(), "ctx7sk-secreto".to_string())]);
        // The warm process changes with the key, yet its signature never holds it.
        let sig = signature(&request);
        assert!(!sig.contains("secreto"));
        let mut other = request.clone();
        other.connectors[0] = crate::connectors::Connector::new("context7", "https://mcp.context7.com/mcp", Some("otra".into()));
        assert_ne!(sig, signature(&other));

        // Without the web permission there are no connectors at all.
        let offline = Claude::arguments(&TurnRequest { no_web: true, ..request });
        let joined = offline.join(" ");
        assert!(!joined.contains("context7") && !joined.contains("deepwiki") && joined.contains("--safe-mode"), "{joined}");
    }

    #[test]
    fn accounts_keep_claude_ai_servers_and_only_reading_tools() {
        let args = Claude::arguments(&TurnRequest { accounts: true, no_web: true, ..Default::default() });
        let joined = args.join(" ");
        assert!(joined.contains("--restricted") && !joined.contains("--safe-mode"), "{joined}");
        assert!(!joined.contains("--strict-mcp-config"), "claude.ai accounts need the CLI's own MCP list");
        let at = |flag: &str| args[args.iter().position(|a| a == flag).unwrap() + 1].clone();
        assert_eq!(at("--mcp-config"), r#"{"mcpServers":{}}"#, "Buddy's own servers still go by --mcp-config");
        let allowed = at("--allowedTools");
        assert!(allowed.contains("mcp__claude_ai_Gmail__search_threads") && allowed.contains("mcp__claude_ai_Gmail__get_message"));
        assert!(allowed.contains("mcp__claude_ai_Notion__notion-create-pages"));
        assert!(!allowed.contains("create_draft") && !allowed.contains("trash") && !allowed.contains("WebSearch"));
        let denied = at("--disallowedTools");
        assert!(denied.contains("mcp__claude_ai_Gmail__create_draft") && denied.contains("mcp__claude_ai_Gmail__trash_message"));
        assert_eq!(at("--tools"), "", "no web, no files");
        // With Buddy's own tools as well, its server is still the only one in --mcp-config.
        let office = Claude::arguments(&TurnRequest {
            accounts: true,
            office: Some(super::super::Office {
                relay: "/d/bin/buddy-hook".into(),
                dir: "/d/documentos".into(),
                skills: "/d/skills".into(),
                link: None,
                read: vec![],
            }),
            ..Default::default()
        });
        let mcp = &office[office.iter().position(|a| a == "--mcp-config").unwrap() + 1];
        assert!(mcp.contains("\"buddy\"") && !office.join(" ").contains("--strict-mcp-config"));
        // Without the permission nothing changes: strict, no account tools.
        let plain = Claude::arguments(&TurnRequest::default()).join(" ");
        assert!(plain.contains("--strict-mcp-config") && !plain.contains("claude_ai") && !plain.contains("--disallowedTools"));
        // A turn with accounts never shares a warm process with one without.
        assert_ne!(signature(&TurnRequest { accounts: true, ..Default::default() }), signature(&TurnRequest::default()));
    }

    #[test]
    fn folders_allow_reading_and_editing_only_inside() {
        let args = Claude::arguments(&TurnRequest {
            folders: vec![
                crate::folders::AuthorizedFolder { path: "/u/docs".into(), can_edit: false },
                crate::folders::AuthorizedFolder { path: "/u/proyecto".into(), can_edit: true },
            ],
            ..Default::default()
        })
        .join(" ");
        assert!(args.contains("--tools WebSearch,WebFetch,Read,Glob,Grep,Edit,Write"), "{args}");
        assert!(args.contains("Read(//u/docs/**)") && args.contains("Read(//u/proyecto/**)"));
        assert!(args.contains("Edit(//u/proyecto/**),Write(//u/proyecto/**)") && !args.contains("Edit(//u/docs"));
        assert!(args.contains("--add-dir /u/docs") && args.contains("--add-dir /u/proyecto"));
    }

    #[test]
    fn haiku_extra_uses_thinking_budget_instead_of_unsupported_effort() {
        let request = TurnRequest { model: Some("claude-haiku-4-5-20251001".into()), effort: Some("xhigh".into()), ..Default::default() };
        let args = Claude::arguments(&request);
        assert!(!args.iter().any(|arg| arg == "--effort"));
        let settings: serde_json::Value = serde_json::from_str(&args[args.iter().position(|arg| arg == "--settings").unwrap() + 1]).unwrap();
        assert_eq!(settings["alwaysThinkingEnabled"], true);
        assert_eq!(settings["env"]["MAX_THINKING_TOKENS"], "31999");
        assert_eq!(settings["env"]["CLAUDE_CODE_MAX_OUTPUT_TOKENS"], "64000");
        let ordinary = TurnRequest { effort: Some("low".into()), ..request.clone() };
        assert!(Claude::arguments(&ordinary).iter().any(|arg| arg == "--effort"));
        assert_ne!(signature(&request), signature(&ordinary));
    }

    #[test]
    fn arguments_keep_the_cli_safe_and_resume() {
        let args = Claude::arguments(&TurnRequest {
            prompt: "hola".into(),
            system: "Eres Buddy".into(),
            resume: Some("s1".into()),
            model: Some("sonnet".into()),
            ..Default::default()
        });
        let joined = args.join(" ");
        assert!(joined.contains("--safe-mode") && joined.contains("--strict-mcp-config"));
        assert!(joined.contains("--tools WebSearch,WebFetch") && joined.contains("--permission-mode dontAsk"));
        assert!(joined.contains("--resume s1") && joined.contains("--model sonnet"));
        assert!(!joined.contains("hola"), "the prompt goes on stdin, never in the arguments");
    }
}
