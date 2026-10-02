//! How much of the Claude and Codex plans is used (the 5-hour and weekly windows), learnt for free from what the
//! providers already say during MIKA's own turns: Claude's `rate_limit_event` and Codex's
//! `account/rateLimits/updated`. Nothing is asked for on purpose, so a provider shows up here after its first turn
//! in MIKA, and a figure carries the time it was seen.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{chat_store::now_ms, settings};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Window {
    /// "5 h", "semana"…
    pub label: String,
    /// 0–100.
    pub used_pct: f64,
    /// Unix seconds when the window resets, if the provider said.
    pub resets_at: Option<i64>,
    /// Unix milliseconds when this was seen.
    pub seen_ms: u64,
}

static LIMITS: Mutex<Option<BTreeMap<String, Vec<Window>>>> = Mutex::new(None);

fn path() -> std::path::PathBuf { settings::local_dir().join("usage-limits.json") }

fn with<R>(f: impl FnOnce(&mut BTreeMap<String, Vec<Window>>) -> R) -> R {
    let mut guard = LIMITS.lock().unwrap();
    let map = guard.get_or_insert_with(|| std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default());
    f(map)
}

fn store(provider: &str, window: Window) {
    with(|map| {
        let list = map.entry(provider.to_string()).or_default();
        match list.iter_mut().find(|w| w.label == window.label) { Some(w) => *w = window, None => list.push(window) }
        if let Ok(bytes) = serde_json::to_vec(map) { let _ = std::fs::write(path(), bytes); }
    });
}

fn pct(v: &Value) -> Option<f64> {
    let n = v.as_f64()?;
    // Claude sends a fraction (0.42); Codex a percentage (42).
    Some(if n <= 1.0 { n * 100.0 } else { n }.clamp(0.0, 100.0))
}

fn secs(v: &Value) -> Option<i64> { v.as_i64().map(|s| if s > 10_000_000_000 { s / 1000 } else { s }) }

fn minutes_label(mins: i64) -> String {
    match mins { 0..=359 => format!("{} h", (mins as f64 / 60.0).round().max(1.0)), 360..=2880 => "día".into(), _ => "semana".into() }
}

/// Claude Code: `{"type":"rate_limit_event","rate_limit_info":{"rateLimitType":"five_hour","utilization":0.42,"resetsAt":…}}`.
pub fn record_claude(event: &Value) {
    let info = &event["rate_limit_info"];
    let Some(used) = pct(&info["utilization"]) else { return };
    let label = match info["rateLimitType"].as_str() {
        Some("five_hour") => "5 h",
        Some(t) if t.starts_with("seven_day") => "semana",
        _ => return,
    };
    store("claude", Window { label: label.into(), used_pct: used, resets_at: secs(&info["resetsAt"]), seen_ms: now_ms() });
}

/// Codex: `account/rateLimits/updated` with `rateLimits.primary` / `.secondary` (`usedPercent`, `windowDurationMins`, `resetsAt`).
pub fn record_codex(params: &Value) {
    for key in ["primary", "secondary"] {
        let w = &params["rateLimits"][key];
        let Some(used) = w["usedPercent"].as_f64() else { continue };
        let label = w["windowDurationMins"].as_i64().map(minutes_label).unwrap_or_else(|| if key == "primary" { "5 h".into() } else { "semana".into() });
        store("codex", Window { label, used_pct: used.clamp(0.0, 100.0), resets_at: secs(&w["resetsAt"]), seen_ms: now_ms() });
    }
}

pub fn snapshot() -> BTreeMap<String, Vec<Window>> { with(|m| m.clone()) }

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_fractions_and_codex_percentages_become_percent() {
        assert_eq!(pct(&json!(0.42)), Some(42.0));
        assert_eq!(pct(&json!(42)), Some(42.0));
        assert_eq!(minutes_label(300), "5 h");
        assert_eq!(minutes_label(10080), "semana");
        assert_eq!(secs(&json!(1_791_164_280_000i64)), Some(1_791_164_280));
    }
}
