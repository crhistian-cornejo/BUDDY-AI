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
];

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

impl Store {
    /// Creates the chat if it does not exist yet.
    pub fn ensure_chat(&self, id: &str, title: &str) -> Result<(), CoreError> {
        self.conn.execute("INSERT OR IGNORE INTO chats (id, title) VALUES (?1, ?2)", params![id, title])?;
        Ok(())
    }

    pub fn add_message(&self, m: NewMessage<'_>) -> Result<i64, CoreError> {
        let sources = serde_json::to_string(m.sources).unwrap_or_else(|_| "[]".into());
        self.conn.execute(
            "INSERT INTO messages (chat_id, role, agent, provider, text, sources, failed) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![m.chat_id, m.role, m.agent, m.provider, m.text, sources, m.failed],
        )?;
        let id = self.conn.last_insert_rowid();
        self.conn.execute("UPDATE chats SET updated_at = unixepoch() WHERE id = ?1", params![m.chat_id])?;
        Ok(id)
    }

    pub fn chats(&self, limit: u32) -> Result<Vec<ChatSummary>, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT c.id, c.title, c.updated_at,
                    COALESCE((SELECT substr(text, 1, 120) FROM messages m WHERE m.chat_id = c.id ORDER BY m.id DESC LIMIT 1), '')
             FROM chats c ORDER BY c.updated_at DESC, c.rowid DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |r| {
            Ok(ChatSummary { id: r.get(0)?, title: r.get(1)?, updated_at: r.get(2)?, preview: r.get(3)? })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn messages(&self, chat_id: &str) -> Result<Vec<ChatMessage>, CoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, role, agent, provider, text, sources, failed, created_at FROM messages WHERE chat_id = ?1 ORDER BY id",
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
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
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
            chat_id: "c1", role, agent: "buddy", provider: Some("claude"), text, sources, failed: false,
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
}
