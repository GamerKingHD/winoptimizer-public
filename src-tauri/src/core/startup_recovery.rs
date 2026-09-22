//! Durable Startup Manager recovery records.
//!
//! New startup disable operations use this app-owned store rather than adding
//! rows to the retained legacy UndoLog. The row shape intentionally reuses
//! `UndoEntry` so the existing, validated startup restoration implementation
//! can consume both current and legacy records.

use super::undo_log::UndoEntry;
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum StartupRecoveryOpenError {
    #[error("application data path unavailable: {0}")]
    AppDataPath(#[from] std::io::Error),
    #[error("startup recovery database unavailable: {0}")]
    Database(#[from] rusqlite::Error),
}

pub struct StartupRecoveryStore {
    conn: Mutex<Connection>,
    db_path: PathBuf,
}

impl StartupRecoveryStore {
    /// Open the canonical app-owned Startup Manager recovery database.
    pub fn open() -> Result<Self, StartupRecoveryOpenError> {
        let db_path = crate::core::paths::AppPaths::installed()?.startup_recovery();
        Self::open_database_path(&db_path)
    }

    fn open_database_path(db_path: &Path) -> Result<Self, StartupRecoveryOpenError> {
        if let Some(db_dir) = db_path.parent() {
            std::fs::create_dir_all(db_dir)?;
        }
        Self::open_at(db_path).map_err(Into::into)
    }

    /// Open (or create) a recovery database at a specific path.
    pub fn open_at(db_path: &Path) -> Result<Self, rusqlite::Error> {
        if let Some(db_dir) = db_path.parent() {
            std::fs::create_dir_all(db_dir)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        }
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA busy_timeout=5000;
             PRAGMA foreign_keys=OFF;
             CREATE TABLE IF NOT EXISTS startup_recovery (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                target TEXT NOT NULL,
                reversal_command TEXT NOT NULL,
                original_size_bytes INTEGER,
                created_at TEXT NOT NULL,
                rolled_back_at TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_startup_recovery_session
                ON startup_recovery(session_id);
            CREATE INDEX IF NOT EXISTS idx_startup_recovery_created
                ON startup_recovery(created_at);",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
            db_path: db_path.to_path_buf(),
        })
    }

    pub fn db_path(&self) -> &PathBuf {
        &self.db_path
    }

    pub fn record(
        &self,
        session_id: &str,
        kind: &str,
        target: &str,
        reversal_command: &str,
        size_bytes: Option<i64>,
    ) -> Result<UndoEntry, rusqlite::Error> {
        let entry = UndoEntry {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            kind: kind.to_string(),
            target: target.to_string(),
            reversal_command: reversal_command.to_string(),
            original_size_bytes: size_bytes,
            created_at: chrono::Utc::now().to_rfc3339(),
            rolled_back_at: None,
        };

        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO startup_recovery
             (id, session_id, kind, target, reversal_command, original_size_bytes, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                entry.id,
                entry.session_id,
                entry.kind,
                entry.target,
                entry.reversal_command,
                entry.original_size_bytes,
                entry.created_at,
            ],
        )?;

        Ok(entry)
    }

    /// Import retained startup rows without changing the legacy database.
    /// Existing IDs are ignored so this is safe to run at each Startup Manager
    /// initialization while the compatibility database is still retained.
    pub fn import_legacy_entries(&self, entries: &[UndoEntry]) -> Result<usize, rusqlite::Error> {
        let mut conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let transaction = conn.transaction()?;
        let mut imported = 0;
        for entry in entries
            .iter()
            .filter(|entry| entry.kind == "startup_change")
        {
            imported += transaction.execute(
                "INSERT OR IGNORE INTO startup_recovery
                 (id, session_id, kind, target, reversal_command, original_size_bytes,
                  created_at, rolled_back_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    entry.id,
                    entry.session_id,
                    entry.kind,
                    entry.target,
                    entry.reversal_command,
                    entry.original_size_bytes,
                    entry.created_at,
                    entry.rolled_back_at,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(imported)
    }

    pub fn mark_rolled_back(&self, id: &str) -> Result<(), rusqlite::Error> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "UPDATE startup_recovery SET rolled_back_at = ?1 WHERE id = ?2",
            params![chrono::Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<UndoEntry>, rusqlite::Error> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, session_id, kind, target, reversal_command, original_size_bytes,
                    created_at, rolled_back_at
             FROM startup_recovery ORDER BY created_at DESC",
        )?;

        let entries = stmt
            .query_map([], |row| {
                Ok(UndoEntry {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    kind: row.get(2)?,
                    target: row.get(3)?,
                    reversal_command: row.get(4)?,
                    original_size_bytes: row.get(5)?,
                    created_at: row.get(6)?,
                    rolled_back_at: row.get(7)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_recovery_records_survive_reopen_and_rollback() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recovery.db");
        let store = StartupRecoveryStore::open_at(&path).unwrap();
        let entry = store
            .record(
                "startup-session",
                "startup_change",
                "fixture-target",
                "{\"action\":\"restore_file\"}",
                None,
            )
            .unwrap();
        store.mark_rolled_back(&entry.id).unwrap();
        drop(store);

        let reopened = StartupRecoveryStore::open_at(&path).unwrap();
        let entries = reopened.list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, entry.id);
        assert!(entries[0].rolled_back_at.is_some());
    }

    #[test]
    fn legacy_import_is_idempotent_and_ignores_other_domains() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recovery.db");
        let store = StartupRecoveryStore::open_at(&path).unwrap();
        let startup = UndoEntry {
            id: "startup-id".into(),
            session_id: "session".into(),
            kind: "startup_change".into(),
            target: "target".into(),
            reversal_command: "{}".into(),
            original_size_bytes: None,
            created_at: "2026-08-14T00:00:00Z".into(),
            rolled_back_at: None,
        };
        let service = UndoEntry {
            kind: "service_change".into(),
            id: "service-id".into(),
            ..startup.clone()
        };

        assert_eq!(
            store
                .import_legacy_entries(&[startup.clone(), service])
                .unwrap(),
            1
        );
        assert_eq!(store.import_legacy_entries(&[startup]).unwrap(), 0);
        assert_eq!(store.list().unwrap().len(), 1);
    }
}
