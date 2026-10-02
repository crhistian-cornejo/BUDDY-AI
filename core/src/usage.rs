//! How much of the Claude and Codex plans is used (5-hour, weekly, monthly windows). Ported from MIKA (MIT, revision
//! d050bc5): Services/Limits.swift + UsageRefresher.swift.
//!
//! Learnt for free from what the providers already say during Buddy's own turns (Claude's `rate_limit_event`, Codex's
//! `account/rateLimits/updated`). Fresh figures are asked for only when the user looks at them (the notch opens) and
//! the last ones are old: Codex answers `account/rateLimits/read` for free (at most every 5 min); Claude only reports
//! while it answers, so a one-word turn on the smallest model (at most every 30 min, and never when a real turn just
//! reported). Crossing 95 % of a window raises `UsageLow` once (the notch says «te queda 5 %»).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
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
    /// "claude" | "codex"
    pub provider: String,
    pub name: String,
    pub windows: Vec<UsageWindow>,
}

pub struct Usage {
    store: Arc<Mutex<Store>>,
    bus: Arc<EventBus>,
    windows: Mutex<HashMap<String, Vec<UsageWindow>>>,
    last_codex_read: AtomicI64,
    last_claude_read: AtomicI64,
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
        Self { store, bus, windows: Mutex::new(saved), last_codex_read: AtomicI64::new(0), last_claude_read: AtomicI64::new(0) }
    }

    pub fn snapshot(&self) -> Vec<ProviderUsage> {
        let all = self.windows.lock().unwrap();
        ["claude", "codex"]
            .iter()
            .filter_map(|p| {
                let windows = all.get(*p).filter(|w| !w.is_empty())?.clone();
                Some(ProviderUsage { provider: p.to_string(), name: if *p == "codex" { "Codex" } else { "Claude" }.into(), windows })
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
        if now - self.last_claude_read.load(Ordering::SeqCst) >= CLAUDE_EVERY {
            self.last_claude_read.store(now, Ordering::SeqCst);
            let me = self.clone();
            std::thread::spawn(move || {
                if let Some(info) = read_claude() {
                    me.record_claude(&info);
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

/// A one-word turn on the smallest Claude model: its `rate_limit_event` carries the windows.
fn read_claude() -> Option<Value> {
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
    let mut info = None;
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        if v["type"] == "rate_limit_event" {
            info = Some(v["rate_limit_info"].clone());
        }
        if v["type"] == "result" {
            break;
        }
    }
    let _ = child.kill();
    info
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
