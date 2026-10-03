// Ported from MIKA (MIT, revision d050bc5): apps/windows/hook/src/main.rs
//! buddy-hook — the relay Claude Code, Codex and the Antigravity CLI (`agy`, Gemini) run on every hook event.
//!
//! Reads the hook JSON on stdin, adds a little terminal context, and hands it to Buddy: over the Unix socket
//! `<data_dir>/hooks.sock` on Mac (and other Unix), over the named pipe `\\.\pipe\buddy-<sid>` on Windows.
//!
//! Hard rule: **never block the agent.**
//! * If nobody is listening — Buddy is closed — we exit 0 immediately with nothing on stdout, and the session
//!   carries on untouched.
//! * Every step runs under a deadline enforced by the main thread, so a server that accepts the connection and
//!   then stops reading cannot wedge the session either: we abandon the worker and exit.
//! * Only `PermissionRequest` waits for an answer, because approving from the notch is the whole point. No answer
//!   means empty stdout, and the agent asks in the terminal exactly as if Buddy were not installed.
//!
//! Usage: `buddy-hook <EventName>` (Claude Code), `buddy-hook --codex <EventName>` (Codex) or
//! `buddy-hook --antigravity <EventName>` (agy). The name is also read from the JSON. Every payload leaves tagged
//! `_agent: "claude" | "codex" | "antigravity"` so Buddy knows whose session it is: the JSON itself can't say,
//! because Claude Code and Codex send the same field names.
//!
//! Antigravity speaks its own dialect (camelCase, `conversationId`, `workspacePaths`, no event name in the JSON), so
//! its payload is put into the shape Buddy reads (`antigravity_fields`), and agy, which reads a JSON object from
//! every hook's stdout, always gets the neutral `{}` back — Buddy never answers for an agy tool.
//!
//! Wire protocol: one JSON line from us; for `PermissionRequest` the server answers one line (`allow` or `deny`)
//! on the same connection.
//!
//! The same binary is also a stdio MCP server: `buddy-hook --mcp [--out <dir>] [--skills <dir>] [--read <dir>]…`
//! gives the agents Buddy runs the Office tools (create Word, Excel, PowerPoint; read documents), writing only inside
//! `<dir>` and reading only inside `<dir>`, the skills folder and each `--read` folder (see `mcp`).

use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::Duration;

mod mcp;
mod office;
mod tools;
mod transport;
#[cfg(windows)]
mod win;

/// Whole-run budget for an event nobody waits on: connect and write, no more.
const FIRE_AND_FORGET_BUDGET: Duration = Duration::from_secs(2);
/// How long a permission prompt may stay on screen before the terminal takes over.
const DECISION_BUDGET: Duration = Duration::from_secs(110);

/// Fields that are pointless to forward and can be enormous (a whole file read, a full command output).
const DROPPED_FIELDS: &[&str] =
    &["tool_response", "transcript_path", "agent_transcript_path", "transcriptPath", "artifactDirectoryPath"];
/// Longest string forwarded for an ordinary field (a file being written, a search pattern…). Buddy never needs
/// more than a preview of those.
const MAX_FIELD_LEN: usize = 2_000;
/// A `command` is what a person is asked to approve, so it gets a far larger allowance: cutting it would hide the
/// tail of what they are about to allow.
const MAX_COMMAND_LEN: usize = 16 * 1024;

/// Which coding agent ran us. Decided by argv alone: the hook command Buddy writes into each agent's own config is
/// the only thing that can say it reliably.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Agent {
    /// `buddy-hook <Event>`, from `~/.claude/settings.json`.
    Claude,
    /// `buddy-hook --codex <Event>`, from `~/.codex/hooks.json`.
    Codex,
    /// `buddy-hook --antigravity <Event>`, from `~/.gemini/config/hooks.json`.
    Antigravity,
}

impl Agent {
    /// The `_agent` tag Buddy routes on.
    fn tag(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
            Agent::Antigravity => "antigravity",
        }
    }
}

/// `[--codex | --antigravity] <Event>` → who ran us, and the event name argv carries (may be empty).
fn parse_args(args: &[String]) -> (Agent, String) {
    match args.first().map(String::as_str) {
        Some("--codex") => (Agent::Codex, args.get(1).cloned().unwrap_or_default()),
        Some("--antigravity") => (Agent::Antigravity, args.get(1).cloned().unwrap_or_default()),
        _ => (Agent::Claude, args.first().cloned().unwrap_or_default()),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--mcp") {
        std::process::exit(mcp::main(&args[1..]));
    }
    if args.first().map(String::as_str) == Some("--gate") {
        run_gate();
    }
    let (agent, _) = parse_args(&args);
    // A hook of one of Buddy's own turns (Buddy started that Claude/Codex/agy): not a session of the user's.
    if std::env::var_os("BUDDY_OWN_RUN").is_some() {
        finish(agent);
    }
    let Some((payload, event)) = read_event(&args) else { finish(agent) };

    // agy has no permission event: whatever its JSON says, an Antigravity hook never waits.
    let waits_for_answer = event == "PermissionRequest" && agent != Agent::Antigravity;
    let budget = if waits_for_answer { DECISION_BUDGET } else { FIRE_AND_FORGET_BUDGET };

    // The worker owns every blocking call. If it overruns the budget we simply stop listening and exit: the
    // process dying takes the connection with it.
    let (tx, rx) = mpsc::channel::<Option<String>>();
    std::thread::spawn(move || {
        let _ = tx.send(talk(&payload, waits_for_answer));
    });

    if let Ok(Some(decision)) = rx.recv_timeout(budget)
        && waits_for_answer
        && let Some(json) = decision_json(&decision)
    {
        let mut out = std::io::stdout();
        let _ = writeln!(out, "{json}");
        let _ = out.flush();
    }
    // Nothing printed: the agent asks in the terminal, as if we were not here.
    finish(agent);
}

/// What an agent reads on stdout when Buddy has nothing to say. Claude Code and Codex: nothing. agy reads a JSON
/// object from every hook (hooks.md, "Input/Output Contract"); `{}` changes nothing for PreInvocation, PostToolUse
/// and Stop, the only events Buddy installs for it.
fn silence(agent: Agent) -> &'static str {
    if agent == Agent::Antigravity { "{}" } else { "" }
}

fn finish(agent: Agent) -> ! {
    let text = silence(agent);
    if !text.is_empty() {
        let mut out = std::io::stdout();
        let _ = writeln!(out, "{text}");
        let _ = out.flush();
    }
    std::process::exit(0)
}

/// The PreToolUse output of the gate (Buddy's own agents, `buddy-hook --gate PreToolUse`). Here silence is not
/// an option: the command runs only if Buddy answers `allow`; anything else, including no answer, is a denial with
/// the reason Claude Code shows the model. Ported from MIKA's `mika-hook --gate` (MIT, revision d050bc5).
fn gate_json(answer: Option<&str>) -> String {
    let (decision, reason) = match answer.map(str::trim) {
        Some("allow") => ("allow", "Aprobado por el usuario en Buddy"),
        Some("deny") => ("deny", "El usuario denegó el comando en Buddy: no se ejecutó"),
        _ => ("deny", "Buddy no respondió: el comando no se ejecutó"),
    };
    format!(
        r#"{{"hookSpecificOutput":{{"hookEventName":"PreToolUse","permissionDecision":"{decision}","permissionDecisionReason":"{reason}"}}}}"#
    )
}

/// The gate: forwards the hook with this run's secret (BUDDY_GATE_TOKEN, set only on the `claude` processes Buddy
/// starts), waits for the word and prints the verdict. Always exits here.
fn run_gate() -> ! {
    let verdict = |answer: Option<String>| -> ! {
        let mut out = std::io::stdout();
        let _ = writeln!(out, "{}", gate_json(answer.as_deref()));
        let _ = out.flush();
        std::process::exit(0)
    };
    let token = std::env::var("BUDDY_GATE_TOKEN").unwrap_or_default();
    let mut raw = Vec::new();
    if token.trim().is_empty() || std::io::stdin().read_to_end(&mut raw).is_err() || raw.is_empty() {
        verdict(None)
    }
    let Some((payload, event)) = prepare(raw, Agent::Claude, "PreToolUse".into()) else { verdict(None) };
    if event != "PreToolUse" {
        verdict(None)
    }
    let Ok(mut json) = serde_json::from_str::<serde_json::Value>(payload.trim_end()) else { verdict(None) };
    json["_gate"] = serde_json::Value::String(token);
    let line = format!("{json}\n");
    let (tx, rx) = mpsc::channel::<Option<String>>();
    std::thread::spawn(move || {
        let _ = tx.send(talk(&line, true));
    });
    verdict(rx.recv_timeout(DECISION_BUDGET).ok().flatten())
}

/// The documented PermissionRequest output. Anything we do not recognise prints nothing at all rather than
/// guessing — silence is the safe answer.
///
/// Claude Code and Codex document the very same shape, so one function serves both:
/// https://code.claude.com/docs/en/hooks and https://developers.openai.com/codex/hooks ("PermissionRequest").
/// Codex rejects (fails closed on) `updatedInput`, `updatedPermissions` and `interrupt`, which is one more reason
/// this only ever prints a bare behaviour.
fn decision_json(decision: &str) -> Option<String> {
    let behavior = match decision.trim() {
        "allow" => r#"{"behavior":"allow"}"#.to_string(),
        "deny" => r#"{"behavior":"deny","message":"Denegado desde Buddy"}"#.to_string(),
        _ => return None,
    };
    Some(format!(r#"{{"hookSpecificOutput":{{"hookEventName":"PermissionRequest","decision":{behavior}}}}}"#))
}

/// Reads stdin, and returns the payload to forward plus the event name (`args` is argv without the program).
fn read_event(args: &[String]) -> Option<(String, String)> {
    let mut raw = Vec::new();
    if std::io::stdin().read_to_end(&mut raw).is_err() || raw.is_empty() {
        return None;
    }
    let (agent, arg_event) = parse_args(args);
    prepare(raw, agent, arg_event)
}

/// Everything `read_event` does once the bytes are in: parse, tag, trim.
fn prepare(mut raw: Vec<u8>, agent: Agent, arg_event: String) -> Option<(String, String)> {
    // Some shells hand us a UTF-8 BOM; serde_json would choke on it.
    if raw.starts_with(&[0xEF, 0xBB, 0xBF]) {
        raw.drain(..3);
    }

    let mut payload = serde_json::from_slice::<serde_json::Value>(&raw).ok()?;
    let map = payload.as_object_mut()?;

    // The event name is passed on argv by the hook command; the JSON usually carries it too. Trust argv when the
    // JSON is missing it. agy's JSON never carries it: argv decides, after the mapping.
    let event = if agent == Agent::Antigravity {
        antigravity_fields(map, &arg_event)
    } else {
        map.get("hook_event_name")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|s| !s.is_empty())
            .unwrap_or(arg_event)
    };
    map.insert("hook_event_name".into(), serde_json::Value::String(event.clone()));
    // Always overwritten: whatever the agent sent under this name, argv decides.
    map.insert("_agent".into(), serde_json::Value::String(agent.tag().into()));
    // Only the relay may say a payload was cut, and only gate mode may carry a secret.
    map.remove("_truncated");
    map.remove("_gate");
    map.remove("_app");

    for field in DROPPED_FIELDS {
        map.remove(*field);
    }

    // agy runs its hooks in the folder of hooks.json, which says nothing about the session.
    let cwd_missing = map.get("cwd").and_then(|v| v.as_str()).map(str::is_empty).unwrap_or(true);
    if cwd_missing && agent != Agent::Antigravity && let Ok(cwd) = std::env::current_dir() {
        map.insert("cwd".into(), serde_json::Value::String(cwd.to_string_lossy().to_string()));
    }

    // Which terminal the session runs in: context only, never a filter.
    for (key, var) in [
        ("term_program", "TERM_PROGRAM"),
        ("term_session_id", "TERM_SESSION_ID"),
        ("iterm_session_id", "ITERM_SESSION_ID"),
        ("bundle_id", "__CFBundleIdentifier"),
        ("wt_session", "WT_SESSION"),
        ("vscode_pid", "VSCODE_PID"),
        ("session_pid", "CLAUDE_CODE_SSE_PORT"),
    ] {
        // Claude Code's own variable says nothing about a Codex session (and would be plain wrong for Codex started
        // from a Claude Code terminal).
        if agent != Agent::Claude && key == "session_pid" {
            continue;
        }
        if !map.contains_key(key) {
            let value = std::env::var(var).unwrap_or_default();
            map.insert(key.into(), serde_json::Value::String(value));
        }
    }

    let mut cut = false;
    truncate_strings(&mut payload, "", &mut cut);
    if cut {
        // Buddy refuses to offer "Allow" for something it could not show whole.
        payload["_truncated"] = serde_json::Value::Bool(true);
    }

    let mut line = payload.to_string();
    line.push('\n');
    Some((line, event))
}

/// agy's payload (hooks.md: camelCase, `conversationId`, `workspacePaths`, `toolCall`) in the fields Buddy reads:
/// `session_id`, `cwd` (the first workspace folder), `workspace_paths`, `tool_name`/`tool_input`, and the event
/// name. A `Stop` that carries an error becomes `StopFailure`. Returns the event.
fn antigravity_fields(map: &mut serde_json::Map<String, serde_json::Value>, arg_event: &str) -> String {
    use serde_json::Value;
    let text = |v: Option<&Value>| v.and_then(Value::as_str).unwrap_or("").trim().to_string();
    if let Some(id) = map.remove("conversationId").and_then(|v| v.as_str().map(str::to_string)) {
        map.entry("session_id").or_insert(Value::String(id));
    }
    if let Some(dirs) = map.remove("workspacePaths").filter(Value::is_array) {
        if let Some(first) = dirs.as_array().and_then(|d| d.first()).and_then(Value::as_str) {
            map.entry("cwd").or_insert(Value::String(first.to_string()));
        }
        map.insert("workspace_paths".into(), dirs);
    }
    if let Some(call) = map.remove("toolCall") {
        map.insert("tool_name".into(), Value::String(text(call.get("name"))));
        map.insert("tool_input".into(), call.get("args").cloned().unwrap_or(Value::Null));
    }
    let reason = text(map.remove("terminationReason").as_ref());
    let failed = !text(map.get("error")).is_empty() || reason.to_ascii_lowercase().contains("error");
    if !reason.is_empty() {
        map.insert("termination_reason".into(), Value::String(reason));
    }
    if arg_event == "Stop" && failed { "StopFailure".into() } else { arg_event.to_string() }
}

fn limit_for(key: &str) -> usize {
    if key == "command" { MAX_COMMAND_LEN } else { MAX_FIELD_LEN }
}

/// Caps every string in the payload (a single Write can carry a whole file) and records in `cut` whether anything
/// was actually shortened. `key` is the name of the field the value sits under.
fn truncate_strings(value: &mut serde_json::Value, key: &str, cut: &mut bool) {
    match value {
        serde_json::Value::String(s) => {
            let limit = limit_for(key);
            if s.len() > limit {
                // Cut on a char boundary; a lone byte index can split UTF-8.
                let mut end = limit;
                while end > 0 && !s.is_char_boundary(end) {
                    end -= 1;
                }
                s.truncate(end);
                s.push('…');
                *cut = true;
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(|v| truncate_strings(v, key, cut)),
        serde_json::Value::Object(map) => map.iter_mut().for_each(|(k, v)| truncate_strings(v, k, cut)),
        _ => {}
    }
}

/// Connect, send, and — for a permission request — wait for Buddy's word.
pub(crate) fn talk(payload: &str, waits_for_answer: bool) -> Option<String> {
    let mut conn = transport::connect()?;

    if conn.write_all(payload.as_bytes()).is_err() {
        return None;
    }
    let _ = conn.flush();

    if !waits_for_answer {
        return None;
    }

    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        match conn.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.contains(&b'\n') || buf.len() > 4096 {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let answer = String::from_utf8_lossy(&buf).lines().next().unwrap_or_default().trim().to_string();
    (!answer.is_empty()).then_some(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_json_matches_the_documented_shape() {
        assert_eq!(
            decision_json("allow").unwrap(),
            r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}"#
        );
        assert_eq!(
            decision_json("deny\n").unwrap(),
            r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"Denegado desde Buddy"}}}"#
        );
        for json in [decision_json("allow").unwrap(), decision_json("deny").unwrap()] {
            let v: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PermissionRequest");
        }
    }

    #[test]
    fn anything_unrecognised_prints_nothing() {
        for answer in ["", "maybe", "always", "ALLOW", "allow please", r#"{"permissionDecision":"allow"}"#] {
            assert!(decision_json(answer).is_none(), "{answer:?}");
        }
    }

    #[test]
    fn argv_says_which_agent_ran_us() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(parse_args(&args(&["PreToolUse"])), (Agent::Claude, "PreToolUse".into()));
        assert_eq!(parse_args(&args(&["--codex", "PermissionRequest"])), (Agent::Codex, "PermissionRequest".into()));
        assert_eq!(parse_args(&args(&["--codex"])), (Agent::Codex, String::new()));
        assert_eq!(parse_args(&[]), (Agent::Claude, String::new()));
    }

    #[test]
    fn every_payload_is_tagged_with_its_agent() {
        let raw = br#"{"hook_event_name":"Stop","session_id":"s","cwd":"/p"}"#.to_vec();
        let (line, event) = prepare(raw.clone(), Agent::Codex, "Stop".into()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(event, "Stop");
        assert_eq!(v["_agent"], "codex");
        assert!(v.get("session_pid").is_none(), "Claude Code's variable is not Codex context");
        assert!(line.ends_with('\n') && line.matches('\n').count() == 1, "exactly one line on the wire");

        let (line, _) = prepare(raw, Agent::Claude, "Stop".into()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["_agent"], "claude");
    }

    #[test]
    fn forged_relay_keys_are_overwritten_or_dropped() {
        let raw = br#"{"hook_event_name":"PreToolUse","_agent":"codex","_truncated":false}"#.to_vec();
        let (line, _) = prepare(raw, Agent::Claude, "PreToolUse".into()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["_agent"], "claude");
        assert!(v.get("_truncated").is_none());
    }

    #[test]
    fn a_bom_is_skipped() {
        let mut raw = vec![0xEF, 0xBB, 0xBF];
        raw.extend_from_slice(br#"{"hook_event_name":"Stop"}"#);
        assert!(prepare(raw, Agent::Claude, String::new()).is_some());
    }

    #[test]
    fn payloads_keep_their_fields_and_lose_the_bulky_ones() {
        // Codex's SessionStart carries its own `source` ("startup"…): it must survive untouched.
        let raw = br#"{"hook_event_name":"SessionStart","source":"startup","transcript_path":"x"}"#.to_vec();
        let (line, _) = prepare(raw, Agent::Codex, String::new()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["source"], "startup");
        assert!(v.get("transcript_path").is_none());

        let raw = br#"{"hook_event_name":"PostToolUse","tool_response":{"stdout":"lots"},"agent_transcript_path":"y"}"#;
        let (line, _) = prepare(raw.to_vec(), Agent::Claude, String::new()).unwrap();
        assert!(!line.contains("agent_transcript_path"));
        assert!(!line.contains("tool_response"));
    }

    #[test]
    fn the_event_comes_from_argv_when_the_json_has_none() {
        let (line, event) = prepare(br#"{"cwd":"/p"}"#.to_vec(), Agent::Codex, "Interrupt".into()).unwrap();
        assert_eq!(event, "Interrupt");
        assert!(line.contains(r#""hook_event_name":"Interrupt""#));
    }

    #[test]
    fn a_missing_cwd_is_filled_in() {
        let (line, _) = prepare(br#"{"hook_event_name":"Stop"}"#.to_vec(), Agent::Claude, String::new()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert!(!v["cwd"].as_str().unwrap().is_empty());
    }

    #[test]
    fn input_that_is_not_a_json_object_is_dropped() {
        assert!(prepare(b"not json".to_vec(), Agent::Codex, "Stop".into()).is_none());
        assert!(prepare(b"[1,2]".to_vec(), Agent::Claude, "Stop".into()).is_none());
    }

    #[test]
    fn long_strings_are_cut_on_a_char_boundary() {
        let mut v = serde_json::json!({ "tool_input": { "content": "é".repeat(4000) } });
        let mut cut = false;
        truncate_strings(&mut v, "", &mut cut);
        let s = v["tool_input"]["content"].as_str().unwrap();
        assert!(s.len() <= MAX_FIELD_LEN + 4);
        assert!(s.ends_with('…'));
        assert!(cut, "a cut must be reported");
    }

    #[test]
    fn a_command_keeps_far_more_than_an_ordinary_field() {
        let long = "x".repeat(10_000);
        let mut v = serde_json::json!({ "tool_input": { "command": long, "content": long } });
        let mut cut = false;
        truncate_strings(&mut v, "", &mut cut);
        assert_eq!(v["tool_input"]["command"].as_str().unwrap().len(), 10_000);
        assert!(v["tool_input"]["content"].as_str().unwrap().len() <= MAX_FIELD_LEN + 4);
        assert!(cut, "the content was cut, so the payload is flagged");
    }

    #[test]
    fn a_cut_payload_is_flagged_on_the_wire() {
        let raw = format!(r#"{{"hook_event_name":"PermissionRequest","tool_input":{{"command":"{}"}}}}"#, "y".repeat(MAX_COMMAND_LEN + 1));
        let (line, _) = prepare(raw.into_bytes(), Agent::Claude, String::new()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["_truncated"], true);
        assert!(v["tool_input"]["command"].as_str().unwrap().ends_with('…'));
    }

    #[test]
    fn nothing_is_flagged_when_nothing_is_cut() {
        let mut v = serde_json::json!({ "tool_input": { "command": "ls -la", "description": "list" } });
        let mut cut = false;
        truncate_strings(&mut v, "", &mut cut);
        assert!(!cut);
    }

    #[test]
    fn the_gate_allows_only_on_an_explicit_allow() {
        assert!(gate_json(Some("allow\n")).contains(r#""permissionDecision":"allow""#));
        for answer in [Some("deny"), Some("maybe"), None] {
            assert!(gate_json(answer).contains(r#""permissionDecision":"deny""#), "{answer:?}");
        }
    }

    /// Payloads as agy 1.2.15 sent them to a hook (trimmed paths), see core/src/sessions/antigravity_hooks.rs.
    const AGY_POST_TOOL: &str = r#"{"artifactDirectoryPath":"/Users/u/.gemini/antigravity-cli/brain/c-1","conversationId":"c-1","error":"","modelName":"gemini-3.8-flash-low","stepIdx":2,"toolCall":{"args":{"AbsolutePath":"/Users/u/web/note.txt","toolAction":"Viewing note file","toolSummary":"View note.txt"},"name":"view_file"},"transcriptPath":"/Users/u/.gemini/antigravity-cli/brain/c-1/.system_generated/logs/transcript_full.jsonl","workspacePaths":["/Users/u/web","/Users/u/docs"]}"#;
    const AGY_STOP: &str = r#"{"conversationId":"c-1","error":"","executionNum":0,"fullyIdle":true,"modelName":"gemini-3.8-flash-low","terminationReason":"NO_TOOL_CALL","workspacePaths":["/Users/u/web"]}"#;

    #[test]
    fn argv_says_when_agy_ran_us() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(parse_args(&args(&["--antigravity", "Stop"])), (Agent::Antigravity, "Stop".into()));
        assert_eq!(Agent::Antigravity.tag(), "antigravity");
    }

    #[test]
    fn an_agy_payload_is_put_in_the_shape_buddy_reads() {
        let (line, event) = prepare(AGY_POST_TOOL.as_bytes().to_vec(), Agent::Antigravity, "PostToolUse".into()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(event, "PostToolUse");
        assert_eq!(v["hook_event_name"], "PostToolUse");
        assert_eq!(v["_agent"], "antigravity");
        assert_eq!(v["session_id"], "c-1");
        assert_eq!(v["cwd"], "/Users/u/web", "the first workspace folder, never the folder of hooks.json");
        assert_eq!(v["workspace_paths"], serde_json::json!(["/Users/u/web", "/Users/u/docs"]));
        assert_eq!(v["tool_name"], "view_file");
        assert_eq!(v["tool_input"]["AbsolutePath"], "/Users/u/web/note.txt");
        for gone in ["conversationId", "workspacePaths", "toolCall", "transcriptPath", "artifactDirectoryPath", "session_pid"] {
            assert!(v.get(gone).is_none(), "{gone} must not be forwarded");
        }
        assert!(line.ends_with('\n') && line.matches('\n').count() == 1);
    }

    #[test]
    fn an_agy_stop_is_done_unless_it_carries_an_error() {
        let (line, event) = prepare(AGY_STOP.as_bytes().to_vec(), Agent::Antigravity, "Stop".into()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!((event.as_str(), v["termination_reason"].as_str()), ("Stop", Some("NO_TOOL_CALL")));
        let failed = AGY_STOP.replace(r#""error":"""#, r#""error":"RESOURCE_EXHAUSTED: quota""#);
        assert_eq!(prepare(failed.into_bytes(), Agent::Antigravity, "Stop".into()).unwrap().1, "StopFailure");
        let failed = AGY_STOP.replace("NO_TOOL_CALL", "error");
        assert_eq!(prepare(failed.into_bytes(), Agent::Antigravity, "Stop".into()).unwrap().1, "StopFailure");
    }

    #[test]
    fn an_agy_payload_cannot_pretend_to_ask_for_permission() {
        // argv decides the event for agy, and the relay never waits for an Antigravity hook.
        let raw = br#"{"hook_event_name":"PermissionRequest","conversationId":"c"}"#.to_vec();
        let (line, event) = prepare(raw, Agent::Antigravity, "PreInvocation".into()).unwrap();
        assert_eq!(event, "PreInvocation");
        assert!(line.contains(r#""hook_event_name":"PreInvocation""#));
    }

    #[test]
    fn an_agy_payload_without_folders_gets_no_made_up_cwd() {
        let (line, _) = prepare(br#"{"conversationId":"c"}"#.to_vec(), Agent::Antigravity, "Stop".into()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert!(v.get("cwd").is_none());
    }

    #[test]
    fn agy_always_reads_a_json_object_and_the_others_silence() {
        assert_eq!(silence(Agent::Antigravity), "{}");
        assert!(serde_json::from_str::<serde_json::Value>(silence(Agent::Antigravity)).unwrap().is_object());
        assert_eq!(silence(Agent::Claude), "");
        assert_eq!(silence(Agent::Codex), "");
    }

    #[test]
    fn a_forwarded_payload_never_carries_a_secret() {
        let (line, _) = prepare(br#"{"hook_event_name":"Stop","_gate":"robado","_app":"robado"}"#.to_vec(), Agent::Claude, String::new()).unwrap();
        assert!(!line.contains("robado"));
    }
}
