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
        let mut tools = TOOLS.to_string();
        if !reads.is_empty() {
            tools.push_str(",Read,Glob,Grep");
        }
        if !edits.is_empty() {
            tools.push_str(",Edit,Write");
        }
        // Bash is available only behind the gate, and never pre-allowed: the PreToolUse hook asks the user each time.
        if request.gate.is_some() {
            tools.push_str(",Bash");
        }
        let rule = |verb: &str, dir: &str| format!("{verb}(//{}/**)", dir.trim_start_matches('/'));
        let allowed = std::iter::once(TOOLS.to_string())
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
            if request.gate.is_some() { "--restricted" } else { "--safe-mode" },
            "--strict-mcp-config",
            "--mcp-config",
            r#"{"mcpServers":{}}"#,
            "--permission-mode",
            "dontAsk",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        args.extend(["--tools".into(), tools, "--allowedTools".into(), allowed]);
        for dir in &dirs {
            args.extend(["--add-dir".into(), dir.clone()]);
        }
        if let Some(gate) = &request.gate {
            let command = format!("\"{}\" --gate PreToolUse", gate.relay.to_string_lossy().replace('\\', "/"));
            let settings = serde_json::json!({
                "hooks": { "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": command, "timeout": 120 }] }] }
            });
            args.extend(["--settings".into(), settings.to_string()]);
        }
        if let Some(model) = &request.model {
            args.extend(["--model".into(), model.clone()]);
        }
        if let Some(effort) = &request.effort {
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

impl Default for Claude {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for Claude {
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
    format!("{}\u{1f}{}", Claude::arguments(&fresh).join("\u{1f}"), request.workspace.display())
}

/// Starts `claude` reading messages as stream-json, with readers for its output.
fn spawn_live(exe: &std::path::Path, request: &TurnRequest) -> Result<Live, String> {
    let _ = std::fs::create_dir_all(&request.workspace);
    let mut cmd = process::command(exe);
    cmd.args(Claude::arguments(request)).args(["--input-format", "stream-json"]).current_dir(&request.workspace);
    if let Some(gate) = &request.gate {
        cmd.env("BUDDY_GATE_TOKEN", &gate.token).env("BUDDY_DATA_DIR", &gate.data_dir);
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
        let Ok(obj) = serde_json::from_str::<Value>(line) else { return vec![] };
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
                    Some("content_block_delta") if event["delta"]["type"] == "text_delta" => {
                        match event["delta"]["text"].as_str() {
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
                        }
                    }
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
    let Some(start) = text.find("Links: [").map(|i| i + "Links: ".len()) else { return vec![] };
    let mut stream = serde_json::Deserializer::from_str(&text[start..]).into_iter::<Value>();
    let Some(Ok(Value::Array(items))) = stream.next() else { return vec![] };
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
        let delta = |t: &str| format!(r#"{{"type":"stream_event","event":{{"type":"content_block_delta","delta":{{"type":"text_delta","text":"{t}"}}}}}}"#);
        let tool = r#"{"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"tool_use","name":"WebSearch"}}}"#;
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
