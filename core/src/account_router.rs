//! Subscription routing shared by Niko's chat and background reviews. No API credentials.
use crate::providers::{Failure, ProviderId};
use crate::store::Store;

pub fn compatible(id: ProviderId) -> bool {
    matches!(id, ProviderId::Claude | ProviderId::Codex)
}

pub fn model(id: ProviderId, complex: bool) -> &'static str {
    match (id, complex) {
        (ProviderId::Codex, false) => "gpt-6-luna",
        (ProviderId::Codex, true) => "gpt-6.1-sol",
        (_, false) => "haiku",
        (_, true) => "sonnet",
    }
}

fn key(id: ProviderId) -> String {
    format!("accounts.pause.{}", id.as_str())
}

/// Quota is read from Buddy's cached usage, never with an extra model turn. Expired data cannot block recovery.
pub fn available(store: &Store, id: ProviderId, now: i64) -> bool {
    if !compatible(id) {
        return false;
    }
    if store
        .setting(&key(id))
        .ok()
        .flatten()
        .and_then(|s| s.parse::<i64>().ok())
        .is_some_and(|at| at > now)
    {
        return false;
    }
    quota_reset(store, id, now).is_none()
}

fn quota_reset(store: &Store, id: ProviderId, now: i64) -> Option<i64> {
    let json = store.setting("usage.limits").ok().flatten()?;
    let all: serde_json::Value = serde_json::from_str(&json).ok()?;
    all[id.as_str()]
        .as_array()?
        .iter()
        .filter_map(|w| {
            // An Opus/Sonnet-only weekly bucket must not disable the fast Haiku route.
            let label = w["label"].as_str().unwrap_or_default().to_lowercase();
            if id == ProviderId::Claude && (label.contains("opus") || label.contains("sonnet")) {
                return None;
            }
            if w["usedPct"].as_f64()? < 100.0 {
                return None;
            }
            let at = w["resetsAt"].as_i64()?;
            (at > now).then_some(at)
        })
        .max()
}

/// Persist only real exhaustion (not transient throttling). A short bounded cooldown when no reset is known.
pub fn failed(store: &Store, id: ProviderId, failure: &Failure, now: i64) {
    if failure.is_no_usage() {
        let at = quota_reset(store, id, now)
            .or_else(|| crate::usage::resets_in(&failure.message).map(|s| now + s))
            .or_else(|| clock_reset(&failure.message, now))
            .unwrap_or(now + 15 * 60);
        let _ = store.set_setting(&key(id), &at.to_string());
    }
}

/// Claude reports a local clock plus an IANA zone, e.g. «resets 1:10am (America/Lima)».
fn clock_reset(message: &str, now: i64) -> Option<i64> {
    use chrono::TimeZone;
    let lower = message.to_lowercase();
    let clock = lower.split("resets ").nth(1)?.split_whitespace().next()?;
    let time = chrono::NaiveTime::parse_from_str(clock, "%I:%M%p").ok()?;
    let zone = message
        .rsplit_once('(')?
        .1
        .trim_end_matches(')')
        .trim()
        .parse::<chrono_tz::Tz>()
        .ok()?;
    let local = chrono::DateTime::from_timestamp(now, 0)?.with_timezone(&zone);
    let mut date = local.date_naive();
    if time <= local.time() {
        date = date.succ_opt()?;
    }
    Some(
        zone.from_local_datetime(&date.and_time(time))
            .single()?
            .timestamp(),
    )
}

pub fn succeeded(store: &Store, id: ProviderId) {
    let _ = store.set_setting(&key(id), "0");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quota_skips_dead_routes_and_recovers_without_restarting() {
        let store = Store::open_in_memory().unwrap();
        let f = Failure::new("usage limit reached, resets in 2h");
        failed(&store, ProviderId::Claude, &f, 100);
        assert!(!available(&store, ProviderId::Claude, 101));
        assert!(available(&store, ProviderId::Codex, 101));
        assert!(available(&store, ProviderId::Claude, 7301));
        failed(
            &store,
            ProviderId::Codex,
            &Failure::new("rate limit: too many requests"),
            100,
        );
        assert!(available(&store, ProviderId::Codex, 101));
        assert!(!available(&store, ProviderId::Antigravity, 101));
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-03T05:00:00Z")
            .unwrap()
            .timestamp();
        assert_eq!(
            clock_reset(
                "You've hit your session limit · resets 1:10am (America/Lima)",
                now
            ),
            Some(now + 70 * 60)
        );
    }
    #[test]
    fn cached_quota_honours_reset_and_models_are_efficient_on_both_routes() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_setting(
                "usage.limits",
                r#"{"codex":[{"usedPct":100,"resetsAt":200,"seenAt":50}]}"#,
            )
            .unwrap();
        assert!(!available(&store, ProviderId::Codex, 100));
        assert!(available(&store, ProviderId::Codex, 201));
        assert_eq!(model(ProviderId::Claude, false), "haiku");
        assert_eq!(model(ProviderId::Codex, false), "gpt-6-luna");
    }
}
