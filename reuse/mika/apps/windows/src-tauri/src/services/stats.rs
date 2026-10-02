//! Free team and player form for PARLEY, from two keyless public sources: Football-Data.co.uk (football results CSVs)
//! and TennisMyLife (tennis match CSVs). Off until the user turns on `useFreeStats`. Once a day, after the morning odds
//! snapshot, each file is requested at most once (conditional GET with ETag / Last-Modified, a short pause between
//! files, `User-Agent: MIKA`), parsed, reduced to per-team / per-player form and cached in `odds/stats-cache.json`.
//! The form of today's fixtures is appended to `resumen-del-dia.md` ("## Forma reciente") and written to
//! `stats-hoy.csv`. A team or player that cannot be matched with certainty is reported as "sin datos"; names are never
//! guessed (exact match after normalisation and a fixed alias table, nothing fuzzy). See docs/PARLEY-DATA.md.

use super::odds::{self, Fixture};
use super::settings;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const FD_BASE: &str = "https://football-data.co.uk";
const TML_BASE: &str = "https://stats.tennismylife.org/data";
const USER_AGENT: &str = "MIKA";
/// Pause between two requests to the same site.
const GAP: Duration = Duration::from_millis(900);
const MAX_BYTES: usize = 30_000_000;
/// After a failed run, wait this long before asking again.
const RETRY: Duration = Duration::from_secs(15 * 60);

/// Football-Data's main leagues: file code and readable name. `/mmz4281/<season>/<code>.csv`.
const MAIN_LEAGUES: [(&str, &str); 12] = [
    ("E0", "Premier League"), ("E1", "Championship"), ("SP1", "La Liga"), ("D1", "Bundesliga"), ("I1", "Serie A"), ("F1", "Ligue 1"),
    ("N1", "Eredivisie"), ("P1", "Liga Portugal"), ("B1", "Pro League"), ("T1", "Süper Lig"), ("SC0", "Scottish Premiership"), ("G1", "Super League Grecia"),
];
/// Football-Data's "extra leagues": `/new/<code>.csv`, the whole history of one country in one file. Peru, Colombia and
/// Chile have no file (COL/CHL answer 200 with Poland's/China's data: never used).
const EXTRA_COUNTRIES: [&str; 16] = ["ARG", "BRA", "MEX", "USA", "JPN", "CHN", "DNK", "NOR", "SWE", "RUS", "FIN", "IRL", "AUT", "POL", "ROU", "SWZ"];

// ── CSV and dates ────────────────────────────────────────────────────────────────────────────────

/// RFC 4180 reader: BOM, quoted fields with `""`, commas and line breaks inside quotes, CRLF, blank lines.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let (mut rows, mut row, mut field, mut quoted) = (Vec::new(), Vec::new(), String::new(), false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') { field.push('"'); chars.next(); } else { quoted = false; }
            } else { field.push(c); }
        } else {
            match c {
                '"' if field.is_empty() => quoted = true,
                ',' => row.push(std::mem::take(&mut field)),
                '\r' => {}
                '\n' => { row.push(std::mem::take(&mut field)); rows.push(std::mem::take(&mut row)); }
                _ => field.push(c),
            }
        }
    }
    if !field.is_empty() || !row.is_empty() { row.push(field); rows.push(row); }
    rows.retain(|r| !(r.len() == 1 && r[0].trim().is_empty()));
    rows
}

/// A CSV with a header row; missing columns read as "".
struct Table { index: HashMap<String, usize>, rows: Vec<Vec<String>> }

impl Table {
    fn new(text: &str) -> Table {
        let mut rows = parse_csv(text);
        let header = if rows.is_empty() { Vec::new() } else { rows.remove(0) };
        let index = header.iter().enumerate().map(|(i, h)| (h.trim().to_string(), i)).collect();
        Table { index, rows }
    }
    fn has(&self, columns: &[&str]) -> bool { columns.iter().all(|c| self.index.contains_key(*c)) }
    fn get<'a>(&self, row: &'a [String], column: &str) -> &'a str {
        self.index.get(column).and_then(|i| row.get(*i)).map_or("", |s| s.trim())
    }
}

/// Days since 1970-01-01 of a civil date (Howard Hinnant).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// `dd/mm/yyyy`, `dd/mm/yy` (old Football-Data files), `yyyy-mm-dd` or `yyyymmdd` → days since the epoch.
pub fn parse_date(text: &str) -> Option<i64> {
    let t = text.trim();
    let ok = |y: i64, m: i64, d: i64| ((1..=12).contains(&m) && (1..=31).contains(&d) && (1900..=2200).contains(&y)).then(|| days_from_civil(y, m, d));
    if t.len() == 8 && t.chars().all(|c| c.is_ascii_digit()) {
        return ok(t[..4].parse().ok()?, t[4..6].parse().ok()?, t[6..].parse().ok()?);
    }
    if t.contains('/') {
        let p: Vec<&str> = t.split('/').collect();
        if p.len() != 3 { return None; }
        let (d, m, mut y): (i64, i64, i64) = (p[0].parse().ok()?, p[1].parse().ok()?, p[2].parse().ok()?);
        if y < 100 { y += if y < 70 { 2000 } else { 1900 }; }
        return ok(y, m, d);
    }
    let p: Vec<&str> = t.split('-').collect();
    if p.len() == 3 { return ok(p[0].parse().ok()?, p[1].parse().ok()?, p[2].parse().ok()?); }
    None
}

pub fn iso(days: i64) -> String { let (y, m, d) = civil_from_days(days); format!("{y:04}-{m:02}-{d:02}") }
fn dmy(days: i64) -> String { let (y, m, d) = civil_from_days(days); format!("{d:02}/{m:02}/{y:04}") }

/// Football-Data's season folder for a date: seasons start in July (`2526` from 2025-07 to 2026-06).
pub fn season_code(year: i64, month: i64) -> String {
    let start = if month >= 7 { year } else { year - 1 };
    format!("{:02}{:02}", start.rem_euclid(100), (start + 1).rem_euclid(100))
}

fn previous_season(code: &str) -> String {
    let start: i64 = code.get(..2).and_then(|s| s.parse().ok()).unwrap_or(0);
    format!("{:02}{:02}", (start - 1).rem_euclid(100), start.rem_euclid(100))
}

// ── Football ─────────────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct FMatch { pub day: i64, pub home: String, pub away: String, pub hg: u32, pub ag: u32, pub season: String, pub league: String }

/// Played matches of a Football-Data file: main layout (`HomeTeam, AwayTeam, FTHG, FTAG`) or extra-leagues layout
/// (`Home, Away, HG, AG, Season, League`). Unplayed rows (no score) and rows with a broken date are skipped.
pub fn parse_football(text: &str, default_league: &str) -> Result<Vec<FMatch>, String> {
    let table = Table::new(text);
    let (home_col, away_col, hg_col, ag_col) = if table.has(&["HomeTeam", "AwayTeam", "FTHG", "FTAG"]) { ("HomeTeam", "AwayTeam", "FTHG", "FTAG") }
        else if table.has(&["Home", "Away", "HG", "AG"]) { ("Home", "Away", "HG", "AG") }
        else { return Err("Football-Data: el archivo no tiene las columnas esperadas.".into()); };
    if !table.has(&["Date"]) { return Err("Football-Data: falta la columna Date.".into()); }
    let mut out = Vec::new();
    for row in &table.rows {
        let (Some(day), Ok(hg), Ok(ag)) = (parse_date(table.get(row, "Date")), table.get(row, hg_col).parse::<u32>(), table.get(row, ag_col).parse::<u32>()) else { continue };
        let (home, away) = (table.get(row, home_col), table.get(row, away_col));
        if home.is_empty() || away.is_empty() { continue; }
        let league = if table.has(&["League"]) {
            let country = table.get(row, "Country");
            let name = table.get(row, "League");
            if country.is_empty() { name.to_string() } else { format!("{country} {name}") }
        } else { default_league.to_string() };
        out.push(FMatch { day, home: home.into(), away: away.into(), hg, ag, season: table.get(row, "Season").into(), league });
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Split { pub form: String, pub pts: u32, pub n: u32 }

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TeamForm {
    pub name: String,
    pub league: String,
    /// Cache key of the file it came from; two files with the same canonical name make a name ambiguous.
    pub source: String,
    /// Last 5 results, oldest to newest (the last letter is the latest match).
    pub form5: String,
    pub gf5: u32,
    pub ga5: u32,
    pub n5: u32,
    /// Last 5 as home team and last 5 as away team.
    pub home: Split,
    pub away: Split,
    /// Points per game in the team's current season (all the file's competitions) and games played.
    pub ppg: f64,
    pub played: u32,
    /// Date of the latest match (ISO).
    pub last: String,
}

fn letter(gf: u32, ga: u32) -> char { if gf > ga { 'W' } else if gf == ga { 'D' } else { 'L' } }
fn points(c: char) -> u32 { match c { 'W' => 3, 'D' => 1, _ => 0 } }

/// Form of every team in a list of matches (any order).
pub fn football_forms(matches: &[FMatch], source: &str) -> Vec<TeamForm> {
    struct E { day: i64, idx: usize, home: bool, gf: u32, ga: u32, season: String, league: String }
    let mut by_team: BTreeMap<&str, Vec<E>> = BTreeMap::new();
    for (idx, m) in matches.iter().enumerate() {
        by_team.entry(&m.home).or_default().push(E { day: m.day, idx, home: true, gf: m.hg, ga: m.ag, season: m.season.clone(), league: m.league.clone() });
        by_team.entry(&m.away).or_default().push(E { day: m.day, idx, home: false, gf: m.ag, ga: m.hg, season: m.season.clone(), league: m.league.clone() });
    }
    let split = |entries: &[&E]| {
        let tail = &entries[entries.len().saturating_sub(5)..];
        let form: String = tail.iter().map(|e| letter(e.gf, e.ga)).collect();
        Split { pts: form.chars().map(points).sum(), n: tail.len() as u32, form }
    };
    by_team.into_iter().map(|(name, mut entries)| {
        entries.sort_by_key(|e| (e.day, e.idx));
        let all: Vec<&E> = entries.iter().collect();
        let last = *all.last().expect("a team has at least one match");
        let tail = &all[all.len().saturating_sub(5)..];
        let season: Vec<&&E> = all.iter().filter(|e| e.season == last.season).collect();
        let season_pts: u32 = season.iter().map(|e| points(letter(e.gf, e.ga))).sum();
        let (homes, aways): (Vec<&E>, Vec<&E>) = (all.iter().copied().filter(|e| e.home).collect(), all.iter().copied().filter(|e| !e.home).collect());
        TeamForm {
            name: name.to_string(), league: last.league.clone(), source: source.to_string(),
            form5: tail.iter().map(|e| letter(e.gf, e.ga)).collect(),
            gf5: tail.iter().map(|e| e.gf).sum(), ga5: tail.iter().map(|e| e.ga).sum(), n5: tail.len() as u32,
            home: split(&homes), away: split(&aways),
            ppg: (f64::from(season_pts) / season.len() as f64 * 100.0).round() / 100.0, played: season.len() as u32,
            last: iso(last.day),
        }
    }).collect()
}

/// Lower case, no accents, letters and digits separated by single spaces (apostrophes vanish).
pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.to_lowercase().chars() {
        match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => out.push('a'),
            'ç' | 'ć' | 'č' => out.push('c'),
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ė' | 'ę' | 'ě' => out.push('e'),
            'ì' | 'í' | 'î' | 'ï' | 'ī' | 'ı' => out.push('i'),
            'ñ' | 'ń' | 'ň' => out.push('n'),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ő' | 'ō' => out.push('o'),
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' | 'ű' => out.push('u'),
            'ý' | 'ÿ' => out.push('y'),
            'š' | 'ś' | 'ş' => out.push('s'),
            'ž' | 'ź' | 'ż' => out.push('z'),
            'ł' => out.push('l'),
            'đ' | 'ð' => out.push('d'),
            'ğ' => out.push('g'),
            'ß' => out.push_str("ss"),
            '\'' | '’' | '`' | '´' => {}
            '&' => out.push_str(" and "),
            c if c.is_alphanumeric() => out.push(c),
            _ => out.push(' '),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Club-form words that sources put in front of or behind the same name ("CD Godoy Cruz", "Godoy Cruz", "1. FC Köln").
const NOISE: [&str; 24] = ["fc", "cf", "cd", "ca", "ac", "afc", "sc", "ud", "sd", "ad", "rcd", "ssc", "as", "fk", "sk", "bk", "club", "cp", "sv", "vfl", "vfb", "tsv", "fsv", "1"];

fn squash(name: &str) -> String {
    let folded = fold(name);
    let mut tokens: Vec<&str> = folded.split(' ').collect();
    while tokens.len() > 1 && NOISE.contains(&tokens[0]) { tokens.remove(0); }
    while tokens.len() > 1 && NOISE.contains(&tokens[tokens.len() - 1]) { tokens.pop(); }
    tokens.join(" ")
}

/// Canonical name → the spellings the two sources (and Football-Data itself) use for the same club.
const ALIASES: &[(&str, &[&str])] = &[
    ("man united", &["manchester united", "man utd", "manchester utd"]),
    ("man city", &["manchester city"]),
    ("tottenham", &["tottenham hotspur", "spurs"]),
    ("nottm forest", &["nottingham forest", "nott m forest", "nottm forest"]),
    ("wolves", &["wolverhampton", "wolverhampton wanderers"]),
    ("newcastle", &["newcastle united", "newcastle utd"]),
    ("west ham", &["west ham united"]),
    ("leeds", &["leeds united"]),
    ("brighton", &["brighton and hove albion", "brighton hove albion"]),
    ("sheffield united", &["sheffield utd"]),
    ("sheffield weds", &["sheffield wednesday"]),
    ("qpr", &["queens park rangers"]),
    ("west brom", &["west bromwich albion", "west bromwich"]),
    ("leicester", &["leicester city"]),
    ("norwich", &["norwich city"]),
    ("ath madrid", &["atletico madrid", "atletico de madrid", "atl madrid"]),
    ("ath bilbao", &["athletic bilbao", "athletic club", "athletic de bilbao"]),
    ("betis", &["real betis"]),
    ("sociedad", &["real sociedad"]),
    ("celta", &["celta vigo", "celta de vigo"]),
    ("espanol", &["espanyol", "rcd espanyol"]),
    ("vallecano", &["rayo vallecano"]),
    ("alaves", &["deportivo alaves"]),
    ("bayern munich", &["bayern munchen", "bayern"]),
    ("dortmund", &["borussia dortmund"]),
    ("m gladbach", &["mgladbach", "monchengladbach", "borussia monchengladbach", "gladbach"]),
    ("leverkusen", &["bayer leverkusen", "bayer 04 leverkusen"]),
    ("ein frankfurt", &["eintracht frankfurt", "frankfurt"]),
    ("rb leipzig", &["leipzig", "rasenballsport leipzig"]),
    ("stuttgart", &["vfb stuttgart"]),
    ("hoffenheim", &["tsg hoffenheim", "tsg 1899 hoffenheim"]),
    ("mainz", &["mainz 05", "fsv mainz 05"]),
    ("wolfsburg", &["vfl wolfsburg"]),
    ("werder bremen", &["bremen", "sv werder bremen"]),
    ("union berlin", &["1 fc union berlin"]),
    ("koln", &["fc koln", "cologne", "1 fc koln"]),
    ("st pauli", &["fc st pauli"]),
    ("inter", &["inter milan", "internazionale", "internazionale milano"]),
    ("milan", &["ac milan"]),
    ("napoli", &["ssc napoli"]),
    ("roma", &["as roma"]),
    ("lazio", &["ss lazio"]),
    ("verona", &["hellas verona"]),
    ("paris sg", &["psg", "paris saint germain", "paris saint-germain"]),
    ("marseille", &["olympique marseille", "olympique de marseille"]),
    ("lyon", &["olympique lyonnais", "olympique lyon"]),
    ("st etienne", &["saint etienne", "as saint etienne"]),
    ("monaco", &["as monaco"]),
    ("sp lisbon", &["sporting cp", "sporting lisbon", "sporting lisboa", "sporting clube de portugal", "sporting"]),
    ("sp braga", &["braga", "sporting braga", "sc braga"]),
    ("benfica", &["sl benfica"]),
    ("porto", &["fc porto"]),
    ("psv", &["psv eindhoven"]),
    ("ajax", &["ajax amsterdam"]),
    ("feyenoord", &["feyenoord rotterdam"]),
    ("club brugge", &["brugge", "club brugge kv"]),
    ("st truiden", &["sint truiden", "sint truidense"]),
    ("anderlecht", &["rsc anderlecht"]),
    ("standard", &["standard liege"]),
    ("rangers", &["glasgow rangers"]),
    ("hearts", &["heart of midlothian"]),
    ("river plate", &["ca river plate"]),
    ("boca juniors", &["ca boca juniors"]),
    ("ind rivadavia", &["independiente rivadavia"]),
    ("estudiantes l p", &["estudiantes la plata", "estudiantes de la plata"]),
    ("velez sarsfield", &["velez", "ca velez sarsfield"]),
    ("argentinos jrs", &["argentinos juniors"]),
    ("newells old boys", &["newells", "newell s old boys"]),
    ("gimnasia l p", &["gimnasia la plata", "gimnasia y esgrima la plata"]),
    ("talleres cordoba", &["talleres de cordoba", "talleres"]),
    ("central cordoba", &["central cordoba sde"]),
    ("union de santa fe", &["union santa fe", "union"]),
    ("belgrano", &["belgrano de cordoba", "belgrano cordoba"]),
    ("flamengo rj", &["flamengo"]),
    ("sao paulo", &["sao paulo fc"]),
    ("atletico mg", &["atletico mineiro"]),
    ("athletico pr", &["athletico paranaense", "atletico paranaense"]),
    ("botafogo rj", &["botafogo"]),
    ("vasco", &["vasco da gama"]),
    ("internacional", &["sc internacional"]),
    ("red bull bragantino", &["bragantino", "rb bragantino"]),
    ("santos", &["santos fc"]),
    ("sport recife", &["sport"]),
    ("america", &["club america", "america mexico"]),
    ("guadalajara", &["chivas", "guadalajara chivas", "chivas guadalajara"]),
    ("pumas unam", &["unam", "pumas", "unam pumas"]),
    ("tigres uanl", &["tigres"]),
    ("monterrey", &["cf monterrey"]),
    ("leon", &["club leon"]),
    ("tijuana", &["club tijuana", "xolos"]),
    ("la galaxy", &["los angeles galaxy", "l a galaxy"]),
    ("los angeles fc", &["lafc"]),
    ("new york city", &["nycfc", "new york city fc"]),
    ("new york red bulls", &["ny red bulls"]),
    ("inter miami", &["inter miami cf"]),
];

fn alias_map() -> &'static HashMap<String, String> {
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut map = HashMap::new();
        for (canon, variants) in ALIASES {
            let canon = squash(canon);
            for v in *variants { map.insert(squash(v), canon.clone()); }
            map.insert(canon.clone(), canon);
        }
        map
    })
}

/// The key two spellings of one club share. Exact equality of these keys is the only way two names match.
pub fn team_key(name: &str) -> String {
    let s = squash(name);
    alias_map().get(&s).cloned().unwrap_or(s)
}

#[derive(Default)]
pub struct FootballIndex { teams: HashMap<String, Vec<TeamForm>> }

impl FootballIndex {
    pub fn build<'a>(sources: impl IntoIterator<Item = &'a Vec<TeamForm>>) -> FootballIndex {
        let mut teams: HashMap<String, Vec<TeamForm>> = HashMap::new();
        for list in sources { for t in list { teams.entry(team_key(&t.name)).or_default().push(t.clone()); } }
        FootballIndex { teams }
    }
    /// Found, or the reason there is no data. Two different files with the same key are "ambiguous", not a guess.
    pub fn lookup(&self, name: &str) -> Result<&TeamForm, String> {
        match self.teams.get(&team_key(name)).map(|v| v.as_slice()) {
            None | Some([]) => Err("no está en las ligas de Football-Data".into()),
            Some([one]) => Ok(one),
            Some(many) => Err(format!("nombre ambiguo ({} equipos con ese nombre)", many.len())),
        }
    }
}

/// Women's, youth and reserve competitions share club names with the men's first team: never matched.
fn not_senior_men(tournament: &str) -> bool {
    let t = fold(tournament);
    const WORDS: [&str; 18] = ["women", "womens", "femenino", "femenina", "feminino", "feminin", "reserve", "reserves", "reserva", "reservas", "youth", "juvenil", "u17", "u18", "u19", "u20", "u21", "u23"];
    t.split(' ').any(|w| WORDS.contains(&w)) || t.contains("sub 20") || t.contains("sub 23") || t.contains("sub 17") || t.contains("sub 19")
}

// ── Tennis ───────────────────────────────────────────────────────────────────────────────────────

/// One finished singles match, compact for the cache (serialised as an array).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TMatch(
    /// tourney_date (start of the tournament) as days since the epoch
    pub i64,
    /// tour: "atp", "challenger" or "wta"
    pub String,
    /// tourney_name
    pub String,
    /// surface
    pub String,
    /// round
    pub String,
    /// winner_id
    pub String,
    /// winner_name
    pub String,
    /// loser_id
    pub String,
    /// loser_name
    pub String,
    /// winner_rank
    pub String,
    /// loser_rank
    pub String,
    /// tourney_id + match number, to drop a match seen in the year file and in the ongoing file
    pub String,
);

fn round_order(round: &str) -> i32 {
    match round { "R128" => 10, "R64" => 20, "R32" => 30, "R16" => 40, "QF" => 50, "SF" => 60, "BR" => 65, "F" => 70, "RR" => 5, r if r.starts_with('Q') => 1, _ => 0 }
}

/// Played singles matches of a TennisMyLife file. Walkovers are skipped (nobody played); retirements count.
pub fn parse_tennis(text: &str, tour: &str) -> Result<Vec<TMatch>, String> {
    let table = Table::new(text);
    if !table.has(&["tourney_name", "surface", "tourney_date", "winner_id", "winner_name", "loser_id", "loser_name", "score", "round"]) {
        return Err("TennisMyLife: el archivo no tiene las columnas esperadas.".into());
    }
    let mut out = Vec::new();
    for row in &table.rows {
        let Some(day) = parse_date(table.get(row, "tourney_date")) else { continue };
        let (wid, lid) = (table.get(row, "winner_id"), table.get(row, "loser_id"));
        let score = table.get(row, "score");
        if wid.is_empty() || lid.is_empty() || score.is_empty() || score.contains("W/O") || score.contains("DEF") { continue; }
        out.push(TMatch(day, tour.into(), table.get(row, "tourney_name").into(), table.get(row, "surface").into(), table.get(row, "round").into(),
            wid.into(), table.get(row, "winner_name").into(), lid.into(), table.get(row, "loser_name").into(),
            table.get(row, "winner_rank").into(), table.get(row, "loser_rank").into(),
            format!("{}#{}#{wid}#{lid}", table.get(row, "tourney_id"), table.get(row, "match_num"))));
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlayerForm {
    pub id: String,
    pub name: String,
    pub tour: String,
    /// Last 5 results, oldest to newest.
    pub form5: String,
    /// This year's wins and losses by surface, as (surface, wins, losses).
    pub surfaces: Vec<(String, u32, u32)>,
    pub last_tourney: String,
    pub last_round: String,
    pub last_won: bool,
    /// Start date of the last tournament (ISO); TennisMyLife dates a match by its tournament.
    pub last: String,
    pub rank: String,
    tokens: Vec<String>,
}

pub struct TennisIndex { players: Vec<PlayerForm> }

fn name_tokens(name: &str) -> Vec<String> {
    fold(&name.replace(['.', '-'], " ")).split(' ').filter(|t| !t.is_empty()).map(String::from).collect()
}

/// "Surname I." / "Surname I J" (initials) or "First Surname" (any order of the same words) against a full name.
pub fn tennis_name_matches(fixture: &[String], player: &[String]) -> bool {
    if fixture.len() < 2 || player.len() < 2 { return false; }
    let mut k = fixture.len();
    while k > 0 && fixture[k - 1].chars().count() == 1 { k -= 1; }
    if k < fixture.len() {
        if k == 0 { return false; }
        let (surname, initials) = (&fixture[..k], &fixture[k..]);
        return (1..player.len()).any(|s| {
            player[s..] == *surname && {
                let given: Vec<char> = player[..s].iter().filter_map(|t| t.chars().next()).collect();
                given.len() >= initials.len() && initials.iter().zip(&given).all(|(a, b)| a.chars().next() == Some(*b))
            }
        });
    }
    let (mut a, mut b) = (fixture.to_vec(), player.to_vec());
    a.sort();
    b.sort();
    a == b
}

impl TennisIndex {
    /// `year` is the current year (surface record "this year").
    pub fn build(matches: &[TMatch], year: i64) -> TennisIndex {
        let mut all: Vec<&TMatch> = matches.iter().collect();
        all.sort_by_key(|m| (m.0, round_order(&m.4), m.11.clone()));
        all.dedup_by(|a, b| a.11 == b.11);
        let mut by_id: HashMap<&str, Vec<(&TMatch, bool)>> = HashMap::new();
        for m in &all {
            by_id.entry(&m.5).or_default().push((m, true));
            by_id.entry(&m.7).or_default().push((m, false));
        }
        let mut players: Vec<PlayerForm> = by_id.into_iter().map(|(id, games)| {
            let (last, last_won) = *games.last().expect("a player has a match");
            let tail = &games[games.len().saturating_sub(5)..];
            let mut surfaces: BTreeMap<&str, (u32, u32)> = BTreeMap::new();
            for (m, won) in &games {
                if civil_from_days(m.0).0 != year || m.3.is_empty() { continue; }
                let e = surfaces.entry(&m.3).or_default();
                if *won { e.0 += 1 } else { e.1 += 1 }
            }
            let name = if last_won { &last.6 } else { &last.8 };
            PlayerForm {
                id: id.to_string(), name: name.clone(), tour: last.1.clone(),
                form5: tail.iter().map(|(_, w)| if *w { 'W' } else { 'L' }).collect(),
                surfaces: surfaces.into_iter().map(|(s, (w, l))| (s.to_string(), w, l)).collect(),
                last_tourney: last.2.clone(), last_round: last.4.clone(), last_won, last: iso(last.0),
                rank: if last_won { last.9.clone() } else { last.10.clone() },
                tokens: name_tokens(name),
            }
        }).collect();
        players.sort_by(|a, b| a.id.cmp(&b.id));
        TennisIndex { players }
    }

    /// `wta`: Some(true) when the tournament says women's tour, Some(false) for ATP/Challenger, None when unknown.
    pub fn lookup(&self, name: &str, wta: Option<bool>) -> Result<&PlayerForm, String> {
        let tokens = name_tokens(name);
        let found: Vec<&PlayerForm> = self.players.iter()
            .filter(|p| wta.is_none_or(|w| (p.tour == "wta") == w) && tennis_name_matches(&tokens, &p.tokens)).collect();
        match found.as_slice() {
            [] => Err("no está en los archivos ATP/Challenger/WTA de TennisMyLife".into()),
            [one] => Ok(one),
            many => Err(format!("nombre ambiguo ({} jugadores)", many.len())),
        }
    }
}

fn tour_hint(tournament: &str) -> Option<bool> {
    let t = fold(tournament);
    let words: Vec<&str> = t.split(' ').collect();
    if words.iter().any(|w| ["wta", "femenino", "femenina", "women", "womens"].contains(w)) { return Some(true); }
    if words.iter().any(|w| ["atp", "challenger", "masculino"].contains(w)) { return Some(false); }
    None
}

// ── Rows, text and files ─────────────────────────────────────────────────────────────────────────

/// One side of one fixture: the form found or the reason there is none.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Row {
    pub fixture_id: String, pub sport: String, pub tournament: String, pub start: String,
    /// "local" or "visita" (tennis: first and second player as the provider lists them).
    pub side: String, pub name: String,
    pub found: bool, pub reason: String, pub matched: String, pub source: String, pub data_day: String, pub last_match: String,
    pub form5: String, pub gf5: String, pub ga5: String, pub n5: String,
    pub cond: String, pub cond_form: String, pub cond_pts: String, pub cond_n: String,
    pub ppg: String, pub played: String,
    pub surfaces: String, pub last_tourney: String, pub rank: String,
    /// The fragment of the "Forma:" line.
    pub text: String,
}

fn is_football(sport: &str) -> bool { let s = fold(sport); s.contains("futbol") || s.contains("soccer") || s == "football" }
fn is_tennis(sport: &str) -> bool { let s = fold(sport); s.contains("tenis") || s.contains("tennis") }
fn is_doubles(name: &str) -> bool { name.contains('/') }

fn sin_datos(f: &Fixture, side: &str, name: &str, reason: &str) -> Row {
    Row { fixture_id: f.id.clone(), sport: f.sport.clone(), tournament: f.tournament.clone(), start: odds::iso_utc(f.start), side: side.into(), name: name.into(),
        reason: reason.into(), text: format!("{name} sin datos ({reason})"), ..Row::default() }
}

fn football_row(f: &Fixture, home: bool, index: &FootballIndex, data_day: &str) -> Row {
    let (side, name) = if home { ("local", f.home.as_str()) } else { ("visita", f.away.as_str()) };
    if not_senior_men(&f.tournament) { return sin_datos(f, side, name, "torneo femenino, juvenil o de reservas: Football-Data trae solo primeros equipos masculinos"); }
    match index.lookup(name) {
        Err(reason) => sin_datos(f, side, name, &reason),
        Ok(t) => {
            let cond = if home { &t.home } else { &t.away };
            let mut text = format!("{name} {} ({} GF/{} GC en {} PJ", t.form5, t.gf5, t.ga5, t.n5);
            if cond.n > 0 { text.push_str(&format!("; {side} {} pts en {} PJ", cond.pts, cond.n)); }
            text.push_str(&format!("; {:.2} pts/PJ en {} PJ de la temporada)", t.ppg, t.played));
            Row { fixture_id: f.id.clone(), sport: f.sport.clone(), tournament: f.tournament.clone(), start: odds::iso_utc(f.start), side: side.into(), name: name.into(),
                found: true, matched: t.name.clone(), source: format!("Football-Data {}", t.league), data_day: data_day.into(), last_match: t.last.clone(),
                form5: t.form5.clone(), gf5: t.gf5.to_string(), ga5: t.ga5.to_string(), n5: t.n5.to_string(),
                cond: side.into(), cond_form: cond.form.clone(), cond_pts: cond.pts.to_string(), cond_n: cond.n.to_string(),
                ppg: format!("{:.2}", t.ppg), played: t.played.to_string(), text, ..Row::default() }
        }
    }
}

fn tennis_row(f: &Fixture, first: bool, index: &TennisIndex, data_day: &str) -> Row {
    let (side, name) = if first { ("local", f.home.as_str()) } else { ("visita", f.away.as_str()) };
    if is_doubles(name) { return sin_datos(f, side, name, "dobles: no se calcula forma de parejas"); }
    match index.lookup(name, tour_hint(&f.tournament)) {
        Err(reason) => sin_datos(f, side, name, &reason),
        Ok(p) => {
            let surfaces = p.surfaces.iter().map(|(s, w, l)| format!("{s} {w}-{l}")).collect::<Vec<_>>().join(", ");
            let last = format!("{} ({}, {}, {})", p.last_tourney, dmy(parse_date(&p.last).unwrap_or(0)), p.last_round, if p.last_won { "ganó" } else { "perdió" });
            let mut text = format!("{name} {}", p.form5);
            let mut parts = Vec::new();
            if !surfaces.is_empty() { parts.push(format!("este año {surfaces}")); }
            parts.push(format!("último torneo {last}"));
            if !p.rank.is_empty() { parts.push(format!("ranking {}", p.rank)); }
            text.push_str(&format!(" ({})", parts.join("; ")));
            Row { fixture_id: f.id.clone(), sport: f.sport.clone(), tournament: f.tournament.clone(), start: odds::iso_utc(f.start), side: side.into(), name: name.into(),
                found: true, matched: p.name.clone(), source: "TennisMyLife".into(), data_day: data_day.into(), last_match: p.last.clone(),
                form5: p.form5.clone(), n5: p.form5.chars().count().to_string(), surfaces, last_tourney: last, rank: p.rank.clone(), text, ..Row::default() }
        }
    }
}

/// Two rows per football fixture and per tennis singles fixture, in fixture order. Other sports have no free source here.
pub fn rows_for(fixtures: &[Fixture], football: &FootballIndex, tennis: &TennisIndex, data_day: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    for f in fixtures {
        if is_football(&f.sport) {
            rows.push(football_row(f, true, football, data_day));
            rows.push(football_row(f, false, football, data_day));
        } else if is_tennis(&f.sport) {
            rows.push(tennis_row(f, true, tennis, data_day));
            rows.push(tennis_row(f, false, tennis, data_day));
        }
    }
    rows
}

pub const FORMA_HEADING: &str = "## Forma reciente";

/// The "Forma reciente" block for `resumen-del-dia.md`: one `Forma:` line per match, with the source and data dates.
pub fn forma_section(rows: &[Row], day: &str) -> String {
    let mut out = format!("{FORMA_HEADING} (Football-Data.co.uk y TennisMyLife, descargado el {day})\n\n\
        Cada línea trae los últimos 5 resultados (el último carácter es el partido más reciente; W ganó, D empató, L perdió), goles a favor/en contra de esos 5, la forma de local o de visita y los puntos por partido de la temporada. \
        «sin datos» significa que el equipo o jugador no está en esas fuentes: no lo supongas, búscalo en la web. stats-hoy.csv trae las mismas cifras en columnas. Cita Football-Data o TennisMyLife como fuente de estas cifras.\n\n");
    let mut i = 0;
    while i < rows.len() {
        let (a, b) = (&rows[i], rows.get(i + 1).filter(|r| r.fixture_id == rows[i].fixture_id));
        i += if b.is_some() { 2 } else { 1 };
        let Some(b) = b else { continue };
        let source = [a, b].iter().filter(|r| r.found).map(|r| r.source.split(' ').next().unwrap_or("").to_string()).next();
        let label = match source.as_deref() { Some("Football-Data") => "Football-Data", Some(_) => "TennisMyLife", None => "" };
        let mut line = format!("- {}: {} vs {}. Forma: {} · {}", a.tournament, a.name, b.name, a.text, b.text);
        if !label.is_empty() {
            let dates: Vec<String> = [a, b].iter().filter(|r| r.found).filter_map(|r| parse_date(&r.last_match)).map(dmy).collect();
            line.push_str(&format!(", fuente {label} (descargado {day}; último partido registrado {})", dates.join(" y ")));
        }
        line.push('\n');
        out.push_str(&line);
    }
    out
}

/// Replaces (or adds) the "Forma reciente" block at the end of the summary; running twice never duplicates it.
pub fn apply_to_summary(md: &str, section: &str) -> String {
    let base = match md.find(&format!("\n{FORMA_HEADING}")) { Some(at) => &md[..at], None => md };
    format!("{}\n\n{section}", base.trim_end())
}

pub fn stats_csv(rows: &[Row]) -> String {
    let mut out = String::from("fixtureId,deporte,torneo,inicio_utc,lado,nombre,estado,motivo,coincide_con,fuente,fecha_descarga,ultimo_partido,forma5,gf5,gc5,pj5,condicion,forma_condicion,pts_condicion,pj_condicion,ppg_temporada,pj_temporada,superficie_ano,ultimo_torneo,ranking\n");
    for r in rows {
        let cells = [&r.fixture_id, &r.sport, &r.tournament, &r.start, &r.side, &r.name, &(if r.found { "ok".to_string() } else { "sin datos".to_string() }), &r.reason, &r.matched, &r.source,
            &r.data_day, &r.last_match, &r.form5, &r.gf5, &r.ga5, &r.n5, &r.cond, &r.cond_form, &r.cond_pts, &r.cond_n, &r.ppg, &r.played, &r.surfaces, &r.last_tourney, &r.rank];
        out.push_str(&cells.iter().map(|c| odds::csv_cell(c)).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    out
}

// ── Cache: one request per file per day ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Source {
    /// Local day (ISO) the file was last asked for; it is not asked for again that day.
    pub day: String,
    pub etag: String,
    pub modified: String,
    /// "ok" or "missing" (404).
    pub status: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Cache {
    pub sources: BTreeMap<String, Source>,
    pub football: BTreeMap<String, Vec<TeamForm>>,
    pub tennis: BTreeMap<String, Vec<TMatch>>,
    /// Day (ISO) every source needed for that day's fixtures was reached.
    #[serde(default)]
    pub done_day: String,
}

/// A file is requested only when it has not been asked for today.
pub fn needs_fetch(cache: &Cache, key: &str, day: &str) -> bool { cache.sources.get(key).is_none_or(|s| s.day != day) }

fn cache_path() -> PathBuf { settings::local_dir().join("odds").join("stats-cache.json") }
fn load_cache() -> Cache { std::fs::read_to_string(cache_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default() }
fn save_cache(cache: &Cache) {
    let _ = std::fs::create_dir_all(settings::local_dir().join("odds"));
    if let Ok(text) = serde_json::to_string(cache) { let _ = std::fs::write(cache_path(), text); }
}

enum Got { Body { text: String, etag: String, modified: String }, NotModified, Missing }

fn decode(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) { Ok(s) => s.to_string(), Err(_) => bytes.iter().map(|b| *b as char).collect() }
}

/// The only network code: a polite client (identified, redirects followed, a pause between requests, size capped).
struct Net { http: reqwest::Client, last: Option<Instant> }

impl Net {
    fn new() -> Result<Net, String> {
        let http = reqwest::Client::builder().user_agent(USER_AGENT).timeout(Duration::from_secs(60)).build().map_err(|_| "No se pudo preparar la conexión.".to_string())?;
        Ok(Net { http, last: None })
    }
    async fn get(&mut self, url: &str, prev: Option<&Source>) -> Result<Got, String> {
        if let Some(last) = self.last { if let Some(wait) = GAP.checked_sub(last.elapsed()) { tokio::time::sleep(wait).await; } }
        self.last = Some(Instant::now());
        let mut request = self.http.get(url).header("Accept", "text/csv,*/*");
        if let Some(p) = prev {
            if !p.etag.is_empty() { request = request.header("If-None-Match", &p.etag); }
            if !p.modified.is_empty() { request = request.header("If-Modified-Since", &p.modified); }
        }
        let response = request.send().await.map_err(|_| format!("Sin conexión con {}.", url.split('/').nth(2).unwrap_or("la fuente")))?;
        let status = response.status().as_u16();
        let header = |name: &str| response.headers().get(name).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        match status {
            304 => Ok(Got::NotModified),
            404 => Ok(Got::Missing),
            200 => {
                let (etag, modified) = (header("etag"), header("last-modified"));
                let bytes = response.bytes().await.map_err(|_| "La descarga se cortó.".to_string())?;
                if bytes.len() > MAX_BYTES { return Err("El archivo es más grande de lo esperado.".into()); }
                Ok(Got::Body { text: decode(&bytes), etag, modified })
            }
            other => Err(format!("{} respondió {other}.", url.split('/').nth(2).unwrap_or("La fuente"))),
        }
    }
}

fn mark(cache: &mut Cache, key: &str, day: &str, status: &str, etag: &str, modified: &str) {
    cache.sources.insert(key.to_string(), Source { day: day.into(), etag: etag.into(), modified: modified.into(), status: status.into() });
}

/// Refreshes one football file unless it was asked for today. True when the cache has data for it afterwards.
async fn refresh_football(net: &mut Net, cache: &mut Cache, key: &str, url: &str, league: &str, day: &str) -> Result<bool, String> {
    if !needs_fetch(cache, key, day) { return Ok(cache.football.contains_key(key)); }
    let prev = if cache.football.contains_key(key) { cache.sources.get(key).cloned() } else { None };
    match net.get(url, prev.as_ref()).await? {
        Got::NotModified => { let s = cache.sources.get(key).cloned().unwrap_or_default(); mark(cache, key, day, "ok", &s.etag, &s.modified); Ok(true) }
        Got::Missing => { cache.football.remove(key); mark(cache, key, day, "missing", "", ""); Ok(false) }
        Got::Body { text, etag, modified } => {
            let matches = parse_football(&text, league)?;
            if matches.is_empty() { cache.football.remove(key); mark(cache, key, day, "missing", "", ""); return Ok(false); }
            cache.football.insert(key.to_string(), football_forms(&matches, key));
            mark(cache, key, day, "ok", &etag, &modified);
            Ok(true)
        }
    }
}

async fn refresh_tennis(net: &mut Net, cache: &mut Cache, key: &str, file: &str, tour: &str, day: &str) -> Result<(), String> {
    if !needs_fetch(cache, key, day) { return Ok(()); }
    let prev = if cache.tennis.contains_key(key) { cache.sources.get(key).cloned() } else { None };
    match net.get(&format!("{TML_BASE}/{file}.csv"), prev.as_ref()).await? {
        Got::NotModified => { let s = cache.sources.get(key).cloned().unwrap_or_default(); mark(cache, key, day, "ok", &s.etag, &s.modified); }
        // No ongoing tournaments (or no file yet this year): nothing to add.
        Got::Missing => { cache.tennis.remove(key); mark(cache, key, day, "missing", "", ""); }
        Got::Body { text, etag, modified } => {
            cache.tennis.insert(key.to_string(), parse_tennis(&text, tour)?);
            mark(cache, key, day, "ok", &etag, &modified);
        }
    }
    Ok(())
}

/// Drops cached files that can no longer be used (an old season, an old year).
fn prune(cache: &mut Cache, season: &str, year: i64, month: i64) {
    let prev = previous_season(season);
    let keep_fb = |k: &str| !k.starts_with("fd:") || k.split(':').nth(2).is_none_or(|s| s == season || s == prev);
    cache.sources.retain(|k, _| keep_fb(k));
    cache.football.retain(|k, _| keep_fb(k));
    let years: Vec<String> = [year, year - 1].iter().map(|y| y.to_string()).collect();
    let keep_tn = |k: &str| !k.starts_with("tml:") || k.contains("ongoing") || years[..if month <= 2 { 2 } else { 1 }].iter().any(|y| k.starts_with(&format!("tml:{y}")));
    cache.sources.retain(|k, _| keep_tn(k));
    cache.tennis.retain(|k, _| keep_tn(k));
}

fn main_key(code: &str, season: &str) -> String { format!("fd:{code}:{season}") }

/// All the downloads of a day and the rows built from them. `Ok((rows, complete))`; incomplete when some file could
/// not be reached (it is asked for again later; the files already reached today are not).
async fn collect(fixtures: &[Fixture], cache: &mut Cache, now: i64, offset_min: i64) -> Result<(Vec<Row>, bool), String> {
    let today = (now + offset_min * 60).div_euclid(86_400);
    let (year, month, _) = civil_from_days(today);
    let (day, season) = (iso(today), season_code(year, month));
    prune(cache, &season, year, month);
    let mut net = Net::new()?;
    let mut complete = true;
    let mut note = |what: &str, e: String| { complete = false; super::log::line(format!("stats: {what}: {e}")); };

    let wants_football: Vec<&Fixture> = fixtures.iter().filter(|f| is_football(&f.sport) && !not_senior_men(&f.tournament)).collect();
    let wants_tennis = fixtures.iter().any(|f| is_tennis(&f.sport) && (!is_doubles(&f.home) || !is_doubles(&f.away)));

    let mut fresh: Vec<String> = Vec::new();
    if !wants_football.is_empty() {
        for (code, label) in MAIN_LEAGUES {
            let key = main_key(code, &season);
            match refresh_football(&mut net, cache, &key, &format!("{FD_BASE}/mmz4281/{season}/{code}.csv"), label, &day).await {
                Ok(true) => fresh.push(key),
                Ok(false) => {
                    // Early in a season the new folder may not exist yet: use the previous one.
                    let (pseason, pkey) = (previous_season(&season), main_key(code, &previous_season(&season)));
                    match refresh_football(&mut net, cache, &pkey, &format!("{FD_BASE}/mmz4281/{pseason}/{code}.csv"), label, &day).await {
                        Ok(true) => fresh.push(pkey),
                        Ok(false) => {}
                        Err(e) => note(code, e),
                    }
                }
                Err(e) => note(code, e),
            }
        }
        let unmatched = |cache: &Cache, fresh: &[String]| {
            let index = FootballIndex::build(fresh.iter().filter_map(|k| cache.football.get(k)));
            wants_football.iter().flat_map(|f| [&f.home, &f.away]).any(|n| matches!(index.lookup(n), Err(r) if r.starts_with("no está")))
        };
        if unmatched(cache, &fresh) {
            for country in EXTRA_COUNTRIES {
                let key = format!("fd:{country}");
                match refresh_football(&mut net, cache, &key, &format!("{FD_BASE}/new/{country}.csv"), country, &day).await {
                    Ok(true) => fresh.push(key),
                    Ok(false) => {}
                    Err(e) => note(country, e),
                }
                if !unmatched(cache, &fresh) { break; }
            }
        }
    }
    if wants_tennis {
        let mut files: Vec<(String, String, &str)> = Vec::new();
        let years: Vec<i64> = if month <= 2 { vec![year, year - 1] } else { vec![year] };
        for y in years {
            files.push((format!("tml:{y}"), format!("{y}"), "atp"));
            files.push((format!("tml:{y}_challenger"), format!("{y}_challenger"), "challenger"));
            files.push((format!("tml:{y}_wta"), format!("{y}_wta"), "wta"));
        }
        files.push(("tml:ongoing_tourneys".into(), "ongoing_tourneys".into(), "atp"));
        files.push(("tml:challenger_ongoing_tourneys".into(), "challenger_ongoing_tourneys".into(), "challenger"));
        files.push(("tml:wta_ongoing_tourneys".into(), "wta_ongoing_tourneys".into(), "wta"));
        for (key, file, tour) in &files {
            if let Err(e) = refresh_tennis(&mut net, cache, key, file, tour, &day).await { note(file, e); }
        }
    }

    let football = FootballIndex::build(fresh.iter().filter_map(|k| cache.football.get(k)));
    let all_tennis: Vec<TMatch> = cache.tennis.iter().filter(|(k, _)| cache.sources.get(*k).is_some_and(|s| s.day == day)).flat_map(|(_, v)| v.clone()).collect();
    let tennis = TennisIndex::build(&all_tennis, year);
    Ok((rows_for(fixtures, &football, &tennis, &day), complete))
}

// ── The daily step ───────────────────────────────────────────────────────────────────────────────

/// Today's fixtures, as the morning snapshot saved them (`odds/daily-fixtures.json`).
pub fn load_daily_fixtures(day: &str) -> Option<Vec<Fixture>> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(settings::local_dir().join("odds").join("daily-fixtures.json")).ok()?).ok()?;
    if v["day"].as_str() != Some(day) { return None; }
    let text = |x: &Value| x.as_str().unwrap_or("").to_string();
    Some(v["fixtures"].as_array()?.iter().map(|f| Fixture {
        id: text(&f["id"]), home: text(&f["home"]), away: text(&f["away"]), tournament: text(&f["tournament"]), tournament_id: 0,
        sport: text(&f["sport"]), start: f["start"].as_i64().unwrap_or(0), live: false,
    }).collect())
}

pub fn fixtures_json(day: &str, fixtures: &[Fixture]) -> Value {
    json!({ "day": day, "fixtures": fixtures.iter().map(|f| json!({ "id": f.id, "home": f.home, "away": f.away, "tournament": f.tournament, "sport": f.sport, "start": f.start })).collect::<Vec<_>>() })
}

static DONE: Mutex<String> = Mutex::new(String::new());
static TRIED: Mutex<Option<Instant>> = Mutex::new(None);

fn write_outputs(rows: &[Row], day: &str) {
    let agent_dir = super::named_agents::workspace("parley").join("odds");
    let _ = std::fs::create_dir_all(&agent_dir);
    let csv = format!("\u{feff}{}", stats_csv(rows));
    let section = forma_section(rows, day);
    for dir in [settings::local_dir().join("odds"), agent_dir] {
        let _ = std::fs::write(dir.join("stats-hoy.csv"), &csv);
        let summary = dir.join("resumen-del-dia.md");
        if let Ok(md) = std::fs::read_to_string(&summary) { let _ = std::fs::write(&summary, apply_to_summary(&md, &section)); }
    }
}

/// Called every minute by the morning loop once `useFreeStats` is on: does nothing when today's work is done, when
/// there is no fixture list for today yet, or within 15 minutes of a failed attempt.
pub async fn daily_step(now: i64, offset_min: i64) {
    let day = iso((now + offset_min * 60).div_euclid(86_400));
    if *DONE.lock().unwrap() == day { return; }
    let Some(fixtures) = load_daily_fixtures(&day) else { return };
    let mut cache = load_cache();
    if cache.done_day == day { *DONE.lock().unwrap() = day; return; }
    {
        let mut tried = TRIED.lock().unwrap();
        if tried.is_some_and(|at| at.elapsed() < RETRY) { return; }
        *tried = Some(Instant::now());
    }
    match collect(&fixtures, &mut cache, now, offset_min).await {
        Ok((rows, complete)) => {
            write_outputs(&rows, &day);
            if complete { cache.done_day = day.clone(); *DONE.lock().unwrap() = day; }
            save_cache(&cache);
        }
        Err(e) => { save_cache(&cache); super::log::line(format!("stats: {e}")); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fx(id: &str, sport: &str, tournament: &str, home: &str, away: &str) -> Fixture {
        Fixture { id: id.into(), home: home.into(), away: away.into(), tournament: tournament.into(), tournament_id: 1, sport: sport.into(), start: 1_790_000_000, live: false }
    }

    const MAIN: &str = "\u{feff}Div,Date,Time,HomeTeam,AwayTeam,FTHG,FTAG,FTR\r\n\
E0,21/08/2026,20:00,Arsenal,Coventry,3,0,H\r\n\
E0,22/08/2026,12:30,Hull,Man United,2,0,H\r\n\
E0,29/08/2026,15:00,Man United,Arsenal,1,1,D\r\n\
E0,05/09/2026,15:00,Arsenal,Hull,2,1,H\r\n\
E0,12/09/2026,15:00,Coventry,Arsenal,0,4,A\r\n\
E0,19/09/2026,15:00,Arsenal,\"Man United\",0,1,A\r\n\
E0,26/09/2026,15:00,Hull,Coventry,,,\r\n\
\r\n";

    #[test]
    fn csv_handles_bom_quotes_crlf_and_blank_lines() {
        let rows = parse_csv("\u{feff}a,b,c\r\n\"x, y\",\"he said \"\"hi\"\"\",z\r\n\r\n\"two\nlines\",,end");
        assert_eq!(rows[0], vec!["a", "b", "c"]);
        assert_eq!(rows[1], vec!["x, y", "he said \"hi\"", "z"]);
        assert_eq!(rows[2], vec!["two\nlines", "", "end"]);
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn missing_columns_and_short_rows_read_as_empty() {
        let t = Table::new("a,b\n1\n");
        assert_eq!(t.get(&t.rows[0], "b"), "");
        assert_eq!(t.get(&t.rows[0], "nope"), "");
        assert!(parse_football("x,y\n1,2\n", "L").is_err());
        assert!(parse_tennis("<html>not a csv</html>", "atp").is_err());
    }

    #[test]
    fn dates_in_every_layout_and_back() {
        assert_eq!(parse_date("21/08/2026"), parse_date("2026-08-21"));
        assert_eq!(parse_date("21/08/2026"), parse_date("20260821"));
        assert_eq!(parse_date("21/08/26"), parse_date("21/08/2026"));
        assert_eq!(parse_date("01/01/99"), parse_date("1999-01-01"));
        assert_eq!(parse_date("31/13/2026"), None);
        assert_eq!(parse_date("hola"), None);
        assert_eq!(iso(parse_date("2026-10-01").unwrap()), "2026-10-01");
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn season_codes_follow_the_july_start() {
        assert_eq!(season_code(2026, 10), "2627");
        assert_eq!(season_code(2026, 7), "2627");
        assert_eq!(season_code(2026, 6), "2526");
        assert_eq!(season_code(2027, 3), "2627");
        assert_eq!(season_code(2099, 12), "9900");
        assert_eq!(previous_season("2627"), "2526");
        assert_eq!(previous_season("0001"), "9900");
    }

    #[test]
    fn football_form_is_computed_from_played_matches_only() {
        let matches = parse_football(MAIN, "Premier League").unwrap();
        assert_eq!(matches.len(), 6, "the unplayed row is skipped");
        let forms = football_forms(&matches, "fd:E0:2627");
        let arsenal = forms.iter().find(|t| t.name == "Arsenal").unwrap();
        assert_eq!(arsenal.form5, "WDWWL");
        assert_eq!((arsenal.gf5, arsenal.ga5, arsenal.n5), (10, 3, 5), "last 5 of 5 played: 3-0, 1-1, 2-1, 4-0, 0-1");
        assert_eq!(arsenal.home, Split { form: "WWL".into(), pts: 6, n: 3 });
        assert_eq!(arsenal.away, Split { form: "DW".into(), pts: 4, n: 2 });
        assert_eq!((arsenal.played, arsenal.ppg), (5, 2.0));
        assert_eq!(arsenal.last, "2026-09-19");
        assert_eq!(arsenal.league, "Premier League");
        let united = forms.iter().find(|t| t.name == "Man United").unwrap();
        assert_eq!(united.form5, "LDW", "lost at Hull, drew with Arsenal... oldest to newest");
        assert_eq!(united.n5, 3);
    }

    #[test]
    fn only_the_last_five_count_and_the_season_label_limits_ppg() {
        let mut text = String::from("Country,League,Season,Date,Time,Home,Away,HG,AG,Res\n");
        for (i, (hg, ag)) in [(1, 0), (1, 0), (0, 1), (2, 2), (0, 3), (1, 0), (1, 0)].iter().enumerate() {
            let season = if i < 2 { "2025" } else { "2026" };
            text.push_str(&format!("Peru,Liga 1 ,{season},{:02}/0{}/2026,20:00,Alianza Lima,Cristal,{hg},{ag},X\n", i + 1, if i < 2 { 1 } else { 3 }));
        }
        let m = parse_football(&text, "x").unwrap();
        assert_eq!(m[0].league, "Peru Liga 1");
        let a = football_forms(&m, "fd:PER").into_iter().find(|t| t.name == "Alianza Lima").unwrap();
        assert_eq!(a.form5, "LDLWW");
        assert_eq!(a.n5, 5);
        assert_eq!(a.played, 5, "only the matches of the latest season label");
        assert_eq!(a.ppg, 1.4);
    }

    #[test]
    fn names_are_normalised_and_aliases_converge() {
        assert_eq!(team_key("Man Utd"), team_key("Manchester United"));
        assert_eq!(team_key("Man United"), team_key("manchester united fc"));
        assert_eq!(team_key("Nott'm Forest"), team_key("Nottingham Forest"));
        assert_eq!(team_key("1. FC Köln"), team_key("FC Koln"));
        assert_eq!(team_key("CD Godoy Cruz"), team_key("Godoy Cruz"));
        assert_eq!(team_key("Atlético Madrid"), team_key("Ath Madrid"));
        assert_eq!(team_key("Borussia Mönchengladbach"), team_key("M'gladbach"));
        assert_eq!(team_key("Sporting CP"), team_key("Sp Lisbon"));
        assert_eq!(team_key("Club América"), team_key("America"));
        assert_eq!(team_key("Paris Saint-Germain"), team_key("Paris SG"));
        assert_ne!(team_key("Sporting Cristal"), team_key("Sporting"));
        assert_ne!(team_key("Arsenal"), team_key("Arsenal Sarandi"));
        assert_ne!(team_key("Real Madrid"), team_key("Madrid"));
        assert_eq!(fold("  Ünited & Sons-FC! "), "united and sons fc");
    }

    fn england() -> FootballIndex {
        let forms = football_forms(&parse_football(MAIN, "Premier League").unwrap(), "fd:E0:2627");
        let mut peru = vec![TeamForm { name: "Alianza Lima".into(), source: "fd:PER".into(), form5: "WWDLW".into(), gf5: 7, ga5: 3, n5: 5, ppg: 1.6, played: 5, last: "2026-09-28".into(), ..TeamForm::default() }];
        peru.push(TeamForm { name: "Arsenal".into(), source: "fd:ARG".into(), ..TeamForm::default() });
        FootballIndex::build([&forms, &peru])
    }

    #[test]
    fn an_unknown_team_is_reported_never_guessed() {
        let rows = rows_for(&[fx("a", "Fútbol", "Liga 1", "Sporting Cristal", "Sport Huancayo")], &england(), &TennisIndex { players: vec![] }, "2026-10-01");
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| !r.found && r.text.contains("sin datos")));
        let md = forma_section(&rows, "2026-10-01");
        assert!(md.contains("Sporting Cristal sin datos") && md.contains("Sport Huancayo sin datos"));
        assert!(!md.contains(", fuente "), "no source is cited for figures that do not exist");
    }

    #[test]
    fn two_teams_with_one_name_in_different_files_are_ambiguous() {
        let index = england();
        let e = index.lookup("Arsenal").unwrap_err();
        assert!(e.contains("ambiguo"), "{e}");
        assert!(index.lookup("Hull").is_ok());
    }

    #[test]
    fn women_youth_and_reserve_tournaments_never_match_first_teams() {
        let rows = rows_for(&[fx("a", "Fútbol", "Primera LPF, Reserves", "Hull", "Coventry"), fx("b", "Fútbol", "Women's Super League", "Hull", "Coventry")], &england(), &TennisIndex { players: vec![] }, "d");
        assert!(rows.iter().all(|r| !r.found && r.reason.contains("juvenil")));
    }

    #[test]
    fn a_football_line_carries_form_home_or_away_split_and_dates() {
        let rows = rows_for(&[fx("a", "Fútbol", "Premier League", "Manchester United", "Alianza Lima")], &england(), &TennisIndex { players: vec![] }, "2026-10-01");
        assert!(rows[0].found && rows[1].found);
        assert_eq!(rows[0].matched, "Man United");
        assert_eq!(rows[0].cond, "local");
        assert_eq!(rows[1].cond, "visita");
        let md = forma_section(&rows, "2026-10-01");
        assert!(md.contains("Forma: Manchester United LDW (2 GF/3 GC en 3 PJ"), "{md}");
        assert!(md.contains("local 1 pts en 1 PJ"), "{md}");
        assert!(md.contains("Alianza Lima WWDLW (7 GF/3 GC en 5 PJ"), "{md}");
        assert!(md.contains("fuente Football-Data (descargado 2026-10-01; último partido registrado"), "{md}");
        let csv = stats_csv(&rows);
        assert!(csv.starts_with("fixtureId,deporte,torneo"));
        assert_eq!(csv.lines().count(), 3);
        assert!(csv.contains("\"ok\""));
    }

    #[test]
    fn the_forma_block_replaces_itself() {
        let rows = rows_for(&[fx("a", "Fútbol", "Premier League", "Hull", "Coventry")], &england(), &TennisIndex { players: vec![] }, "2026-10-01");
        let section = forma_section(&rows, "2026-10-01");
        let once = apply_to_summary("# Partidos de hoy\n\nTexto.\n", &section);
        let twice = apply_to_summary(&once, &section);
        assert_eq!(once, twice);
        assert!(twice.starts_with("# Partidos de hoy\n\nTexto.\n\n## Forma reciente"));
        assert_eq!(twice.matches(FORMA_HEADING).count(), 1);
    }

    const TENNIS: &str = "tourney_id,tourney_name,surface,draw_size,tourney_level,indoor,tourney_date,match_num,winner_id,winner_seed,winner_entry,winner_name,winner_hand,winner_ht,winner_ioc,winner_age,winner_rank,winner_rank_points,loser_id,loser_seed,loser_entry,loser_name,loser_hand,loser_ht,loser_ioc,loser_age,loser_rank,loser_rank_points,score,best_of,round,minutes\n\
2026-1,Brisbane,Hard,32,250,O,20260105,10,DH50,,,Alejandro Davidovich Fokina,R,180,ESP,27,29,1680,HB71,,,Hubert Hurkacz,R,196,POL,29,46,1120,6-4 7-6(7),3,R32,100\n\
2026-1,Brisbane,Hard,32,250,O,20260105,11,HB71,,,Hubert Hurkacz,R,196,POL,29,46,1120,AA11,,,Andy Alpha,R,180,USA,25,90,500,6-3 6-3,3,R16,80\n\
2026-2,Madrid,Clay,32,1000,O,20260420,5,HB71,,,Hubert Hurkacz,R,196,POL,29,45,1120,DH50,,,Alejandro Davidovich Fokina,R,180,ESP,27,29,1680,6-4 6-4,3,R64,90\n\
2026-2,Madrid,Clay,32,1000,O,20260420,6,HB71,,,Hubert Hurkacz,R,196,POL,29,45,1120,CC33,,,Carl Charlie,R,180,USA,25,90,500,W/O,3,R32,0\n\
2026-3,Beijing,Hard,32,500,O,20260929,14,T0HA,,,Learner Tien,L,180,USA,20,15,2395,HB71,,,Hubert Hurkacz,R,196,POL,29,41,1220,5-7 6-3 6-4,3,R32,117\n\
2026-3,Beijing,Hard,32,500,O,20260929,15,ZZ01,,,Wei Zhang,R,180,CHN,20,100,100,ZZ02,,,Zhizhen Zhang,R,180,CHN,20,60,100,6-1 6-1,3,R32,60\n\
2025-9,Old,Hard,32,500,O,20251001,15,HB71,,,Hubert Hurkacz,R,196,POL,29,41,1220,ZZ01,,,Wei Zhang,R,180,CHN,20,60,100,6-1 6-1,3,R32,60\n";

    fn tennis() -> TennisIndex { TennisIndex::build(&parse_tennis(TENNIS, "atp").unwrap(), 2026) }

    #[test]
    fn tennis_form_surface_record_and_last_tournament() {
        let index = tennis();
        let p = index.lookup("Hurkacz H.", None).unwrap();
        assert_eq!(p.name, "Hubert Hurkacz");
        assert_eq!(p.form5, "WLWWL", "oldest to newest, 2025 included; the walkover is not a match");
        assert_eq!(p.surfaces, vec![("Clay".to_string(), 1, 0), ("Hard".to_string(), 1, 2)], "this year only");
        assert_eq!((p.last_tourney.as_str(), p.last_round.as_str(), p.last_won, p.last.as_str()), ("Beijing", "R32", false, "2026-09-29"));
        assert_eq!(p.rank, "41");
    }

    #[test]
    fn tennis_names_match_by_initials_or_full_name_and_never_when_ambiguous() {
        let index = tennis();
        assert_eq!(index.lookup("Davidovich Fokina A.", None).unwrap().id, "DH50");
        assert_eq!(index.lookup("Alejandro Davidovich Fokina", None).unwrap().id, "DH50");
        assert_eq!(index.lookup("Tien L", None).unwrap().id, "T0HA");
        assert!(index.lookup("Zhang Z.", None).unwrap().id == "ZZ02", "Zhang Z. is Zhizhen; Wei is W.");
        assert!(index.lookup("Hurkacz K.", None).is_err(), "wrong initial");
        assert!(index.lookup("Hurkacz", None).is_err(), "a surname alone is not a name");
        let two = TennisIndex::build(&parse_tennis(&format!("{TENNIS}2026-3,Beijing,Hard,32,500,O,20260929,16,ZY01,,,Zeke Zhang,R,180,CHN,20,100,100,ZZ02,,,Zhizhen Zhang,R,180,CHN,20,60,100,6-1 6-1,3,R16,60\n"), "atp").unwrap(), 2026);
        let e = two.lookup("Zhang Z.", None).unwrap_err();
        assert!(e.contains("ambiguo"), "{e}");
    }

    #[test]
    fn tennis_fixture_rows_and_doubles() {
        let index = tennis();
        let rows = rows_for(&[fx("t", "Tenis", "ATP Beijing", "Hurkacz H.", "Nadie X."), fx("d", "Tenis", "ATP Challenger Curitiba Dobles", "A B / C D", "E F / G H")], &FootballIndex::default(), &index, "2026-10-01");
        assert!(rows[0].found && !rows[1].found);
        assert!(rows[0].text.starts_with("Hurkacz H. WLWWL"), "{}", rows[0].text);
        assert!(rows[0].text.contains("este año") && rows[0].text.contains("Beijing") && rows[0].text.contains("ranking 41"), "{}", rows[0].text);
        assert!(rows[1].text.contains("sin datos"));
        assert!(rows[2].reason.contains("dobles"));
        let md = forma_section(&rows, "2026-10-01");
        assert!(md.contains("fuente TennisMyLife (descargado 2026-10-01"), "{md}");
    }

    #[test]
    fn women_and_men_with_the_same_name_are_told_apart_by_the_tournament_only() {
        let mut all = parse_tennis(TENNIS, "atp").unwrap();
        all.extend(parse_tennis(&TENNIS.replace("DH50", "100001").replace("HB71", "100002").replace("Beijing", "Beijing W"), "wta").unwrap());
        let index = TennisIndex::build(&all, 2026);
                assert_eq!(index.lookup("Hurkacz H.", Some(false)).unwrap().tour, "atp");
        assert_eq!(index.lookup("Hurkacz H.", Some(true)).unwrap().tour, "wta");
        assert!(index.lookup("Hurkacz H.", None).unwrap_err().contains("ambiguo"));
        assert_eq!(tour_hint("WTA 1000 Beijing"), Some(true));
        assert_eq!(tour_hint("ATP Challenger Curitiba"), Some(false));
        assert_eq!(tour_hint("Roland Garros"), None);
    }

    #[test]
    fn a_file_is_requested_once_per_day() {
        let mut cache = Cache::default();
        assert!(needs_fetch(&cache, "fd:E0:2627", "2026-10-01"));
        mark(&mut cache, "fd:E0:2627", "2026-10-01", "ok", "\"abc\"", "Mon, 21 Sep 2026 17:50:49 GMT");
        assert!(!needs_fetch(&cache, "fd:E0:2627", "2026-10-01"));
        assert!(needs_fetch(&cache, "fd:E0:2627", "2026-10-02"));
        mark(&mut cache, "fd:ARG", "2026-10-01", "missing", "", "");
        assert!(!needs_fetch(&cache, "fd:ARG", "2026-10-01"), "a 404 is also remembered for the day");
        let json = serde_json::to_string(&cache).unwrap();
        let back: Cache = serde_json::from_str(&json).unwrap();
        assert_eq!(back.sources["fd:E0:2627"].etag, "\"abc\"");
        assert!(!needs_fetch(&back, "fd:E0:2627", "2026-10-01"));
    }

    #[test]
    fn the_cache_forgets_old_seasons_and_years() {
        let mut cache = Cache::default();
        for k in ["fd:E0:2425", "fd:E0:2526", "fd:E0:2627", "fd:ARG", "tml:2024", "tml:2025", "tml:2026", "tml:wta_ongoing_tourneys"] { mark(&mut cache, k, "d", "ok", "", ""); }
        prune(&mut cache, "2627", 2026, 10);
        let keys: Vec<&str> = cache.sources.keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["fd:ARG", "fd:E0:2526", "fd:E0:2627", "tml:2026", "tml:wta_ongoing_tourneys"]);
    }

    #[test]
    fn tennis_matches_survive_the_cache_as_arrays() {
        let m = parse_tennis(TENNIS, "atp").unwrap();
        let back: Vec<TMatch> = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(m, back);
        assert!(serde_json::to_string(&m[0]).unwrap().starts_with('['));
    }

    #[test]
    fn the_daily_fixture_list_round_trips() {
        let f = vec![fx("a", "Fútbol", "Liga", "A", "B")];
        let v = fixtures_json("2026-10-01", &f);
        assert_eq!(v["day"], "2026-10-01");
        assert_eq!(v["fixtures"][0]["home"], "A");
    }

    /// One real download per source (a few MB). `cargo test -p mika a_real_download -- --ignored --nocapture`
    #[test]
    #[ignore = "uses the network"]
    fn a_real_download() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let mut net = Net::new().unwrap();
            let mut cache = Cache::default();
            let season = season_code(2026, 10);
            let ok = refresh_football(&mut net, &mut cache, "fd:E0", &format!("{FD_BASE}/mmz4281/{season}/E0.csv"), "Premier League", "today").await.unwrap();
            assert!(ok && !cache.football["fd:E0"].is_empty());
            refresh_tennis(&mut net, &mut cache, "tml:ongoing_tourneys", "ongoing_tourneys", "atp", "today").await.unwrap();
            let index = FootballIndex::build([&cache.football["fd:E0"]]);
            let team = index.lookup("Manchester United").expect("Man United is in the Premier League file");
            println!("{} teams, {} tennis matches; {} {} ppg {}", cache.football["fd:E0"].len(), cache.tennis.get("tml:ongoing_tourneys").map_or(0, Vec::len), team.name, team.form5, team.ppg);
            let tennis = TennisIndex::build(&cache.tennis["tml:ongoing_tourneys"], 2026);
            println!("{} players", tennis.players.len());
        });
    }
}
