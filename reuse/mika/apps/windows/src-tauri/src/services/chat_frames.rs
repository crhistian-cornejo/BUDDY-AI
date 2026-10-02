//! What the official CLIs print while they answer, reduced to the few things the chat shows: pieces of the answer,
//! what the agent is doing (a web search), the pages it used, and failures. Pure and platform independent, so it is
//! tested on any machine. Twin of ClaudeStreamParser.swift and CodexEventMapper on macOS.

use std::collections::HashMap;

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum Frame {
    /// A piece of the answer: a delta, or a whole Codex message. It already carries the paragraph break when it
    /// follows a tool call, so the pieces can simply be joined.
    Text(String),
    Tool { name: String, detail: String },
    Source { title: String, url: String },
    Progress(String),
    Error(String),
    /// Claude's final `result` text. Only used when nothing streamed.
    Final(String),
    /// The provider's session (Claude) or thread (Codex) id, to continue the conversation on the next turn.
    Session(String),
    /// Codex started creating an image: the chat shows a skeleton square in its place.
    ImageStarted,
    /// The image is ready at this path.
    Image(String),
    ImageFailed(String),
    /// The turn ended well (Codex app-server).
    Done,
    /// A command the user approved ran: what it was and what it printed (cut short). Shown collapsed in the chat.
    Command { command: String, output: String, code: Option<i32> },
}

pub struct FrameParser {
    provider: &'static str,
    saw_delta: bool,
    has_text: bool,
    ends_with_newline: bool,
    break_before_text: bool,
    fetch_urls: HashMap<String, String>,
    /// The commands (Bash / PowerShell) Claude called, by tool-use id, to pair them with their results.
    command_calls: HashMap<String, String>,
    /// The user's offset from UTC, for reset times ("se reinicia el domingo 20:38").
    utc_offset_minutes: i64,
}

impl FrameParser {
    pub fn new(provider: &'static str) -> Self {
        Self { provider, saw_delta: false, has_text: false, ends_with_newline: false, break_before_text: false, fetch_urls: HashMap::new(), command_calls: HashMap::new(), utc_offset_minutes: 0 }
    }

    pub fn with_utc_offset(mut self, minutes: i64) -> Self { self.utc_offset_minutes = minutes.clamp(-14 * 60, 14 * 60); self }

    pub fn feed(&mut self, line: &str) -> Vec<Frame> {
        let Ok(v) = serde_json::from_str::<Value>(line) else { return Vec::new() };
        let mut out = Vec::new();
        if self.provider == "codex" { self.codex(&v, &mut out) } else { self.claude(&v, &mut out) }
        out
    }

    fn text(&mut self, text: String, out: &mut Vec<Frame>) {
        if text.is_empty() { return; }
        let piece = if self.break_before_text && self.has_text && !self.ends_with_newline { format!("\n\n{text}") } else { text };
        self.break_before_text = false;
        self.has_text = true;
        self.ends_with_newline = piece.ends_with('\n');
        out.push(Frame::Text(piece));
    }

    /// Text that comes after a tool call starts a new paragraph instead of sticking to what was said before.
    fn tool(&mut self, name: &str, detail: &str, out: &mut Vec<Frame>) {
        if self.has_text { self.break_before_text = true; }
        out.push(Frame::Tool { name: name.to_string(), detail: detail.chars().take(160).collect() });
    }

    fn claude(&mut self, v: &Value, out: &mut Vec<Frame>) {
        match v["type"].as_str() {
            Some("stream_event") => {
                let event = &v["event"];
                match event["type"].as_str() {
                    Some("content_block_delta") if event["delta"]["type"] == "text_delta" => {
                        if let Some(t) = event["delta"]["text"].as_str() { self.saw_delta = true; self.text(t.to_string(), out); }
                    }
                    Some("content_block_start") if event["content_block"]["type"] == "tool_use" => {
                        if let Some(name) = event["content_block"]["name"].as_str() { self.tool(name, "", out); }
                    }
                    _ => {}
                }
            }
            Some("assistant") => {
                let Some(blocks) = v["message"]["content"].as_array() else { return };
                if !self.saw_delta {
                    let text = blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n");
                    self.text(text, out);
                }
                // The complete call says what is being searched or read (the start event only knows the tool's name).
                for block in blocks.iter().filter(|b| b["type"] == "tool_use") {
                    let input = &block["input"];
                    if block["name"] == "WebFetch" {
                        if let (Some(id), Some(url)) = (block["id"].as_str(), input["url"].as_str()) { self.fetch_urls.insert(id.to_string(), url.to_string()); }
                    }
                    if matches!(block["name"].as_str(), Some("Bash" | "PowerShell")) {
                        if let (Some(id), Some(command)) = (block["id"].as_str(), input["command"].as_str()) { self.command_calls.insert(id.to_string(), command.to_string()); }
                    }
                    let detail = input["query"].as_str().or_else(|| input["url"].as_str()).unwrap_or("");
                    if let (Some(name), false) = (block["name"].as_str(), detail.is_empty()) { self.tool(name, detail, out); }
                }
            }
            Some("user") => {
                let Some(blocks) = v["message"]["content"].as_array() else { return };
                for block in blocks.iter().filter(|b| b["type"] == "tool_result") {
                    // A command that ran: its output goes to the chat. What the gate refused (its reasons name MIKA) did not run.
                    if let Some(command) = block["tool_use_id"].as_str().and_then(|id| self.command_calls.remove(id)) {
                        let output = result_text(&block["content"]);
                        let refused = block["is_error"] == true && output.contains("MIKA");
                        if !refused { out.push(Frame::Command { command, output, code: None }); }
                        continue;
                    }
                    if block["is_error"] == true { continue; }
                    let links = search_links(&result_text(&block["content"]));
                    if !links.is_empty() {
                        out.extend(links.into_iter().map(|(title, url)| Frame::Source { title, url }));
                    } else if let Some(url) = block["tool_use_id"].as_str().and_then(|id| self.fetch_urls.get(id)) {
                        if let Some(host) = web_host(url) { out.push(Frame::Source { title: host, url: url.clone() }); }
                    }
                }
            }
            Some("result") if v["is_error"] == true => {
                out.push(Frame::Error(v["result"].as_str().unwrap_or("Claude no pudo completar el mensaje.").to_string()));
            }
            Some("result") => {
                if let Some(text) = v["result"].as_str() { out.push(Frame::Final(text.to_string())); }
            }
            Some("system") if v["subtype"] == "init" => {
                if let Some(id) = v["session_id"].as_str() { out.push(Frame::Session(id.to_string())); }
                out.push(Frame::Progress("Sesión de Claude iniciada".into()));
            }
            Some("rate_limit_event") => super::limits::record_claude(v),
            Some("system") if v["subtype"] == "api_retry" => out.push(Frame::Progress("Reintentando…".into())),
            _ => {}
        }
    }

    fn codex(&mut self, v: &Value, out: &mut Vec<Frame>) {
        let item = &v["item"];
        match v["type"].as_str() {
            Some("item.completed") => match item["type"].as_str() {
                Some("agent_message") => {
                    if let Some(text) = item["text"].as_str() {
                        if self.has_text { self.break_before_text = true; }
                        self.text(text.to_string(), out);
                    }
                }
                Some("web_search") => {
                    if item["action"]["type"] == "open_page" || item["action"]["type"] == "openPage" {
                        if let Some(url) = item["action"]["url"].as_str() {
                            if let Some(host) = web_host(url) { out.push(Frame::Source { title: host, url: url.to_string() }); return; }
                        }
                    }
                    let query = search_query(item);
                    if !query.is_empty() { self.tool("web_search", &query, out); }
                }
                _ => {}
            },
            Some("item.started") => match item["type"].as_str() {
                Some("web_search") => { let query = search_query(item); self.tool("web_search", &query, out); }
                Some("command_execution") => self.tool("command_execution", "", out),
                _ => out.push(Frame::Progress("Procesando el prompt".into())),
            },
            Some("turn.started") => out.push(Frame::Progress("Preparando respuesta".into())),
            Some("error") | Some("turn.failed") => {
                out.push(Frame::Error(v["message"].as_str().or_else(|| v["error"]["message"].as_str()).unwrap_or("El proveedor no pudo completar el mensaje.").to_string()));
            }
            _ => {}
        }
    }
}

impl FrameParser {
    /// One notification of `codex app-server` that belongs to the current turn (the caller filters by `turnId`).
    /// Twin of CodexEventMapper on macOS.
    pub fn codex_notification(&mut self, method: &str, params: &Value) -> Vec<Frame> {
        let mut out = Vec::new();
        let item = &params["item"];
        match method {
            "item/agentMessage/delta" => {
                if let Some(delta) = params["delta"].as_str() { self.text(delta.to_string(), &mut out); }
            }
            "item/started" => match item["type"].as_str() {
                Some("agentMessage") => { if self.has_text { self.break_before_text = true; } }
                Some("imageGeneration") => { if self.has_text { self.break_before_text = true; } out.push(Frame::ImageStarted); }
                Some("webSearch") => { let query = search_query(item); self.tool("webSearch", &query, &mut out); }
                Some(kind @ ("commandExecution" | "mcpToolCall" | "fileChange")) => self.tool(kind, "", &mut out),
                // MIKA's own tools: a command (after the click) or a read of the workspace.
                Some("dynamicToolCall") => match item["tool"].as_str() {
                    Some("run_command") => self.tool("commandExecution", "", &mut out),
                    Some("read_file" | "list_files" | "read_parley_odds") => self.tool("read", "", &mut out),
                    _ => {}
                },
                _ => {}
            },
            "item/completed" => match item["type"].as_str() {
                Some("webSearch") => {
                    if item["action"]["type"] == "openPage" || item["action"]["type"] == "open_page" {
                        if let Some(url) = item["action"]["url"].as_str() {
                            if let Some(host) = web_host(url) { out.push(Frame::Source { title: host, url: url.to_string() }); }
                        }
                    } else {
                        let query = search_query(item);
                        if !query.is_empty() { self.tool("webSearch", &query, &mut out); }
                    }
                }
                Some("imageGeneration") => {
                    if let Some(path) = item["savedPath"].as_str().filter(|p| !p.is_empty()) {
                        out.push(Frame::Image(path.to_string()));
                    } else {
                        out.push(Frame::ImageFailed(image_failure(&item["failure"], self.utc_offset_minutes)));
                    }
                }
                _ => {}
            },
            // Synthetic: MIKA ran a command the user approved (codex_server.rs).
            "mika/commandOutput" => {
                if let Some(command) = params["command"].as_str() {
                    out.push(Frame::Command { command: command.to_string(), output: params["output"].as_str().unwrap_or("").to_string(), code: params["code"].as_i64().map(|c| c as i32) });
                }
            }
            "turn/completed" => {
                if params["turn"]["status"] == "failed" {
                    out.push(Frame::Error(params["turn"]["error"]["message"].as_str().unwrap_or("Codex no pudo completar el mensaje.").to_string()));
                } else {
                    out.push(Frame::Done);
                }
            }
            "error" if params["willRetry"] == true => out.push(Frame::Progress("Reintentando…".into())),
            "error" => out.push(Frame::Error(params["error"]["message"].as_str().unwrap_or("Codex no pudo completar el mensaje.").to_string())),
            _ => {}
        }
        out
    }
}

/// Why an image could not be made, in the user's words. A usage limit says when it resets.
fn image_failure(failure: &Value, utc_offset_minutes: i64) -> String {
    let kind = failure["type"].as_str().or_else(|| failure["codexErrorInfo"].as_str()).or_else(|| failure.as_str()).unwrap_or("");
    if kind.contains("usageLimitExceeded") {
        if let Some(when) = failure["resetsAt"].as_i64().map(|t| reset_label(t, utc_offset_minutes)) {
            return format!("Límite de uso alcanzado; no se pudo crear la imagen. Se reinicia el {when}.");
        }
        return "Límite de uso alcanzado; no se pudo crear la imagen.".into();
    }
    "No se pudo crear la imagen.".into()
}

/// An epoch second as "domingo 20:38" in the user's local time.
fn reset_label(epoch: i64, utc_offset_minutes: i64) -> String {
    let local = epoch + utc_offset_minutes * 60;
    let days = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    // 1970-01-01 was a Thursday.
    const DAYS: [&str; 7] = ["jueves", "viernes", "sábado", "domingo", "lunes", "martes", "miércoles"];
    format!("{} {:02}:{:02}", DAYS[days.rem_euclid(7) as usize], secs / 3600, secs % 3600 / 60)
}

fn search_query(item: &Value) -> String {
    [item["query"].as_str(), item["action"]["query"].as_str()].into_iter().flatten().map(str::trim).find(|q| !q.is_empty()).unwrap_or("").to_string()
}

fn result_text(content: &Value) -> String {
    if let Some(text) = content.as_str() { return text.to_string(); }
    content.as_array().map(|blocks| blocks.iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default()
}

/// The note that goes in front of every question: today's date as the user's own machine writes it, so a search for
/// "today" or "the latest" looks for the right day even in a chat opened days ago. The text comes from the app's own
/// webview but ends up inside a prompt, so only plain date characters are kept.
pub fn today_note(today: Option<&str>) -> Option<String> {
    let clean: String = today?.chars().filter(|c| c.is_alphanumeric() || " ,.:;()/-+·_".contains(*c)).take(120).collect();
    let clean = clean.trim();
    if clean.is_empty() { return None; }
    Some(format!("Hoy es {clean}. Si piden noticias, resultados, precios u otro dato reciente, busca en la web tomando esa fecha como «hoy». Cita lo que uses como enlaces markdown [título](url)."))
}

/// The site of an http(s) address (without `www.`). Anything else — `javascript:`, `file:`, text — is refused:
/// only web addresses are ever sources, and the model's text decides them.
pub fn web_host(url: &str) -> Option<String> {
    let lower = url.trim().to_ascii_lowercase();
    let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = host.split(':').next()?;
    if host.is_empty() || host.chars().any(|c| c.is_whitespace() || c.is_control()) { return None; }
    Some(host.strip_prefix("www.").unwrap_or(host).to_string())
}

/// A web search answers with `Links: [{"title":…,"url":…}, …]` followed by a summary.
pub fn search_links(text: &str) -> Vec<(String, String)> {
    let Some(at) = text.find("Links: [") else { return Vec::new() };
    let start = at + "Links: ".len();
    let (mut depth, mut in_string, mut escaped, mut end) = (0i32, false, false, None);
    for (i, c) in text[start..].char_indices() {
        if in_string {
            if escaped { escaped = false } else if c == '\\' { escaped = true } else if c == '"' { in_string = false }
            continue;
        }
        match c {
            '"' => in_string = true,
            '[' => depth += 1,
            ']' => { depth -= 1; if depth == 0 { end = Some(start + i + 1); break; } }
            _ => {}
        }
    }
    let Some(end) = end else { return Vec::new() };
    let Ok(items) = serde_json::from_str::<Vec<Value>>(&text[start..end]) else { return Vec::new() };
    let mut seen: Vec<String> = Vec::new();
    let mut links = Vec::new();
    for item in items {
        let Some(url) = item["url"].as_str() else { continue };
        let Some(host) = web_host(url) else { continue };
        if seen.iter().any(|u| u == url) { continue; }
        seen.push(url.to_string());
        let title = item["title"].as_str().map(str::trim).filter(|t| !t.is_empty()).unwrap_or(&host).to_string();
        links.push((title, url.to_string()));
        if links.len() == 10 { break; }
    }
    links
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(provider: &'static str, lines: &[&str]) -> Vec<Frame> {
        let mut parser = FrameParser::new(provider);
        lines.iter().flat_map(|l| parser.feed(l)).collect()
    }

    const SEARCH_CALL: &str = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"WebSearch","input":{"query":"alianza lima resultado hoy"}}]}}"#;

    #[test]
    fn claude_text_streams_as_deltas_and_the_complete_message_is_not_repeated() {
        let frames = feed("claude", &[
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Ho"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"la"}}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hola"}]}}"#,
            r#"{"type":"result","result":"Hola"}"#,
        ]);
        assert_eq!(frames, vec![Frame::Text("Ho".into()), Frame::Text("la".into()), Frame::Final("Hola".into())]);
    }

    #[test]
    fn claude_without_partials_uses_the_complete_message() {
        let frames = feed("claude", &[r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"private"},{"type":"text","text":"hola"}]}}"#]);
        assert_eq!(frames, vec![Frame::Text("hola".into())]);
    }

    #[test]
    fn a_search_says_what_it_looks_for_and_its_links_become_sources() {
        let result = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"Web search results for query: \"x\"\n\nLinks: [{\"title\":\"Alianza - El Comercio\",\"url\":\"https://elcomercio.pe/a\"},{\"title\":\"Liga 1\",\"url\":\"https://liga1.pe/b\"}]\n\nAlianza ganó 2-1."}]}}"#;
        assert_eq!(feed("claude", &[SEARCH_CALL, result]), vec![
            Frame::Tool { name: "WebSearch".into(), detail: "alianza lima resultado hoy".into() },
            Frame::Source { title: "Alianza - El Comercio".into(), url: "https://elcomercio.pe/a".into() },
            Frame::Source { title: "Liga 1".into(), url: "https://liga1.pe/b".into() },
        ]);
    }

    #[test]
    fn a_fetched_page_is_a_source_only_when_it_worked() {
        let call = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"f1","name":"WebFetch","input":{"url":"https://www.depor.com/n/1"}}]}}"#;
        let ok = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"f1","content":"contenido"}]}}"#;
        let bad = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"f1","is_error":true,"content":"404"}]}}"#;
        assert_eq!(feed("claude", &[call, ok]).last(), Some(&Frame::Source { title: "depor.com".into(), url: "https://www.depor.com/n/1".into() }));
        assert!(!feed("claude", &[call, bad]).iter().any(|f| matches!(f, Frame::Source { .. })));
    }

    #[test]
    fn unsafe_or_broken_links_are_ignored() {
        for content in [r#"Links: [{\"title\":\"x\",\"url\":\"javascript:alert(1)\"}]"#, r#"Links: [{\"title\":"#, "nada"] {
            let line = format!(r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"t1","content":"{content}"}}]}}}}"#);
            assert!(feed("claude", &[&line]).is_empty(), "{content}");
        }
        assert_eq!(web_host("file:///etc/passwd"), None);
        assert_eq!(web_host("https://"), None);
        assert_eq!(web_host("https://user@www.Example.com:8080/x"), Some("example.com".into()));
    }

    #[test]
    fn text_after_a_tool_starts_a_new_paragraph() {
        let frames = feed("claude", &[
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Voy a buscar."}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"tool_use","name":"WebSearch"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Alianza ganó."}}}"#,
        ]);
        let joined: String = frames.iter().filter_map(|f| if let Frame::Text(t) = f { Some(t.as_str()) } else { None }).collect();
        assert_eq!(joined, "Voy a buscar.\n\nAlianza ganó.");
        let first = feed("claude", &[
            r#"{"type":"stream_event","event":{"type":"content_block_start","content_block":{"type":"tool_use","name":"WebSearch"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hola"}}}"#,
        ]);
        assert!(first.contains(&Frame::Text("Hola".into())), "sin texto antes de la herramienta no se añade salto");
    }

    #[test]
    fn every_question_carries_todays_date() {
        let note = today_note(Some("miércoles, 30 de septiembre de 2026, 23:41 (America/Lima)")).unwrap();
        assert!(note.starts_with("Hoy es miércoles, 30 de septiembre de 2026, 23:41 (America/Lima)."), "{note}");
        assert!(note.contains("[título](url)"));
        assert_eq!(today_note(None), None);
        assert_eq!(today_note(Some("  \n\t ")), None);
        let hostile = today_note(Some(&format!("hoy\n{}", "x".repeat(500)))).unwrap();
        assert!(!hostile.contains('\n') && hostile.len() < 400, "control characters are dropped and the length is capped");
    }

    #[test]
    fn claude_errors_and_results() {
        assert_eq!(feed("claude", &[r#"{"type":"result","is_error":true,"result":"Usage limit reached"}"#]), vec![Frame::Error("Usage limit reached".into())]);
    }

    #[test]
    fn codex_messages_are_whole_and_separate_paragraphs() {
        let frames = feed("codex", &[
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"Busco."}}"#,
            r#"{"type":"item.completed","item":{"type":"reasoning","text":"private"}}"#,
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"Ganó 2-1."}}"#,
        ]);
        assert_eq!(frames, vec![Frame::Text("Busco.".into()), Frame::Text("\n\nGanó 2-1.".into())]);
    }

    #[test]
    fn codex_searches_and_opened_pages() {
        assert_eq!(feed("codex", &[r#"{"type":"item.started","item":{"type":"web_search","id":"w","query":"alianza lima hoy"}}"#]),
                   vec![Frame::Tool { name: "web_search".into(), detail: "alianza lima hoy".into() }]);
        assert_eq!(feed("codex", &[r#"{"type":"item.completed","item":{"type":"web_search","id":"w","query":"","action":{"type":"open_page","url":"https://www.depor.com/n/1"}}}"#]),
                   vec![Frame::Source { title: "depor.com".into(), url: "https://www.depor.com/n/1".into() }]);
        assert_eq!(feed("codex", &[r#"{"type":"turn.failed","error":{"message":"Usage limit"}}"#]), vec![Frame::Error("Usage limit".into())]);
    }

    fn codex(lines: &[(&str, &str)]) -> Vec<Frame> {
        let mut parser = FrameParser::new("codex").with_utc_offset(-300);
        lines.iter().flat_map(|(m, p)| parser.codex_notification(m, &serde_json::from_str(p).unwrap())).collect()
    }

    #[test]
    fn codex_app_server_streams_deltas_and_ends_the_turn() {
        assert_eq!(codex(&[
            ("item/started", r#"{"item":{"type":"agentMessage","text":""}}"#),
            ("item/agentMessage/delta", r#"{"delta":"hola "}"#),
            ("item/agentMessage/delta", r#"{"delta":"mundo"}"#),
            ("item/completed", r#"{"item":{"type":"agentMessage","text":"hola mundo"}}"#),
            ("turn/completed", r#"{"turn":{"id":"t","status":"completed"}}"#),
        ]), vec![Frame::Text("hola ".into()), Frame::Text("mundo".into()), Frame::Done]);
        assert_eq!(codex(&[("turn/completed", r#"{"turn":{"status":"failed","error":{"message":"Usage limit"}}}"#)]), vec![Frame::Error("Usage limit".into())]);
        assert_eq!(codex(&[("error", r#"{"willRetry":true,"error":{"message":"x"}}"#)]), vec![Frame::Progress("Reintentando…".into())]);
    }

    #[test]
    fn codex_app_server_searches_pages_and_paragraphs() {
        let frames = codex(&[
            ("item/agentMessage/delta", r#"{"delta":"Busco."}"#),
            ("item/started", r#"{"item":{"type":"webSearch","query":"alianza lima hoy"}}"#),
            ("item/completed", r#"{"item":{"type":"webSearch","action":{"type":"openPage","url":"https://www.depor.com/n/1"}}}"#),
            ("item/started", r#"{"item":{"type":"agentMessage","text":""}}"#),
            ("item/agentMessage/delta", r#"{"delta":"Ganó."}"#),
        ]);
        assert_eq!(frames, vec![
            Frame::Text("Busco.".into()),
            Frame::Tool { name: "webSearch".into(), detail: "alianza lima hoy".into() },
            Frame::Source { title: "depor.com".into(), url: "https://www.depor.com/n/1".into() },
            Frame::Text("\n\nGanó.".into()),
        ]);
    }

    #[test]
    fn codex_images_become_a_skeleton_then_a_path_or_a_reason() {
        assert_eq!(codex(&[
            ("item/started", r#"{"item":{"type":"imageGeneration"}}"#),
            ("item/completed", r#"{"item":{"type":"imageGeneration","savedPath":"C:\\Users\\u\\.codex\\generated_images\\t\\a.png"}}"#),
        ]), vec![Frame::ImageStarted, Frame::Image(r"C:\Users\u\.codex\generated_images\t\a.png".into())]);
        // 2026-10-04 20:38 in Lima (UTC-5) is a Sunday.
        let failed = codex(&[("item/completed", r#"{"item":{"type":"imageGeneration","failure":{"type":"usageLimitExceeded","resetsAt":1791164280}}}"#)]);
        assert_eq!(failed, vec![Frame::ImageFailed("Límite de uso alcanzado; no se pudo crear la imagen. Se reinicia el domingo 20:38.".into())]);
        assert_eq!(codex(&[("item/completed", r#"{"item":{"type":"imageGeneration"}}"#)]), vec![Frame::ImageFailed("No se pudo crear la imagen.".into())]);
    }

    #[test]
    fn a_command_claude_ran_shows_its_output_and_a_refused_one_shows_nothing() {
        let call = |id: &str| format!(r#"{{"type":"assistant","message":{{"content":[{{"type":"tool_use","id":"{id}","name":"PowerShell","input":{{"command":"python hola.py"}}}}]}}}}"#);
        let result = |id: &str, error: bool, text: &str| format!(r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"{id}","is_error":{error},"content":"{text}"}}]}}}}"#);
        let ran = feed("claude", &[&call("a"), &result("a", false, "hola mundo")]);
        assert_eq!(ran.last(), Some(&Frame::Command { command: "python hola.py".into(), output: "hola mundo".into(), code: None }));
        let failed = feed("claude", &[&call("b"), &result("b", true, "Exit code 1: boom")]);
        assert!(matches!(failed.last(), Some(Frame::Command { output, .. }) if output.contains("boom")));
        let refused = feed("claude", &[&call("c"), &result("c", true, "El usuario denegó el comando en MIKA: no se ejecutó")]);
        assert!(!refused.iter().any(|f| matches!(f, Frame::Command { .. })));
        // A result that does not belong to a command call is not a command.
        assert!(!feed("claude", &[&result("zzz", false, "x")]).iter().any(|f| matches!(f, Frame::Command { .. })));
    }

    #[test]
    fn codex_tools_of_mika_are_announced_and_their_command_output_is_a_frame() {
        let frames = codex(&[
            ("item/started", r#"{"item":{"type":"dynamicToolCall","tool":"run_command"}}"#),
            ("mika/commandOutput", r#"{"turnId":"u","command":"dir","output":"a.txt","code":0}"#),
            ("item/started", r#"{"item":{"type":"dynamicToolCall","tool":"read_file"}}"#),
            ("item/started", r#"{"item":{"type":"dynamicToolCall","tool":"otra"}}"#),
        ]);
        assert_eq!(frames, vec![
            Frame::Tool { name: "commandExecution".into(), detail: String::new() },
            Frame::Command { command: "dir".into(), output: "a.txt".into(), code: Some(0) },
            Frame::Tool { name: "read".into(), detail: String::new() },
        ]);
    }

    #[test]
    fn claude_init_gives_the_session_to_resume() {
        assert_eq!(feed("claude", &[r#"{"type":"system","subtype":"init","session_id":"abc"}"#]), vec![Frame::Session("abc".into()), Frame::Progress("Sesión de Claude iniciada".into())]);
    }
}
