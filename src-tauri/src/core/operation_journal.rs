//! Append-only operation journal and captured-state undo planning.
//!
//! Terminology:
//! - a backup is a captured original artifact or state;
//! - undo is the automated operation that restores prior state;
//! - exact recovery requires tested restoration and verification;
//! - best-effort recovery does not guarantee exact equivalence;
//! - a restore point is Windows System Restore and is independent of
//!   WinOptimizer exact undo.
//!
//! `UndoEligibility` is an explicit, persisted UI contract. It is never
//! inferred from a backup path or from captured-state presence alone.

use super::operation::{
    ExecutionStatus, ExpectedEffect, OperationDescriptor, OperationError, OperationErrorCode,
    OperationExecutionState, OperationId, OperationPlan, OperationResult, OperationState,
    OperationStateMachine, OperationWarning, PlannedTarget, RebootPolicy, RecoveryLevel,
    Reversibility, StateSnapshot, TargetExecutionState, TargetOutcome, TargetStateDiff,
    UndoAvailability, VerificationStatus,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const OPERATION_SCHEMA_VERSION: u32 = 1;
const JOURNAL_SCHEMA_VERSION: i64 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedOperation {
    pub operation_id: OperationId,
    pub state: OperationExecutionState,
    pub confirmed_plan: OperationPlan,
    pub targets: Vec<UnresolvedTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedTarget {
    pub operation_id: OperationId,
    pub target_id: String,
    pub state: TargetExecutionState,
    pub payload_json: String,
}

/// Outcome of a startup reconciliation pass. A per-operation `failure` means
/// that operation could not be reconciled; unrelated operations are still
/// processed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconciliationReport {
    pub reconciled: usize,
    pub failures: Vec<ReconciliationFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciliationFailure {
    pub operation_id: OperationId,
    pub message: String,
}

/// Result of a domain-specific current-state inspection during recovery.
///
/// The journal remains conservative by default, but a domain may prove that
/// an unresolved target already satisfies its requested postcondition without
/// replaying the original operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconciliationDecision {
    PostconditionVerified { payload_json: String },
    OutcomeUnknown { payload_json: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationRelationship {
    pub operation_id: OperationId,
    pub parent_operation_id: Option<OperationId>,
    pub retry_of_operation_id: Option<OperationId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentScope {
    User,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CapturedOriginalState {
    RegistryValue {
        target_id: String,
        hive: String,
        key_path: String,
        value_name: String,
        existed: bool,
        value_type: Option<String>,
        raw_value_data: Option<Vec<u8>>,
    },
    EnvironmentVariable {
        target_id: String,
        name: String,
        scope: EnvironmentScope,
        existed: bool,
        value_type: Option<String>,
        value: Option<String>,
    },
    StartupEntry {
        target_id: String,
        identifier: String,
        name: String,
        location: String,
        value_type: Option<String>,
        enabled: bool,
        command: String,
    },
    HostsFile {
        target_id: String,
        path: String,
        bytes: Vec<u8>,
        encoding: String,
        #[serde(default)]
        acl: Option<String>,
        #[serde(default)]
        attributes: Option<u32>,
        #[serde(default)]
        creation_time: Option<u64>,
        #[serde(default)]
        last_access_time: Option<u64>,
        #[serde(default)]
        last_write_time: Option<u64>,
    },
    /// Structured pre-mutation evidence for domains without an exact undo
    /// adapter. This is intentionally distinct from the test-only synthetic
    /// restoration contract below and must not advertise undo by itself.
    TypedState {
        target_id: String,
        state: StateSnapshot,
    },
    Synthetic {
        target_id: String,
        state: StateSnapshot,
    },
}

impl CapturedOriginalState {
    pub fn target_id(&self) -> &str {
        match self {
            Self::RegistryValue { target_id, .. }
            | Self::EnvironmentVariable { target_id, .. }
            | Self::StartupEntry { target_id, .. }
            | Self::HostsFile { target_id, .. }
            | Self::TypedState { target_id, .. }
            | Self::Synthetic { target_id, .. } => target_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationRecord {
    pub target_id: String,
    pub status: VerificationStatus,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoEligibility {
    /// Explicit UI-facing eligibility. This is not derived from backup
    /// artifacts; `OperationJournalRecord::validate` requires recovery
    /// evidence before an eligible record can be persisted.
    pub eligible: bool,
    /// User-facing explanation for the eligibility decision.
    pub reason: String,
    /// Identifier for the automated restoration test proving exact recovery.
    pub automated_restoration_test: Option<String>,
}

impl UndoEligibility {
    /// Construct an eligibility decision backed by a named exact-recovery
    /// restoration test.
    pub fn exact(restoration_test: impl Into<String>) -> Self {
        Self {
            eligible: true,
            reason: "Exact recovery has a tested restoration path.".into(),
            automated_restoration_test: Some(restoration_test.into()),
        }
    }

    /// Construct an explicit unavailable decision.
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            eligible: false,
            reason: reason.into(),
            automated_restoration_test: None,
        }
    }

    /// Return the explicit signal safe for a UI to consume after the record
    /// has passed journal validation.
    pub fn is_ui_available(&self) -> bool {
        self.eligible
            && self
                .automated_restoration_test
                .as_deref()
                .is_some_and(|test| !test.trim().is_empty())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalRecordKind {
    Operation,
    Undo,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoExecutionResult {
    pub source_record_id: String,
    pub lifecycle: Vec<OperationState>,
    pub result: OperationResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct OperationJournalRecord {
    pub journal_id: String,
    pub record_kind: JournalRecordKind,
    pub related_record_id: Option<String>,
    pub operation_id: OperationId,
    pub operation_schema_version: u32,
    pub application_version: String,
    pub timestamp: String,
    pub terminal_state: OperationState,
    pub confirmed_plan: OperationPlan,
    pub requested_targets: Vec<PlannedTarget>,
    pub captured_original_states: Vec<CapturedOriginalState>,
    pub target_outcomes: Vec<TargetOutcome>,
    pub verification_results: Vec<VerificationRecord>,
    pub restart_requirement: RebootPolicy,
    pub undo_eligibility: UndoEligibility,
    pub undo_execution_result: Option<UndoExecutionResult>,
    pub diagnostic: Option<OperationError>,
}

#[derive(Serialize)]
struct OperationJournalRecordWire<'a> {
    journal_id: &'a str,
    record_kind: &'a JournalRecordKind,
    related_record_id: &'a Option<String>,
    operation_id: &'a OperationId,
    operation_schema_version: u32,
    application_version: &'a str,
    timestamp: &'a str,
    terminal_state: &'a OperationState,
    confirmed_plan: &'a OperationPlan,
    requested_targets: &'a [PlannedTarget],
    captured_original_states: &'a [CapturedOriginalState],
    target_outcomes: &'a [TargetOutcome],
    verification_results: &'a [VerificationRecord],
    restart_requirement: &'a RebootPolicy,
    undo_eligibility: &'a UndoEligibility,
    undo_execution_result: &'a Option<UndoExecutionResult>,
    diagnostic: &'a Option<OperationError>,
    state_diffs: Vec<TargetStateDiff>,
}

impl Serialize for OperationJournalRecord {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        OperationJournalRecordWire {
            journal_id: &self.journal_id,
            record_kind: &self.record_kind,
            related_record_id: &self.related_record_id,
            operation_id: &self.operation_id,
            operation_schema_version: self.operation_schema_version,
            application_version: &self.application_version,
            timestamp: &self.timestamp,
            terminal_state: &self.terminal_state,
            confirmed_plan: &self.confirmed_plan,
            requested_targets: &self.requested_targets,
            captured_original_states: &self.captured_original_states,
            target_outcomes: &self.target_outcomes,
            verification_results: &self.verification_results,
            restart_requirement: &self.restart_requirement,
            undo_eligibility: &self.undo_eligibility,
            undo_execution_result: &self.undo_execution_result,
            diagnostic: &self.diagnostic,
            state_diffs: self
                .target_outcomes
                .iter()
                .map(TargetOutcome::state_diff)
                .collect(),
        }
        .serialize(serializer)
    }
}

impl OperationJournalRecord {
    /// Return the persisted UI undo signal.
    ///
    /// Backups and captured originals are evidence inputs, not eligibility
    /// signals. Records returned by [`OperationJournal::list`] and `get` have
    /// already passed [`Self::validate`].
    pub fn ui_undo_available(&self) -> bool {
        self.validate().is_ok() && self.undo_eligibility.is_ui_available()
    }

    pub fn validate(&self) -> Result<(), OperationError> {
        if self.operation_schema_version != OPERATION_SCHEMA_VERSION {
            return Err(validation("unsupported operation journal schema version"));
        }
        if self.confirmed_plan.id != self.operation_id {
            return Err(validation(
                "journal operation id does not match its confirmed plan",
            ));
        }
        if self.requested_targets != self.confirmed_plan.targets {
            return Err(validation(
                "journal requested targets do not match the confirmed plan",
            ));
        }
        let requested_target_ids: BTreeSet<&str> = self
            .requested_targets
            .iter()
            .map(|target| target.target_id.as_str())
            .collect();
        let outcome_target_ids: BTreeSet<&str> = self
            .target_outcomes
            .iter()
            .map(|outcome| outcome.target_id.as_str())
            .collect();
        if requested_target_ids.len() != self.requested_targets.len()
            || outcome_target_ids.len() != self.target_outcomes.len()
            || outcome_target_ids != requested_target_ids
        {
            return Err(validation(
                "journal target outcomes must contain each requested target exactly once",
            ));
        }
        let verification_target_ids: BTreeSet<&str> = self
            .verification_results
            .iter()
            .map(|verification| verification.target_id.as_str())
            .collect();
        if verification_target_ids.len() != self.verification_results.len()
            || verification_target_ids != requested_target_ids
            || self.target_outcomes.iter().any(|outcome| {
                !self.verification_results.iter().any(|verification| {
                    verification.target_id == outcome.target_id
                        && verification.status == outcome.verification
                })
            })
        {
            return Err(validation(
                "journal verification results do not match target outcomes",
            ));
        }
        if self.terminal_state == OperationState::Interrupted && self.diagnostic.is_none() {
            return Err(validation("an interrupted operation requires a diagnostic"));
        }
        // A mutating operation that advertises a verification contract must not
        // record success unless every target was actually verified.
        if self.terminal_state == OperationState::Succeeded
            && self.confirmed_plan.descriptor.requires_verification
            && self
                .target_outcomes
                .iter()
                .any(|outcome| outcome.verification != VerificationStatus::Verified)
        {
            return Err(validation(
                "an operation that requires verification cannot record success without verifying every target",
            ));
        }
        if self.record_kind == JournalRecordKind::Undo && self.undo_execution_result.is_none() {
            return Err(validation(
                "an undo journal record requires an execution result",
            ));
        }
        if self.record_kind == JournalRecordKind::Undo && self.undo_eligibility.eligible {
            return Err(validation(
                "an undo record cannot advertise recursive UI undo",
            ));
        }
        if self.undo_eligibility.eligible {
            if self.confirmed_plan.reversibility != Reversibility::FullyReversible
                || self.confirmed_plan.descriptor.reversibility != Reversibility::FullyReversible
            {
                return Err(validation(
                    "Undo eligibility requires a fully reversible operation",
                ));
            }
            if self.confirmed_plan.descriptor.recovery == RecoveryLevel::BestEffort
                || (self.confirmed_plan.descriptor.mutates_system
                    && self.confirmed_plan.descriptor.recovery != RecoveryLevel::Exact)
            {
                return Err(validation(
                    "Undo eligibility requires an Exact recovery contract",
                ));
            }
            if self.captured_original_states.is_empty() {
                return Err(validation(
                    "Undo eligibility requires captured original state",
                ));
            }
            if !self.undo_eligibility.is_ui_available() {
                return Err(validation(
                    "undo requires an automated restoration test identifier",
                ));
            }
            let restoration_test = self
                .undo_eligibility
                .automated_restoration_test
                .as_deref()
                .unwrap_or_default();
            if self.captured_original_states.iter().any(|capture| {
                !matches!(
                    (capture, restoration_test),
                    (
                        CapturedOriginalState::Synthetic { .. },
                        "operation_journal_tests::undo_restores_captured_original_state"
                    ) | (
                        CapturedOriginalState::EnvironmentVariable { .. },
                        "env_operation::captured_user_variable_is_restored_and_verified"
                    ) | (
                        CapturedOriginalState::TypedState { .. },
                        "system_tweak_operation::captured_state_is_restored_and_verified"
                    )
                )
            }) {
                return Err(validation(
                    "undo is not available without a matching automated restoration test",
                ));
            }
            for outcome in self
                .target_outcomes
                .iter()
                .filter(|outcome| outcome.execution == ExecutionStatus::Changed)
            {
                if !self
                    .captured_original_states
                    .iter()
                    .any(|capture| capture.target_id() == outcome.target_id)
                {
                    return Err(validation(format!(
                        "changed target {} has no captured original state",
                        outcome.target_id
                    )));
                }
            }
        }
        Ok(())
    }
}

pub struct OperationJournal {
    conn: Mutex<Connection>,
    db_path: PathBuf,
    #[cfg(any(test, debug_assertions))]
    fail_next_journal_write: Mutex<bool>,
    #[cfg(any(test, debug_assertions))]
    fail_after_plan_persist: Mutex<bool>,
    #[cfg(any(test, debug_assertions))]
    fail_after_execution_start: Mutex<bool>,
    #[cfg(any(test, debug_assertions))]
    fail_before_verification: Mutex<bool>,
}

impl OperationJournal {
    pub fn open() -> Result<Self, rusqlite::Error> {
        let paths = crate::core::paths::AppPaths::installed()
            .map_err(|_| rusqlite::Error::InvalidPath(PathBuf::from("LOCALAPPDATA")))?;
        Self::open_at(&paths.journal)
    }

    pub fn open_at(path: &Path) -> Result<Self, rusqlite::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
        }
        let conn = Connection::open(path)?;
        let stored_schema_version: i64 =
            conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if stored_schema_version > JOURNAL_SCHEMA_VERSION {
            return Err(rusqlite::Error::InvalidQuery);
        }
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA busy_timeout=5000;
             CREATE TABLE IF NOT EXISTS operation_journal (
                journal_id TEXT PRIMARY KEY,
                operation_id TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                record_json TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_operation_journal_timestamp
                ON operation_journal(timestamp DESC);",
        )?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS operation_execution_events (
                event_sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                event_id TEXT NOT NULL UNIQUE,
                operation_id TEXT NOT NULL,
                state TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                payload_json TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_operation_execution_events_operation
                ON operation_execution_events(operation_id, event_sequence DESC);",
        )?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS operation_target_states (
                target_sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                target_event_id TEXT NOT NULL UNIQUE,
                operation_id TEXT NOT NULL,
                target_id TEXT NOT NULL,
                state TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                payload_json TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_operation_target_states_target
             ON operation_target_states(operation_id, target_id, target_sequence DESC);",
        )?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS operation_relationships (
                relationship_id TEXT PRIMARY KEY,
                operation_id TEXT NOT NULL UNIQUE,
                parent_operation_id TEXT,
                retry_of_operation_id TEXT,
                timestamp TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_operation_relationships_parent
                ON operation_relationships(parent_operation_id);
             CREATE INDEX IF NOT EXISTS idx_operation_relationships_retry
                ON operation_relationships(retry_of_operation_id);",
        )?;
        if stored_schema_version < JOURNAL_SCHEMA_VERSION {
            conn.execute_batch("PRAGMA user_version = 3;")?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
            db_path: path.to_path_buf(),
            #[cfg(any(test, debug_assertions))]
            fail_next_journal_write: Mutex::new(false),
            #[cfg(any(test, debug_assertions))]
            fail_after_plan_persist: Mutex::new(false),
            #[cfg(any(test, debug_assertions))]
            fail_after_execution_start: Mutex::new(false),
            #[cfg(any(test, debug_assertions))]
            fail_before_verification: Mutex::new(false),
        })
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    #[cfg(any(test, debug_assertions))]
    #[allow(dead_code)]
    pub(crate) fn fail_next_journal_write(&self) {
        *self
            .fail_next_journal_write
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = true;
    }

    #[cfg(any(test, debug_assertions))]
    fn take_fail_next_journal_write(&self) -> bool {
        let mut failure = self
            .fail_next_journal_write
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let should_fail = *failure;
        *failure = false;
        should_fail
    }

    #[cfg(any(test, debug_assertions))]
    #[allow(dead_code)]
    pub(crate) fn fail_after_plan_persist(&self) {
        *self
            .fail_after_plan_persist
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = true;
    }

    #[cfg(any(test, debug_assertions))]
    fn take_fail_after_plan_persist(&self) -> bool {
        let mut failure = self
            .fail_after_plan_persist
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let should_fail = *failure;
        *failure = false;
        should_fail
    }

    #[cfg(any(test, debug_assertions))]
    #[allow(dead_code)]
    pub(crate) fn fail_after_execution_start(&self) {
        *self
            .fail_after_execution_start
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = true;
    }

    #[cfg(any(test, debug_assertions))]
    fn take_fail_after_execution_start(&self) -> bool {
        let mut failure = self
            .fail_after_execution_start
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let should_fail = *failure;
        *failure = false;
        should_fail
    }

    #[cfg(any(test, debug_assertions))]
    #[allow(dead_code)]
    pub(crate) fn fail_before_verification(&self) {
        *self
            .fail_before_verification
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = true;
    }

    #[cfg(any(test, debug_assertions))]
    fn take_fail_before_verification(&self) -> bool {
        let mut failure = self
            .fail_before_verification
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let should_fail = *failure;
        *failure = false;
        should_fail
    }

    pub fn contains_operation_id(&self, operation_id: &str) -> Result<bool, OperationError> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        conn.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM operation_journal WHERE operation_id = ?1
                UNION ALL
                SELECT 1 FROM operation_execution_events WHERE operation_id = ?1
            )",
            params![operation_id],
            |row| row.get(0),
        )
        .map_err(database_error)
    }

    fn has_durable_execution_start(&self, plan: &OperationPlan) -> Result<bool, OperationError> {
        // Compare stable plan identity rather than a serialized byte
        // representation so representation drift cannot orphan a real
        // execution-start record.
        let expected = plan.identity_digest();
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let mut statement = conn
            .prepare(
                "SELECT payload_json FROM operation_execution_events
                 WHERE operation_id = ?1
                   AND state IN ('execution_starting', 'executing', 'outcome_unknown')",
            )
            .map_err(database_error)?;
        let rows = statement
            .query_map(params![plan.id.0], |row| row.get::<_, String>(0))
            .map_err(database_error)?;
        for row in rows {
            let payload = row.map_err(database_error)?;
            if let Ok(stored) = serde_json::from_str::<OperationPlan>(&payload) {
                if stored.identity_digest() == expected {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Durable evidence that the exact plan (by stable identity) was confirmed.
    ///
    /// Accepts both current digest-based confirmation records and legacy
    /// serialized-plan confirmation records. A legacy payload only counts when
    /// it parses and derives the exact current plan identity; malformed payloads
    /// and payloads whose targets/actions changed are not confirmation.
    pub fn has_durable_confirmation(&self, plan: &OperationPlan) -> Result<bool, OperationError> {
        let expected = plan.identity_digest();
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let mut statement = conn
            .prepare(
                "SELECT payload_json FROM operation_execution_events
                 WHERE operation_id = ?1 AND state = 'confirmed'",
            )
            .map_err(database_error)?;
        let rows = statement
            .query_map(params![plan.id.0], |row| row.get::<_, String>(0))
            .map_err(database_error)?;
        for row in rows {
            let payload = row.map_err(database_error)?;
            if payload == expected {
                return Ok(true);
            }
            if let Ok(stored) = serde_json::from_str::<OperationPlan>(&payload) {
                if stored.identity_digest() == expected {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Persist confirmation for the exact plan before any mutation may run.
    ///
    /// No-op for plans that do not require confirmation. Idempotent for an
    /// already-confirmed plan. Rejected once execution has started.
    pub fn record_confirmation(&self, plan: &OperationPlan) -> Result<(), OperationError> {
        plan.descriptor.validate_contract()?;
        plan.validate_relationships()?;
        if !plan.descriptor.requires_confirmation {
            return Ok(());
        }
        if self.has_durable_confirmation(plan)? {
            return Ok(());
        }
        let digest = plan.identity_digest();
        let mut conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let transaction = conn.transaction().map_err(database_error)?;
        insert_execution_event(
            &transaction,
            &plan.id,
            OperationExecutionState::Confirmed,
            &digest,
        )?;
        transaction.commit().map_err(database_error)
    }

    pub fn schema_version(&self) -> Result<i64, OperationError> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        conn.pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(database_error)
    }

    pub fn list_relationships(&self) -> Result<Vec<OperationRelationship>, OperationError> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let mut statement = conn
            .prepare(
                "SELECT operation_id, parent_operation_id, retry_of_operation_id
                 FROM operation_relationships ORDER BY timestamp ASC",
            )
            .map_err(database_error)?;
        let rows = statement
            .query_map([], |row| {
                let operation_id: String = row.get(0)?;
                let parent_operation_id: Option<String> = row.get(1)?;
                let retry_of_operation_id: Option<String> = row.get(2)?;
                Ok((operation_id, parent_operation_id, retry_of_operation_id))
            })
            .map_err(database_error)?;
        rows.map(|row| {
            let (operation_id, parent_operation_id, retry_of_operation_id) =
                row.map_err(database_error)?;
            Ok(OperationRelationship {
                operation_id: OperationId(operation_id),
                parent_operation_id: parent_operation_id.map(OperationId),
                retry_of_operation_id: retry_of_operation_id.map(OperationId),
            })
        })
        .collect()
    }

    pub fn append(&self, record: &OperationJournalRecord) -> Result<(), OperationError> {
        record.validate()?;
        record.confirmed_plan.descriptor.validate_contract()?;
        record.confirmed_plan.validate_relationships()?;
        let json = serde_json::to_string(record).map_err(|error| OperationError {
            code: OperationErrorCode::SerializationFailed,
            message: error.to_string(),
            target_id: None,
            retryable: false,
        })?;
        if record.confirmed_plan.descriptor.mutates_system
            && !self.has_durable_execution_start(&record.confirmed_plan)?
        {
            return Err(validation(
                "mutating operation must persist ExecutionStarting before terminal journal append",
            ));
        }
        #[cfg(any(test, debug_assertions))]
        if self.take_fail_next_journal_write() {
            return Err(OperationError {
                code: OperationErrorCode::InternalError,
                message: "failure injection: fail_journal_write".into(),
                target_id: None,
                retryable: true,
            });
        }
        let mut conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let transaction = conn.transaction().map_err(database_error)?;
        transaction
            .execute(
                "INSERT INTO operation_journal
                (journal_id, operation_id, timestamp, record_json)
             VALUES (?1, ?2, ?3, ?4)",
                params![
                    record.journal_id,
                    record.operation_id.0,
                    record.timestamp,
                    json
                ],
            )
            .map_err(database_error)?;
        insert_relationship(&transaction, &record.confirmed_plan, &record.timestamp)?;
        insert_execution_event(
            &transaction,
            &record.operation_id,
            OperationExecutionState::MutationCompleted,
            &json,
        )?;
        for outcome in &record.target_outcomes {
            insert_target_event(
                &transaction,
                &record.operation_id,
                &outcome.target_id,
                target_execution_state(outcome),
                &serde_json::to_string(outcome).map_err(|error| OperationError {
                    code: OperationErrorCode::SerializationFailed,
                    message: error.to_string(),
                    target_id: Some(outcome.target_id.clone()),
                    retryable: false,
                })?,
            )?;
        }
        #[cfg(any(test, debug_assertions))]
        if self.take_fail_before_verification() {
            return Err(failure_injection_error("fail_before_verification"));
        }
        insert_execution_event(
            &transaction,
            &record.operation_id,
            OperationExecutionState::Verifying,
            &json,
        )?;
        insert_execution_event(
            &transaction,
            &record.operation_id,
            terminal_execution_state(record.terminal_state),
            &json,
        )?;
        transaction.commit().map_err(database_error)?;
        Ok(())
    }

    /// Commit the execution-start marker before any system mutation runs.
    ///
    /// A mutating operation that requires confirmation must already have a
    /// durable confirmation for the exact plan identity. Callers that obtain
    /// confirmation at the same boundary should use
    /// [`begin_confirmed_execution`](Self::begin_confirmed_execution).
    pub fn begin_execution(&self, plan: &OperationPlan) -> Result<(), OperationError> {
        plan.descriptor.validate_contract()?;
        plan.validate_relationships()?;
        if plan.descriptor.mutates_system
            && plan.descriptor.requires_confirmation
            && !self.has_durable_confirmation(plan)?
        {
            return Err(validation(
                "mutating operation requires durable confirmation of the exact plan before execution",
            ));
        }
        if let Some(state) = self.latest_execution_state(&plan.id)? {
            if !matches!(
                state,
                OperationExecutionState::Planned
                    | OperationExecutionState::AwaitingConfirmation
                    | OperationExecutionState::Confirmed
            ) {
                return Err(validation(
                    "operation execution has already started; retry must use a new operation id",
                ));
            }
        }
        let payload = serde_json::to_string(plan).map_err(|error| OperationError {
            code: OperationErrorCode::SerializationFailed,
            message: error.to_string(),
            target_id: None,
            retryable: false,
        })?;
        let mut conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let transaction = conn.transaction().map_err(database_error)?;
        insert_execution_event(
            &transaction,
            &plan.id,
            OperationExecutionState::ExecutionStarting,
            &payload,
        )?;
        for target in &plan.targets {
            insert_target_event(
                &transaction,
                &plan.id,
                &target.target_id,
                TargetExecutionState::Pending,
                &serde_json::to_string(target).map_err(|error| OperationError {
                    code: OperationErrorCode::SerializationFailed,
                    message: error.to_string(),
                    target_id: Some(target.target_id.clone()),
                    retryable: false,
                })?,
            )?;
        }
        insert_relationship(&transaction, plan, &chrono::Utc::now().to_rfc3339())?;
        transaction.commit().map_err(database_error)?;
        drop(conn);

        #[cfg(any(test, debug_assertions))]
        if self.take_fail_after_plan_persist() {
            return Err(failure_injection_error("fail_after_plan_persist"));
        }

        let restore_point_policy = plan.descriptor.restore_point_policy;
        if restore_point_policy.should_attempt() {
            let description = format!("WinOptimizer: {}", plan.descriptor.title);
            match crate::core::restore_point::create_restore_point(&description) {
                Ok(status) => log::info!(
                    "Restore point created for {} with sequence {}",
                    plan.id.0,
                    status.ll_sequence_number
                ),
                Err(error) if restore_point_policy.blocks_on_failure() => {
                    return Err(OperationError {
                        code: OperationErrorCode::ExecutionFailed,
                        message: format!(
                            "required restore point could not be created before mutation: {error}"
                        ),
                        target_id: None,
                        retryable: true,
                    });
                }
                Err(error) => log::warn!(
                    "Restore point was not created for {}; continuing per {:?} policy: {}",
                    plan.id.0,
                    restore_point_policy,
                    error
                ),
            }
        }
        self.record_execution_started(plan, &payload)?;
        #[cfg(any(test, debug_assertions))]
        if self.take_fail_after_execution_start() {
            return Err(failure_injection_error("fail_after_execution_start"));
        }
        Ok(())
    }

    /// Record durable confirmation for this exact plan, then begin execution.
    ///
    /// This is the boundary for non-elevated operations whose execution
    /// command is reached only after the UI completed its confirmation step.
    /// The elevated resume path records confirmation separately in
    /// `confirm_and_request_elevation` and calls `begin_execution` directly.
    pub fn begin_confirmed_execution(&self, plan: &OperationPlan) -> Result<(), OperationError> {
        self.record_confirmation(plan)?;
        self.begin_execution(plan)
    }

    fn record_execution_started(
        &self,
        plan: &OperationPlan,
        payload: &str,
    ) -> Result<(), OperationError> {
        let mut conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let transaction = conn.transaction().map_err(database_error)?;
        insert_execution_event(
            &transaction,
            &plan.id,
            OperationExecutionState::Executing,
            payload,
        )?;
        // Starting the operation does not start every target mutation. Each
        // domain must durably report its own Ready and MutationStarted states
        // around its actual revalidation and mutation loop.
        transaction.commit().map_err(database_error)
    }

    /// Persist a pre-execution lifecycle state before an elevated handoff.
    pub fn record_execution_state(
        &self,
        plan: &OperationPlan,
        state: OperationExecutionState,
    ) -> Result<(), OperationError> {
        plan.descriptor.validate_contract()?;
        plan.validate_relationships()?;
        let payload = serde_json::to_string(plan).map_err(|error| OperationError {
            code: OperationErrorCode::SerializationFailed,
            message: error.to_string(),
            target_id: None,
            retryable: false,
        })?;
        let mut conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let transaction = conn.transaction().map_err(database_error)?;
        insert_execution_event(&transaction, &plan.id, state, &payload)?;
        transaction.commit().map_err(database_error)
    }

    pub fn latest_execution_state(
        &self,
        operation_id: &OperationId,
    ) -> Result<Option<OperationExecutionState>, OperationError> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let state = conn
            .query_row(
                "SELECT state FROM operation_execution_events
                 WHERE operation_id = ?1 ORDER BY event_sequence DESC LIMIT 1",
                params![operation_id.0],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error)?;
        state
            .map(|value| {
                execution_state_from_str(&value).ok_or_else(|| {
                    validation("operation journal contains an unknown execution state")
                })
            })
            .transpose()
    }

    pub fn list_unresolved_targets(
        &self,
        operation_id: &OperationId,
    ) -> Result<Vec<UnresolvedTarget>, OperationError> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let mut statement = conn
            .prepare(
                "SELECT latest.operation_id, latest.target_id, latest.state, latest.payload_json
                 FROM operation_target_states AS latest
                   WHERE latest.operation_id = ?1
                   AND latest.target_sequence = (
                    SELECT MAX(previous.target_sequence)
                    FROM operation_target_states AS previous
                    WHERE previous.operation_id = latest.operation_id
                      AND previous.target_id = latest.target_id
                   )
                   AND NOT EXISTS (
                    SELECT 1 FROM operation_journal AS terminal
                    WHERE terminal.operation_id = latest.operation_id
                   )
                   AND latest.state IN (
                   'pending', 'revalidation_failed', 'ready', 'mutation_started',
                    'changed', 'unchanged', 'verification_failed', 'failed',
                    'outcome_unknown'
                   )
                 ORDER BY (
                    SELECT MIN(initial.target_sequence)
                    FROM operation_target_states AS initial
                    WHERE initial.operation_id = latest.operation_id
                      AND initial.target_id = latest.target_id
                 ) ASC",
            )
            .map_err(database_error)?;
        let rows = statement
            .query_map(params![operation_id.0], |row| {
                let operation_id: String = row.get(0)?;
                let target_id: String = row.get(1)?;
                let state: String = row.get(2)?;
                let payload_json: String = row.get(3)?;
                Ok((operation_id, target_id, state, payload_json))
            })
            .map_err(database_error)?;
        rows.map(|row| {
            let (operation_id, target_id, state, payload_json) = row.map_err(database_error)?;
            let state = target_execution_state_from_str(&state)
                .ok_or_else(|| validation("operation journal contains an unknown target state"))?;
            Ok(UnresolvedTarget {
                operation_id: OperationId(operation_id),
                target_id,
                state,
                payload_json,
            })
        })
        .collect()
    }

    /// Select only targets that are unresolved in the current durable journal
    /// state. Domain retry builders should rediscover these targets and create
    /// a new immutable plan rather than replaying the original plan.
    pub fn select_unresolved_targets_for_retry(
        &self,
        operation_id: &OperationId,
        requested_target_ids: &[String],
    ) -> Result<Vec<UnresolvedTarget>, OperationError> {
        if requested_target_ids.is_empty() {
            return Err(validation("retry requires at least one unresolved target"));
        }

        let requested: BTreeSet<&str> = requested_target_ids.iter().map(String::as_str).collect();
        if requested.len() != requested_target_ids.len() {
            return Err(validation("retry targets cannot contain duplicates"));
        }

        let unresolved = self.list_unresolved_targets(operation_id)?;
        let selected = unresolved
            .into_iter()
            .filter(|target| requested.contains(target.target_id.as_str()))
            .collect::<Vec<_>>();
        if selected.len() != requested_target_ids.len() {
            return Err(validation(
                "retry targets must be currently unresolved in the operation journal",
            ));
        }
        Ok(selected)
    }

    /// Apply the result of a domain-specific current-state inspection to one
    /// unresolved target. This records evidence only; it never replays the
    /// original mutation or closes the operation's unresolved execution.
    pub fn reconcile_unresolved_target(
        &self,
        operation_id: &OperationId,
        target_id: &str,
        decision: ReconciliationDecision,
    ) -> Result<TargetExecutionState, OperationError> {
        if !self
            .list_unresolved_targets(operation_id)?
            .iter()
            .any(|target| target.target_id == target_id)
        {
            return Err(validation(
                "reconciliation requires a currently unresolved target",
            ));
        }

        let (state, payload_json) = match decision {
            ReconciliationDecision::PostconditionVerified { payload_json } => {
                (TargetExecutionState::VerificationSucceeded, payload_json)
            }
            ReconciliationDecision::OutcomeUnknown { payload_json } => {
                (TargetExecutionState::OutcomeUnknown, payload_json)
            }
        };
        self.record_target_state(operation_id, target_id, state, &payload_json)?;
        Ok(state)
    }

    pub fn record_target_state(
        &self,
        operation_id: &OperationId,
        target_id: &str,
        state: TargetExecutionState,
        payload_json: &str,
    ) -> Result<(), OperationError> {
        let mut conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let transaction = conn.transaction().map_err(database_error)?;
        let current = transaction
            .query_row(
                "SELECT state FROM operation_target_states
                 WHERE operation_id = ?1 AND target_id = ?2
                 ORDER BY target_sequence DESC LIMIT 1",
                params![operation_id.0, target_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error)?
            .ok_or_else(|| {
                validation(format!(
                    "target {target_id} has no durable state for operation {}",
                    operation_id.0
                ))
            })?;
        let current = target_execution_state_from_str(&current)
            .ok_or_else(|| validation("operation journal contains an unknown target state"))?;
        if !target_state_can_transition(current, state) {
            return Err(validation(format!(
                "forbidden target transition: {current:?} -> {state:?}"
            )));
        }
        insert_target_event(&transaction, operation_id, target_id, state, payload_json)?;
        transaction.commit().map_err(database_error)
    }

    /// Persist the durable target state represented by an observed outcome.
    ///
    /// Domain executors call this immediately after their target-level work so
    /// an interruption between targets leaves an honest recovery boundary.
    pub fn record_target_outcome(
        &self,
        operation_id: &OperationId,
        outcome: &TargetOutcome,
    ) -> Result<(), OperationError> {
        let payload = serde_json::to_string(outcome).map_err(|error| OperationError {
            code: OperationErrorCode::SerializationFailed,
            message: error.to_string(),
            target_id: Some(outcome.target_id.clone()),
            retryable: false,
        })?;
        let intermediate = match outcome.execution {
            ExecutionStatus::Changed => TargetExecutionState::Changed,
            ExecutionStatus::Unchanged => TargetExecutionState::Unchanged,
            ExecutionStatus::Skipped => TargetExecutionState::Skipped,
            ExecutionStatus::Failed => TargetExecutionState::Failed,
        };
        self.record_target_state(operation_id, &outcome.target_id, intermediate, &payload)?;
        let final_state = target_execution_state(outcome);
        if final_state != intermediate {
            self.record_target_state(operation_id, &outcome.target_id, final_state, &payload)?;
        }
        Ok(())
    }

    pub fn list_unresolved_operations(&self) -> Result<Vec<UnresolvedOperation>, OperationError> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let mut statement = conn
            .prepare(
                "SELECT event.operation_id, event.state, event.payload_json
                 FROM operation_execution_events AS event
                 WHERE event.event_sequence = (
                    SELECT MAX(latest.event_sequence)
                    FROM operation_execution_events AS latest
                    WHERE latest.operation_id = event.operation_id
                 )
                   AND event.state IN ('execution_starting', 'executing', 'mutation_completed', 'verifying', 'outcome_unknown')
                   AND NOT EXISTS (
                    SELECT 1 FROM operation_journal AS terminal
                    WHERE terminal.operation_id = event.operation_id
                   )
                 ORDER BY event.event_sequence ASC",
            )
            .map_err(database_error)?;
        let rows = statement
            .query_map([], |row| {
                let operation_id: String = row.get(0)?;
                let state: String = row.get(1)?;
                let payload: String = row.get(2)?;
                Ok((operation_id, state, payload))
            })
            .map_err(database_error)?;
        let mut result = rows
            .map(|row| {
                let (operation_id, state, payload) = row.map_err(database_error)?;
                let state = execution_state_from_str(&state).ok_or_else(|| {
                    validation("operation journal contains an unknown execution state")
                })?;
                let confirmed_plan =
                    serde_json::from_str(&payload).map_err(|error| OperationError {
                        code: OperationErrorCode::SerializationFailed,
                        message: error.to_string(),
                        target_id: Some(operation_id.clone()),
                        retryable: false,
                    })?;
                Ok(UnresolvedOperation {
                    operation_id: OperationId(operation_id),
                    state,
                    confirmed_plan,
                    targets: Vec::new(),
                })
            })
            .collect::<Result<Vec<_>, OperationError>>()?;
        drop(statement);
        drop(conn);
        for operation in &mut result {
            operation.targets = self.list_unresolved_targets(&operation.operation_id)?;
        }
        Ok(result)
    }

    /// Mark work from a process that disappeared before it could append a
    /// terminal record as outcome-unknown. This preserves the unresolved
    /// record for review while preventing the next launch from presenting it
    /// as safely executing.
    ///
    /// Only genuinely ambiguous states are rewritten. Durable target
    /// observations such as `Changed`, `Unchanged`, `Failed`, or
    /// `VerificationSucceeded` are evidence and must survive reconciliation
    /// unchanged.
    pub fn reconcile_unresolved_operations(&self) -> Result<usize, OperationError> {
        Ok(self.reconcile_unresolved_operations_reporting()?.reconciled)
    }

    /// Reconcile unresolved operations one at a time so a single corrupt or
    /// problematic operation cannot strand independent later operations in an
    /// ambiguous `executing` state. Global listing failures still propagate;
    /// per-operation failures are recorded and iteration continues.
    pub fn reconcile_unresolved_operations_reporting(
        &self,
    ) -> Result<ReconciliationReport, OperationError> {
        let operations = self.list_unresolved_operation_ids()?;
        let mut report = ReconciliationReport::default();
        for operation_id in operations {
            match self.reconcile_one_unresolved_operation(&operation_id) {
                Ok(true) => report.reconciled += 1,
                Ok(false) => {}
                Err(error) => {
                    log::warn!(
                        "operation journal reconciliation skipped {}: {}",
                        operation_id.0,
                        error
                    );
                    report.failures.push(ReconciliationFailure {
                        operation_id,
                        message: error.to_string(),
                    });
                }
            }
        }
        Ok(report)
    }

    /// Operation ids whose latest execution event is unresolved and that have
    /// no terminal journal row. This intentionally does not parse the stored
    /// plan so a malformed payload cannot abort the whole listing.
    fn list_unresolved_operation_ids(&self) -> Result<Vec<OperationId>, OperationError> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let mut statement = conn
            .prepare(
                "SELECT event.operation_id
                 FROM operation_execution_events AS event
                 WHERE event.event_sequence = (
                    SELECT MAX(latest.event_sequence)
                    FROM operation_execution_events AS latest
                    WHERE latest.operation_id = event.operation_id
                 )
                   AND event.state IN ('execution_starting', 'executing', 'mutation_completed', 'verifying', 'outcome_unknown')
                   AND NOT EXISTS (
                    SELECT 1 FROM operation_journal AS terminal
                    WHERE terminal.operation_id = event.operation_id
                   )
                 ORDER BY event.event_sequence ASC",
            )
            .map_err(database_error)?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(database_error)?;
        rows.map(|row| row.map(OperationId).map_err(database_error))
            .collect()
    }

    fn reconcile_one_unresolved_operation(
        &self,
        operation_id: &OperationId,
    ) -> Result<bool, OperationError> {
        let mut changed = false;
        for target in self.list_unresolved_targets(operation_id)? {
            if target.state.is_ambiguous() {
                self.record_target_state(
                    operation_id,
                    &target.target_id,
                    TargetExecutionState::OutcomeUnknown,
                    &target.payload_json,
                )?;
                changed = true;
            }
        }
        if let Some(state) = self.latest_execution_state(operation_id)? {
            if state.is_ambiguous() {
                // The stored plan is required to write a well-formed execution
                // event. A corrupt payload fails this one operation only; the
                // caller reports it and continues with independent operations.
                let plan = self.latest_execution_plan(operation_id)?;
                self.record_execution_state(&plan, OperationExecutionState::OutcomeUnknown)?;
                changed = true;
            }
        }
        Ok(changed)
    }

    fn latest_execution_plan(
        &self,
        operation_id: &OperationId,
    ) -> Result<OperationPlan, OperationError> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let payload = conn
            .query_row(
                "SELECT payload_json FROM operation_execution_events
                 WHERE operation_id = ?1 ORDER BY event_sequence DESC LIMIT 1",
                params![operation_id.0],
                |row| row.get::<_, String>(0),
            )
            .map_err(database_error)?;
        serde_json::from_str(&payload).map_err(|error| OperationError {
            code: OperationErrorCode::SerializationFailed,
            message: format!(
                "operation {} has an unreadable execution plan: {error}",
                operation_id.0
            ),
            target_id: Some(operation_id.0.clone()),
            retryable: false,
        })
    }

    pub fn list(&self) -> Result<Vec<OperationJournalRecord>, OperationError> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let mut statement = conn
            .prepare("SELECT record_json FROM operation_journal ORDER BY timestamp DESC")
            .map_err(database_error)?;
        let records = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(database_error)?
            .map(|row| {
                let json = row.map_err(database_error)?;
                deserialize_record(&json, None)
            })
            .collect();
        records
    }

    pub fn get(&self, journal_id: &str) -> Result<OperationJournalRecord, OperationError> {
        let conn = self.conn.lock().unwrap_or_else(|error| error.into_inner());
        let json = conn
            .query_row(
                "SELECT record_json FROM operation_journal WHERE journal_id = ?1",
                params![journal_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(database_error)?
            .ok_or_else(|| OperationError {
                code: OperationErrorCode::TargetNotFound,
                message: "operation journal record was not found".into(),
                target_id: Some(journal_id.into()),
                retryable: false,
            })?;
        deserialize_record(&json, Some(journal_id.into()))
    }
}

fn deserialize_record(
    json: &str,
    target_id: Option<String>,
) -> Result<OperationJournalRecord, OperationError> {
    let record =
        serde_json::from_str::<OperationJournalRecord>(json).map_err(|error| OperationError {
            code: OperationErrorCode::SerializationFailed,
            message: error.to_string(),
            target_id: target_id.clone(),
            retryable: false,
        })?;
    record.validate().map_err(|mut error| {
        if error.target_id.is_none() {
            error.target_id = target_id;
        }
        error
    })?;
    Ok(record)
}

fn insert_target_event(
    transaction: &rusqlite::Transaction<'_>,
    operation_id: &OperationId,
    target_id: &str,
    state: TargetExecutionState,
    payload: &str,
) -> Result<(), OperationError> {
    transaction
        .execute(
            "INSERT INTO operation_target_states
                (target_event_id, operation_id, target_id, state, timestamp, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                uuid::Uuid::new_v4().to_string(),
                operation_id.0,
                target_id,
                target_execution_state_name(state),
                chrono::Utc::now().to_rfc3339(),
                payload
            ],
        )
        .map_err(database_error)?;
    Ok(())
}

fn insert_relationship(
    transaction: &rusqlite::Transaction<'_>,
    plan: &OperationPlan,
    timestamp: &str,
) -> Result<(), OperationError> {
    if plan.parent_operation_id.is_none() && plan.retry_of_operation_id.is_none() {
        return Ok(());
    }
    transaction
        .execute(
            "INSERT INTO operation_relationships
                (relationship_id, operation_id, parent_operation_id, retry_of_operation_id, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(operation_id) DO UPDATE SET
                parent_operation_id = excluded.parent_operation_id,
                retry_of_operation_id = excluded.retry_of_operation_id,
                timestamp = excluded.timestamp",
            params![
                uuid::Uuid::new_v4().to_string(),
                plan.id.0,
                plan.parent_operation_id.as_ref().map(|id| &id.0),
                plan.retry_of_operation_id.as_ref().map(|id| &id.0),
                timestamp,
            ],
        )
        .map_err(database_error)?;
    Ok(())
}

fn target_execution_state(outcome: &TargetOutcome) -> TargetExecutionState {
    match outcome.execution {
        ExecutionStatus::Changed => match outcome.verification {
            VerificationStatus::Verified => TargetExecutionState::VerificationSucceeded,
            VerificationStatus::Failed => TargetExecutionState::VerificationFailed,
            VerificationStatus::Unverified | VerificationStatus::NotApplicable => {
                TargetExecutionState::OutcomeUnknown
            }
        },
        ExecutionStatus::Unchanged => match outcome.verification {
            VerificationStatus::Verified | VerificationStatus::NotApplicable => {
                TargetExecutionState::Unchanged
            }
            VerificationStatus::Failed => TargetExecutionState::VerificationFailed,
            VerificationStatus::Unverified => TargetExecutionState::OutcomeUnknown,
        },
        ExecutionStatus::Skipped => TargetExecutionState::Skipped,
        ExecutionStatus::Failed => TargetExecutionState::Failed,
    }
}

fn target_state_can_transition(current: TargetExecutionState, next: TargetExecutionState) -> bool {
    current == next
        || matches!(
            (current, next),
            (
                // `Ready` is the durable revalidation-complete marker. A target
                // must pass through it before any mutation may be recorded, so
                // `requires_revalidation` cannot be silently skipped.
                TargetExecutionState::Pending,
                TargetExecutionState::RevalidationFailed
                    | TargetExecutionState::Ready
                    | TargetExecutionState::OutcomeUnknown
                    | TargetExecutionState::VerificationSucceeded
            ) | (
                TargetExecutionState::RevalidationFailed,
                TargetExecutionState::OutcomeUnknown | TargetExecutionState::VerificationSucceeded
            ) | (
                TargetExecutionState::Ready,
                TargetExecutionState::MutationStarted
                    | TargetExecutionState::Skipped
                    | TargetExecutionState::Failed
                    | TargetExecutionState::OutcomeUnknown
                    | TargetExecutionState::VerificationSucceeded
            ) | (
                TargetExecutionState::MutationStarted,
                TargetExecutionState::Changed
                    | TargetExecutionState::Unchanged
                    | TargetExecutionState::Skipped
                    | TargetExecutionState::Failed
                    | TargetExecutionState::OutcomeUnknown
                    | TargetExecutionState::VerificationSucceeded
            ) | (
                TargetExecutionState::Changed | TargetExecutionState::Unchanged,
                TargetExecutionState::VerificationSucceeded
                    | TargetExecutionState::VerificationFailed
                    | TargetExecutionState::OutcomeUnknown
            ) | (
                TargetExecutionState::VerificationFailed,
                TargetExecutionState::OutcomeUnknown | TargetExecutionState::VerificationSucceeded
            ) | (
                TargetExecutionState::Failed,
                TargetExecutionState::OutcomeUnknown | TargetExecutionState::VerificationSucceeded
            ) | (
                TargetExecutionState::OutcomeUnknown,
                TargetExecutionState::VerificationSucceeded
            )
        )
}

fn target_execution_state_name(state: TargetExecutionState) -> &'static str {
    match state {
        TargetExecutionState::Pending => "pending",
        TargetExecutionState::RevalidationFailed => "revalidation_failed",
        TargetExecutionState::Ready => "ready",
        TargetExecutionState::MutationStarted => "mutation_started",
        TargetExecutionState::Changed => "changed",
        TargetExecutionState::Unchanged => "unchanged",
        TargetExecutionState::Skipped => "skipped",
        TargetExecutionState::VerificationSucceeded => "verification_succeeded",
        TargetExecutionState::VerificationFailed => "verification_failed",
        TargetExecutionState::Failed => "failed",
        TargetExecutionState::OutcomeUnknown => "outcome_unknown",
    }
}

fn target_execution_state_from_str(state: &str) -> Option<TargetExecutionState> {
    Some(match state {
        "pending" => TargetExecutionState::Pending,
        "revalidation_failed" => TargetExecutionState::RevalidationFailed,
        "ready" => TargetExecutionState::Ready,
        "mutation_started" => TargetExecutionState::MutationStarted,
        "changed" => TargetExecutionState::Changed,
        "unchanged" => TargetExecutionState::Unchanged,
        "skipped" => TargetExecutionState::Skipped,
        "verification_succeeded" => TargetExecutionState::VerificationSucceeded,
        "verification_failed" => TargetExecutionState::VerificationFailed,
        "failed" => TargetExecutionState::Failed,
        "outcome_unknown" => TargetExecutionState::OutcomeUnknown,
        _ => return None,
    })
}

fn insert_execution_event(
    transaction: &rusqlite::Transaction<'_>,
    operation_id: &OperationId,
    state: OperationExecutionState,
    payload: &str,
) -> Result<(), OperationError> {
    let current: Option<String> = transaction
        .query_row(
            "SELECT state FROM operation_execution_events
             WHERE operation_id = ?1
             ORDER BY event_sequence DESC LIMIT 1",
            params![operation_id.0],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error)?;
    if let Some(current) = current {
        let current = execution_state_from_str(&current)
            .ok_or_else(|| validation("operation journal contains an unknown execution state"))?;
        if !current.can_transition_to(state) {
            return Err(validation(format!(
                "forbidden operation transition: {current:?} -> {state:?}"
            )));
        }
    }
    transaction
        .execute(
            "INSERT INTO operation_execution_events
                (event_id, operation_id, state, timestamp, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                uuid::Uuid::new_v4().to_string(),
                operation_id.0,
                execution_state_name(state),
                chrono::Utc::now().to_rfc3339(),
                payload
            ],
        )
        .map_err(database_error)?;
    Ok(())
}

fn execution_state_name(state: OperationExecutionState) -> &'static str {
    match state {
        OperationExecutionState::Planned => "planned",
        OperationExecutionState::AwaitingConfirmation => "awaiting_confirmation",
        OperationExecutionState::Confirmed => "confirmed",
        OperationExecutionState::ExecutionStarting => "execution_starting",
        OperationExecutionState::Executing => "executing",
        OperationExecutionState::MutationCompleted => "mutation_completed",
        OperationExecutionState::Verifying => "verifying",
        OperationExecutionState::Succeeded => "succeeded",
        OperationExecutionState::PartiallySucceeded => "partially_succeeded",
        OperationExecutionState::Failed => "failed",
        OperationExecutionState::OutcomeUnknown => "outcome_unknown",
        OperationExecutionState::CancelledBeforeMutation => "cancelled_before_mutation",
    }
}

fn execution_state_from_str(state: &str) -> Option<OperationExecutionState> {
    Some(match state {
        "planned" => OperationExecutionState::Planned,
        "awaiting_confirmation" => OperationExecutionState::AwaitingConfirmation,
        "confirmed" => OperationExecutionState::Confirmed,
        "execution_starting" => OperationExecutionState::ExecutionStarting,
        "executing" => OperationExecutionState::Executing,
        "mutation_completed" => OperationExecutionState::MutationCompleted,
        "verifying" => OperationExecutionState::Verifying,
        "succeeded" => OperationExecutionState::Succeeded,
        "partially_succeeded" => OperationExecutionState::PartiallySucceeded,
        "failed" => OperationExecutionState::Failed,
        "outcome_unknown" => OperationExecutionState::OutcomeUnknown,
        "cancelled_before_mutation" => OperationExecutionState::CancelledBeforeMutation,
        _ => return None,
    })
}

fn terminal_execution_state(state: OperationState) -> OperationExecutionState {
    match state {
        OperationState::PartiallySucceeded => OperationExecutionState::PartiallySucceeded,
        OperationState::Failed => OperationExecutionState::Failed,
        OperationState::Interrupted => OperationExecutionState::OutcomeUnknown,
        OperationState::Cancelled => OperationExecutionState::CancelledBeforeMutation,
        _ => OperationExecutionState::Succeeded,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UndoPreview {
    pub source_record_id: String,
    pub plan: OperationPlan,
    pub lifecycle: Vec<OperationState>,
}

pub fn preview_undo(record: &OperationJournalRecord) -> Result<UndoPreview, OperationError> {
    record.validate()?;
    if !record.ui_undo_available() {
        return Err(validation(record.undo_eligibility.reason.clone()));
    }
    let targets = record
        .captured_original_states
        .iter()
        .map(|capture| PlannedTarget {
            target_id: capture.target_id().to_owned(),
            requested_action: "restore_captured_original_state".into(),
            description: format!(
                "Restore captured original state for {}",
                capture.target_id()
            ),
        })
        .collect::<Vec<_>>();
    if targets.is_empty() {
        return Err(validation("undo has no captured original states"));
    }
    let descriptor = OperationDescriptor {
        id: OperationId(format!("undo.{}", record.journal_id)),
        capability_id: "history".into(),
        title: format!("Undo {}", record.confirmed_plan.descriptor.title),
        category: record.confirmed_plan.descriptor.category,
        risk: record.confirmed_plan.descriptor.risk,
        privilege: record.confirmed_plan.descriptor.privilege,
        reversibility: Reversibility::NotReversible,
        reboot_policy: record.restart_requirement,
        experimental: record.confirmed_plan.descriptor.experimental,
        proof_status: record.confirmed_plan.descriptor.proof_status,
        exposure: record.confirmed_plan.descriptor.exposure,
        recovery: RecoveryLevel::Exact,
        restore_point_policy: record.confirmed_plan.descriptor.restore_point_policy,
        mutates_system: true,
        requires_confirmation: true,
        requires_revalidation: true,
        requires_verification: true,
        requires_journal: true,
    };
    let plan = OperationPlan {
        id: descriptor.id.clone(),
        descriptor,
        targets,
        warnings: vec![OperationWarning {
            code: "captured_state_restore".into(),
            message: "Exact undo restores the captured original state and verifies it; a restore point is independent.".into(),
        }],
        expected_effects: record
            .captured_original_states
            .iter()
            .map(|capture| ExpectedEffect {
                target_id: capture.target_id().into(),
                description: "Exact recovery restores and verifies the captured original state."
                    .into(),
            })
            .collect(),
        requires_elevation: record.confirmed_plan.requires_elevation,
        requires_restart: record.confirmed_plan.requires_restart,
        reversibility: Reversibility::NotReversible,
        parent_operation_id: None,
        retry_of_operation_id: None,
    };
    let mut lifecycle = OperationStateMachine::default();
    lifecycle.transition(OperationState::Previewing)?;
    lifecycle.transition(OperationState::AwaitingConfirmation)?;
    Ok(UndoPreview {
        source_record_id: record.journal_id.clone(),
        plan,
        lifecycle: lifecycle.history,
    })
}

pub fn execute_synthetic_undo(
    source: &OperationJournalRecord,
    confirmed_plan: &OperationPlan,
) -> Result<OperationJournalRecord, OperationError> {
    let outcomes = source
        .captured_original_states
        .iter()
        .map(|capture| {
            let before = match capture {
                CapturedOriginalState::Synthetic { state, .. } => state.clone(),
                _ => {
                    return Err(validation(
                        "this journal phase only executes tested synthetic restorations",
                    ))
                }
            };
            Ok(TargetOutcome {
                target_id: capture.target_id().into(),
                requested_action: "restore_captured_original_state".into(),
                before: None,
                after: Some(before),
                execution: ExecutionStatus::Changed,
                verification: VerificationStatus::Verified,
                error: None,
            })
        })
        .collect::<Result<Vec<_>, OperationError>>()?;
    finish_captured_undo(source, confirmed_plan, outcomes)
}

pub fn finish_captured_undo(
    source: &OperationJournalRecord,
    confirmed_plan: &OperationPlan,
    outcomes: Vec<TargetOutcome>,
) -> Result<OperationJournalRecord, OperationError> {
    let preview = preview_undo(source)?;
    if &preview.plan != confirmed_plan {
        return Err(validation("confirmed undo plan does not match the preview"));
    }
    if outcomes.len() != source.captured_original_states.len()
        || outcomes
            .iter()
            .zip(&source.captured_original_states)
            .any(|(outcome, capture)| outcome.target_id != capture.target_id())
    {
        return Err(validation("undo outcomes do not match captured targets"));
    }
    let mut lifecycle = OperationStateMachine {
        current: OperationState::AwaitingConfirmation,
        history: preview.lifecycle,
    };
    if confirmed_plan.requires_elevation {
        lifecycle.transition(OperationState::AwaitingElevation)?;
    }
    lifecycle.transition(OperationState::Running)?;
    lifecycle.transition(OperationState::Verifying)?;
    lifecycle.transition(
        if outcomes.iter().any(|outcome| {
            outcome.execution == ExecutionStatus::Failed
                || outcome.verification != VerificationStatus::Verified
        }) {
            OperationState::Failed
        } else {
            OperationState::Succeeded
        },
    )?;
    lifecycle.transition(OperationState::NotReversible)?;
    let result = OperationResult::new(
        confirmed_plan.id.clone(),
        outcomes.clone(),
        confirmed_plan.requires_restart,
        UndoAvailability::NotReversible,
    )?;
    let execution = UndoExecutionResult {
        source_record_id: source.journal_id.clone(),
        lifecycle: lifecycle.history,
        result,
    };
    Ok(OperationJournalRecord {
        journal_id: uuid::Uuid::new_v4().to_string(),
        record_kind: JournalRecordKind::Undo,
        related_record_id: Some(source.journal_id.clone()),
        operation_id: confirmed_plan.id.clone(),
        operation_schema_version: OPERATION_SCHEMA_VERSION,
        application_version: env!("CARGO_PKG_VERSION").into(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        terminal_state: OperationState::NotReversible,
        confirmed_plan: confirmed_plan.clone(),
        requested_targets: confirmed_plan.targets.clone(),
        captured_original_states: Vec::new(),
        target_outcomes: outcomes.clone(),
        verification_results: outcomes
            .iter()
            .map(|outcome| VerificationRecord {
                target_id: outcome.target_id.clone(),
                status: outcome.verification,
                message: "Exact recovery restored and verified the captured original state.".into(),
            })
            .collect(),
        restart_requirement: source.restart_requirement,
        undo_eligibility: UndoEligibility::unavailable(
            "Undo records are not recursively reversible.",
        ),
        undo_execution_result: Some(execution),
        diagnostic: None,
    })
}

pub fn execute_confirmed_synthetic_operation(
    confirmed_plan: &OperationPlan,
) -> Result<OperationJournalRecord, OperationError> {
    let contract = crate::core::operation::synthetic_operation_contract()?;
    if confirmed_plan != &contract.plan {
        return Err(validation(
            "resumed execution does not match the confirmed synthetic plan",
        ));
    }
    let capture = CapturedOriginalState::Synthetic {
        target_id: "synthetic.changed".into(),
        state: contract.result.outcomes[0]
            .before
            .clone()
            .ok_or_else(|| validation("synthetic operation lacks original state"))?,
    };
    Ok(OperationJournalRecord {
        journal_id: uuid::Uuid::new_v4().to_string(),
        record_kind: JournalRecordKind::Operation,
        related_record_id: None,
        operation_id: confirmed_plan.id.clone(),
        operation_schema_version: OPERATION_SCHEMA_VERSION,
        application_version: env!("CARGO_PKG_VERSION").into(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        terminal_state: contract.result.completion_state(),
        confirmed_plan: confirmed_plan.clone(),
        requested_targets: confirmed_plan.targets.clone(),
        captured_original_states: vec![capture],
        target_outcomes: contract.result.outcomes.clone(),
        verification_results: contract
            .result
            .outcomes
            .iter()
            .map(|outcome| VerificationRecord {
                target_id: outcome.target_id.clone(),
                status: outcome.verification,
                message: format!("{:?}", outcome.verification),
            })
            .collect(),
        restart_requirement: RebootPolicy::NotRequired,
        undo_eligibility: UndoEligibility::exact(
            "operation_journal_tests::undo_restores_captured_original_state",
        ),
        undo_execution_result: None,
        diagnostic: contract.result.outcomes[1].error.clone(),
    })
}

fn validation(message: impl Into<String>) -> OperationError {
    OperationError::validation(message)
}

#[cfg(any(test, debug_assertions))]
fn failure_injection_error(hook: &str) -> OperationError {
    OperationError {
        code: OperationErrorCode::InternalError,
        message: format!("failure injection: {hook}"),
        target_id: None,
        retryable: true,
    }
}

fn database_error(error: rusqlite::Error) -> OperationError {
    OperationError {
        code: OperationErrorCode::InternalError,
        message: error.to_string(),
        target_id: None,
        retryable: true,
    }
}

#[cfg(test)]
#[path = "operation_journal_tests.rs"]
mod tests;
