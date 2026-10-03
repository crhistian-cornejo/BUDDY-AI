//! How much of the Claude, Codex and Gemini plans is used (5-hour, weekly, monthly windows). Ported from MIKA (MIT,
//! revision d050bc5): Services/Limits.swift + UsageRefresher.swift; Gemini is Buddy's own.
//!
//! Learnt for free from what the providers already say during Buddy's own turns (Claude's `rate_limit_event`, Codex's
//! `account/rateLimits/updated`). Fresh figures are asked for only when the user looks at them (the notch opens) and
//! the last ones are old: Codex answers `account/rateLimits/read` for free (at most every 5 min); Claude only reports
//! while it answers, so a one-word turn on the smallest model (at most every 30 min, and never when a real turn just
//! reported). Gemini (the Antigravity CLI) answers `agy -p /usage --output-format json` locally, with no model turn
//! and no quota spent (agy's changelog: "without starting an agent turn, spending quota, or leaving a conversation
//! behind"), at most every 10 min; and when a Gemini turn fails on quota (`antigravity_exhausted`) the notch says
//! «agotado» until a fresh read comes. Crossing 95 % of a window raises `UsageLow` once (the notch says «te queda 5 %»).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::events::{Event, EventBus};
use crate::providers::process;
use crate::store::Store;

const STORE_KEY: &str = "usage.limits";
/// A window at or over this share used raises `UsageLow` (once, until it drops again).
pub const LOW_AT: f64 = 95.0;
const CODEX_EVERY: i64 = 5 * 60;
const CLAUDE_EVERY: i64 = 30 * 60;
const ANTIGRAVITY_EVERY: i64 = 10 * 60;
/// The label of the window recorded when a Gemini turn ran out before any figures came.
pub const EXHAUSTED_LABEL: &str = "agotado";

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct UsageWindow {
    /// "5 h", "semana", "semana · Opus"…
    pub label: String,
    /// 0–100.
    pub used_pct: f64,
    /// Unix seconds when it resets, when the provider said.
    pub resets_at: Option<i64>,
    /// Unix seconds when this was seen.
    pub seen_at: i64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ProviderUsage {
    /// "claude" | "codex" | "antigravity"
    pub provider: String,
    /// "Claude" | "Codex" | "Gemini"
    pub name: String,
    pub windows: Vec<UsageWindow>,
}

pub struct Usage {
    store: Arc<Mutex<Store>>,
    bus: Arc<EventBus>,
    windows: Mutex<HashMap<String, Vec<UsageWindow>>>,
    last_codex_read: AtomicI64,
    last_claude_read: AtomicI64,
    last_antigravity_read: AtomicI64,
}

impl Usage {
    pub fn new(store: Arc<Mutex<Store>>, bus: Arc<EventBus>) -> Self {
        let saved = store
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .setting(STORE_KEY)
            .ok()
            .flatten()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default();
        Self {
            store,
            bus,
            windows: Mutex::new(saved),
            last_codex_read: AtomicI64::new(0),
            last_claude_read: AtomicI64::new(0),
            last_antigravity_read: AtomicI64::new(0),
        }
    }

    pub fn snapshot(&self) -> Vec<ProviderUsage> {
        let all = self.windows.lock().unwrap();
        [("claude", "Claude"), ("codex", "Codex"), ("antigravity", "Gemini")]
            .iter()
            .filter_map(|(p, name)| {
                let windows = all.get(*p).filter(|w| !w.is_empty())?.clone();
                Some(ProviderUsage { provider: p.to_string(), name: name.to_string(), windows })
            })
            .collect()
    }

    /// Claude Code: `rate_limit_info` of a `rate_limit_event`.
    pub fn record_claude(&self, info: &Value) {
        self.last_claude_read.store(now(), Ordering::SeqCst);
        let updated = claude_windows(info, now());
        if updated.is_empty() {
            return;
        }
        let mut all = self.windows.lock().unwrap();
        let list = all.entry("claude".into()).or_default();
        let before = list.clone();
        for w in updated {
            match list.iter_mut().find(|x| x.label == w.label) {
                Some(slot) => *slot = w,
                None => list.push(w),
            }
        }
        let after = list.clone();
        drop(all);
        self.changed("claude", &before, &after);
    }

    /// Codex: the result of `account/rateLimits/read` or the params of `account/rateLimits/updated`. Authoritative:
    /// windows no longer reported are dropped.
    pub fn record_codex(&self, params: &Value) {
        self.last_codex_read.store(now(), Ordering::SeqCst);
        let Some(windows) = codex_windows(params, now()) else { return };
        let before = self.windows.lock().unwrap().insert("codex".into(), windows.clone()).unwrap_or_default();
        self.changed("codex", &before, &windows);
    }

    /// Gemini: `command.data` of `agy -p /usage --output-format json`. Authoritative, like Codex's.
    pub fn record_antigravity(&self, data: &Value) {
        self.last_antigravity_read.store(now(), Ordering::SeqCst);
        let Some(windows) = antigravity_windows(data, now()) else { return };
        let before = self.windows.lock().unwrap().insert("antigravity".into(), windows.clone()).unwrap_or_default();
        self.changed("antigravity", &before, &windows);
    }

    /// A Gemini turn failed for lack of quota (`message` is what agy said, e.g. "RESOURCE_EXHAUSTED … Resets in
    /// 3h"): the notch shows «Gemini · agotado» (with the reset when agy said it) right away, and fresh figures are
    /// asked for, which replace it.
    pub fn antigravity_exhausted(self: &Arc<Self>, message: &str) {
        self.mark_antigravity_exhausted(message);
        let me = self.clone();
        std::thread::spawn(move || {
            if let Some(data) = read_antigravity() {
                me.record_antigravity(&data);
            }
        });
    }

    /// The «agotado» window alone (no fresh read).
    fn mark_antigravity_exhausted(&self, message: &str) {
        let seen = now();
        let window = UsageWindow {
            label: EXHAUSTED_LABEL.into(),
            used_pct: 100.0,
            resets_at: resets_in(message).map(|secs| seen + secs),
            seen_at: seen,
        };
        let before = self.windows.lock().unwrap().insert("antigravity".into(), vec![window.clone()]).unwrap_or_default();
        self.changed("antigravity", &before, &[window]);
        self.last_antigravity_read.store(seen, Ordering::SeqCst);
    }

    fn changed(&self, provider: &str, before: &[UsageWindow], after: &[UsageWindow]) {
        if let Ok(json) = serde_json::to_string(&*self.windows.lock().unwrap()) {
            let _ = self.store.lock().unwrap_or_else(|p| p.into_inner()).set_setting(STORE_KEY, &json);
        }
        self.bus.publish(Event::UsageChanged);
        for w in after {
            let was = before.iter().find(|b| b.label == w.label).map_or(0.0, |b| b.used_pct);
            if w.used_pct >= LOW_AT && was < LOW_AT {
                self.bus.publish(Event::UsageLow {
                    provider: provider.into(),
                    label: w.label.clone(),
                    left_pct: (100.0 - w.used_pct).max(0.0).round() as u32,
                });
            }
        }
    }

    /// Fresh figures when the user looks and the last ones are old (never more often than the limits above).
    pub fn refresh(self: &Arc<Self>) {
        let now = now();
        if now - self.last_codex_read.load(Ordering::SeqCst) >= CODEX_EVERY {
            self.last_codex_read.store(now, Ordering::SeqCst);
            let me = self.clone();
            std::thread::spawn(move || {
                if let Some(result) = read_codex() {
                    me.record_codex(&result);
                }
            });
        }
        if now - self.last_antigravity_read.load(Ordering::SeqCst) >= ANTIGRAVITY_EVERY {
            self.last_antigravity_read.store(now, Ordering::SeqCst);
            let me = self.clone();
            std::thread::spawn(move || {
                if let Some(data) = read_antigravity() {
                    me.record_antigravity(&data);
                }
            });
        }
        if now - self.last_claude_read.load(Ordering::SeqCst) >= CLAUDE_EVERY {
            self.last_claude_read.store(now, Ordering::SeqCst);
            let me = self.clone();
            std::thread::spawn(move || {
                if let Some((info, tokens)) = read_claude() {
                    me.record_claude(&info);
                    if let Some(t) = tokens {
                        let _ = me.store.lock().unwrap_or_else(|p| p.into_inner()).record_tokens("uso de planes", "claude", &t);
                    }
                }
            });
        }
    }
}

/// Claude sends a fraction (0.42); Codex a percentage (42).
fn percent(n: f64) -> f64 {
    if !n.is_finite() {
        return 0.0;
    }
    (if n <= 1.0 { n * 100.0 } else { n }).clamp(0.0, 100.0)
}

/// Unix seconds, from seconds or milliseconds.
fn seconds(n: f64) -> Option<i64> {
    if !n.is_finite() || n.abs() >= 1e15 {
        return None;
    }
    let v = n as i64;
    Some(if v > 10_000_000_000 { v / 1000 } else { v })
}

fn minutes_label(mins: i64) -> String {
    match mins {
        0..=359 => format!("{} h", ((mins as f64 / 60.0).round() as i64).max(1)),
        360..=2880 => "día".into(),
        10080 => "semana".into(),
        2881..=10079 => format!("{} días", mins / 1440),
        m if m >= 40320 && m <= 44640 => "mes".into(),
        m => format!("{} días", m / 1440),
    }
}

pub fn claude_windows(info: &Value, seen_at: i64) -> Vec<UsageWindow> {
    let mut out = Vec::new();
    if let Some(unified) = info["unifiedWindows"].as_object() {
        for (kind, window) in unified {
            let mut w = window.clone();
            w["rateLimitType"] = json!(kind);
            out.extend(claude_windows(&w, seen_at));
        }
    }
    let (Some(used), Some(kind)) = (info["utilization"].as_f64(), info["rateLimitType"].as_str()) else { return out };
    let label = match kind {
        "five_hour" => "5 h".to_string(),
        "seven_day" => "semana".into(),
        "seven_day_sonnet" => "semana · Sonnet".into(),
        "seven_day_opus" => "semana · Opus".into(),
        "monthly" | "thirty_day" => "mes".into(),
        other => other.replace('_', " "),
    };
    out.push(UsageWindow { label, used_pct: percent(used), resets_at: info["resetsAt"].as_f64().and_then(seconds), seen_at });
    out
}

pub fn codex_windows(params: &Value, seen_at: i64) -> Option<Vec<UsageWindow>> {
    let mut buckets: Vec<(String, Value)> = match params["rateLimitsByLimitId"].as_object() {
        Some(map) if !map.is_empty() => map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        _ if !params["rateLimits"].is_null() => vec![("codex".into(), params["rateLimits"].clone())],
        _ => return None,
    };
    buckets.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = Vec::new();
    for (id, bucket) in buckets {
        for key in ["primary", "secondary"] {
            let w = &bucket[key];
            let Some(used) = w["usedPercent"].as_f64().filter(|n| n.is_finite()) else { continue };
            let period = w["windowDurationMins"]
                .as_f64()
                .map(|m| minutes_label(m.round() as i64))
                .unwrap_or_else(|| if key == "primary" { "principal" } else { "secundaria" }.into());
            let label = if id == "codex" { period } else { format!("{} · {period}", bucket["limitName"].as_str().unwrap_or(&id)) };
            out.push(UsageWindow { label, used_pct: used.clamp(0.0, 100.0), resets_at: w["resetsAt"].as_f64().and_then(seconds), seen_at });
        }
    }
    Some(out)
}

/// The windows of agy's `/usage` (`command.data`): `groups[]` of `buckets[]` with `window` ("5h", "weekly"),
/// `remaining_fraction` (0–1) and `reset_time` (RFC 3339, UTC). The Gemini group's windows go plainly ("5 h",
/// "semana"); another group's (agy also serves Claude and GPT models) only once something of it is used, under its
/// name ("Claude y GPT · 5 h"). `None` when the answer has no groups.
pub fn antigravity_windows(data: &Value, seen_at: i64) -> Option<Vec<UsageWindow>> {
    let groups = data["groups"].as_array()?;
    let mut out = Vec::new();
    for group in groups {
        let name = group["name"].as_str().unwrap_or("").trim();
        let gemini = name.to_ascii_lowercase().starts_with("gemini");
        let buckets = group["buckets"].as_array().map(Vec::as_slice).unwrap_or_default();
        if !gemini && !buckets.iter().any(|b| b["remaining_fraction"].as_f64().is_some_and(|r| r < 1.0)) {
            continue;
        }
        for bucket in buckets {
            let Some(left) = bucket["remaining_fraction"].as_f64().filter(|n| n.is_finite()) else { continue };
            let period = match bucket["window"].as_str().unwrap_or("") {
                "5h" => "5 h".to_string(),
                "weekly" => "semana".into(),
                "daily" => "día".into(),
                "monthly" => "mes".into(),
                other if !other.is_empty() => other.into(),
                _ => bucket["name"].as_str().unwrap_or("límite").into(),
            };
            let label = if gemini { period } else { format!("{} · {period}", group_label(name)) };
            out.push(UsageWindow {
                label,
                used_pct: ((1.0 - left) * 100.0).clamp(0.0, 100.0),
                resets_at: bucket["reset_time"].as_str().and_then(iso_seconds),
                seen_at,
            });
        }
    }
    // The Gemini group first (agy lists it first today; the notch shows three rows).
    out.sort_by_key(|w| w.label.contains(" · "));
    Some(out)
}

/// "Claude and GPT models" → "Claude y GPT".
fn group_label(name: &str) -> String {
    let name = name.trim();
    let name = name.strip_suffix(" models").or_else(|| name.strip_suffix(" Models")).unwrap_or(name);
    name.replace(" and ", " y ")
}

/// Unix seconds of an RFC 3339 time (`2026-10-10T02:27:15Z`, `…T02:27:15.123+02:00`), without a date crate.
pub fn iso_seconds(text: &str) -> Option<i64> {
    let t = text.trim();
    let num = |s: &str| s.parse::<i64>().ok();
    let (date, rest) = t.split_once(['T', 't', ' '])?;
    let mut d = date.splitn(3, '-');
    let (y, mo, day) = (num(d.next()?)?, num(d.next()?)?, num(d.next()?)?);
    let (clock, offset) = match rest.find(['Z', 'z', '+', '-']) {
        Some(i) => rest.split_at(i),
        None => (rest, ""),
    };
    let mut c = clock.split(':');
    let (h, mi) = (num(c.next()?)?, num(c.next()?)?);
    let s = c.next().map(|s| s.split('.').next().unwrap_or("0")).and_then(num).unwrap_or(0);
    let shift = match offset.chars().next() {
        Some(sign @ ('+' | '-')) => {
            let (oh, om) = offset[1..].split_once(':').unwrap_or((&offset[1..], "0"));
            let secs = num(oh)? * 3600 + num(om)? * 60;
            if sign == '+' { secs } else { -secs }
        }
        _ => 0,
    };
    if !(1..=12).contains(&mo) || !(1..=31).contains(&day) {
        return None;
    }
    // Days from civil (Howard Hinnant), the inverse of log::utc_parts.
    let y = if mo <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + s - shift)
}

/// Seconds until the quota comes back, from agy's "Resets in 3h", "resets in 2h 15m", "Resets in 45m" or
/// "resets in 30s".
pub fn resets_in(message: &str) -> Option<i64> {
    let lower = message.to_ascii_lowercase();
    let at = lower.find("resets in")? + "resets in".len();
    let mut total = 0;
    let mut number = String::new();
    let mut any = false;
    for c in lower[at..].chars() {
        match c {
            '0'..='9' => number.push(c),
            ' ' => {}
            'd' | 'h' | 'm' | 's' if !number.is_empty() => {
                let n: i64 = number.parse().ok()?;
                total += n * match c {
                    'd' => 86_400,
                    'h' => 3600,
                    'm' => 60,
                    _ => 1,
                };
                number.clear();
                any = true;
            }
            // Letters after a unit ("3 hours", "45 min") are skipped.
            'a'..='z' if number.is_empty() => {}
            _ => break,
        }
    }
    any.then_some(total)
}

/// `agy`, as the Gemini provider finds it (never the Antigravity IDE's launcher of the same name).
fn locate_agy() -> Option<std::path::PathBuf> {
    use crate::providers::gemini::{EXE, is_ide_launcher};
    process::locate(EXE).filter(|p| !is_ide_launcher(p)).or_else(|| {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(std::path::PathBuf::from)?;
        let name = if cfg!(windows) { format!("{EXE}.exe") } else { EXE.to_string() };
        Some(home.join(".local/bin").join(name)).filter(|p| p.is_file() && !is_ide_launcher(p))
    })
}

/// `agy -p /usage --output-format json`: answered by the CLI itself, no model turn, no quota spent. Returns
/// `command.data`.
fn read_antigravity() -> Option<Value> {
    let exe = locate_agy()?;
    let mut cmd = process::command(&exe);
    cmd.args(["-p", "/usage", "--output-format", "json"]).current_dir(std::env::temp_dir());
    cmd.env(crate::providers::OWN_RUN_ENV, "1");
    let mut child = cmd.spawn().ok()?;
    drop(child.stdin.take());
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = (&mut stdout).take(256 * 1024).read_to_string(&mut text);
        let _ = tx.send(text);
    });
    let text = rx.recv_timeout(Duration::from_secs(45)).ok();
    let _ = child.kill();
    let _ = child.wait();
    usage_data(&text?)
}

/// `command.data` of agy's JSON answer to `/usage`, when it succeeded.
pub fn usage_data(text: &str) -> Option<Value> {
    let v: Value = serde_json::from_str(text.trim()).ok()?;
    if v["status"].as_str().is_some_and(|s| s != "SUCCESS") {
        return None;
    }
    v["command"]["data"].get("groups").is_some().then(|| v["command"]["data"].clone())
}

/// `account/rateLimits/read` on a short-lived `codex app-server` (free, no model).
fn read_codex() -> Option<Value> {
    let exe = process::locate("codex")?;
    let mut cmd = process::command(&exe);
    cmd.args(["app-server"]).current_dir(std::env::temp_dir());
    let mut child = cmd.spawn().ok()?;
    let mut stdin = child.stdin.take()?;
    let stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let send = |stdin: &mut std::process::ChildStdin, v: Value| writeln!(stdin, "{v}").and_then(|_| stdin.flush()).ok();
    send(&mut stdin, json!({ "id": 1, "method": "initialize", "params": { "clientInfo": { "name": "buddy", "version": env!("CARGO_PKG_VERSION") } } }))?;
    let mut result = None;
    while let Ok(line) = rx.recv_timeout(Duration::from_secs(20)) {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        match msg["id"].as_u64() {
            Some(1) => {
                send(&mut stdin, json!({ "method": "initialized" }))?;
                send(&mut stdin, json!({ "id": 2, "method": "account/rateLimits/read", "params": {} }))?;
            }
            Some(2) => {
                result = msg.get("result").cloned().filter(|r| !r.is_null());
                break;
            }
            _ => {}
        }
    }
    let _ = child.kill();
    result
}

/// A one-word turn on the smallest Claude model: its `rate_limit_event` carries the windows (and its cost is metered).
fn read_claude() -> Option<(Value, Option<crate::providers::TokenCount>)> {
    let exe = process::locate("claude")?;
    let mut cmd = process::command(&exe);
    cmd.args([
        "-p", "--safe-mode", "--output-format", "stream-json", "--verbose", "--permission-mode", "dontAsk", "--tools", "",
        "--strict-mcp-config", "--mcp-config", r#"{"mcpServers":{}}"#, "--model", "haiku",
    ])
    .current_dir(std::env::temp_dir());
    let mut child = cmd.spawn().ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(b"responde: ok");
    }
    let stdout = child.stdout.take()?;
    let (mut info, mut tokens) = (None, None);
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        if v["type"] == "rate_limit_event" {
            info = Some(v["rate_limit_info"].clone());
        }
        if v["type"] == "result" {
            tokens = crate::providers::TokenCount::from_claude_result(&v);
            break;
        }
    }
    let _ = child.kill();
    info.map(|i| (i, tokens))
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage() -> (Arc<Usage>, std::sync::mpsc::Receiver<Event>) {
        let bus = Arc::new(EventBus::default());
        let rx = bus.subscribe();
        (Arc::new(Usage::new(Arc::new(Mutex::new(Store::open_in_memory().unwrap())), bus)), rx)
    }

    #[test]
    fn claude_windows_are_read_from_rate_limit_events() {
        let w = claude_windows(&json!({ "rateLimitType": "five_hour", "utilization": 0.42, "resetsAt": 1_790_000_000_000u64 }), 1);
        assert_eq!((w[0].label.as_str(), w[0].used_pct.round(), w[0].resets_at), ("5 h", 42.0, Some(1_790_000_000)));
        let unified = claude_windows(&json!({ "unifiedWindows": { "seven_day": { "utilization": 0.9 } } }), 1);
        assert_eq!(unified[0].label, "semana");
    }

    #[test]
    fn codex_windows_name_their_periods() {
        let w = codex_windows(&json!({ "rateLimits": {
            "primary": { "usedPercent": 10, "windowDurationMins": 300 },
            "secondary": { "usedPercent": 63.5, "windowDurationMins": 10080, "resetsAt": 1_790_000_000 } } }), 1).unwrap();
        assert_eq!(w.iter().map(|x| x.label.as_str()).collect::<Vec<_>>(), ["5 h", "semana"]);
        assert!(codex_windows(&json!({}), 1).is_none());
    }

    /// What `agy -p /usage --output-format json` printed (agy 1.2.15).
    const AGY_USAGE: &str = r#"{"conversation_id":"","status":"SUCCESS","response":"…","num_turns":0,"usage":{"input_tokens":0,"output_tokens":0,"total_tokens":0},"command":{"name":"usage","data":{"description":"…","groups":[{"name":"Gemini Models","description":"Models within this group: Gemini Flash, Gemini Pro","buckets":[{"id":"gemini-weekly","name":"Weekly Limit Remaining","window":"weekly","remaining_fraction":0.9894470572471619,"reset_time":"2026-10-10T02:27:15Z"},{"id":"gemini-5h","name":"Five Hour Limit Remaining","window":"5h","remaining_fraction":0.25,"reset_time":"2026-10-03T07:27:15Z"}]},{"name":"Claude and GPT models","description":"…","buckets":[{"id":"3p-weekly","name":"Weekly Limit Remaining","window":"weekly","remaining_fraction":1,"reset_time":"2026-10-10T02:30:59Z"},{"id":"3p-5h","name":"Five Hour Limit Remaining","window":"5h","remaining_fraction":1,"reset_time":"2026-10-03T07:30:59Z"}]}]}}}"#;

    #[test]
    fn gemini_windows_come_from_agys_usage_command() {
        let data = usage_data(AGY_USAGE).unwrap();
        let w = antigravity_windows(&data, 1).unwrap();
        let labels: Vec<&str> = w.iter().map(|x| x.label.as_str()).collect();
        assert_eq!(labels, ["semana", "5 h"], "an untouched Claude/GPT group stays out of the strip");
        assert_eq!((w[0].used_pct.round(), w[1].used_pct.round()), (1.0, 75.0));
        assert_eq!(w[1].resets_at, Some(1_791_012_435), "2026-10-03T07:27:15Z");

        // Once something of the other group is used, it shows under its name, after Gemini's.
        let used = AGY_USAGE.replacen(r#""remaining_fraction":1,"#, r#""remaining_fraction":0.5,"#, 1);
        let w = antigravity_windows(&usage_data(&used).unwrap(), 1).unwrap();
        let labels: Vec<&str> = w.iter().map(|x| x.label.as_str()).collect();
        assert_eq!(labels, ["semana", "5 h", "Claude y GPT · semana", "Claude y GPT · 5 h"]);

        assert!(usage_data(r#"{"status":"ERROR","command":{"data":{"groups":[]}}}"#).is_none());
        assert!(usage_data("not json").is_none());
        assert!(antigravity_windows(&json!({}), 1).is_none());
    }

    #[test]
    fn rfc3339_times_are_read_without_a_date_crate() {
        assert_eq!(iso_seconds("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(iso_seconds("2026-10-02T12:00:00Z"), Some(1_790_942_400));
        assert_eq!(iso_seconds("2000-02-29T00:00:00.250Z"), Some(951_782_400));
        assert_eq!(iso_seconds("2026-10-02T14:00:00+02:00"), Some(1_790_942_400));
        assert_eq!(iso_seconds("2026-10-02T07:00:00-05:00"), Some(1_790_942_400));
        assert_eq!(iso_seconds("mañana"), None);
        assert_eq!(iso_seconds("2026-13-02T00:00:00Z"), None);
    }

    #[test]
    fn the_reset_is_read_from_a_quota_error() {
        assert_eq!(resets_in("RESOURCE_EXHAUSTED: Individual quota reached. Resets in 3h."), Some(3 * 3600));
        assert_eq!(resets_in("quota reached, resets in 2h 15m"), Some(2 * 3600 + 15 * 60));
        assert_eq!(resets_in("Resets in 45 minutes"), Some(45 * 60));
        assert_eq!(resets_in("Resets in 1 day"), Some(86_400));
        assert_eq!(resets_in("RESOURCE_EXHAUSTED"), None);
        assert_eq!(resets_in("Resets in soon"), None);
    }

    #[test]
    fn gemini_shows_next_to_claude_and_an_exhausted_turn_says_so() {
        let (u, rx) = usage();
        u.record_antigravity(&usage_data(AGY_USAGE).unwrap());
        let snap = u.snapshot();
        assert_eq!((snap[0].provider.as_str(), snap[0].name.as_str(), snap[0].windows.len()), ("antigravity", "Gemini", 2));
        assert!(rx.try_iter().any(|e| e == Event::UsageChanged));

        // The provider's failure: «Gemini · agotado», back in 3 h (until a fresh read replaces it).
        let before = now();
        u.mark_antigravity_exhausted("RESOURCE_EXHAUSTED: Individual quota reached. Resets in 3h.");
        let lows: Vec<Event> = rx.try_iter().filter(|e| matches!(e, Event::UsageLow { .. })).collect();
        assert_eq!(lows, vec![Event::UsageLow { provider: "antigravity".into(), label: EXHAUSTED_LABEL.into(), left_pct: 0 }]);
        let gemini = u.snapshot().into_iter().find(|p| p.provider == "antigravity").unwrap();
        assert_eq!((gemini.windows.len(), gemini.windows[0].label.as_str()), (1, EXHAUSTED_LABEL));
        let at = gemini.windows[0].resets_at.unwrap();
        assert!(at >= before + 3 * 3600 && at <= now() + 3 * 3600);
        // Real figures replace it.
        u.record_antigravity(&usage_data(AGY_USAGE).unwrap());
        assert!(u.snapshot()[0].windows.iter().all(|w| w.label != EXHAUSTED_LABEL));
    }

    #[test]
    fn crossing_95_percent_warns_once_and_figures_persist() {
        let (u, rx) = usage();
        u.record_claude(&json!({ "rateLimitType": "seven_day", "utilization": 0.9 }));
        u.record_claude(&json!({ "rateLimitType": "seven_day", "utilization": 0.96 }));
        u.record_claude(&json!({ "rateLimitType": "seven_day", "utilization": 0.97 }));
        let lows: Vec<Event> = rx.try_iter().filter(|e| matches!(e, Event::UsageLow { .. })).collect();
        assert_eq!(lows, vec![Event::UsageLow { provider: "claude".into(), label: "semana".into(), left_pct: 4 }]);
        let snap = u.snapshot();
        assert_eq!((snap[0].name.as_str(), snap[0].windows[0].used_pct.round()), ("Claude", 97.0));
        // A new Usage on the same store reads the saved figures.
        let again = Usage::new(u.store.clone(), Arc::new(EventBus::default()));
        assert_eq!(again.snapshot()[0].windows.len(), 1);
    }
}
