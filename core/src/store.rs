//! The SQLite base in the app's data folder, with versioned migrations (`PRAGMA user_version`).
//!
//! Each migration runs once, in order, inside a transaction. Never edit a shipped migration: add the next one.
//! Big files (attachments, screenshots, HTML pages) live in folders next to the base, never inside it.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, params};

use crate::CoreError;

/// Index + 1 is the schema version it produces.
const MIGRATIONS: &[&str] = &[
    // v1: settings (key/value). Chats, messages, memory, picks, stats and usage arrive with their phases.
    "CREATE TABLE settings (
        key        TEXT PRIMARY KEY,
        value      TEXT NOT NULL,
        updated_at INTEGER NOT NULL DEFAULT (unixepoch())
    );",
    // v2: chats with Buddy and its specialists, and each provider's conversation id to continue it.
    "CREATE TABLE chats (
        id         TEXT PRIMARY KEY,
        title      TEXT NOT NULL,
        created_at INTEGER NOT NULL DEFAULT (unixepoch()),
        updated_at INTEGER NOT NULL DEFAULT (unixepoch())
    );
    CREATE TABLE messages (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        chat_id    TEXT NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
        role       TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
        agent      TEXT NOT NULL,
        provider   TEXT,
        text       TEXT NOT NULL,
        sources    TEXT NOT NULL DEFAULT '[]',
        failed     INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL DEFAULT (unixepoch())
    );
    CREATE INDEX messages_by_chat ON messages(chat_id, id);
    CREATE TABLE provider_sessions (
        chat_id    TEXT NOT NULL REFERENCES chats(id) ON DELETE CASCADE,
        agent      TEXT NOT NULL,
        provider   TEXT NOT NULL,
        session_id TEXT NOT NULL,
        PRIMARY KEY (chat_id, agent, provider)
    );",
    // v3: files attached to a message (paths of Buddy's own copies).
    "ALTER TABLE messages ADD COLUMN attachments TEXT NOT NULL DEFAULT '[]';",
    // v4: the token meter, one row per turn and feature.
    "CREATE TABLE token_events (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        at         INTEGER NOT NULL DEFAULT (unixepoch()),
        feature    TEXT NOT NULL,
        provider   TEXT NOT NULL,
        model      TEXT,
        input      INTEGER NOT NULL,
        output     INTEGER NOT NULL,
        cached     INTEGER NOT NULL,
        cost_usd   REAL
    );
    CREATE INDEX token_events_by_time ON token_events(at);",
    // v5: the briefing («mensajitos»).
    "CREATE TABLE briefing_items (
        id    INTEGER PRIMARY KEY AUTOINCREMENT,
        at    INTEGER NOT NULL,
        topic TEXT NOT NULL,
        text  TEXT NOT NULL,
        url   TEXT
    );",
    // v6: which model wrote each answer («Opus 5.5 · esfuerzo alto»), for its tooltip.
    "ALTER TABLE messages ADD COLUMN model TEXT;",
];

/// Tokens spent by one feature (and provider) over a period.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct TokenReport {
    pub feature: String,
    pub provider: String,
    pub turns: i64,
    pub input: i64,
    pub output: i64,
    pub cached: i64,
    /// What it would have cost on the API (subscriptions pay nothing extra), when known.
    pub cost_usd: f64,
}

/// Recorded activity per local calendar day and provider, for the usage calendar (no invented empty-day data).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct TokenDay {
    pub date: String,
    pub provider: String,
    pub turns: i64,
    pub input: i64,
    pub output: i64,
    pub cached: i64,
}

/// A web page an answer used.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SourceLink {
    pub title: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ChatSummary {
    pub id: String,
    pub title: String,
    pub updated_at: i64,
    /// The start of the last message.
    pub preview: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct ChatMessage {
    pub id: i64,
    pub role: String,
    pub agent: String,
    pub provider: Option<String>,
    pub text: String,
    pub sources: Vec<SourceLink>,
    pub failed: bool,
    pub created_at: i64,
    /// Buddy's copies of the files attached to this message.
    pub attachments: Vec<String>,
    /// The model that wrote an answer, in words («Opus 5.5 · esfuerzo alto»); None for older ones.
    pub model: Option<String>,
}

/// What a new message carries.
pub struct NewMessage<'a> {
    pub chat_id: &'a str,
    pub role: &'a str,
    pub agent: &'a str,
    pub provider: Option<&'a str>,
    pub text: &'a str,
    pub sources: &'a [SourceLink],
    pub failed: bool,
    pub attachments: &'a [String],
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, CoreError> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self, CoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, CoreError> {
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 3000;")?;
        let mut store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<(), CoreError> {
        let current = self.schema_version()? as usize;
        if current > MIGRATIONS.len() {
            return Err(CoreError::Store(format!(
                "la base es de una versión más nueva de Buddy (v{current}, esta app conoce v{})",
                MIGRATIONS.len()
            )));
        }
        for (index, sql) in MIGRATIONS.iter().enumerate().skip(current) {
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", (index + 1) as i64)?;
            tx.commit()?;
        }
        Ok(())
    }

    pub fn schema_version(&self) -> Result<u32, CoreError> {
        Ok(self.conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))? as u32)
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>, CoreError> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |row| row.get(0))
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = unixepoch()",
            params![key, value],
        )?;
        Ok(())
    }
}

/// A one-line preview without Markdown marks or Buddy's hand-off line.
pub fn plain_preview(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let mut line = line.trim();
        if line.starts_with("[[pasar:") {
            line = line.find("]]").map_or("", |i| line[i + 2..].trim());
        }
        let line = line.trim_start_matches(['#', '>', '-', '*', ' ']);
        if line.is_empty() || line.starts_with("```") || line.starts_with('|') {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.extend(line.chars().filter(|c| !matches!(c, '*' | '`' | '_')));
        if out.chars().count() >= 120 {
            break;
        }
    }
    out.chars().take(120).collect()
}

/// Lower case without accents (Spanish and the usual Latin letters), for matching what the user types.
pub fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'ä' | 'â' | 'ã' => 'a',
            'é' | 'è' | 'ë' | 'ê' => 'e',
            'í' | 'ì' | 'ï' | 'î' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
            'ú' | 'ù' | 'ü' | 'û' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

impl Store {
    /// Creates the chat if it does not exist yet.
    pub fn ensure_chat(&self, id: &str, title: &str) -> Result<(), CoreError> {
        self.conn.execute("INSERT OR IGNORE INTO chats (id, title) VALUES (?1, ?2)", params![id, title])?;
        Ok(())
    }

    pub fn add_message(&self, m: NewMessage<'_>) -> Result<i64, CoreError> {
        let sources = serde_json::to_string(m.sources).unwrap_or_else(|_| "[]".into());
        let attachments = serde_json::to_string(m.attachments).unwrap_or_else(|_| "[]".into());
        self.conn.execute(
            "INSERT INTO messages (chat_id, role, agent, provider, text, sources, failed, attachments)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![m.chat_id, m.role, m.agent, m.provider, m.text, sources, m.failed, attachments],
        )?;
        let id = self.conn.last_insert_rowid();
        self.conn.execute("UPDATE chats SET updated_at = unixepoch() WHERE id = ?1", params![m.chat_id])?;
        Ok(id)
    }

    /// Notes which model wrote a saved answer.
    pub fn set_message_model(&self, message_id: i64, model: &str) -> Result<(), CoreError> {
        self.conn.execute("UPDATE messages SET model = ?2 WHERE id = ?1", params![message_id, model])?;
        Ok(())
    }

    pub fn chats(&self, limit: u32) -> Result<Vec<ChatSummary>, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.title, c.updated_at,
                    COALESCE((SELECT substr(text, 1, 600) FROM messages m WHERE m.chat_id = c.id ORDER BY m.id DESC LIMIT 1), '')
             FROM chats c ORDER BY c.updated_at DESC, c.rowid DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |r| {
            Ok(ChatSummary { id: r.get(0)?, title: r.get(1)?, updated_at: r.get(2)?, preview: plain_preview(&r.get::<_, String>(3)?) })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn messages(&self, chat_id: &str) -> Result<Vec<ChatMessage>, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, role, agent, provider, text, sources, failed, created_at, attachments, model FROM messages WHERE chat_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![chat_id], |r| {
            let sources: String = r.get(5)?;
            Ok(ChatMessage {
                id: r.get(0)?,
                role: r.get(1)?,
                agent: r.get(2)?,
                provider: r.get(3)?,
                text: r.get(4)?,
                sources: serde_json::from_str(&sources).unwrap_or_default(),
                failed: r.get(6)?,
                created_at: r.get(7)?,
                attachments: serde_json::from_str(&r.get::<_, String>(8)?).unwrap_or_default(),
                model: r.get(9)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn record_tokens(
        &self,
        feature: &str,
        provider: &str,
        t: &crate::providers::TokenCount,
    ) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT INTO token_events (feature, provider, model, input, output, cached, cost_usd) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![feature, provider, t.model, t.input, t.output, t.cached, t.cost_usd],
        )?;
        Ok(())
    }

    pub fn add_briefing_items(&self, items: &[crate::briefing::BriefingItem]) -> Result<(), CoreError> {
        for i in items {
            self.conn.execute(
                "INSERT INTO briefing_items (at, topic, text, url) VALUES (?1, ?2, ?3, ?4)",
                params![i.at, i.topic, i.text, i.url],
            )?;
        }
        Ok(())
    }

    /// Briefing lines of the last `seconds`, newest first.
    pub fn briefing_items(&self, seconds: i64) -> Result<Vec<crate::briefing::BriefingItem>, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT topic, text, url, at FROM briefing_items WHERE at >= unixepoch() - ?1 ORDER BY at DESC, id ASC LIMIT 30",
        )?;
        let rows = stmt.query_map(params![seconds], |r| {
            Ok(crate::briefing::BriefingItem { topic: r.get(0)?, text: r.get(1)?, url: r.get(2)?, at: r.get(3)? })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Tokens per feature and provider over the last `days` days, the biggest first.
    pub fn token_report(&self, days: u32) -> Result<Vec<TokenReport>, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT feature, provider, COUNT(*), SUM(input), SUM(output), SUM(cached), COALESCE(SUM(cost_usd), 0)
             FROM token_events WHERE at >= unixepoch() - ?1 * 86400
             GROUP BY feature, provider ORDER BY SUM(input) + SUM(output) DESC",
        )?;
        let rows = stmt.query_map(params![days], |r| {
            Ok(TokenReport {
                feature: r.get(0)?,
                provider: r.get(1)?,
                turns: r.get(2)?,
                input: r.get(3)?,
                output: r.get(4)?,
                cached: r.get(5)?,
                cost_usd: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn token_activity(&self, days: u32) -> Result<Vec<TokenDay>, CoreError> {
        let since = format!("-{} days", days.clamp(1, 371) - 1);
        let mut stmt = self.conn.prepare(
            "SELECT date(at, 'unixepoch', 'localtime'), provider, COUNT(*), SUM(input), SUM(output), SUM(cached)
             FROM token_events WHERE at >= unixepoch('now', 'localtime', 'start of day', ?1, 'utc') AND at <= unixepoch()
             GROUP BY date(at, 'unixepoch', 'localtime'), provider ORDER BY 1, 2",
        )?;
        let rows = stmt.query_map(params![since], |r| Ok(TokenDay {
            date: r.get(0)?, provider: r.get(1)?, turns: r.get(2)?, input: r.get(3)?, output: r.get(4)?, cached: r.get(5)?,
        }))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Removes the message `from_id` and every later one of the chat (to write an answer again).
    pub fn delete_messages_from(&self, chat_id: &str, from_id: i64) -> Result<(), CoreError> {
        self.conn.execute("DELETE FROM messages WHERE chat_id = ?1 AND id >= ?2", params![chat_id, from_id])?;
        Ok(())
    }

    /// Chats whose title or messages contain every word of `query` (any order, ignoring case and accents), newest
    /// first. An empty query lists the latest ones.
    pub fn search_chats(&self, query: &str, limit: u32) -> Result<Vec<ChatSummary>, CoreError> {
        let words: Vec<String> = fold(query).split_whitespace().map(str::to_string).collect();
        if words.is_empty() {
            return self.chats(limit);
        }
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.title, c.updated_at,
                    COALESCE((SELECT substr(text, 1, 600) FROM messages m WHERE m.chat_id = c.id ORDER BY m.id DESC LIMIT 1), ''),
                    COALESCE((SELECT group_concat(substr(text, 1, 4000), ' ') FROM messages m WHERE m.chat_id = c.id), '')
             FROM chats c ORDER BY c.updated_at DESC, c.rowid DESC LIMIT 2000",
        )?;
        let rows = stmt.query_map([], |r| {
            let preview = plain_preview(&r.get::<_, String>(3)?);
            Ok((ChatSummary { id: r.get(0)?, title: r.get(1)?, updated_at: r.get(2)?, preview }, r.get::<_, String>(4)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (summary, text) = row?;
            let haystack = fold(&format!("{} {}", summary.title, text));
            if words.iter().all(|w| haystack.contains(w.as_str())) {
                out.push(summary);
                if out.len() >= limit as usize {
                    break;
                }
            }
        }
        Ok(out)
    }

    pub fn delete_chat(&self, chat_id: &str) -> Result<(), CoreError> {
        self.conn.execute("DELETE FROM chats WHERE id = ?1", params![chat_id])?;
        Ok(())
    }

    pub fn session(&self, chat_id: &str, agent: &str, provider: &str) -> Result<Option<String>, CoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT session_id FROM provider_sessions WHERE chat_id = ?1 AND agent = ?2 AND provider = ?3",
                params![chat_id, agent, provider],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn set_session(&self, chat_id: &str, agent: &str, provider: &str, session: &str) -> Result<(), CoreError> {
        self.conn.execute(
            "INSERT INTO provider_sessions (chat_id, agent, provider, session_id) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(chat_id, agent, provider) DO UPDATE SET session_id = excluded.session_id",
            params![chat_id, agent, provider, session],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_base_reaches_the_latest_version() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.schema_version().unwrap() as usize, MIGRATIONS.len());
    }

    #[test]
    fn reopening_does_not_rerun_migrations_and_keeps_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("buddy.sqlite");
        Store::open(&path).unwrap().set_setting("a", "1").unwrap();
        let again = Store::open(&path).unwrap();
        assert_eq!(again.schema_version().unwrap() as usize, MIGRATIONS.len());
        assert_eq!(again.setting("a").unwrap().as_deref(), Some("1"));
    }

    #[test]
    fn a_base_from_a_newer_buddy_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("buddy.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 999).unwrap();
        drop(conn);
        assert!(matches!(Store::open(&path), Err(CoreError::Store(_))));
    }

    #[test]
    fn settings_overwrite() {
        let store = Store::open_in_memory().unwrap();
        store.set_setting("k", "1").unwrap();
        store.set_setting("k", "2").unwrap();
        assert_eq!(store.setting("k").unwrap().as_deref(), Some("2"));
        assert_eq!(store.setting("missing").unwrap(), None);
    }

    #[test]
    fn chats_keep_messages_sources_and_sessions() {
        let store = Store::open_in_memory().unwrap();
        store.ensure_chat("c1", "Clima").unwrap();
        store.ensure_chat("c1", "otro título").unwrap();
        let src = [SourceLink { title: "SENAMHI".into(), url: "https://senamhi.gob.pe".into() }];
        let msg = |role, text, sources: &'static [SourceLink]| NewMessage {
            chat_id: "c1", role, agent: "buddy", provider: Some("claude"), text, sources, failed: false, attachments: &[],
        };
        store.add_message(msg("user", "¿Llueve?", &[])).unwrap();
        let src: &'static [SourceLink] = Box::leak(Box::new(src));
        store.add_message(msg("assistant", "No.", src)).unwrap();
        let chats = store.chats(10).unwrap();
        assert_eq!((chats[0].title.as_str(), chats[0].preview.as_str()), ("Clima", "No."));
        let messages = store.messages("c1").unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].sources[0].url, "https://senamhi.gob.pe");
        store.set_session("c1", "buddy", "claude", "s1").unwrap();
        store.set_session("c1", "buddy", "claude", "s2").unwrap();
        assert_eq!(store.session("c1", "buddy", "claude").unwrap().as_deref(), Some("s2"));
        store.delete_chat("c1").unwrap();
        assert!(store.messages("c1").unwrap().is_empty());
        assert_eq!(store.session("c1", "buddy", "claude").unwrap(), None);
    }

    #[test]
    fn search_ignores_case_accents_and_order() {
        let store = Store::open_in_memory().unwrap();
        for (id, title, text) in [("a", "Clásico", "¿Quién gana el Madrid?"), ("b", "Clima", "Lluvia en Lima")] {
            store.ensure_chat(id, title).unwrap();
            store.add_message(NewMessage { chat_id: id, role: "user", agent: "buddy", provider: None, text, sources: &[], failed: false, attachments: &[] }).unwrap();
        }
        let ids = |q: &str| store.search_chats(q, 10).unwrap().into_iter().map(|c| c.id).collect::<Vec<_>>();
        assert_eq!(ids("CLASICO"), ["a"]);
        assert_eq!(ids("madrid quien"), ["a"]);
        assert_eq!(ids("lima"), ["b"]);
        assert!(ids("tenis").is_empty());
        assert_eq!(ids("  ").len(), 2);
        store.delete_messages_from("a", 1).unwrap();
        assert!(store.messages("a").unwrap().is_empty());
    }

    #[test]
    fn previews_are_plain_text() {
        assert_eq!(plain_preview("## Hola\n\n1. **Usa `let`** por defecto"), "Hola 1. Usa let por defecto");
        assert_eq!(plain_preview("[[pasar:parley]] Analiza"), "Analiza");
        assert_eq!(plain_preview("| a | b |\n|---|---|\nTexto"), "Texto");
    }

    #[test]
    fn the_token_meter_adds_up_by_feature() {
        let store = Store::open_in_memory().unwrap();
        let t = |i, o| crate::providers::TokenCount { input: i, output: o, cached: 10, cost_usd: Some(0.01), model: None };
        store.record_tokens("chat · Buddy", "claude", &t(100, 20)).unwrap();
        store.record_tokens("chat · Buddy", "claude", &t(50, 5)).unwrap();
        store.record_tokens("uso de planes", "claude", &t(30, 1)).unwrap();
        let report = store.token_report(7).unwrap();
        assert_eq!((report[0].feature.as_str(), report[0].turns, report[0].input, report[0].output), ("chat · Buddy", 2, 150, 25));
        assert!((report[0].cost_usd - 0.02).abs() < 1e-9);
        assert_eq!(report.len(), 2);
    }

    #[test]
    fn calendar_activity_aggregates_local_days_and_keeps_providers_separate() {
        let store = Store::open_in_memory().unwrap();
        let t = crate::providers::TokenCount { input: 100, output: 20, cached: 50, ..Default::default() };
        store.record_tokens("chat", "claude", &t).unwrap();
        store.record_tokens("briefing", "claude", &t).unwrap();
        store.record_tokens("chat", "codex", &t).unwrap();
        store.conn.execute(
            "INSERT INTO token_events (at, feature, provider, input, output, cached) VALUES
             (unixepoch('now', '-2 days'), 'chat', 'claude', 10, 2, 3),
             (unixepoch('now', '-400 days'), 'chat', 'claude', 900, 900, 900)", [],
        ).unwrap();
        let today: String = store.conn.query_row("SELECT date('now', 'localtime')", [], |r| r.get(0)).unwrap();
        let activity = store.token_activity(1).unwrap();
        assert_eq!(activity.len(), 2);
        assert_eq!(activity[0], TokenDay { date: today.clone(), provider: "claude".into(), turns: 2, input: 200, output: 40, cached: 100 });
        assert_eq!(activity[1].date, today);
        assert_eq!(activity[1].provider, "codex");
        assert_eq!(store.token_activity(371).unwrap().len(), 3, "old data is outside the calendar");
        assert_eq!(store.token_activity(0).unwrap(), activity, "clamps to at least today");
    }
}
