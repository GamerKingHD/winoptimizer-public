//! Retained legacy recovery history backed by SQLite at
//! `%LOCALAPPDATA%\WinOptimizer\undo\undo.db`.
//!
//! Each entry captures:
//! - What was changed (file path, registry key, service name)
//! - How to reverse it (reversal command + parameters)
//! - When it happened
//! - A session ID to group batch operations
//!
//! The operation journal is the current history store. This database is now a
//! read-only compatibility source for History and Startup migration; existing
//! rows remain readable and no governed operation writes new rows here.
//!
//! A legacy reversal command supports only a best-effort recovery attempt.
//! This schema has no automated restoration-test or verification fields, so it
//! cannot claim exact recovery merely because a backup artifact exists. The
//! operation journal's explicit `UndoEligibility` remains the UI-facing undo
//! contract.

use super::operation::RecoveryLevel;
use rusqlite::{params, Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum UndoLogOpenError {
    #[error("application data path unavailable: {0}")]
    AppDataPath(#[from] std::io::Error),
    #[error("undo database unavailable: {0}")]
    Database(#[from] rusqlite::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoEntry {
    pub id: String,               // UUID
    pub session_id: String,       // groups a batch of mutations
    pub kind: String,             // "file_delete" | "registry_change" | "service_change"
    pub target: String,           // what was changed (path, reg key, service name)
    pub reversal_command: String, // JSON payload describing how to undo
    pub original_size_bytes: Option<i64>,
    pub created_at: String, // ISO-8601
    /// Legacy reversal completion marker; it is not exact-recovery
    /// verification.
    pub rolled_back_at: Option<String>,
}

impl UndoEntry {
    /// Classify the recovery claim supported by this legacy record.
    ///
    /// A non-empty reversal command establishes an attempted restoration
    /// path, but not the exact state capture and verification required for
    /// exact recovery.
    pub fn recovery_level(&self) -> RecoveryLevel {
        if self.reversal_command.trim().is_empty() {
            RecoveryLevel::None
        } else {
            RecoveryLevel::BestEffort
        }
    }
}

pub struct UndoLog {
    conn: Mutex<Connection>,
    db_path: PathBuf,
}

impl UndoLog {
    /// Open the canonical retained legacy history database only when it
    /// already exists. Compatibility history is read-only; absence is normal.
    pub fn open_existing() -> Result<Option<Self>, UndoLogOpenError> {
        let db_path = crate::core::paths::AppPaths::installed()?.undo_log();
        Self::open_existing_at(&db_path).map_err(UndoLogOpenError::Database)
    }

    pub fn open_existing_at(db_path: &std::path::Path) -> Result<Option<Self>, rusqlite::Error> {
        if !db_path.is_file() {
            return Ok(None);
        }
        Self::open_read_only_at(db_path).map(Some)
    }

    /// Test-only helper for constructing a legacy-shaped database fixture.
    #[cfg(test)]
    pub fn open_at(db_path: &std::path::Path) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(db_path)?;

        // Performance PRAGMAs: WAL mode for concurrent reads during writes,
        // NORMAL sync for speed, 64MB mmap, 16MB cache, 5s busy timeout.
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA mmap_size=67108864;
             PRAGMA cache_size=-16384;
             PRAGMA busy_timeout=5000;
             PRAGMA foreign_keys=OFF;
             CREATE TABLE IF NOT EXISTS undo_log (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                target TEXT NOT NULL,
                reversal_command TEXT NOT NULL,
                original_size_bytes INTEGER,
                created_at TEXT NOT NULL,
                rolled_back_at TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_undo_session ON undo_log(session_id);
            CREATE INDEX IF NOT EXISTS idx_undo_created ON undo_log(created_at);",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
            db_path: db_path.to_path_buf(),
        })
    }

    /// Open an existing legacy database without creating, migrating, or
    /// pruning any data.
    pub fn open_read_only_at(db_path: &std::path::Path) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        Ok(Self {
            conn: Mutex::new(conn),
            db_path: db_path.to_path_buf(),
        })
    }

    pub fn db_path(&self) -> &PathBuf {
        &self.db_path
    }

    /// Read retained legacy history, newest first.
    pub fn list(&self) -> Result<Vec<UndoEntry>, rusqlite::Error> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, session_id, kind, target, reversal_command, original_size_bytes, created_at, rolled_back_at
             FROM undo_log ORDER BY created_at DESC",
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

    /// Get entries by session.
    pub fn list_by_session(&self, session_id: &str) -> Result<Vec<UndoEntry>, rusqlite::Error> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, session_id, kind, target, reversal_command, original_size_bytes, created_at, rolled_back_at
             FROM undo_log WHERE session_id = ?1 ORDER BY created_at ASC",
        )?;

        let entries = stmt
            .query_map(params![session_id], |row| {
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
    fn read_only_open_does_not_create_a_missing_database() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("undo.db");

        assert!(UndoLog::open_read_only_at(&path).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn existing_legacy_history_remains_readable_after_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("undo.db");
        drop(UndoLog::open_at(&path).unwrap());
        let connection = Connection::open(&path).unwrap();
        connection
            .execute(
                "INSERT INTO undo_log
                 (id, session_id, kind, target, reversal_command, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    "legacy-id",
                    "legacy-session",
                    "startup_change",
                    "legacy-target",
                    "{}",
                    "2026-08-14T00:00:00Z",
                ],
            )
            .unwrap();
        drop(connection);

        let reopened = UndoLog::open_read_only_at(&path).unwrap();
        let entries = reopened.list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "legacy-id");
        assert_eq!(entries[0].target, "legacy-target");
    }

    #[test]
    fn legacy_reversal_command_is_best_effort_not_exact_recovery() {
        let entry = UndoEntry {
            id: "id".into(),
            session_id: "session".into(),
            kind: "file_delete".into(),
            target: "target".into(),
            reversal_command: "{\"action\":\"restore_file\"}".into(),
            original_size_bytes: Some(12),
            created_at: "2026-08-10T00:00:00Z".into(),
            rolled_back_at: None,
        };

        assert_eq!(entry.recovery_level(), RecoveryLevel::BestEffort);

        let mut without_reversal = entry;
        without_reversal.reversal_command.clear();
        assert_eq!(without_reversal.recovery_level(), RecoveryLevel::None);
    }
}
