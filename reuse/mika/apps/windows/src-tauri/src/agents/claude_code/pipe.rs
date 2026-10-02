// Named-pipe server for mika-hook.
//
// `\\.\pipe\mika-<sid>` — one instance per connection. Every hook event is
// forwarded to the island as a `hook` event. `PermissionRequest` is the only one
// that keeps its connection open: it waits for the island's decision and writes
// it back on the same pipe, which is how approving from the island works.
//
// Claude Code is never blocked by us. Three things guarantee it:
//   * mika-hook gives the connection 300 ms and exits cleanly if we are closed;
//   * we only wait for a human once the island has *confirmed* the card is on
//     screen, so a paused island or a webview that is not listening costs a few
//     hundred milliseconds, not two minutes;
//   * whatever happens we drop the connection after the decision timeout, and
//     the terminal takes over.
//
// What we write back is the bare word `allow` or `deny`. Turning that into the
// documented hookSpecificOutput JSON is mika-hook's job, so the wire format
// Claude Code expects lives in exactly one place.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::mpsc;

use crate::platform::island::WINDOW_LABEL;
use crate::platform::win_user::OwnerOnly;
use crate::services::log;

/// Slightly under mika-hook's own 110 s wait, so we always answer first.
const DECISION_TIMEOUT: Duration = Duration::from_secs(108);
/// How long the island gets to say "the card is up". This is the whole of B4:
/// without it, an island that is paused, hidden behind a crashed webview or
/// simply not listening would leave Claude Code staring at a prompt nobody can
/// see for nearly two minutes.
const ACK_TIMEOUT: Duration = Duration::from_millis(800);
const MAX_PAYLOAD: usize = 1 << 20;
/// A client has this long to send its request line. A silent client would
/// otherwise hold a pipe instance (and a task) forever.
const READ_TIMEOUT: Duration = Duration::from_secs(2);
/// Connections served at the same time. A real session needs one or two; the cap
/// only exists so that nothing can exhaust the pipe by opening instances and
/// saying nothing. Past it the connection is dropped and mika-hook exits cleanly.
const MAX_CONNECTIONS: usize = 32;

static ACTIVE: AtomicUsize = AtomicUsize::new(0);

/// Counts a served connection; releases its slot when dropped.
struct Slot;

impl Slot {
    fn take() -> Option<Slot> {
        if ACTIVE.fetch_add(1, Ordering::AcqRel) >= MAX_CONNECTIONS {
            ACTIVE.fetch_sub(1, Ordering::AcqRel);
            None
        } else {
            Some(Slot)
        }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::AcqRel);
    }
}

/// What the island can say about a permission request.
pub enum Reply {
    /// The card is on screen and a human can act on it.
    Ack,
    /// A human clicked: `allow` or `deny`.
    Decision(String),
    /// Nobody can act on it — paused, or another request already holds the card.
    Decline,
}

/// Permission requests the island has been told about.
#[derive(Default)]
pub struct Pending(pub Mutex<HashMap<String, mpsc::Sender<Reply>>>);

static COUNTER: AtomicU64 = AtomicU64::new(1);

/// `\\.\pipe\mika-<sid>` — must match mika-hook's `pipe_path()` exactly.
pub fn pipe_name() -> String {
    let key = crate::platform::win_user::current_user_sid()
        .unwrap_or_else(|| std::env::var("USERNAME").unwrap_or_else(|_| "user".into()));
    format!(r"\\.\pipe\mika-{key}")
}

/// One pipe instance whose access list names the current user and nobody else.
/// Without this, Windows' default DACL would let other accounts on the machine
/// read from the pipe. `first` also means we refuse to join a pipe somebody else
/// already owns under our name, rather than serving on top of it.
fn create_instance(name: &str, first: bool) -> std::io::Result<NamedPipeServer> {
    let mut options = ServerOptions::new();
    options.first_pipe_instance(first);
    match OwnerOnly::new() {
        // SAFETY: the pointer refers to a live SECURITY_ATTRIBUTES for the whole
        // call, and the kernel copies the descriptor before it returns.
        Some(mut security) => unsafe {
            options.create_with_security_attributes_raw(name, security.as_raw())
        },
        None => {
            log::line("could not build an owner-only pipe ACL — using the default one");
            options.create(name)
        }
    }
}

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let name = pipe_name();
        let mut server = match create_instance(&name, true) {
            Ok(s) => s,
            Err(err) => {
                log::line(format!("cannot open the relay pipe: {err}"));
                return;
            }
        };
        loop {
            if server.connect().await.is_err() {
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
            // Hand the connected instance to a task and listen on a fresh one.
            let next = match create_instance(&name, false) {
                Ok(s) => s,
                Err(err) => {
                    log::line(format!("cannot reopen the relay pipe: {err}"));
                    return;
                }
            };
            let connected = std::mem::replace(&mut server, next);
            let Some(slot) = Slot::take() else {
                log::line("too many open relay connections — dropping one");
                let _ = connected.disconnect();
                continue;
            };
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let _slot = slot;
                handle(app, connected).await
            });
        }
    });
}

/// Reads up to the first newline (or EOF), at most `MAX_PAYLOAD` bytes.
async fn read_request(pipe: &mut NamedPipeServer) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match pipe.read(&mut chunk).await {
            Ok(0) => return Some(buf),
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.contains(&b'\n') || buf.len() > MAX_PAYLOAD {
                    return Some(buf);
                }
            }
            Err(_) => return None,
        }
    }
}

async fn handle(app: AppHandle, mut pipe: NamedPipeServer) {
    let Ok(Some(buf)) = tokio::time::timeout(READ_TIMEOUT, read_request(&mut pipe)).await else {
        let _ = pipe.disconnect();
        return;
    };
    let line = match buf.iter().position(|b| *b == b'\n') {
        Some(i) => &buf[..i],
        None => &buf[..],
    };
    let Ok(mut payload) = serde_json::from_slice::<Value>(line) else { return };
    if !payload.is_object() {
        return;
    }

    // The gate: a command an agent of MIKA itself wants to run (see services/agent_gate.rs). It never reaches the
    // normal hook handling below, and a request without a live turn token is simply denied.
    if payload.get("_gate").is_some() {
        let verdict = crate::services::agent_gate::judge_hook(&app, &payload).await;
        let word = if verdict.allowed() { "allow" } else { "deny" };
        log::line(format!("gate {word}"));
        let _ = pipe.write_all(format!("{word}\n").as_bytes()).await;
        let _ = pipe.flush().await;
        let _ = pipe.disconnect();
        return;
    }

    let event = payload
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    if event != "PermissionRequest" {
        log::line(format!("hook {event}"));
        let _ = app.emit_to(WINDOW_LABEL, "hook", payload);
        let _ = pipe.disconnect();
        return;
    }

    let id = format!("{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
    let (tx, mut rx) = mpsc::channel::<Reply>(4);
    {
        let pending = app.state::<Pending>();
        pending.0.lock().unwrap().insert(id.clone(), tx);
    }
    payload["request_id"] = json!(id);
    log::line(format!("hook PermissionRequest id={id}"));
    let _ = app.emit_to(WINDOW_LABEL, "hook", payload);

    let decision = wait_for_decision(&id, &mut rx).await;
    app.state::<Pending>().0.lock().unwrap().remove(&id);

    // No decision: say nothing at all. mika-hook then writes nothing to stdout
    // and Claude Code asks in the terminal, exactly as if MIKA were closed.
    if let Some(d) = decision {
        let _ = pipe.write_all(format!("{d}\n").as_bytes()).await;
        let _ = pipe.flush().await;
    }
    let _ = pipe.disconnect();
}

/// Two waits: a short one for "the card is up", then the long one for a human.
async fn wait_for_decision(id: &str, rx: &mut mpsc::Receiver<Reply>) -> Option<String> {
    match tokio::time::timeout(ACK_TIMEOUT, rx.recv()).await {
        Ok(Some(Reply::Ack)) => {}
        // A click that beats the ack is still a click.
        Ok(Some(Reply::Decision(d))) => {
            log::line(format!("hook id={id} answered {d}"));
            return Some(d);
        }
        Ok(Some(Reply::Decline)) => {
            log::line(format!("hook id={id} not shown — terminal takes over"));
            return None;
        }
        Ok(None) => return None,
        Err(_) => {
            log::line(format!("hook id={id} island never acknowledged — terminal takes over"));
            return None;
        }
    }

    match tokio::time::timeout(DECISION_TIMEOUT, rx.recv()).await {
        Ok(Some(Reply::Decision(d))) => {
            log::line(format!("hook id={id} answered {d}"));
            Some(d)
        }
        Ok(Some(Reply::Decline)) => {
            log::line(format!("hook id={id} released without a decision"));
            None
        }
        _ => {
            log::line(format!("hook id={id} timed out — terminal takes over"));
            None
        }
    }
}

/// What came of a card that MIKA's own agents put on the island.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IslandAnswer {
    /// A click on Allow.
    Allow,
    /// A click on Deny, or the card was dismissed once it was on screen.
    Deny,
    /// The island could not show it (paused, another card up, not listening): nothing was decided.
    NotShown,
    /// The card was on screen and nobody answered in time.
    TimedOut,
    /// The turn was stopped while the card waited.
    Cancelled,
}

/// Puts a `PermissionRequest` card on the island for an agent of MIKA and waits for the human. Anything but an
/// explicit Allow click is "not allowed": the caller maps every other answer to a denial.
pub async fn ask_island(app: &AppHandle, mut payload: Value, cancel: Option<&std::sync::atomic::AtomicBool>) -> IslandAnswer {
    let id = format!("{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed));
    let (tx, mut rx) = mpsc::channel::<Reply>(4);
    app.state::<Pending>().0.lock().unwrap().insert(id.clone(), tx);
    payload["request_id"] = json!(id);
    log::line(format!("agent card id={id}"));
    let _ = app.emit_to(WINDOW_LABEL, "hook", payload);
    let answer = wait_for_click(&id, &mut rx, cancel).await;
    app.state::<Pending>().0.lock().unwrap().remove(&id);
    if matches!(answer, IslandAnswer::TimedOut | IslandAnswer::Cancelled) {
        // The card would be lying now: take it down.
        let _ = app.emit_to(WINDOW_LABEL, "approval-cancel", json!({ "requestId": id }));
    }
    log::line(format!("agent card id={id} -> {answer:?}"));
    answer
}

async fn wait_for_click(id: &str, rx: &mut mpsc::Receiver<Reply>, cancel: Option<&std::sync::atomic::AtomicBool>) -> IslandAnswer {
    let _ = id;
    let word = |d: &str| if d == "allow" { IslandAnswer::Allow } else { IslandAnswer::Deny };
    match tokio::time::timeout(ACK_TIMEOUT, rx.recv()).await {
        Ok(Some(Reply::Ack)) => {}
        // A click that beats the ack is still a click.
        Ok(Some(Reply::Decision(d))) => return word(&d),
        _ => return IslandAnswer::NotShown,
    }
    let deadline = std::time::Instant::now() + DECISION_TIMEOUT;
    loop {
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) { return IslandAnswer::Cancelled; }
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() { return IslandAnswer::TimedOut; }
        match tokio::time::timeout(left.min(Duration::from_millis(200)), rx.recv()).await {
            Ok(Some(Reply::Decision(d))) => return word(&d),
            // Released after being shown (the user closed the card): a refusal, not a retry.
            Ok(Some(Reply::Decline)) | Ok(None) => return IslandAnswer::Deny,
            Ok(Some(Reply::Ack)) | Err(_) => {}
        }
    }
}

fn send(app: &AppHandle, request_id: &str, reply: Reply, keep: bool) {
    let sender = {
        let pending = app.state::<Pending>();
        let mut map = pending.0.lock().unwrap();
        if keep { map.get(request_id).cloned() } else { map.remove(request_id) }
    };
    match sender {
        Some(tx) => {
            let _ = tx.try_send(reply);
        }
        None => log::line(format!("reply for id={request_id} — no pending request")),
    }
}

/// The island has the card on screen; the long wait may begin.
pub fn acknowledge(app: &AppHandle, request_id: &str) {
    send(app, request_id, Reply::Ack, true);
}

/// Nobody can act on this one — paused, or another card already holds the view.
pub fn decline(app: &AppHandle, request_id: &str) {
    log::line(format!("decline id={request_id}"));
    send(app, request_id, Reply::Decline, false);
}

/// Called by the island's Allow / Deny buttons. Only ever a bare word: turning
/// it into Claude Code's JSON is mika-hook's job.
pub fn answer(app: &AppHandle, request_id: &str, decision: &str) {
    let word = match decision {
        "allow" | "always" => "allow",
        _ => "deny",
    };
    log::line(format!("decision id={request_id} {word}"));
    send(app, request_id, Reply::Decision(word.to_string()), false);
}
