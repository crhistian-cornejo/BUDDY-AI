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
];

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
}
