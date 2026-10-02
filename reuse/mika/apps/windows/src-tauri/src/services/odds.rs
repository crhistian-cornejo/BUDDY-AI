//! Real Betano Perú odds for PARLEY, from OddsPapi's v4 API (https://oddspapi.io/en/docs). Betano has no public API;
//! OddsPapi is a third-party feed that carries `betano.pe` (a mirror of Betano's global prices). Nothing is asked
//! until the user saved an OddsPapi key and "Cuotas Betano" is on (saving a key switches it on).
//!
//! Football, tennis and basketball only. The plan is small (the free one is 250 calls) and Betano's in-play prices are
//! not in it, so calls are spent with care:
//! - the schedule (`/fixtures`: which matches and tournaments play, no prices) is asked for once for the next day and
//!   a half and reused for six hours by reviews and chat questions alike;
//! - one `/odds-by-tournaments` call carries five tournaments with every market of their matches, so a review is ONE
//!   call (the five tournaments with most matches in its window, widened when the window holds too few) and a chat
//!   question at most two; each tournament's answer is reused for 2 hours;
//! - `/odds?fixtureId=` (one call per live match) only where `/account` says the plan has Betano's live prices;
//! - today's calls are counted in `usage.json` against a daily allowance (the user's cap, or what is left of the plan
//!   spread over the days left in the month); once spent, the last prices are shown with their time, or PARLEY is told
//!   to use the web.
//!
//! Full returned markets (all lines, players and states) are kept in CSV and JSON; the prompt gets each match's main
//! markets. Sports and market catalogues are fetched once a week. Everything lives in `%LOCALAPPDATA%\MIKA\odds\`.
//!
//! v4 only takes the key as the `apiKey` query parameter; redirects are not followed, so it never leaves for another
//! host. Behind a company proxy the URL is visible to the proxy, like any HTTPS request it inspects.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{secrets, settings};

pub const KEY: &str = "oddspapi-api-key";
const BASE: &str = "https://api.oddspapi.io/v4";
const BOOKMAKER: &str = "betano.pe";
#[cfg(test)]
const FOOTBALL: i64 = 10;
/// Verified 2026-10-01: larger batches return INVALID_PARAMETER, maximum five tournament IDs.
const TOURNAMENT_BATCH: usize = 5;
/// Live matches each cost a request of their own, and only where the plan carries Betano's live prices.
const MAX_LIVE: usize = 2;
const MARKETS_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);
const REUSE: Duration = Duration::from_secs(10 * 60);
/// Billable odds calls per snapshot. One call carries five tournaments with every market of their matches, so a
/// review is one call; a question in the chat may take two.
const AUTO_BATCHES: usize = 1;
const MANUAL_BATCHES: usize = 2;
/// The schedule (matches and tournaments, no prices) is asked for once and reused this long.
const SCHEDULE_REUSE: Duration = Duration::from_secs(6 * 3600);
/// A tournament's prices are shared by reviews and chat questions for this long...
const ODDS_REUSE: Duration = Duration::from_secs(2 * 3600);
/// A tournament with a match starting within this many seconds is asked again after [`ODDS_REUSE_SOON`] instead:
/// prices move most just before kick-off.
const SOON: i64 = 4 * 3600;
const ODDS_REUSE_SOON: Duration = Duration::from_secs(30 * 60);
/// The morning snapshot asks for this many batches of five tournaments (the whole day, once).
const DAILY_BATCHES: usize = 3;
/// ...and, once the day's calls are spent, shown with their time for this long rather than nothing.
const STALE_ODDS: Duration = Duration::from_secs(12 * 3600);
/// Matches a review wants prices for (5 singles and 3 combinadas need a pool); a window with fewer is widened.
const WANTED_FIXTURES: usize = 12;
const WIDEN: i64 = 12 * 3600;
/// Markets per match in the text PARLEY reads (everything else is in the CSV).
const PREVIEW_MARKETS: usize = 8;
const PREVIEW_CHARS: usize = 10_000;
/// Without a cap of the user's, at least this many calls a day (a schedule and two reviews).
const MIN_DAILY_CALLS: usize = 3;

struct Cached { at: Instant, from: i64, to: i64, query: String, table: String }
static LAST: Mutex<Option<Cached>> = Mutex::new(None);
static FETCHING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A match worth looking at: live, or starting within the window.
#[derive(Debug, Clone, PartialEq)]
pub struct Fixture {
    pub id: String,
    pub home: String,
    pub away: String,
    pub tournament: String,
    pub tournament_id: i64,
    pub sport: String,
    /// Unix seconds.
    pub start: i64,
    pub live: bool,
}

/// The table, reused for 10 minutes when it covered the same window. `daily_calls` is the user's cap of billable
/// calls a day (0 = what is left of the plan spread over the days left in the month).
pub async fn snapshot(now: i64, window_end: i64, utc_offset: i64, daily_calls: u32) -> Result<String, String> {
    snapshot_range(now, now - 3 * 3600, window_end, utc_offset, None, daily_calls, AUTO_BATCHES).await
}

/// The morning snapshot: every match of the local day with the prices of the busiest tournaments, saved as CSV, JSON
/// and a short `resumen-del-dia.md`, so reviews and chat questions during the day reuse it instead of asking again.
pub async fn daily_snapshot(now: i64, utc_offset: i64, daily_calls: u32) -> Result<String, String> {
    let (from, to) = manual_window("hoy", now, utc_offset)?;
    snapshot_range(now, from, to, utc_offset, Some("hoy"), daily_calls, DAILY_BATCHES).await
}

/// A manual request follows its own day/hours and never reads the automatic horizon.
pub async fn manual_snapshot(now: i64, utc_offset: i64, query: &str, daily_calls: u32) -> Result<String, String> {
    let (from, to) = manual_window(query, now, utc_offset)?;
    snapshot_range(now, from, to, utc_offset, Some(query), daily_calls, MANUAL_BATCHES).await
}

async fn snapshot_range(now: i64, from: i64, to: i64, utc_offset: i64, query: Option<&str>, daily_calls: u32, batches: usize) -> Result<String, String> {
    let _fetching = FETCHING.lock().await;
    let cache_key = query.map(normalized).unwrap_or_default();
    if let Some(cached) = LAST.lock().unwrap().as_ref() {
        if cached.at.elapsed() < REUSE && (cached.from - from).abs() < 1800 && (cached.to - to).abs() < 1800 && cached.query == cache_key {
            return Ok(format!("Datos de caché; consulta original indicada abajo, reutilizada durante hasta 10 minutos.\n{}", cached.table));
        }
    }
    let table = fetch(now, from, to, utc_offset, query, daily_calls, batches).await?;
    *LAST.lock().unwrap() = Some(Cached { at: Instant::now(), from, to, query: cache_key, table: table.clone() });
    Ok(table)
}

fn normalized(text: &str) -> String {
    text.to_lowercase().chars().map(|c| match c { 'á' => 'a', 'é' => 'e', 'í' => 'i', 'ó' => 'o', 'ú' | 'ü' => 'u', 'ñ' => 'n', _ => c }).collect()
}

/// Supported explicit scopes; unknown date phrases request clarification instead of silently using settings.
pub fn manual_window(query: &str, now: i64, offset: i64) -> Result<(i64, i64), String> {
    let text = normalized(query);
    let day = (now + offset * 60).div_euclid(86400) * 86400 - offset * 60;
    let tokens: Vec<&str> = text.split(|c: char| c.is_whitespace() || matches!(c, ',' | '.' | '?' | '!' | ';' | ':')).filter(|s| !s.is_empty()).collect();
    let mut dates = Vec::new();
    for token in &tokens {
        let date = if token.len() == 10 && token.as_bytes()[4] == b'-' && token.as_bytes()[7] == b'-' {
            Some((*token).to_string())
        } else if token.len() == 10 && token.as_bytes()[2] == b'/' && token.as_bytes()[5] == b'/' {
            Some(format!("{}-{}-{}", &token[6..], &token[3..5], &token[..2]))
        } else { None };
        if let Some(date) = date {
            let utc = parse_iso(&format!("{date}T00:00:00Z")).filter(|s| iso_utc(*s).starts_with(&date))
                .ok_or_else(|| format!("Fecha inválida: {date}. Indica la fecha como AAAA-MM-DD."))?;
            dates.push(utc - offset * 60);
        }
    }
    if dates.len() > 1 { return Err("La consulta indica varias fechas. Precisa un día o un rango en horas para esta consulta de cuotas.".into()); }
    if let Some(&date) = dates.first() { return Ok((date, date + 86400)); }
    if text.contains("proximas") || text.contains("proximos") {
        if let Some(i) = tokens.iter().position(|s| matches!(*s, "horas" | "hora" | "h")) {
            if let Some(hours) = i.checked_sub(1).and_then(|i| tokens[i].parse::<i64>().ok()).filter(|h| (1..=36).contains(h)) {
                return Ok((now, now + hours * 3600));
            }
        }
    }
    if text.contains("pasado manana") || text.contains("pasadomanana") { return Ok((day + 2 * 86400, day + 3 * 86400)); }
    if tokens.contains(&"manana") && !tokens.contains(&"hoy") { return Ok((day + 86400, day + 2 * 86400)); }
    if tokens.contains(&"ayer") { return Ok((day - 86400, day)); }
    if text.contains("semana") || text.contains("mes") || text.contains("proxim") || (tokens.contains(&"manana") && tokens.contains(&"hoy")) {
        return Err("Precisa el día (hoy, mañana o AAAA-MM-DD) o las próximas N horas, hasta 36. La ventana automática no limita esta petición.".into());
    }
    Ok((day, day + 86400))
}

/// Match explicit tournament/team names against received metadata. Unmatched wording leaves coverage broad.
fn requested_fixtures(fixtures: Vec<Fixture>, query: &str) -> Vec<Fixture> {
    let text = normalized(query);
    let selected: Vec<Fixture> = fixtures.iter().filter(|f| {
        let tournament = normalized(f.tournament.split(',').next().unwrap_or(""));
        (tournament.len() > 4 && text.contains(&tournament))
            || ((text.contains("nations league") || text.contains("liga de naciones"))
                && (tournament.contains("nations league") || tournament.contains("liga de naciones"))
                && (!text.contains("uefa") || tournament.contains("uefa"))
                && (!text.contains("concacaf") || tournament.contains("concacaf")))
            || [f.home.as_str(), f.away.as_str()].iter().any(|name| name.len() > 4 && text.contains(&normalized(name)))
    }).cloned().collect();
    if selected.is_empty() { fixtures } else { selected }
}

async fn fetch(now: i64, from: i64, window_end: i64, utc_offset: i64, query: Option<&str>, daily_calls: u32, batches: usize) -> Result<String, String> {
    let key = secrets::get(KEY).ok_or("Falta la clave de OddsPapi.")?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(40))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "No se pudo preparar la conexión.")?;
    let get = |path: String| {
        let (http, key) = (http.clone(), key.clone());
        async move {
            let sep = if path.contains('?') { '&' } else { '?' };
            let response = http.get(format!("{BASE}{path}{sep}apiKey={key}")).header("Accept", "application/json")
                .send().await.map_err(|_| "Sin conexión con OddsPapi.".to_string())?;
            let status = response.status().as_u16();
            if status == 401 || status == 403 { return Err("OddsPapi no acepta la clave (¿es de la API v4?).".to_string()); }
            if status == 429 { return Err("OddsPapi: se acabó la cuota de consultas por ahora.".to_string()); }
            if !(200..300).contains(&status) {
                // OddsPapi says why in a short JSON message; never echo anything that could hold the key.
                let detail = response.json::<Value>().await.ok()
                    .map(|v| v.to_string().replace(&key, "[redacted]").chars().take(350).collect::<String>())
                    .unwrap_or_default();
                return Err(format!("OddsPapi {} respondió {status}{}{detail}.", path.split('?').next().unwrap_or(""), if detail.is_empty() { "" } else { ": " }));
            }
            response.json::<Value>().await.map_err(|_| "OddsPapi devolvió datos que no se entienden.".to_string())
        }
    };

    // Account is unmetered. Do not store or display this response: it includes the API key.
    let account = get("/account".into()).await.ok();
    let quota = account.as_ref().and_then(quota_remaining);
    let live_odds = account.as_ref().is_some_and(has_live_odds);
    let limit = account.as_ref().and_then(quota_limit);
    drop(account);
    if quota == Some(0) { return Err("OddsPapi: no quedan llamadas en la cuota actual.".into()); }

    // Today's allowance: what is left of the plan spread over the days left in the month, or the user's own cap.
    let day = super::chat_store::date_label((now.max(0) as u64) * 1000, utc_offset);
    let mut usage = load_usage();
    if usage.day != day { usage = Usage { day, ..Usage::default() }; }
    usage.remaining = quota;
    usage.limit = limit;
    let allowance = daily_allowance(quota.map(|q| q + usage.calls), days_left_in_month(now, utc_offset), daily_calls);
    usage.allowance = allowance;
    let budget = allowance.saturating_sub(usage.calls).min(quota.unwrap_or(usize::MAX));
    let mut calls = 0usize;
    let spent = |usage: &mut Usage, calls: usize| { usage.calls += calls; usage.remaining = usage.remaining.map(|r| r.saturating_sub(calls)); save_usage(usage); };
    let capped = || format!("OddsPapi: se alcanzó el tope de {allowance} llamadas de hoy (quedan {} en el plan de {}). Súbelo en Ajustes › PARLEY o busca las cuotas en la web.",
        quota.map_or("?".into(), |q| q.to_string()), limit.map_or("?".into(), |l| l.to_string()));

    let sports_json = match load_catalogue("sports-v4.json") {
        Some(v) => v,
        None => { calls += 1; let v = get("/sports?language=en".into()).await?; save_catalogue("sports-v4.json", &v)?; v }
    };
    let sports = allowed_sports(&sports_json);
    if sports.len() != 3 { return Err("OddsPapi no identificó los tres deportes: fútbol, tenis y básquet.".into()); }

    // The schedule (which matches and tournaments play) costs a request and has no prices: it is asked for once for
    // the next day and a half and reused for hours by reviews and chat questions alike.
    let horizon = if query.is_some() { window_end } else { window_end.max(now + WIDEN) };
    let (filtered, schedule_at) = match load_schedule(now, from, horizon) {
        Some(cached) => cached,
        None => {
            if calls >= budget { spent(&mut usage, calls); return Err(capped()); }
            calls += 1;
            // Without sportId the time filter permits under 48 hours.
            let (sched_from, sched_to) = schedule_range(now, from, horizon);
            let fixtures_json = match get(format!("/fixtures?from={}&to={}&hasOdds=true&bookmakers={BOOKMAKER}&language=es", iso_utc(sched_from), iso_utc(sched_to))).await {
                Ok(v) => v,
                Err(e) => { spent(&mut usage, calls); return Err(e); }
            };
            let filtered = Value::Array(fixtures_json.as_array().into_iter().flatten()
                .filter(|f| f["sportId"].as_i64().is_some_and(|id| sports.contains_key(&id))).map(|f| {
                    let mut f = f.clone();
                    if let Some(name) = f["sportId"].as_i64().and_then(|id| sports.get(&id)) { f["sportName"] = Value::String(name.clone()); }
                    f
                }).collect());
            save_schedule(now, sched_from, sched_to, &filtered);
            tokio::time::sleep(Duration::from_millis(2_100)).await;
            (filtered, now)
        }
    };

    // Every match of the pool; a live flag from an old schedule is dropped once the match must be over.
    let mut pool = pick_fixtures(&filtered, if query.is_some() { now.max(from + 300) } else { now }, horizon);
    pool.retain(|f| !f.live || f.start > now - 4 * 3600);
    let pool = if let Some(query) = query { requested_fixtures(pool, query) } else { pool };
    // What this request is about: the asked range, or for a review its window, widened when it holds too few matches.
    let wanted = if query.is_some() { usize::MAX } else { WANTED_FIXTURES };
    let scope_end = if query.is_some() { window_end } else { widened_end(&pool, window_end, WANTED_FIXTURES) };
    let candidates: Vec<Fixture> = pool.iter().filter(|f| f.live || f.start <= scope_end).cloned().collect();

    // One call carries five tournaments with every market of their matches: the tournaments with most matches first.
    let ranked = rank_tournaments(&candidates);
    let mut cache = load_odds_cache();
    let age = |cache: &serde_json::Map<String, Value>, id: i64| cache.get(&id.to_string()).and_then(|c| c["at"].as_i64()).map(|at| now - at);
    let (mut selected, mut to_fetch, mut skipped): (Vec<i64>, Vec<i64>, Vec<i64>) = (Vec::new(), Vec::new(), Vec::new());
    for id in ranked {
        let next = candidates.iter().filter(|f| f.tournament_id == id).map(|f| f.start).min().unwrap_or(i64::MAX);
        let reuse = if f_live_or_soon(next, now) { ODDS_REUSE_SOON } else { ODDS_REUSE };
        if age(&cache, id).is_some_and(|a| a < reuse.as_secs() as i64) { selected.push(id); }
        else if to_fetch.len() < batches * TOURNAMENT_BATCH { to_fetch.push(id); }
        else { skipped.push(id); }
    }
    let mut failure: Option<String> = None;
    for batch in to_fetch.chunks(TOURNAMENT_BATCH) {
        if calls >= budget || failure.is_some() { skipped.extend(batch); continue; }
        calls += 1;
        let ids: Vec<String> = batch.iter().map(i64::to_string).collect();
        match get(format!("/odds-by-tournaments?tournamentIds={}&bookmakers={BOOKMAKER}&language=es&verbosity=3", ids.join(","))).await {
            Ok(odds) => {
                let entries = odds_entries(&odds);
                for id in batch {
                    let own: Vec<&Value> = entries.iter().filter(|e| e["tournamentId"].as_i64() == Some(*id)).collect();
                    cache.insert(id.to_string(), serde_json::json!({ "at": now, "entries": own }));
                    selected.push(*id);
                }
            }
            Err(e) => { failure = Some(e); skipped.extend(batch); }
        }
        tokio::time::sleep(Duration::from_millis(1_100)).await;
    }
    // A tournament that could not be asked for now still has its last prices, if they are from today's session.
    let mut stale: Vec<i64> = Vec::new();
    skipped.retain(|id| {
        let usable = age(&cache, *id).is_some_and(|a| a < STALE_ODDS.as_secs() as i64);
        if usable { stale.push(*id); }
        !usable
    });
    if !to_fetch.is_empty() { save_odds_cache(&cache, now); }

    // The matches shown: those of the consulted tournaments; a review short of matches takes later ones of the same
    // tournaments (they came in the same answer, at no extra cost).
    let with_odds: Vec<i64> = selected.iter().chain(&stale).copied().collect();
    let mut fixtures: Vec<Fixture> = candidates.iter().filter(|f| !f.live && with_odds.contains(&f.tournament_id)).cloned().collect();
    if fixtures.len() < wanted {
        for f in pool.iter().filter(|f| !f.live && f.start > scope_end && with_odds.contains(&f.tournament_id)) {
            if fixtures.len() >= wanted { break; }
            fixtures.push(f.clone());
        }
    }
    let mut entries: Vec<Value> = Vec::new();
    for id in &with_odds {
        for e in cache.get(&id.to_string()).and_then(|c| c["entries"].as_array()).into_iter().flatten() {
            if fixtures.iter().any(|f| e["fixtureId"].as_str() == Some(&f.id)) { entries.push(e.clone()); }
        }
    }
    // Live prices only where the plan carries them for Betano (the free one does not): one call per match.
    let live: Vec<&Fixture> = if live_odds { candidates.iter().filter(|f| f.live).take(MAX_LIVE.min(budget.saturating_sub(calls))).collect() } else { Vec::new() };
    for f in &live {
        calls += 1;
        tokio::time::sleep(Duration::from_millis(1_100)).await;
        if let Ok(odds) = get(format!("/odds?fixtureId={}&bookmakers={BOOKMAKER}", f.id)).await {
            entries.extend(odds_entries(&odds));
            fixtures.insert(0, (*f).clone());
        }
    }
    let odds = Value::Array(entries);
    let names = match load_markets() {
        Some(names) => names,
        None => {
            tokio::time::sleep(Duration::from_millis(2_100)).await;
            let fresh = if quota.is_some_and(|q| calls >= q) { None } else { calls += 1; get("/markets?language=es".into()).await.ok() };
            let names = fresh.as_ref().map(market_names).unwrap_or_default();
            if !names.is_empty() { if let Some(fresh) = fresh { save_markets(&fresh); } }
            names
        }
    };
    spent(&mut usage, calls);
    if fixtures.is_empty() {
        if let Some(e) = failure { return Err(e); }
        if !skipped.is_empty() { return Err(capped()); }
    }

    let csv = odds_csv(&fixtures, &odds, &names, now);
    let requested_metadata: Vec<&Value> = filtered.as_array().into_iter().flatten().filter(|v| fixtures.iter().any(|f| v["fixtureId"].as_str() == Some(&f.id))).collect();
    let raw_snapshot = serde_json::json!({"queriedAt": iso_utc(now), "fixtures": requested_metadata, "odds": odds});
    save_catalogue("latest-odds.json", &raw_snapshot)?;
    let path = settings::local_dir().join("odds").join("latest-odds.csv");
    std::fs::write(&path, format!("\u{feff}{csv}")).map_err(|_| "No se pudo guardar la tabla de cuotas.")?;
    // Keep the full input inside the read-only agent's working folder so both providers can read it.
    let agent_dir = super::named_agents::workspace("parley").join("odds");
    std::fs::create_dir_all(&agent_dir).map_err(|_| "No se pudo preparar la tabla para PARLEY.")?;
    prune_consultas(&agent_dir);
    // Immutable per-request input: an automatic refresh must not replace data while the agent is reading it.
    let stem = format!("consulta-{}", super::chat_store::now_ms());
    let agent_csv = agent_dir.join(format!("{stem}.csv"));
    let agent_json = agent_dir.join(format!("{stem}.json"));
    std::fs::write(&agent_csv, format!("\u{feff}{csv}")).map_err(|_| "No se pudo guardar la tabla para PARLEY.")?;
    std::fs::write(&agent_json, serde_json::to_vec(&raw_snapshot).map_err(|_| "No se pudo serializar la consulta.")?)
        .map_err(|_| "No se pudo guardar los datos completos para PARLEY.")?;

    if batches == DAILY_BATCHES {
        let digest = odds_preview(&fixtures, &odds, &names, utc_offset, 4);
        let day = super::chat_store::date_label((now.max(0) as u64) * 1000, utc_offset);
        let md = format!("# Partidos de hoy ({day})\n\nConsultado a las {}. {} partidos con cuotas de Betano, de {} torneos. Lee esto primero; hoy.csv trae todos los mercados. Usa la web solo para validar bajas o forma de un partido concreto.\n\n{digest}\n", crate::platform::clock::hour(now, utc_offset), fixtures.len(), with_odds.len());
        for dir in [settings::local_dir().join("odds"), agent_dir.clone()] { let _ = std::fs::write(dir.join("resumen-del-dia.md"), &md); }
        let _ = std::fs::write(agent_dir.join("hoy.csv"), format!("\u{feff}{csv}"));
        // The day's match list, for the free form stats (services/stats.rs).
        let _ = std::fs::write(settings::local_dir().join("odds").join("daily-fixtures.json"), super::stats::fixtures_json(&day, &fixtures).to_string());
    }
    let tournament_name = |id: i64| pool.iter().find(|f| f.tournament_id == id).map(|f| f.tournament.clone()).unwrap_or_else(|| id.to_string());
    let list = |ids: &[i64]| ids.iter().map(|id| tournament_name(*id)).collect::<Vec<_>>().join("; ");
    let preview = odds_preview(&fixtures, &odds, &names, utc_offset, PREVIEW_MARKETS);
    let clipped: String = preview.chars().take(PREVIEW_CHARS).collect();
    let live_seen = candidates.iter().filter(|f| f.live).count();
    let mut out = format!(
        "Consulta UTC: {}. Alcance UTC: {} a {} ({}). Deportes: fútbol, tenis y básquet. {} partidos por empezar con cuotas de Betano, de {} torneos.\n\
         Llamadas a OddsPapi en esta consulta: {calls}; hoy {} de {allowance} permitidas; quedan {} en el plan. El calendario es de las {} y las cuotas de cada torneo se reutilizan hasta 2 h (30 min si un partido empieza en menos de 4 h).\n",
        iso_utc(now), iso_utc(from), iso_utc(scope_end.max(window_end)), if query.is_some() { "petición manual" } else { "revisión automática" },
        fixtures.len(), with_odds.len(), usage.calls, usage.remaining.map_or("?".into(), |r| r.to_string()), crate::platform::clock::hour(schedule_at, utc_offset));
    if query.is_none() && scope_end > window_end { out.push_str(&format!("La ventana tenía pocos partidos: se amplió hasta las {} para reunir {WANTED_FIXTURES}.\n", crate::platform::clock::hour(scope_end, utc_offset))); }
    if !stale.is_empty() { out.push_str(&format!("Cuotas de hace más de 2 h (tope de llamadas): {}.\n", list(&stale))); }
    if !skipped.is_empty() { out.push_str(&format!("Torneos del alcance SIN cuotas consultadas (una llamada trae cinco torneos; para estos busca la cuota en la web): {}.\n", list(&skipped))); }
    if let Some(e) = &failure { out.push_str(&format!("Una llamada falló: {e}\n")); }
    if live_seen > 0 && !live_odds { out.push_str(&format!("{live_seen} partidos en vivo en el alcance: el plan de OddsPapi no trae cuotas en vivo de Betano; para ellos usa la web.\n")); }
    out.push_str(&format!("Tabla completa (todas las líneas y jugadores): {} · JSON: {} (botón Cuotas CSV en el chat).\n", agent_csv.display(), agent_json.display()));
    out.push_str(&format!("Para Codex: read_parley_odds con file=\"{stem}.csv\" (offset y limit en líneas; contains busca por fixtureId). Para Claude: Read en esas rutas.\n"));
    out.push_str(&format!("Mercados principales por partido (hasta {PREVIEW_MARKETS}; de cada escalera de líneas, la más pareja; solo precios activos):\n{clipped}"));
    if clipped.len() < preview.len() { out.push_str("\n(Vista recortada: el resto está en el CSV.)"); }
    Ok(out)
}

fn odds_entries(v: &Value) -> Vec<Value> {
    if let Some(a) = v.as_array() { a.clone() }
    else if v["fixtureId"].is_string() { vec![v.clone()] }
    else { Vec::new() }
}

fn active_subscription(v: &Value) -> Option<&Value> {
    let id = v["current_subscription_id"].as_str();
    v["subscriptions"].as_array()?.iter().find(|s| s["is_active"] == true && (id.is_none() || s["subscription_id"].as_str() == id))
}

fn quota_remaining(v: &Value) -> Option<usize> {
    let sub = active_subscription(v)?;
    Some(sub["request_limit"].as_u64()?.saturating_sub(sub["request_count"].as_u64()?) as usize)
}

fn quota_limit(v: &Value) -> Option<usize> { Some(active_subscription(v)?["request_limit"].as_u64()? as usize) }

/// Whether the plan carries Betano's in-play prices (the free one says `has_live_odds: false`: asking is a wasted call).
fn has_live_odds(v: &Value) -> bool {
    active_subscription(v).is_some_and(|s| s["bookmakers"][BOOKMAKER]["has_live_odds"] == true)
}

/// Billable OddsPapi calls today, and what the plan had left at the last look. `MIKA/odds/usage.json`; the settings
/// show it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Usage {
    /// Local date, `2026-10-01`.
    pub day: String,
    pub calls: usize,
    /// Calls allowed today when this was written.
    pub allowance: usize,
    pub remaining: Option<usize>,
    pub limit: Option<usize>,
}

fn usage_path() -> PathBuf { settings::local_dir().join("odds").join("usage.json") }

pub fn load_usage() -> Usage {
    std::fs::read(usage_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_usage(usage: &Usage) {
    let path = usage_path();
    if let (Some(dir), Ok(bytes)) = (path.parent(), serde_json::to_vec(usage)) {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(path, bytes);
    }
}

/// Billable calls allowed today: the user's cap, or what the plan had left this morning spread over the days left in
/// the month (never under [`MIN_DAILY_CALLS`]), so a 250-call plan lasts the month instead of an afternoon.
pub fn daily_allowance(left_this_morning: Option<usize>, days_left: u32, setting: u32) -> usize {
    if setting > 0 { return setting as usize; }
    match left_this_morning {
        Some(left) => (left / days_left.max(1) as usize).max(MIN_DAILY_CALLS),
        None => 12,
    }
}

/// Days left in the local month, today included.
pub fn days_left_in_month(now: i64, utc_offset: i64) -> u32 {
    let label = super::chat_store::date_label(((now + utc_offset * 60).max(0) as u64) * 1000, 0);
    let n = |r: std::ops::Range<usize>| label.get(r).and_then(|t| t.parse::<u32>().ok());
    let (Some(year), Some(month), Some(day)) = (n(0..4), n(5..7), n(8..10)) else { return 30 };
    let days: u32 = match month { 4 | 6 | 9 | 11 => 30, 2 => if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) { 29 } else { 28 }, _ => 31 };
    days.saturating_sub(day) + 1
}

/// True when a tournament's next match is live or about to start (then its prices are reused for less time).
fn f_live_or_soon(next_start: i64, now: i64) -> bool { next_start - now < SOON }

/// Tournaments of the matches still to start, the ones with most matches first (football before the rest on a tie,
/// then the earliest kick-off): five of them travel in one call.
pub fn rank_tournaments(fixtures: &[Fixture]) -> Vec<i64> {
    let mut by: BTreeMap<i64, (usize, bool, i64)> = BTreeMap::new();
    for f in fixtures.iter().filter(|f| !f.live && f.tournament_id > 0) {
        let entry = by.entry(f.tournament_id).or_insert((0, f.sport != "Fútbol", f.start));
        entry.0 += 1;
        entry.2 = entry.2.min(f.start);
    }
    let mut ranked: Vec<(i64, (usize, bool, i64))> = by.into_iter().collect();
    ranked.sort_by_key(|(id, (count, not_football, start))| (std::cmp::Reverse(*count), *not_football, *start, *id));
    ranked.into_iter().map(|(id, _)| id).collect()
}

/// Where a review stops looking: the end of its window, or later when the window holds fewer than `wanted` matches
/// still to start (then the kick-off of the `wanted`-th one).
pub fn widened_end(pool: &[Fixture], window_end: i64, wanted: usize) -> i64 {
    let mut starts: Vec<i64> = pool.iter().filter(|f| !f.live).map(|f| f.start).collect();
    starts.sort_unstable();
    if starts.iter().filter(|s| **s <= window_end).count() >= wanted { return window_end; }
    starts.get(wanted.saturating_sub(1)).or(starts.last()).copied().unwrap_or(window_end).max(window_end)
}

/// The range a schedule call asks for: from three hours ago to a day and a half ahead when the request fits in it
/// (the API takes under 48 hours without a sport), else the request's own range.
pub fn schedule_range(now: i64, from: i64, to: i64) -> (i64, i64) {
    let early = from.min(now - 3 * 3600);
    if to - early <= 47 * 3600 { (early, to.max(now + 36 * 3600).min(early + 47 * 3600)) } else { (from, to.min(from + 47 * 3600)) }
}

fn schedule_path() -> PathBuf { settings::local_dir().join("odds").join("schedule.json") }

/// The cached schedule and when it was fetched, if it is recent and covers the range.
fn load_schedule(now: i64, from: i64, to: i64) -> Option<(Value, i64)> {
    let v: Value = serde_json::from_slice(&std::fs::read(schedule_path()).ok()?).ok()?;
    let at = v["at"].as_i64()?;
    let covers = v["from"].as_i64()? <= from && to <= v["to"].as_i64()?;
    (now - at < SCHEDULE_REUSE.as_secs() as i64 && now >= at && covers && v["fixtures"].is_array()).then(|| (v["fixtures"].clone(), at))
}

fn save_schedule(now: i64, from: i64, to: i64, fixtures: &Value) {
    let _ = save_catalogue("schedule.json", &serde_json::json!({ "at": now, "from": from, "to": to, "fixtures": fixtures }));
}

fn odds_cache_path() -> PathBuf { settings::local_dir().join("odds").join("tournament-odds.json") }

/// tournament id → `{ at, entries }`: the last answer for each tournament.
fn load_odds_cache() -> serde_json::Map<String, Value> {
    std::fs::read(odds_cache_path()).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v.as_object().cloned()).unwrap_or_default()
}

fn save_odds_cache(cache: &serde_json::Map<String, Value>, now: i64) {
    let kept: serde_json::Map<String, Value> = cache.iter()
        .filter(|(_, c)| c["at"].as_i64().is_some_and(|at| now - at < STALE_ODDS.as_secs() as i64))
        .map(|(k, v)| (k.clone(), v.clone())).collect();
    if let Ok(bytes) = serde_json::to_vec(&Value::Object(kept)) {
        let _ = std::fs::create_dir_all(settings::local_dir().join("odds"));
        let _ = std::fs::write(odds_cache_path(), bytes);
    }
}

/// PARLEY's folder keeps the tables of the last few requests only (each is several megabytes).
fn prune_consultas(dir: &std::path::Path) {
    let mut names: Vec<String> = std::fs::read_dir(dir).into_iter().flatten().flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with("consulta-") && (n.ends_with(".csv") || n.ends_with(".json")))
        .collect();
    names.sort_unstable_by(|a, b| b.cmp(a));
    for name in names.into_iter().skip(10) { let _ = std::fs::remove_file(dir.join(name)); }
}

fn allowed_sports(json: &Value) -> HashMap<i64, String> {
    json.as_array().into_iter().flatten().filter_map(|s| {
        let label = match s["slug"].as_str()? { "soccer" | "football" => "Fútbol", "tennis" => "Tenis", "basketball" => "Básquet", _ => return None };
        Some((s["sportId"].as_i64()?, label.into()))
    }).collect()
}

fn load_catalogue(name: &str) -> Option<Value> {
    let path = settings::local_dir().join("odds").join(name);
    if path.metadata().ok()?.modified().ok()?.elapsed().ok()? >= MARKETS_MAX_AGE { return None; }
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn save_catalogue(name: &str, json: &Value) -> Result<(), String> {
    let dir = settings::local_dir().join("odds");
    std::fs::create_dir_all(&dir).map_err(|_| "No se pudo crear la carpeta de cuotas.")?;
    let bytes = serde_json::to_vec(json).map_err(|_| "No se pudieron guardar los datos de cuotas.")?;
    std::fs::write(dir.join(name), bytes).map_err(|_| "No se pudieron guardar los datos de cuotas.".into())
}

/// `2026-10-01T13:30:00Z`.
pub fn iso_utc(secs: i64) -> String {
    let day = super::chat_store::date_label((secs.max(0) as u64) * 1000, 0);
    let t = secs.rem_euclid(86_400);
    format!("{day}T{:02}:{:02}:{:02}Z", t / 3600, (t % 3600) / 60, t % 60)
}

/// `2026-10-01T13:30:00.000Z` (or with `+hh:mm`) → Unix seconds.
pub fn parse_iso(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || (b[10] != b'T' && b[10] != b' ') || b[13] != b':' || b[16] != b':' { return None; }
    let n = |r: std::ops::Range<usize>| text.get(r)?.parse::<i64>().ok();
    let (y, m, d, hh, mm, ss) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 60 { return None; }
    // Days from civil (Howard Hinnant).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let mut secs = days * 86_400 + hh * 3600 + mm * 60 + ss;
    let rest = text[19..].trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    if let Some(offset) = rest.strip_prefix('+').map(|o| (1, o)).or_else(|| rest.strip_prefix('-').map(|o| (-1, o))) {
        let (sign, o) = offset;
        let oh = o.get(0..2)?.parse::<i64>().ok()?;
        let om = o.get(3..5).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
        secs -= sign * (oh * 3600 + om * 60);
    }
    Some(secs)
}

/// Live matches first, then by kick-off; finished, cancelled and out-of-window ones are left out.
pub fn pick_fixtures(json: &Value, now: i64, window_end: i64) -> Vec<Fixture> {
    let mut out: Vec<Fixture> = json.as_array().into_iter().flatten().filter_map(|f| {
        let status = f["statusId"].as_i64().or_else(|| f["status"]["statusId"].as_i64()).unwrap_or(0);
        let live = status == 1 || f["status"]["live"] == true;
        let start = f["startTime"].as_i64().or_else(|| f["startTime"].as_str().and_then(parse_iso))?;
        if status >= 2 || !(live || (start >= now - 300 && start <= window_end)) { return None; }
        let id = f["fixtureId"].as_str()?;
        if id.is_empty() || id.len() > 40 || !id.chars().all(|c| c.is_ascii_alphanumeric()) { return None; }
        let text = |v: &Value| v.as_str().unwrap_or("").chars().filter(|c| !c.is_control()).take(60).collect::<String>();
        let pick = |a: &Value, b: &Value| { let t = text(a); if t.is_empty() { text(b) } else { t } };
        let tournament = [pick(&f["tournamentName"], &f["tournament"]["tournamentName"]), pick(&f["categoryName"], &f["tournament"]["categoryName"])]
            .into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
        Some(Fixture {
            id: id.to_string(),
            home: pick(&f["participant1Name"], &f["participants"]["participant1Name"]),
            away: pick(&f["participant2Name"], &f["participants"]["participant2Name"]),
            tournament, tournament_id: f["tournamentId"].as_i64().unwrap_or(0), start, live,
            sport: text(&f["sportName"]),
        })
    }).collect();
    out.sort_by_key(|f| (!f.live, f.start));
    out
}

/// outcomeId → (market name, outcome name), from `/markets`.
pub fn market_names(json: &Value) -> HashMap<i64, (String, String)> {
    let mut out = HashMap::new();
    for market in json.as_array().into_iter().flatten() {
        let base = market["marketName"].as_str().unwrap_or("");
        // "Over Under Full Time" exists for every line: the line is part of the name.
        let name = match market["handicap"].as_f64() {
            Some(h) if h != 0.0 => format!("{base} {}", if h.fract() == 0.0 { format!("{h:.0}") } else { format!("{h}") }),
            _ => base.to_string(),
        };
        for outcome in market["outcomes"].as_array().into_iter().flatten() {
            if let Some(id) = outcome["outcomeId"].as_i64() {
                out.insert(id, (name.clone(), outcome["outcomeName"].as_str().unwrap_or("").to_string()));
            }
        }
    }
    out
}

/// The football markets everyone knows, for when `/markets` could not be read.
fn known_outcome(id: i64) -> Option<(&'static str, &'static str)> {
    Some(match id {
        101 => ("Resultado final", "1"), 102 => ("Resultado final", "X"), 103 => ("Resultado final", "2"),
        104 => ("Ambos marcan", "Sí"), 105 => ("Ambos marcan", "No"),
        1010 => ("Más/Menos 2.5 goles", "Más"), 1011 => ("Más/Menos 2.5 goles", "Menos"),
        _ => return None,
    })
}

/// One block per match: `- Local vs Visita (torneo) · 20:30`, with `Mercado: lado @cuota · …`.
/// Every returned market and player price, including inactive and suspended prices labelled as such.
#[cfg(test)]
pub fn odds_text(fixtures: &[Fixture], json: &Value, names: &HashMap<i64, (String, String)>, utc_offset: i64) -> String {
    let by_id: HashMap<&str, &Value> = json.as_array().into_iter().flatten()
        .filter_map(|f| Some((f["fixtureId"].as_str()?, f))).collect();
    let mut out = String::new();
    for fixture in fixtures {
        let Some(entry) = by_id.get(fixture.id.as_str()) else { continue };
        let book = &entry["bookmakerOdds"][BOOKMAKER];
        let mut markets: BTreeMap<(i64, String), Vec<String>> = BTreeMap::new();
        for (market_id, market) in book["markets"].as_object().into_iter().flatten() {
            let market_id: i64 = market_id.parse().unwrap_or(0);
            for (outcome_id, outcome) in market["outcomes"].as_object().into_iter().flatten() {
              for (player_id, quote) in outcome["players"].as_object().into_iter().flatten() {
                let (Ok(outcome_id), Some(price)) = (outcome_id.parse::<i64>(), quote["price"].as_f64()) else { continue };
                if !(1.01..1000.0).contains(&price) { continue; }
                let (market_name, side) = match names.get(&outcome_id) {
                    Some((m, s)) => (m.clone(), s.clone()),
                    None => match known_outcome(outcome_id) {
                        Some((m, s)) => (m.to_string(), s.to_string()),
                        None => (format!("Mercado {market_id}"), format!("Resultado {outcome_id}")),
                    },
                };
                let player = if player_id == "0" { String::new() } else {
                    format!(" [{}]", quote["playerName"].as_str().unwrap_or(player_id))
                };
                let state = if book["suspended"] == true { " [suspendida]" }
                    else if book["bookmakerIsActive"] == false || market["marketActive"] == false || quote["active"] == false { " [inactiva]" }
                    else { "" };
                markets.entry((market_id, market_name)).or_default().push(format!("{side}{player} @{price}{state}"));
              }
            }
        }
        if markets.is_empty() { continue; }
        let when = if fixture.live { "EN VIVO".to_string() } else { crate::platform::clock::hour(fixture.start, utc_offset) };
        let suspended = if book["suspended"] == true { " (cuotas suspendidas en la consulta)" } else { "" };
        let mut block = format!("- {} vs {}{} · {when}{suspended}\n", fixture.home, fixture.away,
                                if fixture.tournament.is_empty() { String::new() } else { format!(" ({})", fixture.tournament) });
        for ((_, market), sides) in markets { block.push_str(&format!("  {market}: {}\n", sides.join(" · "))); }
        out.push_str(&block);
    }
    if out.is_empty() { "OddsPapi no devolvió precios decimales de Betano para los partidos del alcance; revisa la cobertura y el CSV antes de concluir que no existen mercados.".into() } else { out }
}

/// One line of a market: its full name and its sides with their prices.
type Line = (String, Vec<(String, f64)>);

/// `Más/Menos Tiempo Completo 2.5` → `Más/Menos Tiempo Completo`: the lines of one market share a name.
fn base_name(name: &str) -> &str {
    match name.rsplit_once(' ') { Some((base, tail)) if tail.parse::<f64>().is_ok() => base, _ => name }
}

/// The table PARLEY reads in its prompt: per match its first `max_markets` markets (the catalogue numbers the main
/// ones first), active prices only, no player props, and of each ladder of lines (totals, handicaps) only the most
/// even one. Everything else stays in the CSV.
pub fn odds_preview(fixtures: &[Fixture], json: &Value, names: &HashMap<i64, (String, String)>, utc_offset: i64, max_markets: usize) -> String {
    let by_id: HashMap<&str, &Value> = json.as_array().into_iter().flatten()
        .filter_map(|f| Some((f["fixtureId"].as_str()?, f))).collect();
    let mut out = String::new();
    for fixture in fixtures {
        let Some(entry) = by_id.get(fixture.id.as_str()) else { continue };
        let book = &entry["bookmakerOdds"][BOOKMAKER];
        let when = if fixture.live { "EN VIVO".to_string() } else { crate::platform::clock::hour(fixture.start, utc_offset) };
        let head = format!("- {} vs {} · {}{} · {when}", fixture.home, fixture.away, fixture.sport,
                           if fixture.tournament.is_empty() { String::new() } else { format!(" · {}", fixture.tournament) });
        if book["suspended"] == true || book["bookmakerIsActive"] == false {
            out.push_str(&format!("{head} (cuotas suspendidas en la consulta)\n"));
            continue;
        }
        let mut lines: BTreeMap<(i64, String), Vec<(String, f64)>> = BTreeMap::new();
        for (market_id, market) in book["markets"].as_object().into_iter().flatten() {
            if market["marketActive"] == false { continue; }
            let market_id: i64 = market_id.parse().unwrap_or(0);
            for (outcome_id, outcome) in market["outcomes"].as_object().into_iter().flatten() {
                let quote = &outcome["players"]["0"];
                let (Ok(outcome_id), Some(price)) = (outcome_id.parse::<i64>(), quote["price"].as_f64()) else { continue };
                if quote["active"] == false || !(1.01..1000.0).contains(&price) { continue; }
                let (name, side) = match names.get(&outcome_id) {
                    Some((m, s)) => (m.clone(), s.clone()),
                    None => match known_outcome(outcome_id) {
                        Some((m, s)) => (m.to_string(), s.to_string()),
                        None => (format!("Mercado {market_id}"), format!("Resultado {outcome_id}")),
                    },
                };
                lines.entry((market_id, name)).or_default().push((side, price));
            }
        }
        if lines.is_empty() { continue; }
        // The lines of a market, in the catalogue's order.
        let mut ladders: Vec<(String, Vec<Line>)> = Vec::new();
        for ((_, name), sides) in lines {
            let base = base_name(&name).to_string();
            match ladders.iter_mut().find(|l| l.0 == base) {
                Some(ladder) => ladder.1.push((name, sides)),
                None => ladders.push((base, vec![(name, sides)])),
            }
        }
        let total = ladders.len();
        out.push_str(&format!("{head}\n"));
        for (_, ladder) in ladders.into_iter().take(max_markets) {
            let more = ladder.len() - 1;
            let spread = |sides: &[(String, f64)]| if sides.len() < 2 { f64::INFINITY } else {
                sides.iter().map(|s| s.1).fold(f64::MIN, f64::max) - sides.iter().map(|s| s.1).fold(f64::MAX, f64::min)
            };
            let Some((name, sides)) = ladder.into_iter().min_by(|a, b| spread(&a.1).total_cmp(&spread(&b.1))) else { continue };
            let shown: Vec<String> = sides.iter().take(8).map(|(side, price)| format!("{side} @{price}")).collect();
            out.push_str(&format!("  {name}: {}{}{}\n", shown.join(" · "), if sides.len() > 8 { " · …" } else { "" },
                                  if more > 0 { format!(" (+{more} líneas en el CSV)") } else { String::new() }));
        }
        if total > max_markets { out.push_str(&format!("  (+{} mercados más en el CSV)\n", total - max_markets)); }
    }
    if out.is_empty() { "OddsPapi no devolvió cuotas activas de Betano para los partidos del alcance.".into() } else { out }
}

/// One row per returned market/outcome/player, retaining unavailable prices and their states as well.
pub fn odds_csv(fixtures: &[Fixture], json: &Value, names: &HashMap<i64, (String, String)>, queried_at: i64) -> String {
    let mut out = String::from("consultaUTC,deporte,torneo,partido,inicioUTC,enVivo,fixtureId,marketId,mercado,outcomeId,seleccion,playerId,jugador,cuota,probabilidadImplicitaPct,marketActive,active,suspended,bookmakerIsActive,mainLine,changedAt,bookmakerChangedAt,limite,fixturePath\r\n");
    let by_id: HashMap<&str, &Value> = json.as_array().into_iter().flatten()
        .filter_map(|f| Some((f["fixtureId"].as_str()?, f))).collect();
    let empty = Value::Null;
    for f in fixtures {
        let Some(entry) = by_id.get(f.id.as_str()) else { continue };
        let book = &entry["bookmakerOdds"][BOOKMAKER];
        for (mid, market) in book["markets"].as_object().into_iter().flatten() {
            let mut outcomes: Vec<(&str, &Value)> = market["outcomes"].as_object().into_iter().flatten().map(|(k,v)| (k.as_str(),v)).collect();
            if outcomes.is_empty() { outcomes.push(("", &empty)); }
            for (oid, outcome) in outcomes {
                let mut players: Vec<(&str, &Value)> = outcome["players"].as_object().into_iter().flatten().map(|(k,v)| (k.as_str(),v)).collect();
                if players.is_empty() { players.push(("", &empty)); }
                for (pid, q) in players {
                    let labels = oid.parse::<i64>().ok().and_then(|id| names.get(&id));
                    let label = |v: &Value| v.as_str().map(str::to_owned).unwrap_or_else(|| if v.is_null() { String::new() } else { v.to_string() });
                    let market_name = labels.map(|n| n.0.clone()).unwrap_or_else(|| market["marketName"].as_str().map(str::to_owned).unwrap_or_else(|| format!("Mercado {mid}")));
                    let side = labels.map(|n| n.1.clone()).unwrap_or_else(|| label(&q["bookmakerOutcomeId"]));
                    let implied = q["price"].as_f64().filter(|p| p.is_finite() && *p > 1.0).map(|p| format!("{:.6}", 100.0 / p)).unwrap_or_default();
                    let cells = vec![iso_utc(queried_at), f.sport.clone(), f.tournament.clone(), format!("{} vs {}", f.home, f.away),
                        iso_utc(f.start), f.live.to_string(), f.id.clone(), mid.clone(), market_name, oid.to_owned(), side, pid.to_owned(),
                        label(&q["playerName"]), label(&q["price"]), implied, label(&market["marketActive"]), label(&q["active"]),
                        label(&book["suspended"]), label(&book["bookmakerIsActive"]), label(&q["mainLine"]), label(&q["changedAt"]),
                        label(&q["bookmakerChangedAt"]), label(&q["limit"]), label(&book["fixturePath"])];
                    out.push_str(&cells.iter().map(|s| csv_cell(s)).collect::<Vec<_>>().join(","));
                    out.push_str("\r\n");
                }
            }
        }
    }
    out
}

pub(crate) fn csv_cell(s: &str) -> String {
    // Spreadsheet applications can execute formula-like text even in quoted CSV cells.
    let prefix = if s.trim_start().starts_with(['=', '+', '-', '@']) || s.starts_with(['\t', '\r', '\n']) { "'" } else { "" };
    format!("\"{prefix}{}\"", s.replace('"', "\"\""))
}

/// Explicit UI click: copy the existing snapshot to the real Windows Downloads folder, then reveal the CSV.
/// No API request is made. File names come from MIKA, never from a provider or a model.
pub fn export_csv() -> Result<String, String> {
    use std::io::Write;
    let _fetching = FETCHING.try_lock().map_err(|_| "La consulta de cuotas sigue en curso. Descarga la tabla al terminar.")?;
    let bytes = std::fs::read(settings::local_dir().join("odds").join("latest-odds.csv"))
        .map_err(|_| "Todavía no hay cuotas consultadas. Activa Cuotas Betano y consulta a PARLEY.")?;
    let downloads = crate::platform::shell::downloads_dir()?;
    let path = downloads.join(format!("MIKA-cuotas-{}.csv", super::chat_store::now_ms()));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)
        .map_err(|_| "No se pudo crear el CSV en Descargas.")?;
    file.write_all(&bytes).map_err(|_| "No se pudo guardar el CSV en Descargas.")?;
    crate::platform::shell::open_local(&path, true)?;
    Ok(path.display().to_string())
}

fn markets_path() -> PathBuf { settings::local_dir().join("odds").join("markets-v4-all-es.json") }

fn load_markets() -> Option<HashMap<i64, (String, String)>> {
    let path = markets_path();
    let fresh = path.metadata().ok()?.modified().ok()?.elapsed().ok()? < MARKETS_MAX_AGE;
    if !fresh { return None; }
    let names = market_names(&serde_json::from_slice(&std::fs::read(path).ok()?).ok()?);
    (!names.is_empty()).then_some(names)
}

fn save_markets(json: &Value) {
    let path = markets_path();
    if let (Some(dir), Ok(bytes)) = (path.parent(), serde_json::to_vec(json)) {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(path, bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture(id: &str, start: &str, status: i64, tournament: i64) -> Value {
        json!({ "fixtureId": id, "startTime": start, "statusId": status, "tournamentId": tournament,
                "tournamentName": "Liga 1", "categoryName": "Perú",
                "participant1Name": "Alianza Lima", "participant2Name": "Cienciano" })
    }

    #[test]
    fn iso_times_round_trip() {
        assert_eq!(parse_iso("2026-10-01T19:05:00.000Z"), Some(1_790_881_500));
        assert_eq!(parse_iso("2026-10-01T14:05:00-05:00"), Some(1_790_881_500));
        assert_eq!(iso_utc(1_790_881_500), "2026-10-01T19:05:00Z");
        assert_eq!(parse_iso("ayer"), None);
        assert_eq!(parse_iso("2026-13-01T00:00:00Z"), None);
    }

    #[test]
    fn only_live_and_soon_matches_are_kept_live_first() {
        let now = 1_790_881_500;
        let list = json!([
            fixture("idlate", "2026-10-01T23:00:00Z", 0, 1),
            fixture("idsoon", "2026-10-01T19:40:00Z", 0, 2),
            fixture("idlive", "2026-10-01T18:20:00Z", 1, 3),
            fixture("iddone", "2026-10-01T16:00:00Z", 2, 4),
            fixture("bad id!", "2026-10-01T19:10:00Z", 0, 5),
        ]);
        let picked = pick_fixtures(&list, now, now + 2 * 3600);
        assert_eq!(picked.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), vec!["idlive", "idsoon"]);
        assert_eq!((picked[1].tournament.as_str(), picked[1].tournament_id), ("Liga 1, Perú", 2));
    }

    #[test]
    fn the_odds_table_keeps_active_main_lines() {
        let fixtures = vec![Fixture { id: "id1".into(), home: "Alianza Lima".into(), away: "Cienciano".into(),
                                      tournament: "Liga 1".into(), tournament_id: 9, sport: "Fútbol".into(), start: 1_790_881_500, live: false }];
        let quote = |price: f64, active: bool, main: bool| json!({ "players": { "0": { "price": price, "active": active, "mainLine": main } } });
        let odds = json!([{ "fixtureId": "id1", "bookmakerOdds": { "betano.pe": { "suspended": false, "markets": {
            "101": { "marketActive": true, "outcomes": { "101": quote(1.85, true, false), "102": quote(3.4, true, false), "103": quote(4.2, false, false) } },
            "1010": { "marketActive": true, "outcomes": { "1010": quote(1.72, true, true), "1011": quote(2.05, true, true) } },
            "1012": { "marketActive": true, "outcomes": { "1012": quote(3.1, true, false) } },
            "104": { "marketActive": false, "outcomes": { "104": quote(1.9, true, false) } }
        } } } }]);
        let text = odds_text(&fixtures, &odds, &HashMap::new(), -300);
        assert!(text.starts_with("- Alianza Lima vs Cienciano (Liga 1) · 14:05\n"), "{text}");
        assert!(text.contains("Resultado final: 1 @1.85 · X @3.4"), "{text}");
        assert!(text.contains("Más/Menos 2.5 goles: Más @1.72 · Menos @2.05"), "{text}");
        assert!(text.contains("@4.2 [inactiva]") && text.contains("@3.1") && text.contains("@1.9 [inactiva]"), "{text}");
    }

    #[test]
    fn market_names_come_from_the_catalogue() {
        let catalogue = json!([{ "marketId": 1068, "marketName": "Hándicap asiático -0.5", "outcomes": [
            { "outcomeId": 1068, "outcomeName": "1" }, { "outcomeId": 1069, "outcomeName": "2" } ] },
            { "marketId": 5, "playerProp": true, "marketName": "Goleador", "outcomes": [{ "outcomeId": 5, "outcomeName": "x" }] }]);
        let names = market_names(&catalogue);
        assert_eq!(names.get(&1069), Some(&("Hándicap asiático -0.5".to_string(), "2".to_string())));
        assert!(names.contains_key(&5));
    }

    #[test]
    fn parley_contract_all_market_lines_and_players_are_visible() {
        let fixtures = vec![Fixture { id: "id1".into(), home: "A".into(), away: "B".into(),
            tournament: "ATP".into(), tournament_id: 9, sport: "Tenis".into(), start: 1_790_881_500, live: false }];
        let odds = json!([{ "fixtureId": "id1", "bookmakerOdds": { "betano.pe": { "markets": {
            "555": { "marketActive": true, "outcomes": { "777": { "players": {
                "0": { "price": 3.123, "active": true, "mainLine": false },
                "42": { "price": 1.81, "active": true, "playerName": "Jugador C" }
            } } } }
        } } } }]);
        let text = odds_text(&fixtures, &odds, &HashMap::new(), -300);
        assert!(text.contains("3.123") && text.contains("Jugador C") && text.contains("1.81"), "{text}");
    }

    #[test]
    fn parley_contract_keeps_all_consulted_fixtures() {
        let fixtures = Value::Array((0..20).map(|i| fixture(&format!("id{i}"), "2026-10-01T19:40:00Z", 0, i)).collect());
        assert_eq!(pick_fixtures(&fixtures, 1_790_881_500, 1_790_888_700).len(), 20);
    }

    #[test]
    fn parley_contract_sports_are_resolved_from_catalogue_without_guessing_ids() {
        let sports = allowed_sports(&json!([
            {"sportId": 10, "slug": "soccer"}, {"sportId": 72, "slug": "tennis"},
            {"sportId": 11, "slug": "basketball"}, {"sportId": 1, "slug": "baseball"}
        ]));
        assert_eq!(sports.len(), 3);
        assert_eq!(sports.get(&72).map(String::as_str), Some("Tenis"));
        assert!(!sports.contains_key(&1));
    }

    #[test]
    fn parley_contract_csv_retains_every_line_player_state_and_unpriced_market() {
        let fixtures = vec![Fixture { id: "id1".into(), home: "=malicioso,\"A\"".into(), away: "B".into(),
            tournament: "ATP".into(), tournament_id: 9, sport: "Tenis".into(), start: 1_790_881_500, live: false }];
        let mut markets = serde_json::Map::new();
        for i in 0..12 {
            markets.insert((500 + i).to_string(), json!({ "marketActive": true, "outcomes": {
                "777": { "players": { "0": {"price": 1.6789, "active": true, "mainLine": false},
                    "42": {"price": 2.2, "active": false, "playerName": "Jugador C"} } }
            } }));
        }
        markets.insert("999".into(), json!({"marketActive": false, "outcomes": {}}));
        let odds = json!([{ "fixtureId": "id1", "bookmakerOdds": { "betano.pe": {"suspended": true, "markets": markets} } }]);
        let csv = odds_csv(&fixtures, &odds, &HashMap::new(), 1_790_881_500);
        assert_eq!(csv.lines().count(), 26, "12 markets with two players, one empty market, plus header");
        assert!(csv.contains("1.6789") && csv.contains("Jugador C") && csv.contains("Mercado 999"));
        assert!(csv.contains("\"'=malicioso,\"\"A\"\" vs B\""));
        assert!(csv.contains("\"true\",\"false\",\"true\""), "inactive player, suspended book: {csv}");
    }

    #[test]
    fn parley_contract_account_quota_uses_current_subscription_and_response_shapes() {
        let account = json!({"api_key": "do-not-store", "current_subscription_id": "new", "subscriptions": [
            {"subscription_id": "old", "is_active": true, "request_limit": 1000, "request_count": 5},
            {"subscription_id": "new", "is_active": true, "request_limit": 250, "request_count": 247}
        ]});
        assert_eq!(quota_remaining(&account), Some(3));
        assert_eq!(quota_remaining(&json!({})), None);
        assert_eq!(odds_entries(&json!({"fixtureId": "id1"})).len(), 1);
        assert!(odds_entries(&json!({"message": "error"})).is_empty());
    }

    fn fx(id: &str, tournament: i64, sport: &str, start: i64, live: bool) -> Fixture {
        Fixture { id: id.into(), home: "A".into(), away: "B".into(), tournament: format!("T{tournament}"), tournament_id: tournament, sport: sport.into(), start, live }
    }

    #[test]
    fn a_review_is_one_call_for_the_five_busiest_tournaments() {
        let fixtures = vec![fx("a", 1, "Tenis", 100, false), fx("b", 2, "Fútbol", 200, false), fx("c", 2, "Fútbol", 300, false),
                            fx("d", 3, "Fútbol", 50, false), fx("e", 4, "Básquet", 10, true), fx("f", 5, "Tenis", 60, false), fx("g", 5, "Tenis", 70, false)];
        assert_eq!(rank_tournaments(&fixtures), vec![2, 5, 3, 1], "most matches first, football before tennis on a tie, a live-only tournament is not asked for");
        assert_eq!((AUTO_BATCHES * TOURNAMENT_BATCH, MANUAL_BATCHES), (5, 2));
    }

    #[test]
    fn a_window_with_few_matches_is_widened_to_the_twelfth() {
        let pool: Vec<Fixture> = (0..20).map(|i| fx(&format!("f{i}"), i, "Fútbol", 1000 + i * 600, false)).collect();
        assert_eq!(widened_end(&pool, 1000 + 15 * 600, 12), 1000 + 15 * 600, "the window already holds 12");
        assert_eq!(widened_end(&pool, 1000 + 2 * 600, 12), 1000 + 11 * 600, "up to the 12th kick-off");
        assert_eq!(widened_end(&pool[..4], 1000, 12), 1000 + 3 * 600, "fewer than 12 in the pool: the last one");
        assert_eq!(widened_end(&[], 500, 12), 500);
    }

    #[test]
    fn the_daily_allowance_spreads_what_is_left_of_the_plan() {
        assert_eq!(daily_allowance(Some(91), 31, 0), 3, "91 calls for 31 days is under the floor of 3");
        assert_eq!(daily_allowance(Some(250), 10, 0), 25);
        assert_eq!(daily_allowance(Some(91), 31, 20), 20, "the user's cap wins");
        assert_eq!(daily_allowance(None, 31, 0), 12);
        let oct1 = parse_iso("2026-10-01T18:00:00Z").unwrap();
        assert_eq!(days_left_in_month(oct1, -300), 31);
        assert_eq!(days_left_in_month(parse_iso("2026-11-01T03:00:00Z").unwrap(), -300), 1, "still 31 October in Lima");
        assert_eq!(days_left_in_month(parse_iso("2028-02-10T12:00:00Z").unwrap(), 0), 20);
    }

    #[test]
    fn the_schedule_is_asked_for_a_day_and_a_half_within_48_hours() {
        let now = 1_000_000;
        assert_eq!(schedule_range(now, now - 3 * 3600, now + 2 * 3600), (now - 3 * 3600, now + 36 * 3600));
        let (from, to) = schedule_range(now, now + 34 * 3600, now + 58 * 3600);
        assert_eq!((from, to), (now + 34 * 3600, now + 58 * 3600), "a far request asks for its own range");
        assert!(to - from < 48 * 3600);
        let (from, to) = schedule_range(now, now - 20 * 3600, now + 4 * 3600);
        assert!(to - from <= 47 * 3600 && to >= now + 4 * 3600);
    }

    #[test]
    fn the_preview_keeps_main_markets_and_one_line_of_each_ladder() {
        let fixtures = vec![fx("id1", 9, "Fútbol", 1_790_881_500, false)];
        let q = |price: f64, active: bool| json!({ "players": { "0": { "price": price, "active": active }, "42": { "price": 9.0, "active": true, "playerName": "Jugador" } } });
        let odds = json!([{ "fixtureId": "id1", "bookmakerOdds": { "betano.pe": { "markets": {
            "101": { "outcomes": { "101": q(1.85, true), "102": q(3.4, true), "103": q(4.2, false) } },
            "500": { "outcomes": { "5001": q(1.2, true), "5002": q(4.5, true) } },
            "501": { "outcomes": { "5011": q(1.9, true), "5012": q(1.95, true) } },
            "502": { "outcomes": { "5021": q(3.0, true), "5022": q(1.3, true) } },
            "900": { "marketActive": false, "outcomes": { "9001": q(2.0, true) } }
        } } } }]);
        let mut names = HashMap::new();
        for (id, line, side) in [(5001, "1.5", "Más"), (5002, "1.5", "Menos"), (5011, "2.5", "Más"), (5012, "2.5", "Menos"), (5021, "3.5", "Más"), (5022, "3.5", "Menos")] {
            names.insert(id, (format!("Más/Menos Tiempo Completo {line}"), side.to_string()));
        }
        let text = odds_preview(&fixtures, &odds, &names, -300, 12);
        assert!(text.starts_with("- A vs B · Fútbol · T9 · 14:05\n"), "{text}");
        assert!(text.contains("  Resultado final: 1 @1.85 · X @3.4\n"), "the inactive 2 is left out: {text}");
        assert!(text.contains("  Más/Menos Tiempo Completo 2.5: Más @1.9 · Menos @1.95 (+2 líneas en el CSV)\n"), "the most even line: {text}");
        assert!(!text.contains("Jugador") && !text.contains("@2\n") && !text.contains("1.5:"), "{text}");
        let short = odds_preview(&fixtures, &odds, &names, -300, 1);
        assert!(short.contains("(+1 mercados más en el CSV)"), "{short}");
    }

    #[test]
    fn the_account_says_whether_betano_has_live_prices() {
        let account = |live: bool| json!({ "subscriptions": [{ "is_active": true, "request_limit": 250, "request_count": 159,
            "bookmakers": { "betano.pe": { "has_live_odds": live } } }] });
        assert!(!has_live_odds(&account(false)) && has_live_odds(&account(true)));
        assert_eq!((quota_limit(&account(false)), quota_remaining(&account(false))), (Some(250), Some(91)));
        let usage: Usage = serde_json::from_str("{\"day\":\"2026-10-01\",\"calls\":4}").unwrap();
        assert_eq!((usage.calls, usage.remaining), (4, None));
    }

    /// The plan's quota, without the key (`/account` is unmetered). `cargo test -p mika account_quota -- --ignored --nocapture`
    #[test]
    #[ignore = "calls OddsPapi with the user's key"]
    fn account_quota() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let key = secrets::get(KEY).expect("key saved");
        let account = rt.block_on(async { reqwest::Client::new().get(format!("{BASE}/account?apiKey={key}")).send().await.unwrap().json::<Value>().await.unwrap() });
        for sub in account["subscriptions"].as_array().into_iter().flatten() {
            println!("active={} limit={} count={} from={} until={} betano.pe={}", sub["is_active"], sub["request_limit"], sub["request_count"],
                     sub["valid_from"], sub["valid_until"], sub["bookmakers"][BOOKMAKER]);
        }
        println!("remaining: {:?}", quota_remaining(&account));
    }

    /// What the window holds, match by match (2 requests). `cargo test -p mika odds_stats -- --ignored --nocapture`
    #[test]
    #[ignore = "calls OddsPapi with the user's key"]
    fn odds_stats() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let key = secrets::get(KEY).expect("key saved");
        let http = reqwest::Client::new();
        let now = (crate::services::chat_store::now_ms() / 1000) as i64;
        let get = |path: String| rt.block_on(async { http.get(format!("{BASE}{path}&apiKey={key}")).send().await.unwrap().json::<Value>().await.unwrap() });
        let fixtures = get(format!("/fixtures?sportId={FOOTBALL}&from={}&to={}&hasOdds=true&bookmakers={BOOKMAKER}", iso_utc(now - 3 * 3600), iso_utc(now + 3 * 3600)));
        let all = fixtures.as_array().map_or(0, |a| a.len());
        let picked = pick_fixtures(&fixtures, now, now + 3 * 3600);
        println!("fixtures: {all}, picked: {}", picked.len());
        for f in &picked { println!("  {} {} vs {} [{}] live={} t={}", f.id, f.home, f.away, f.tournament, f.live, f.tournament_id); }
        let tournaments: Vec<String> = picked.iter().map(|f| f.tournament_id.to_string()).collect::<std::collections::BTreeSet<_>>().into_iter().take(3).collect();
        std::thread::sleep(Duration::from_millis(2100));
        let odds = get(format!("/odds-by-tournaments?tournamentIds={}&bookmaker={BOOKMAKER}", tournaments.join(",")));
        for entry in odds.as_array().into_iter().flatten() {
            let id = entry["fixtureId"].as_str().unwrap_or("");
            if !picked.iter().any(|f| f.id == id) { continue; }
            let books: Vec<&String> = entry["bookmakerOdds"].as_object().map(|o| o.keys().collect()).unwrap_or_default();
            let book = &entry["bookmakerOdds"][BOOKMAKER];
            let markets = book["markets"].as_object().map_or(0, |m| m.len());
            let mut active = 0; let mut main = 0;
            for (_, m) in book["markets"].as_object().into_iter().flatten() {
                for (_, o) in m["outcomes"].as_object().into_iter().flatten() {
                    if o["players"]["0"]["active"] != false { active += 1; }
                    if o["players"]["0"]["mainLine"] == true { main += 1; }
                }
            }
            println!("  odds {id}: books={books:?} suspended={} markets={markets} active={active} mainLine={main} 101={}", book["suspended"], book["markets"]["101"]);
        }
    }

    /// A real manual Nations League request with the key saved in the Credential Manager.
    /// `cargo test -p mika a_real_snapshot -- --ignored --nocapture`
    #[test]
    #[ignore = "calls OddsPapi with the user's key"]
    fn provider_tournament_contract() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let key = secrets::get(KEY).expect("key saved");
        rt.block_on(async {
            let response = reqwest::Client::new().get(format!("{BASE}/odds-by-tournaments"))
                .query(&[("tournamentIds", "23755"), ("bookmakers", BOOKMAKER), ("language", "es"), ("verbosity", "3"), ("apiKey", key.as_str())])
                .send().await.unwrap();
            let status = response.status();
            let body = response.json::<Value>().await.unwrap();
            println!("documented parameters: {status}");
            if !status.is_success() { println!("{}", body.to_string().replace(&key, "[redacted]").chars().take(1500).collect::<String>()); }
            else { println!("returned fixtures: {}", odds_entries(&body).len()); }
        });
    }

    #[test]
    fn manual_today_reaches_midnight_in_lima() {
        let now = parse_iso("2026-10-01T16:19:00Z").unwrap();
        let (from, to) = manual_window("Nations League hoy", now, -300).unwrap();
        assert_eq!(iso_utc(from), "2026-10-01T05:00:00Z");
        assert_eq!(iso_utc(to), "2026-10-02T05:00:00Z");
        assert!(parse_iso("2026-10-01T18:45:00Z").unwrap() < to);
    }

    #[test]
    fn manual_dates_and_explicit_hours_ignore_automatic_settings() {
        let now = parse_iso("2026-10-01T16:19:00Z").unwrap();
        let (from, to) = manual_window("tenis mañana", now, -300).unwrap();
        assert_eq!(iso_utc(from), "2026-10-02T05:00:00Z");
        assert_eq!(to - from, 86400);
        assert_eq!(manual_window("fútbol el 2026-10-03", now, -300).unwrap().0, from + 86400);
        assert_eq!(manual_window("próximas 6 horas", now, -300).unwrap(), (now, now + 6 * 3600));
        assert!(manual_window("fútbol el 2026-02-30", now, -300).is_err());
        assert!(manual_window("partidos esta semana", now, -300).is_err());
    }

    #[test]
    fn manual_competition_filter_matches_spanish_api_names() {
        let now = parse_iso("2026-10-01T16:19:00Z").unwrap();
        let json = serde_json::json!([
            {"fixtureId":"id1", "startTime":"2026-10-01T18:45:00Z", "tournamentName":"Liga de Naciones UEFA", "tournamentId":23755},
            {"fixtureId":"id2", "startTime":"2026-10-01T20:00:00Z", "tournamentName":"Liga de Naciones CONCACAF", "tournamentId":27420},
            {"fixtureId":"id3", "startTime":"2026-10-01T18:45:00Z", "tournamentName":"NM Cup", "tournamentId":29}
        ]);
        let fixtures = pick_fixtures(&json, now, now + 86400);
        assert_eq!(requested_fixtures(fixtures.clone(), "UEFA Nations League hoy").len(), 1);
        assert_eq!(requested_fixtures(fixtures, "liga de naciones hoy").len(), 2);
    }

    #[test]
    #[ignore = "calls OddsPapi with the user's key"]
    fn a_real_snapshot() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let now = (crate::services::chat_store::now_ms() / 1000) as i64;
        let table = rt.block_on(manual_snapshot(now, crate::platform::clock::utc_offset_minutes(), "hoy", 0));
        match &table { Ok(t) => println!("{}", t.chars().take(6000).collect::<String>()), Err(e) => println!("ERR {e}") }
        println!("{:?}", load_usage());
        assert!(table.is_ok());
    }
}
