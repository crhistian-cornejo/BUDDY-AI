//! The gate between an agent of MIKA and a command: every command an agent wants to run is put in front of the
//! user on the island (the same approval card Claude Code permission requests use) and runs only after an explicit
//! click on Allow. A denial, a timeout, a stopped turn, a card that could not be shown or anything unexpected is a
//! refusal: nothing here ever allows by itself.
//!
//! Two ways in, one gate:
//!   * Claude agents: the turn is started with a PreToolUse hook on the command tools (agent_tools::gate_settings_json)
//!     that runs `mika-hook --gate PreToolUse`. The relay forwards the hook to the pipe with the turn's token (the
//!     `MIKA_GATE_TOKEN` environment variable of that `claude` process); `judge_hook` asks the user and answers
//!     `allow` or `deny`, which the relay turns into the hook's JSON. The relay prints a denial whenever it gets no
//!     answer at all.
//!   * Codex agents: the shell is off; `run_command` is a MIKA tool (`item/tool/call`). `run_command` asks the user
//!     and, only on Allow, runs the command itself in the agent's workspace.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};
use tauri::AppHandle;
use tokio::io::AsyncReadExt;

use crate::agents::claude_code::pipe::{self, IslandAnswer};
use super::agent_tools::{command_notes, Caps};
use super::named_agents::AgentDefinition;

/// The longest command that is shown (and so the longest that can be allowed). The relay cuts at the same size.
pub const MAX_COMMAND: usize = 16 * 1024;
/// A command's own time limit (default and maximum).
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_TIMEOUT: Duration = Duration::from_secs(120);
/// What the model gets back from a command, and what the chat keeps of it.
const MAX_MODEL_OUTPUT: usize = 20_000;
pub const MAX_CHAT_OUTPUT: usize = 4_000;

/// The relay that asks the user: the installed copy, or the one beside the app. None: commands are switched off
/// (a command could not be approved), never allowed without it.
pub fn relay_exe() -> Option<PathBuf> {
    let installed = super::settings::hook_exe_path();
    if installed.is_file() { return Some(installed); }
    std::env::current_exe().ok()?.parent().map(|dir| dir.join("mika-hook.exe")).filter(|p| p.is_file())
}

/// A running turn that may run commands.
#[derive(Clone)]
pub struct GateTurn {
    pub agent_id: String,
    pub agent_name: String,
    pub workspace: PathBuf,
    /// Set when the user presses Stop: a card waiting for a click is taken down and the command is refused.
    pub cancel: Arc<AtomicBool>,
}

fn turns() -> &'static Mutex<HashMap<String, GateTurn>> {
    static TURNS: OnceLock<Mutex<HashMap<String, GateTurn>>> = OnceLock::new();
    TURNS.get_or_init(Mutex::default)
}

/// A turn's token while it lives; dropping it closes the gate for that turn.
pub struct TurnGuard(String);

impl TurnGuard {
    pub fn token(&self) -> &str { &self.0 }
}

impl Drop for TurnGuard {
    fn drop(&mut self) { turns().lock().unwrap_or_else(|e| e.into_inner()).remove(&self.0); }
}

fn new_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
    (0..4).map(|_| {
        // `RandomState` is seeded by the OS: the token cannot be derived from the clock or the counter alone.
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(nanos);
        hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        format!("{:016x}", hasher.finish())
    }).collect()
}

/// Opens the gate for one turn of `agent`; None when the agent cannot run commands at all.
pub fn open_turn(agent: &AgentDefinition, workspace: &Path, cancel: Arc<AtomicBool>) -> Option<TurnGuard> {
    if !Caps::of(agent).run { return None; }
    let token = new_token();
    turns().lock().unwrap_or_else(|e| e.into_inner()).insert(token.clone(), GateTurn { agent_id: agent.id.clone(), agent_name: agent.name.clone(), workspace: workspace.to_path_buf(), cancel });
    Some(TurnGuard(token))
}

pub fn lookup(token: &str) -> Option<GateTurn> {
    if token.len() < 16 { return None; }
    turns().lock().unwrap_or_else(|e| e.into_inner()).get(token).cloned()
}

/// What the user decided, or why nothing ran.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Allowed,
    Denied(String),
}

impl Verdict {
    pub fn allowed(&self) -> bool { *self == Verdict::Allowed }
}

/// Only one card of ours at a time (the island shows one); the others wait their turn.
fn serial() -> &'static tokio::sync::Mutex<()> {
    static SERIAL: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    SERIAL.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// The card: a permission request of the agent, with the full command, its folder and what looks risky.
pub fn card_payload(turn: &GateTurn, tool: &str, command: &str, description: Option<&str>) -> Value {
    let mut input = json!({ "command": command, "workspace": turn.workspace.to_string_lossy() });
    if let Some(note) = description.map(str::trim).filter(|d| !d.is_empty()) { input["description"] = json!(note.chars().take(300).collect::<String>()); }
    let notes = command_notes(command, &turn.workspace);
    if !notes.is_empty() { input["warning"] = json!(notes.join(" ")); }
    json!({
        "hook_event_name": "PermissionRequest",
        "session_id": format!("agent-{}", turn.agent_id),
        "cwd": turn.workspace.to_string_lossy(),
        "tool_name": tool,
        "tool_input": input,
        "_agent": format!("agent:{}", turn.agent_id),
        "_agent_name": turn.agent_name,
    })
}

/// Asks the user whether `command` may run. Up to three tries while the island cannot show the card (another card
/// is up, the island is paused); any other answer ends it.
pub async fn ask(app: &AppHandle, turn: &GateTurn, tool: &str, command: &str, description: Option<&str>) -> Verdict {
    let cancel: &AtomicBool = &turn.cancel;
    ask_with(turn, tool, command, description, move |payload| pipe::ask_island(app, payload, Some(cancel))).await
}

/// `ask` with the island abstracted away (tests drive it with scripted answers).
pub async fn ask_with<F, Fut>(turn: &GateTurn, tool: &str, command: &str, description: Option<&str>, mut island: F) -> Verdict
where F: FnMut(Value) -> Fut, Fut: std::future::Future<Output = IslandAnswer> {
    if command.trim().is_empty() { return Verdict::Denied("El comando está vacío.".into()); }
    if command.len() > MAX_COMMAND { return Verdict::Denied("El comando es demasiado largo para revisarlo en la isla: no se ejecutó.".into()); }
    let stopped = || Verdict::Denied("Respuesta detenida: no se ejecutó.".into());
    // One card at a time; waiting for the turn is also interrupted by Stop.
    let _one = loop {
        if turn.cancel.load(Ordering::Relaxed) { return stopped(); }
        if let Ok(guard) = tokio::time::timeout(Duration::from_millis(200), serial().lock()).await { break guard; }
    };
    for attempt in 0..3 {
        if turn.cancel.load(Ordering::Relaxed) { return stopped(); }
        match island(card_payload(turn, tool, command, description)).await {
            IslandAnswer::Allow => return Verdict::Allowed,
            IslandAnswer::Deny => return Verdict::Denied("El usuario denegó el comando: no se ejecutó.".into()),
            IslandAnswer::TimedOut => return Verdict::Denied("El usuario no respondió a tiempo: no se ejecutó.".into()),
            IslandAnswer::Cancelled => return stopped(),
            IslandAnswer::NotShown => if attempt < 2 { tokio::time::sleep(Duration::from_millis(1500)).await; },
        }
    }
    Verdict::Denied("MIKA no pudo mostrar la solicitud en la isla: no se ejecutó.".into())
}

/// The relay's request (Claude): the hook payload of a PreToolUse, plus the turn token under `_gate`.
pub async fn judge_hook(app: &AppHandle, payload: &Value) -> Verdict {
    let Some(turn) = payload.get("_gate").and_then(Value::as_str).and_then(lookup) else {
        return Verdict::Denied("Este turno no puede ejecutar comandos.".into());
    };
    match hook_command(payload) {
        Ok((tool, command, description)) => ask(app, &turn, &tool, &command, description.as_deref()).await,
        Err(why) => Verdict::Denied(why),
    }
}

/// The tool, the command and its description from a PreToolUse payload; only the command tools pass, and a command
/// the relay had to cut is refused (the card could not show all of it).
pub fn hook_command(payload: &Value) -> Result<(String, String, Option<String>), String> {
    if payload["hook_event_name"] != "PreToolUse" { return Err("Solo se atienden solicitudes de herramientas.".into()); }
    let tool = payload["tool_name"].as_str().unwrap_or("");
    if !super::agent_tools::SHELL_TOOLS.contains(&tool) { return Err("Esta puerta solo acepta comandos.".into()); }
    if payload.get("_truncated").is_some_and(|t| t != &json!(false)) { return Err("El comando es demasiado largo para revisarlo en la isla: no se ejecutó.".into()); }
    let command = payload["tool_input"]["command"].as_str().ok_or("La solicitud no trae un comando.")?;
    Ok((tool.to_string(), command.to_string(), payload["tool_input"]["description"].as_str().map(str::to_string)))
}

// ── Running a command (Codex agents: MIKA runs it after the click) ───────────

/// What a command printed, kept short.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandRun {
    pub command: String,
    pub output: String,
    /// The exit code; None when it was stopped or timed out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<i32>,
}

/// `text` cut to `max` characters, saying so.
pub fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max { return text.to_string(); }
    let mut out: String = text.chars().take(max).collect();
    out.push_str("\n… (recortado)");
    out
}

/// The `run_command` tool: ask, then (only on Allow) run. Returns the text for the model, whether the tool call
/// counts as a success, and the run to show in the chat (None when nothing ran).
pub async fn run_command(app: &AppHandle, turn: &GateTurn, args: &Value) -> (String, bool, Option<CommandRun>) {
    let Some(command) = args["command"].as_str() else { return ("Falta el comando.".into(), false, None) };
    let timeout = Duration::from_secs(args["timeoutSeconds"].as_u64().unwrap_or(DEFAULT_TIMEOUT.as_secs()).clamp(1, MAX_TIMEOUT.as_secs()));
    match ask(app, turn, "PowerShell", command, args["description"].as_str()).await {
        Verdict::Denied(why) => (format!("{why} No lo intentes de otra forma."), false, None),
        Verdict::Allowed => {
            let run = execute(command, &turn.workspace, timeout, &turn.cancel).await;
            let for_model = format!("Código de salida: {}\n{}", run.code.map(|c| c.to_string()).unwrap_or_else(|| "ninguno (detenido o tiempo agotado)".into()), clip(&run.output, MAX_MODEL_OUTPUT));
            let ok = run.code == Some(0);
            let shown = CommandRun { output: clip(&run.output, MAX_CHAT_OUTPUT), ..run };
            (for_model, ok, Some(shown))
        }
    }
}

/// PowerShell with the command exactly as shown on the card. `-EncodedCommand` carries the text untouched (no
/// quoting to get wrong); the only addition is asking for UTF-8 output so accents survive.
fn powershell_args(command: &str) -> Vec<String> {
    let script = format!("$ProgressPreference = 'SilentlyContinue'; try {{ [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false) }} catch {{}}\n{command}");
    let utf16: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    // `-OutputFormat Text`: with an encoded command PowerShell would otherwise wrap its error stream in CLIXML.
    ["-NoLogo", "-NoProfile", "-NonInteractive", "-InputFormat", "None", "-OutputFormat", "Text", "-EncodedCommand"]
        .iter().map(|s| s.to_string()).chain(std::iter::once(super::base64(&utf16))).collect()
}

/// Runs the (already approved) command in `cwd`, with no stdin, no window, a clean environment and a time limit.
/// Stop kills it. Output is read up to 64 KB per stream.
pub async fn execute(command: &str, cwd: &Path, timeout: Duration, cancel: &AtomicBool) -> CommandRun {
    let run = |output: String, code: Option<i32>| CommandRun { command: command.to_string(), output, code };
    let mut cmd = tokio::process::Command::new("powershell.exe");
    cmd.args(powershell_args(command)).current_dir(cwd).stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
    cmd.creation_flags(0x0800_0000);
    for key in ["OPENAI_API_KEY", "CODEX_API_KEY", "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_BASE_URL", "OPENAI_BASE_URL", "CLAUDE_CODE_OAUTH_TOKEN", "MIKA_GATE_TOKEN"] { cmd.env_remove(key); }
    let Ok(mut child) = cmd.spawn() else { return run("No se pudo iniciar PowerShell.".into(), None) };
    let (Some(mut out), Some(mut err)) = (child.stdout.take(), child.stderr.take()) else { return run("No se pudo leer la salida.".into(), None) };
    let out_task = tokio::spawn(async move { let mut bytes = Vec::new(); let _ = (&mut out).take(65_536).read_to_end(&mut bytes).await; bytes });
    let err_task = tokio::spawn(async move { let mut bytes = Vec::new(); let _ = (&mut err).take(65_536).read_to_end(&mut bytes).await; bytes });
    let started = std::time::Instant::now();
    let mut note = String::new();
    let code = loop {
        if cancel.load(Ordering::Relaxed) { kill_tree(&child); let _ = child.kill().await; note = "\n[Detenido]".into(); break None; }
        if started.elapsed() > timeout { kill_tree(&child); let _ = child.kill().await; note = format!("\n[Tiempo agotado: {} s]", timeout.as_secs()); break None; }
        match tokio::time::timeout(Duration::from_millis(100), child.wait()).await {
            Ok(Ok(status)) => break Some(status.code().unwrap_or(-1)),
            Ok(Err(_)) => break None,
            Err(_) => {}
        }
    };
    let out = String::from_utf8_lossy(&out_task.await.unwrap_or_default()).to_string();
    let err = String::from_utf8_lossy(&err_task.await.unwrap_or_default()).to_string();
    let mut text = out.trim_end().to_string();
    if !err.trim().is_empty() { if !text.is_empty() { text.push('\n'); } text.push_str(&format!("[stderr]\n{}", err.trim_end())); }
    text.push_str(&note);
    run(text, code)
}

/// The command's children too (PowerShell may have started others).
fn kill_tree(child: &tokio::process::Child) {
    use std::os::windows::process::CommandExt;
    if let Some(pid) = child.id() {
        let _ = std::process::Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).creation_flags(0x0800_0000)
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::named_agents::{parse, BUILT_INS};

    fn agent(id: &str) -> AgentDefinition { parse(BUILT_INS.iter().find(|(b, _)| *b == id).unwrap().1).unwrap() }
    fn turn() -> GateTurn {
        GateTurn { agent_id: "maki".into(), agent_name: "MAKI".into(), workspace: std::env::temp_dir(), cancel: Arc::new(AtomicBool::new(false)) }
    }
    fn block<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
    }
    /// Answers the island gives in order.
    fn scripted(answers: Vec<IslandAnswer>) -> impl FnMut(Value) -> std::future::Ready<IslandAnswer> {
        let mut answers = answers.into_iter();
        move |_| std::future::ready(answers.next().expect("asked more often than scripted"))
    }
    fn deny_reason(v: Verdict) -> String { match v { Verdict::Denied(why) => why, Verdict::Allowed => panic!("allowed") } }

    #[test]
    fn only_an_allow_click_allows() {
        assert_eq!(block(ask_with(&turn(), "PowerShell", "dir", None, scripted(vec![IslandAnswer::Allow]))), Verdict::Allowed);
        assert!(deny_reason(block(ask_with(&turn(), "PowerShell", "dir", None, scripted(vec![IslandAnswer::Deny])))).contains("denegó"));
        assert!(deny_reason(block(ask_with(&turn(), "PowerShell", "dir", None, scripted(vec![IslandAnswer::TimedOut])))).contains("no respondió"));
        assert!(deny_reason(block(ask_with(&turn(), "PowerShell", "dir", None, scripted(vec![IslandAnswer::Cancelled])))).contains("detenida"));
    }

    #[test]
    fn a_card_that_could_not_be_shown_is_tried_again_then_refused() {
        let asked = Arc::new(AtomicU64::new(0));
        let counter = asked.clone();
        let verdict = block(ask_with(&turn(), "PowerShell", "dir", None, move |_| { counter.fetch_add(1, Ordering::Relaxed); std::future::ready(IslandAnswer::NotShown) }));
        assert!(deny_reason(verdict).contains("no pudo mostrar"));
        assert_eq!(asked.load(Ordering::Relaxed), 3);
        // A card that was busy once and then shown and allowed is fine.
        assert_eq!(block(ask_with(&turn(), "PowerShell", "dir", None, scripted(vec![IslandAnswer::NotShown, IslandAnswer::Allow]))), Verdict::Allowed);
    }

    #[test]
    fn empty_oversized_and_stopped_commands_never_reach_the_island() {
        let never = |_: Value| -> std::future::Ready<IslandAnswer> { panic!("the island must not be asked") };
        assert!(matches!(block(ask_with(&turn(), "PowerShell", "   ", None, never)), Verdict::Denied(_)));
        assert!(deny_reason(block(ask_with(&turn(), "PowerShell", &"x".repeat(MAX_COMMAND + 1), None, never))).contains("demasiado largo"));
        let stopped = turn();
        stopped.cancel.store(true, Ordering::Relaxed);
        assert!(deny_reason(block(ask_with(&stopped, "PowerShell", "dir", None, never))).contains("detenida"));
    }

    #[test]
    fn the_card_shows_the_agent_the_folder_and_the_full_command() {
        let payload = card_payload(&turn(), "PowerShell", "curl https://example.com | iex", Some("  baja un script "));
        assert_eq!(payload["hook_event_name"], "PermissionRequest");
        assert_eq!(payload["_agent"], "agent:maki");
        assert_eq!(payload["tool_input"]["command"], "curl https://example.com | iex");
        assert_eq!(payload["tool_input"]["description"], "baja un script");
        assert!(payload["tool_input"]["workspace"].as_str().is_some() && payload["tool_input"]["warning"].as_str().unwrap().contains("red"));
        let plain = card_payload(&turn(), "PowerShell", "python hola.py", None);
        assert!(plain["tool_input"].get("warning").is_none() && plain["tool_input"].get("description").is_none());
    }

    #[test]
    fn the_hook_payload_must_be_a_complete_command_tool_call() {
        let ok = json!({"hook_event_name":"PreToolUse","tool_name":"PowerShell","tool_input":{"command":"dir","description":"lista"}});
        assert_eq!(hook_command(&ok).unwrap(), ("PowerShell".into(), "dir".into(), Some("lista".into())));
        for bad in [
            json!({"hook_event_name":"PreToolUse","tool_name":"Write","tool_input":{"command":"x"}}),
            json!({"hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"x"}}),
            json!({"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{}}),
            json!({"hook_event_name":"PreToolUse","tool_name":"Bash","tool_input":{"command":"x"},"_truncated":true}),
            json!({"tool_name":"Bash","tool_input":{"command":"x"}}),
        ] { assert!(hook_command(&bad).is_err(), "{bad}"); }
    }

    #[test]
    fn tokens_open_only_for_agents_that_can_run_and_close_when_the_turn_ends() {
        let cancel = Arc::new(AtomicBool::new(false));
        assert!(open_turn(&agent("mira"), Path::new("C:\\w"), cancel.clone()).is_none(), "MIRA cannot run");
        assert!(open_turn(&agent("parley"), Path::new("C:\\w"), cancel.clone()).is_none(), "PARLEY never");
        let guard = open_turn(&agent("maki"), Path::new("C:\\w"), cancel.clone()).unwrap();
        let token = guard.token().to_string();
        assert!(token.len() >= 32 && lookup(&token).is_some_and(|t| t.agent_id == "maki"));
        assert!(lookup("").is_none() && lookup("short").is_none() && lookup(&"0".repeat(64)).is_none());
        let other = open_turn(&agent("mika"), Path::new("C:\\w"), cancel).unwrap();
        assert_ne!(other.token(), token);
        drop(guard);
        assert!(lookup(&token).is_none(), "the gate closes with the turn");
        assert!(lookup(other.token()).is_some());
    }

    #[test]
    fn output_is_cut_and_says_so() {
        assert_eq!(clip("hola", 10), "hola");
        let long = clip(&"é".repeat(50), 10);
        assert!(long.starts_with(&"é".repeat(10)) && long.ends_with("(recortado)"));
    }

    #[test]
    fn an_approved_command_runs_in_the_workspace_and_reports_its_output_and_code() {
        let dir = std::env::temp_dir().join(format!("mika-gate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("dato.txt"), "contenido ñ").unwrap();
        let cancel = AtomicBool::new(false);
        let run = block(execute("Get-Content -Encoding UTF8 dato.txt; Write-Output (Get-Location).Path", &dir, Duration::from_secs(30), &cancel));
        assert_eq!(run.code, Some(0), "{}", run.output);
        assert!(run.output.contains("contenido ñ"), "{}", run.output);
        // (the temp folder may be printed in its long form, so compare the folder's own name)
        assert!(run.output.to_lowercase().contains(&dir.file_name().unwrap().to_string_lossy().to_lowercase()), "cwd is the workspace: {}", run.output);
        let failing = block(execute("exit 7", &dir, Duration::from_secs(30), &cancel));
        assert_eq!(failing.code, Some(7));
        let slow = block(execute("Start-Sleep -Seconds 20", &dir, Duration::from_secs(1), &cancel));
        assert!(slow.code.is_none() && slow.output.contains("Tiempo agotado"), "{}", slow.output);
        let _ = std::fs::remove_dir_all(dir);
    }
}
