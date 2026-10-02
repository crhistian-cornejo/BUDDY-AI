//! Claude through the user's Claude Code subscription: `claude -p --output-format stream-json`, prompt on stdin,
//! one JSON object per line out. Ported from MIKA's ClaudeTurn.swift and ClaudeStreamParser.swift.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

use super::process;
use super::{Cancel, Failure, FailureKind, Provider, ProviderId, TurnEvent, TurnRequest};

/// Chat-only tools: search and read the web. No shell, no file edits (phase 4 adds them behind the approval gate).
const TOOLS: &str = "WebSearch,WebFetch";

pub struct Claude {
    exe: Option<PathBuf>,
}

impl Claude {
    pub fn new() -> Self {
        Self { exe: process::locate("claude") }
    }

    /// `--safe-mode` ignores the user's hooks, plugins and CLAUDE.md (Buddy's own prompt rules); no MCP servers.
    pub fn arguments(request: &TurnRequest) -> Vec<String> {
        let mut args: Vec<String> = [
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--safe-mode",
            "--strict-mcp-config",
            "--mcp-config",
            r#"{"mcpServers":{}}"#,
            "--tools",
            TOOLS,
            "--allowedTools",
            TOOLS,
            "--permission-mode",
            "dontAsk",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
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

    fn run(&self, request: &TurnRequest, cancel: &Cancel, emit: &mut dyn FnMut(TurnEvent)) {
        let Some(exe) = &self.exe else {
            emit(TurnEvent::Failed(Failure { kind: FailureKind::Missing, message: "claude no está instalado".into() }));
            return;
        };
        let _ = std::fs::create_dir_all(&request.workspace);
        let mut cmd = process::command(exe);
        cmd.args(Self::arguments(request)).current_dir(&request.workspace);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return emit(TurnEvent::Failed(Failure::new(format!("No se pudo iniciar Claude: {e}")))),
        };
        if let Some(mut stdin) = child.stdin.take() {
            let prompt = request.prompt.clone();
            std::thread::spawn(move || {
                let _ = stdin.write_all(prompt.as_bytes());
            });
        }
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");
        let err_text = Arc::new(Mutex::new(String::new()));
        let err_reader = {
            let err_text = err_text.clone();
            std::thread::spawn(move || {
                let mut buf = String::new();
                let _ = stderr.take(32_768).read_to_string(&mut buf);
                *err_text.lock().unwrap() = buf;
            })
        };
        let child = Arc::new(Mutex::new(child));
        let watcher = watch_cancel(child.clone(), cancel.clone());

        let mut parser = StreamParser::default();
        let mut finished = false;
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            for event in parser.feed(&line) {
                finished |= matches!(event, TurnEvent::Done | TurnEvent::Failed(_));
                emit(event);
            }
        }
        let _ = child.lock().unwrap().wait();
        cancel.cancel(); // ends the watcher
        let _ = watcher.join();
        let _ = err_reader.join();
        if !finished {
            let detail = err_text.lock().unwrap().trim().to_string();
            if detail.is_empty() {
                emit(TurnEvent::Done);
            } else {
                emit(TurnEvent::Failed(Failure::new(detail)));
            }
        }
    }
}

/// Interrupts the child once the turn is cancelled; polls only while a turn runs.
pub(crate) fn watch_cancel(child: Arc<Mutex<std::process::Child>>, cancel: Cancel) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        loop {
            if cancel.is_cancelled() {
                let mut c = child.lock().unwrap();
                if matches!(c.try_wait(), Ok(None)) {
                    process::interrupt(&mut c);
                }
                return;
            }
            if !matches!(child.lock().unwrap().try_wait(), Ok(None)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
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
            Some("result") => {
                if obj["is_error"] == true {
                    let message = obj["result"].as_str().unwrap_or("Claude no pudo completar la respuesta.");
                    vec![TurnEvent::Failed(Failure::new(message))]
                } else {
                    vec![TurnEvent::Done]
                }
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
