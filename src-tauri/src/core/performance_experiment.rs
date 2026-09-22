//! Durable local evidence for machine-specific performance experiments.
//!
//! The frontend owns the experiment comparison rules, but this store keeps
//! the complete document (including raw samples and conclusions) across app
//! restarts. It records evidence only; it never applies or reverts a system
//! operation.

use super::operation::{
    ExecutionStatus, OperationError, OperationErrorCode, OperationPlan, OperationState,
    OperationWarning, StateSnapshot, VerificationStatus,
};
use super::operation_journal::{JournalRecordKind, OperationJournal, OperationJournalRecord};
use super::system_tweak_operation::{self, SystemTweakTarget, TweakKind};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, Mutex};
use thiserror::Error;

pub const PERFORMANCE_EXPERIMENT_SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceTargetEvidence {
    pub target_id: String,
    pub execution: ExecutionStatus,
    pub verification: VerificationStatus,
    pub before: Option<StateSnapshot>,
    pub after: Option<StateSnapshot>,
    pub verification_message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerformanceOperationEvidence {
    pub operation_id: String,
    pub journal_id: String,
    pub descriptor_id: String,
    pub terminal_state: OperationState,
    pub captured_at: String,
    pub readback_verified: bool,
    pub targets: Vec<PerformanceTargetEvidence>,
}

#[derive(Debug, Error)]
pub enum PerformanceExperimentError {
    #[error("performance experiment storage I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("performance experiment database failed: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("performance experiment serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("performance experiment data is invalid: {0}")]
    InvalidData(String),
}

#[derive(Clone)]
pub struct PerformanceExperimentStore {
    connection: Arc<Mutex<Connection>>,
}

impl PerformanceExperimentStore {
    pub fn open_default() -> Result<Self, PerformanceExperimentError> {
        let path = crate::core::paths::AppPaths::installed()?.performance_experiments;
        if let Some(directory) = path.parent() {
            std::fs::create_dir_all(directory)?;
        }
        Self::open_at(&path)
    }

    pub fn open_at(path: &Path) -> Result<Self, PerformanceExperimentError> {
        let connection = Connection::open(path)?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=FULL;
             PRAGMA busy_timeout=5000;
             CREATE TABLE IF NOT EXISTS performance_experiment_meta (
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );
             INSERT INTO performance_experiment_meta(key, value)
                 VALUES ('schema_version', '1')
                 ON CONFLICT(key) DO NOTHING;
             CREATE TABLE IF NOT EXISTS performance_experiments (
                 experiment_id TEXT PRIMARY KEY,
                 operation_id TEXT,
                 payload_json TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_performance_experiments_updated
                 ON performance_experiments(updated_at DESC);
             CREATE INDEX IF NOT EXISTS idx_performance_experiments_operation
                 ON performance_experiments(operation_id);",
        )?;
        let schema_version: String = connection.query_row(
            "SELECT value FROM performance_experiment_meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )?;
        if schema_version.parse::<i64>().ok() != Some(PERFORMANCE_EXPERIMENT_SCHEMA_VERSION) {
            return Err(PerformanceExperimentError::InvalidData(
                "unsupported performance experiment schema version".into(),
            ));
        }
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub fn save(&self, experiment: Value) -> Result<Value, PerformanceExperimentError> {
        let (experiment_id, operation_id, created_at) = validate_document(&experiment)?;
        let payload_json = serde_json::to_string(&experiment)?;
        let updated_at = chrono::Utc::now().to_rfc3339();
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        connection.execute(
            "INSERT INTO performance_experiments(
                 experiment_id, operation_id, payload_json, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(experiment_id) DO UPDATE SET
                 operation_id = excluded.operation_id,
                 payload_json = excluded.payload_json,
                 updated_at = excluded.updated_at",
            params![
                experiment_id,
                operation_id,
                payload_json,
                created_at,
                updated_at
            ],
        )?;
        Ok(experiment)
    }

    pub fn list(&self) -> Result<Vec<Value>, PerformanceExperimentError> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut statement = connection.prepare(
            "SELECT payload_json FROM performance_experiments
             ORDER BY updated_at DESC, experiment_id ASC",
        )?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .map(|row| {
                let payload = row?;
                serde_json::from_str::<Value>(&payload).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })
            })
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows)
    }
}

pub fn linked_operation_id(
    experiment: &Value,
) -> Result<Option<String>, PerformanceExperimentError> {
    let object = experiment.as_object().ok_or_else(|| {
        PerformanceExperimentError::InvalidData("experiment must be an object".into())
    })?;
    match object.get("operationId") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => {
            let value = value.trim();
            if value.is_empty() {
                Ok(None)
            } else {
                Ok(Some(value.to_owned()))
            }
        }
        Some(_) => Err(PerformanceExperimentError::InvalidData(
            "operationId must be a string or null".into(),
        )),
    }
}

pub(crate) fn find_operation_record(
    journal: &OperationJournal,
    operation_id: &str,
) -> Result<OperationJournalRecord, OperationError> {
    let operation_id = operation_id.trim();
    if operation_id.is_empty() {
        return Err(OperationError::validation(
            "a linked operation id is required",
        ));
    }
    journal
        .list()?
        .into_iter()
        .find(|record| record.operation_id.0 == operation_id)
        .ok_or_else(|| OperationError {
            code: OperationErrorCode::TargetNotFound,
            message: format!("operation journal record '{operation_id}' was not found"),
            target_id: Some(operation_id.into()),
            retryable: false,
        })
}

fn verified_operation_evidence(
    record: &OperationJournalRecord,
) -> Result<PerformanceOperationEvidence, OperationError> {
    record.validate()?;
    if record.record_kind != JournalRecordKind::Operation {
        return Err(OperationError::validation(
            "performance experiments must link to an original operation record",
        ));
    }
    if record.terminal_state != OperationState::Succeeded {
        return Err(OperationError {
            code: OperationErrorCode::VerificationFailed,
            message: "the linked operation did not finish with a verified success".into(),
            target_id: Some(record.operation_id.0.clone()),
            retryable: true,
        });
    }
    if record.target_outcomes.is_empty() {
        return Err(OperationError::validation(
            "the linked operation has no target read-back evidence",
        ));
    }

    let mut targets = Vec::with_capacity(record.target_outcomes.len());
    for outcome in &record.target_outcomes {
        let verification = record
            .verification_results
            .iter()
            .find(|verification| verification.target_id == outcome.target_id)
            .ok_or_else(|| {
                OperationError::validation(format!(
                    "the linked operation is missing read-back evidence for {}",
                    outcome.target_id
                ))
            })?;
        if outcome.execution == ExecutionStatus::Failed
            || outcome.verification != VerificationStatus::Verified
            || verification.status != VerificationStatus::Verified
            || outcome.before.is_none()
            || outcome.after.is_none()
        {
            return Err(OperationError {
                code: OperationErrorCode::VerificationFailed,
                message: format!(
                    "linked operation target '{}' does not have complete verified read-back",
                    outcome.target_id
                ),
                target_id: Some(outcome.target_id.clone()),
                retryable: true,
            });
        }
        targets.push(PerformanceTargetEvidence {
            target_id: outcome.target_id.clone(),
            execution: outcome.execution,
            verification: outcome.verification,
            before: outcome.before.clone(),
            after: outcome.after.clone(),
            verification_message: verification.message.clone(),
        });
    }

    Ok(PerformanceOperationEvidence {
        operation_id: record.operation_id.0.clone(),
        journal_id: record.journal_id.clone(),
        descriptor_id: record.confirmed_plan.descriptor.id.0.clone(),
        terminal_state: record.terminal_state,
        captured_at: record.timestamp.clone(),
        readback_verified: true,
        targets,
    })
}

fn performance_source_target(record: &OperationJournalRecord) -> Result<String, OperationError> {
    if record.confirmed_plan.descriptor.id.0 != "system_tweak.change"
        || record.confirmed_plan.targets.len() != 1
    {
        return Err(OperationError::validation(
            "the linked operation is not a single performance-tweak plan",
        ));
    }
    let target = &record.confirmed_plan.targets[0];
    let Some(tweak_id) = target.target_id.strip_prefix("performance:") else {
        return Err(OperationError::validation(
            "the linked operation target is not a performance tweak",
        ));
    };
    if tweak_id.trim().is_empty() {
        return Err(OperationError::validation(
            "the linked performance tweak id is empty",
        ));
    }
    let requested_action: Value =
        serde_json::from_str(&target.requested_action).map_err(|error| OperationError {
            code: OperationErrorCode::SerializationFailed,
            message: format!("the linked performance plan is invalid: {error}"),
            target_id: Some(target.target_id.clone()),
            retryable: false,
        })?;
    if requested_action.get("apply").and_then(Value::as_bool) != Some(true) {
        return Err(OperationError::validation(
            "the linked operation did not apply the performance tweak",
        ));
    }
    Ok(target.target_id.clone())
}

pub(crate) fn create_revert_plan(
    journal: &OperationJournal,
    operation_id: &str,
) -> Result<OperationPlan, OperationError> {
    let source = find_operation_record(journal, operation_id)?;
    verified_operation_evidence(&source)?;
    let target_id = performance_source_target(&source)?;
    let tweak_id = target_id
        .strip_prefix("performance:")
        .ok_or_else(|| OperationError::validation("performance target id is invalid"))?;
    let current = system_tweak_operation::find_target(&TweakKind::Performance, tweak_id)?;
    build_revert_plan(&source, &target_id, current)
}

fn build_revert_plan(
    source: &OperationJournalRecord,
    target_id: &str,
    current: SystemTweakTarget,
) -> Result<OperationPlan, OperationError> {
    if format!("performance:{}", current.id) != target_id || !current.is_applied {
        return Err(OperationError {
            code: OperationErrorCode::TargetChangedSincePreview,
            message: "the performance tweak is no longer applied; no revert plan was created"
                .into(),
            target_id: Some(target_id.into()),
            retryable: true,
        });
    }

    let mut plan = system_tweak_operation::create_plan(&current)?;
    plan.parent_operation_id = Some(source.operation_id.clone());
    plan.warnings.push(OperationWarning {
        code: "performance_experiment_revert".into(),
        message: "This revert is a new normal operation linked to the experiment's applied operation; current state will be revalidated before mutation and read back afterward.".into(),
    });
    plan.validate_relationships()?;
    Ok(plan)
}

pub(crate) fn finalize_decision(
    mut experiment: Value,
    decision: &str,
    decision_operation_id: Option<&str>,
    journal: &OperationJournal,
) -> Result<Value, OperationError> {
    let state = experiment
        .get("state")
        .and_then(Value::as_str)
        .ok_or_else(|| OperationError::validation("experiment state is required"))?;
    if state != "compared" {
        return Err(OperationError::validation(
            "compare the experiment before recording Keep or Revert",
        ));
    }
    if experiment.get("comparison").is_none_or(Value::is_null) {
        return Err(OperationError::validation(
            "comparison evidence is required before recording a decision",
        ));
    }
    let source_id = linked_operation_id(&experiment)
        .map_err(|error| OperationError::validation(error.to_string()))?
        .ok_or_else(|| {
            OperationError::validation(
                "Keep or Revert requires the reviewed operation id that applied the tweak",
            )
        })?;
    let source = find_operation_record(journal, &source_id)?;
    let source_evidence = verified_operation_evidence(&source)?;
    let source_target_id = performance_source_target(&source)?;
    let decision_operation_id = decision_operation_id
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let object = experiment
        .as_object_mut()
        .ok_or_else(|| OperationError::validation("experiment must be an object"))?;

    match decision {
        "keep" => {
            if decision_operation_id.is_some() {
                return Err(OperationError::validation(
                    "Keep cannot include a revert operation id",
                ));
            }
            object.insert("state".into(), Value::String("keep_recorded".into()));
            object.insert("decision".into(), Value::String("keep".into()));
            object.insert(
                "operationEvidence".into(),
                serde_json::to_value(source_evidence).map_err(|error| OperationError {
                    code: OperationErrorCode::SerializationFailed,
                    message: error.to_string(),
                    target_id: Some(source_id),
                    retryable: false,
                })?,
            );
            object.insert("decisionOperationId".into(), Value::Null);
            object.insert("decisionEvidence".into(), Value::Null);
        }
        "revert" => {
            let decision_operation_id = decision_operation_id.ok_or_else(|| {
                OperationError::validation(
                    "Revert must be finalized with the new journaled revert operation id",
                )
            })?;
            let revert = find_operation_record(journal, decision_operation_id)?;
            let revert_evidence = verified_operation_evidence(&revert)?;
            if revert.confirmed_plan.descriptor.id.0 != "system_tweak.change"
                || revert.confirmed_plan.parent_operation_id.as_ref() != Some(&source.operation_id)
                || revert.confirmed_plan.targets.len() != 1
                || revert.confirmed_plan.targets[0].target_id != source_target_id
            {
                return Err(OperationError::validation(
                    "the revert operation is not linked to the reviewed performance plan",
                ));
            }
            let requested_action: Value = serde_json::from_str(
                &revert.confirmed_plan.targets[0].requested_action,
            )
            .map_err(|error| OperationError {
                code: OperationErrorCode::SerializationFailed,
                message: format!("the revert plan is invalid: {error}"),
                target_id: Some(source_target_id.clone()),
                retryable: false,
            })?;
            if requested_action.get("apply").and_then(Value::as_bool) != Some(false) {
                return Err(OperationError::validation(
                    "the linked revert operation did not request the disabled state",
                ));
            }
            object.insert("state".into(), Value::String("revert_requested".into()));
            object.insert("decision".into(), Value::String("revert".into()));
            object.insert(
                "operationEvidence".into(),
                serde_json::to_value(source_evidence).map_err(|error| OperationError {
                    code: OperationErrorCode::SerializationFailed,
                    message: error.to_string(),
                    target_id: Some(source_id.clone()),
                    retryable: false,
                })?,
            );
            object.insert(
                "decisionOperationId".into(),
                Value::String(decision_operation_id.into()),
            );
            object.insert(
                "decisionEvidence".into(),
                serde_json::to_value(revert_evidence).map_err(|error| OperationError {
                    code: OperationErrorCode::SerializationFailed,
                    message: error.to_string(),
                    target_id: Some(decision_operation_id.into()),
                    retryable: false,
                })?,
            );
        }
        _ => {
            return Err(OperationError::validation(
                "performance experiment decision must be Keep or Revert",
            ));
        }
    }
    Ok(experiment)
}

fn has_verified_evidence(object: &serde_json::Map<String, Value>, field: &str) -> bool {
    object
        .get(field)
        .and_then(Value::as_object)
        .and_then(|evidence| evidence.get("readbackVerified"))
        .and_then(Value::as_bool)
        == Some(true)
}

fn validate_document(
    experiment: &Value,
) -> Result<(String, Option<String>, String), PerformanceExperimentError> {
    let object = experiment.as_object().ok_or_else(|| {
        PerformanceExperimentError::InvalidData("experiment must be an object".into())
    })?;
    let experiment_id = object
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| PerformanceExperimentError::InvalidData("experiment id is required".into()))?
        .to_owned();
    let created_at = object
        .get("createdAt")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            PerformanceExperimentError::InvalidData("experiment createdAt is required".into())
        })?
        .to_owned();
    for phase in ["baseline", "after"] {
        if !object.get(phase).is_some_and(Value::is_array) {
            return Err(PerformanceExperimentError::InvalidData(format!(
                "experiment {phase} samples must be an array"
            )));
        }
    }
    if let Some(state) = object.get("state") {
        let state = state.as_str().ok_or_else(|| {
            PerformanceExperimentError::InvalidData("experiment state must be a string".into())
        })?;
        if !matches!(
            state,
            "draft"
                | "collecting_baseline"
                | "awaiting_reboot"
                | "awaiting_after"
                | "collecting_after"
                | "compared"
                | "keep_recorded"
                | "revert_requested"
                | "unknown"
        ) {
            return Err(PerformanceExperimentError::InvalidData(
                "unsupported performance experiment state".into(),
            ));
        }
        match state {
            "keep_recorded" => {
                if object.get("decision").and_then(Value::as_str) != Some("keep")
                    || !has_verified_evidence(object, "operationEvidence")
                {
                    return Err(PerformanceExperimentError::InvalidData(
                        "Keep requires verified operation read-back evidence".into(),
                    ));
                }
            }
            "revert_requested" => {
                if object.get("decision").and_then(Value::as_str) != Some("revert")
                    || object
                        .get("decisionOperationId")
                        .and_then(Value::as_str)
                        .is_none_or(|value| value.trim().is_empty())
                    || !has_verified_evidence(object, "operationEvidence")
                    || !has_verified_evidence(object, "decisionEvidence")
                {
                    return Err(PerformanceExperimentError::InvalidData(
                        "Revert requires verified apply and revert read-back evidence".into(),
                    ));
                }
            }
            _ => {}
        }
    }
    if let Some(reboot_required) = object.get("rebootRequired") {
        if !reboot_required.is_boolean() {
            return Err(PerformanceExperimentError::InvalidData(
                "experiment rebootRequired must be a boolean".into(),
            ));
        }
    }
    Ok((experiment_id, linked_operation_id(experiment)?, created_at))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::operation::{
        ExpectedEffect, ExposureLevel, OperationCategory, OperationDescriptor, OperationId,
        OperationPlan, PlannedTarget, PrivilegeRequirement, ProofStatus, RebootPolicy,
        Reversibility, RiskLevel, TargetOutcome,
    };
    use crate::core::operation_journal::{
        CapturedOriginalState, OperationJournalRecord, UndoEligibility, VerificationRecord,
        OPERATION_SCHEMA_VERSION,
    };
    use serde_json::json;
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    fn operation_record(
        operation_id: &str,
        parent_operation_id: Option<&str>,
        apply: bool,
    ) -> OperationJournalRecord {
        let target_id = "performance:fixture".to_owned();
        let requested_action = json!({"apply": apply}).to_string();
        let target = PlannedTarget {
            target_id: target_id.clone(),
            requested_action,
            description: "Fixture performance tweak".into(),
        };
        let descriptor = OperationDescriptor {
            id: OperationId("system_tweak.change".into()),
            capability_id: "tweaks.system".into(),
            title: "Fixture performance tweak".into(),
            category: OperationCategory::SystemTweak,
            risk: RiskLevel::Low,
            privilege: PrivilegeRequirement::StandardUser,
            reversibility: Reversibility::FullyReversible,
            reboot_policy: RebootPolicy::NotRequired,
            experimental: false,
            proof_status: ProofStatus::Proven,
            exposure: super::super::operation::ExposureLevel::Advanced,
            recovery: super::super::operation::RecoveryLevel::Exact,
            restore_point_policy: super::super::operation::RestorePointPolicy::NotNeeded,
            mutates_system: true,
            requires_confirmation: true,
            requires_revalidation: true,
            requires_verification: true,
            requires_journal: true,
        };
        let plan = OperationPlan {
            id: OperationId(operation_id.into()),
            descriptor,
            targets: vec![target.clone()],
            warnings: Vec::new(),
            expected_effects: vec![ExpectedEffect {
                target_id: target_id.clone(),
                description: "Fixture state is read back after the operation.".into(),
            }],
            requires_elevation: false,
            requires_restart: false,
            reversibility: Reversibility::FullyReversible,
            parent_operation_id: parent_operation_id.map(OperationId::from),
            retry_of_operation_id: None,
        };
        let before = StateSnapshot {
            summary: "disabled".into(),
            properties: BTreeMap::new(),
        };
        let after = StateSnapshot {
            summary: if apply { "enabled" } else { "disabled" }.into(),
            properties: BTreeMap::new(),
        };
        let outcome = TargetOutcome {
            target_id: target_id.clone(),
            requested_action: if apply { "apply" } else { "revert" }.into(),
            before: Some(before.clone()),
            after: Some(after),
            execution: super::super::operation::ExecutionStatus::Changed,
            verification: VerificationStatus::Verified,
            error: None,
        };
        OperationJournalRecord {
            journal_id: format!("journal-{operation_id}"),
            record_kind: JournalRecordKind::Operation,
            related_record_id: None,
            operation_id: plan.id.clone(),
            operation_schema_version: OPERATION_SCHEMA_VERSION,
            application_version: "test".into(),
            timestamp: "2026-08-12T10:00:00Z".into(),
            terminal_state: OperationState::Succeeded,
            confirmed_plan: plan,
            requested_targets: vec![target],
            captured_original_states: vec![CapturedOriginalState::TypedState {
                target_id,
                state: before,
            }],
            target_outcomes: vec![outcome],
            verification_results: vec![VerificationRecord {
                target_id: "performance:fixture".into(),
                status: VerificationStatus::Verified,
                message: "Fixture state was read back.".into(),
            }],
            restart_requirement: RebootPolicy::NotRequired,
            undo_eligibility: UndoEligibility {
                eligible: false,
                reason: "fixture".into(),
                automated_restoration_test: None,
            },
            undo_execution_result: None,
            diagnostic: None,
        }
    }

    fn journal_with(records: &[OperationJournalRecord]) -> (tempfile::TempDir, OperationJournal) {
        let directory = tempdir().unwrap();
        let journal = OperationJournal::open_at(&directory.path().join("journal.db")).unwrap();
        for record in records {
            journal
                .begin_confirmed_execution(&record.confirmed_plan)
                .unwrap();
            journal.append(record).unwrap();
        }
        (directory, journal)
    }

    fn compared_document(operation_id: Option<&str>) -> Value {
        json!({
            "id": "experiment-decision",
            "createdAt": "2026-08-12T10:00:00Z",
            "state": "compared",
            "operationId": operation_id,
            "baseline": [],
            "after": [],
            "comparison": {"metrics": []},
            "decision": null
        })
    }

    fn document(operation_id: Option<&str>) -> Value {
        json!({
            "id": "experiment-1",
            "name": "Fixture experiment",
            "hypothesis": "The workload should improve.",
            "createdAt": "2026-08-11T10:00:00.000Z",
            "state": "awaiting_reboot",
            "appliedAt": "2026-08-11T10:02:00.000Z",
            "rebootRequired": true,
            "resumedAt": null,
            "operationId": operation_id,
            "baseline": [{"capturedAt": "baseline-1", "metrics": {"workload": 100}}],
            "after": [{"capturedAt": "after-1", "metrics": {"workload": 103}}],
            "comparison": {"metrics": [{"conclusion": "measurable_improvement"}]},
            "decision": "keep"
        })
    }

    #[test]
    fn raw_samples_conclusion_and_operation_link_survive_reopen() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("performance-experiments.db");
        let store = PerformanceExperimentStore::open_at(&path).unwrap();
        store.save(document(Some("operation-1"))).unwrap();
        drop(store);

        let reopened = PerformanceExperimentStore::open_at(&path).unwrap();
        let records = reopened.list().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["baseline"][0]["metrics"]["workload"], 100);
        assert_eq!(records[0]["after"][0]["metrics"]["workload"], 103);
        assert_eq!(
            records[0]["comparison"]["metrics"][0]["conclusion"],
            "measurable_improvement"
        );
        assert_eq!(records[0]["operationId"], "operation-1");
        assert_eq!(records[0]["state"], "awaiting_reboot");
        assert_eq!(records[0]["rebootRequired"], true);
        assert_eq!(records[0]["resumedAt"], Value::Null);
    }

    #[test]
    fn saving_again_updates_one_durable_experiment_record() {
        let directory = tempdir().unwrap();
        let store = PerformanceExperimentStore::open_at(
            &directory.path().join("performance-experiments.db"),
        )
        .unwrap();
        store.save(document(None)).unwrap();
        let mut updated = document(Some("operation-2"));
        updated["after"] = json!([
            {"capturedAt": "after-1", "metrics": {"workload": 104}},
            {"capturedAt": "after-2", "metrics": {"workload": 105}}
        ]);
        store.save(updated).unwrap();

        let records = store.list().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["after"].as_array().unwrap().len(), 2);
        assert_eq!(records[0]["operationId"], "operation-2");
    }

    #[test]
    fn incomplete_documents_are_rejected_before_writing() {
        let directory = tempdir().unwrap();
        let store = PerformanceExperimentStore::open_at(
            &directory.path().join("performance-experiments.db"),
        )
        .unwrap();
        let error = store
            .save(json!({"id": "missing-samples", "createdAt": "now"}))
            .unwrap_err();
        assert!(error.to_string().contains("samples must be an array"));
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn unsupported_lifecycle_state_is_rejected_before_writing() {
        let directory = tempdir().unwrap();
        let store = PerformanceExperimentStore::open_at(
            &directory.path().join("performance-experiments.db"),
        )
        .unwrap();
        let error = store
            .save(json!({
                "id": "invalid-state",
                "createdAt": "now",
                "baseline": [],
                "after": [],
                "state": "not-a-real-state"
            }))
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("unsupported performance experiment state"));
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn keep_requires_a_verified_linked_operation_and_persists_readback_evidence() {
        let source = operation_record("apply-1", None, true);
        let (_directory, journal) = journal_with(std::slice::from_ref(&source));

        let finalized =
            finalize_decision(compared_document(Some("apply-1")), "keep", None, &journal).unwrap();

        assert_eq!(finalized["state"], "keep_recorded");
        assert_eq!(finalized["decision"], "keep");
        assert_eq!(finalized["operationEvidence"]["operationId"], "apply-1");
        assert_eq!(finalized["operationEvidence"]["readbackVerified"], true);
        assert_eq!(
            finalized["operationEvidence"]["targets"][0]["after"]["summary"],
            "enabled"
        );
    }

    #[test]
    fn revert_requires_a_new_linked_verified_operation() {
        let source = operation_record("apply-1", None, true);
        let revert = operation_record("revert-1", Some("apply-1"), false);
        let (_directory, journal) = journal_with(&[source, revert]);

        let finalized = finalize_decision(
            compared_document(Some("apply-1")),
            "revert",
            Some("revert-1"),
            &journal,
        )
        .unwrap();

        assert_eq!(finalized["state"], "revert_requested");
        assert_eq!(finalized["decisionOperationId"], "revert-1");
        assert_eq!(finalized["decisionEvidence"]["readbackVerified"], true);
        assert_eq!(
            finalized["decisionEvidence"]["targets"][0]["verification"],
            "verified"
        );
    }

    #[test]
    fn revert_plan_is_parent_linked_and_fails_closed_when_target_is_already_reverted() {
        let source = operation_record("apply-1", None, true);
        let current = SystemTweakTarget {
            kind: TweakKind::Performance,
            id: "fixture".into(),
            name: "Fixture performance tweak".into(),
            description: "Fixture".into(),
            risk: RiskLevel::Low,
            is_applied: true,
            experimental: false,
            requires_elevation: false,
            requires_restart: false,
            evidence_status: "confirmed_working".into(),
            exposure: ExposureLevel::Supporting,
            reversibility: Reversibility::FullyReversible,
        };
        let plan = build_revert_plan(&source, "performance:fixture", current.clone()).unwrap();
        assert_eq!(
            plan.parent_operation_id,
            Some(OperationId("apply-1".into()))
        );
        assert_eq!(plan.targets[0].target_id, "performance:fixture");
        assert_eq!(
            serde_json::from_str::<Value>(&plan.targets[0].requested_action).unwrap()["apply"],
            false
        );

        let mut already_reverted = current;
        already_reverted.is_applied = false;
        let error =
            build_revert_plan(&source, "performance:fixture", already_reverted).unwrap_err();
        assert_eq!(error.code, OperationErrorCode::TargetChangedSincePreview);
    }

    #[test]
    fn final_decision_documents_without_verified_evidence_are_rejected() {
        let directory = tempdir().unwrap();
        let store = PerformanceExperimentStore::open_at(
            &directory.path().join("performance-experiments.db"),
        )
        .unwrap();
        let error = store
            .save(json!({
                "id": "unsafe-decision",
                "createdAt": "now",
                "baseline": [],
                "after": [],
                "state": "keep_recorded",
                "decision": "keep"
            }))
            .unwrap_err();
        assert!(error.to_string().contains("verified operation read-back"));
    }
}
