use super::operation::{
    ExecutionStatus, ExpectedEffect, ExposureLevel, OperationCategory, OperationDescriptor,
    OperationError, OperationErrorCode, OperationId, OperationPlan, OperationResult,
    OperationWarning, PlannedTarget, PrivilegeRequirement, ProofStatus, RebootPolicy,
    RecoveryLevel, Reversibility, RiskLevel, StateSnapshot, TargetExecutionState, TargetOutcome,
    UndoAvailability, VerificationStatus,
};
use super::operation_journal::{
    CapturedOriginalState, JournalRecordKind, OperationJournal, OperationJournalRecord,
    UndoEligibility, VerificationRecord, OPERATION_SCHEMA_VERSION,
};
use crate::processes::snapshots::ProcessInfo;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub start_time: u64,
    pub name: String,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessExecution {
    pub result: OperationResult,
    pub message: String,
}

pub fn create_plan(process: &ProcessInfo) -> Result<OperationPlan, OperationError> {
    let identity = identity(process);
    validate_protection(&identity)?;
    let requires_elevation = crate::processes::kill::requires_elevation(identity.pid);
    let action = serde_json::to_string(&identity)
        .map_err(|error| OperationError::validation(error.to_string()))?;
    Ok(OperationPlan {
        id: OperationId(uuid::Uuid::new_v4().to_string()),
        descriptor: OperationDescriptor {
            id: OperationId("process.terminate".into()),
            capability_id: "process.manager".into(),
            title: "Terminate Process".into(),
            category: OperationCategory::ProcessManagement,
            risk: RiskLevel::High,
            privilege: if requires_elevation {
                PrivilegeRequirement::Administrator
            } else {
                PrivilegeRequirement::StandardUser
            },
            reversibility: Reversibility::NotReversible,
            reboot_policy: RebootPolicy::NotRequired,
            experimental: true,
            proof_status: ProofStatus::Experimental,
            exposure: ExposureLevel::Advanced,
            recovery: RecoveryLevel::None,
            restore_point_policy: crate::core::operation::RestorePointPolicy::NotNeeded,
            mutates_system: true,
            requires_confirmation: true,
            requires_revalidation: true,
            requires_verification: true,
            requires_journal: true,
        },
        targets: vec![PlannedTarget {
            target_id: identity.pid.to_string(),
            requested_action: action,
            description: format!(
                "Terminate {} (PID {}, start time {}, path {}).",
                identity.name,
                identity.pid,
                identity.start_time,
                identity.path.as_deref().unwrap_or("unavailable")
            ),
        }],
        warnings: vec![OperationWarning {
            code: "unsaved_data_loss".into(),
            message: "Termination is immediate, cannot be undone, and may lose unsaved data."
                .into(),
        }],
        expected_effects: vec![ExpectedEffect {
            target_id: identity.pid.to_string(),
            description: "The exact PID/start-time identity is absent after bounded polling."
                .into(),
        }],
        requires_elevation,
        requires_restart: false,
        reversibility: Reversibility::NotReversible,
        parent_operation_id: None,
        retry_of_operation_id: None,
    })
}

pub fn create_retry_plan(
    current: &[ProcessInfo],
    previous_plan: &OperationPlan,
    unresolved_target_ids: &[String],
) -> Result<OperationPlan, OperationError> {
    if previous_plan.descriptor.id.0 != "process.terminate" {
        return Err(OperationError::validation(
            "retry requires domain-specific rediscovery for this operation",
        ));
    }
    if unresolved_target_ids.len() != 1 {
        return Err(OperationError::validation(
            "process retry requires exactly one target",
        ));
    }

    let previous_identity = decode(previous_plan)?;
    let expected_target_id = previous_identity.pid.to_string();
    if unresolved_target_ids[0] != expected_target_id {
        return Err(OperationError::validation(
            "retry target does not match the previous process plan",
        ));
    }

    let current_process = current
        .iter()
        .find(|process| process.pid == previous_identity.pid)
        .ok_or_else(|| {
            OperationError::validation(
                "process identity was not rediscovered; no retry plan was created",
            )
        })?;
    if identity(current_process) != previous_identity {
        return Err(OperationError::validation(
            "process identity changed or PID was reused; no retry plan was created",
        ));
    }

    let mut plan = create_plan(current_process)?;
    plan.parent_operation_id = Some(previous_plan.id.clone());
    plan.retry_of_operation_id = Some(previous_plan.id.clone());
    plan.warnings.push(OperationWarning {
        code: "retry_of_operation".into(),
        message: previous_plan.id.0.clone(),
    });
    plan.validate_relationships()?;
    Ok(plan)
}

pub fn confirmed_identity(
    plan: &OperationPlan,
    current: &[ProcessInfo],
) -> Result<ProcessIdentity, OperationError> {
    let expected = decode(plan)?;
    validate_protection(&expected)?;
    if !plan.requires_elevation && crate::processes::kill::requires_elevation(expected.pid) {
        return Err(OperationError {
            code: OperationErrorCode::ElevationRequired,
            message: "Process access changed and now requires Administrator approval.".into(),
            target_id: Some(expected.pid.to_string()),
            retryable: true,
        });
    }
    let Some(actual) = current.iter().find(|process| process.pid == expected.pid) else {
        return Err(not_found(&expected));
    };
    if identity(actual) != expected {
        return Err(OperationError {
            code: OperationErrorCode::TargetChangedSincePreview,
            message: "The PID now belongs to a different process; refresh before terminating it."
                .into(),
            target_id: Some(expected.pid.to_string()),
            retryable: true,
        });
    }
    Ok(expected)
}

pub fn finish(
    plan: &OperationPlan,
    current: &[ProcessInfo],
) -> Result<ProcessExecution, OperationError> {
    let expected = decode(plan)?;
    let still_running = current.iter().any(|process| identity(process) == expected);
    let error = still_running.then(|| OperationError {
        code: OperationErrorCode::VerificationFailed,
        message: "The confirmed process identity is still running.".into(),
        target_id: Some(expected.pid.to_string()),
        retryable: true,
    });
    let result = OperationResult::new(
        plan.id.clone(),
        vec![TargetOutcome {
            target_id: expected.pid.to_string(),
            requested_action: "terminate".into(),
            before: Some(state(&expected)),
            after: None,
            execution: if still_running {
                ExecutionStatus::Failed
            } else {
                ExecutionStatus::Changed
            },
            verification: if still_running {
                VerificationStatus::Failed
            } else {
                VerificationStatus::Verified
            },
            error,
        }],
        false,
        UndoAvailability::NotReversible,
    )?;
    Ok(ProcessExecution {
        message: if result.failed == 0 {
            "Process termination verified.".into()
        } else {
            "Process remained present after termination.".into()
        },
        result,
    })
}

pub(crate) fn mark_target_ready_and_mutation_started(
    journal: &OperationJournal,
    plan: &OperationPlan,
    target_id: &str,
) -> Result<(), OperationError> {
    mark_target_ready(journal, plan, target_id)?;
    let payload = target_payload(plan, target_id)?;
    journal.record_target_state(
        &plan.id,
        target_id,
        TargetExecutionState::MutationStarted,
        &payload,
    )
}

pub(crate) fn mark_target_ready(
    journal: &OperationJournal,
    plan: &OperationPlan,
    target_id: &str,
) -> Result<(), OperationError> {
    let payload = target_payload(plan, target_id)?;
    journal.record_target_state(&plan.id, target_id, TargetExecutionState::Ready, &payload)
}

pub(crate) fn mark_target_revalidation_failed(
    journal: &OperationJournal,
    plan: &OperationPlan,
    target_id: &str,
) -> Result<(), OperationError> {
    let payload = target_payload(plan, target_id)?;
    journal.record_target_state(
        &plan.id,
        target_id,
        TargetExecutionState::RevalidationFailed,
        &payload,
    )
}

pub(crate) fn record_target_outcome(
    journal: &OperationJournal,
    plan: &OperationPlan,
    outcome: &TargetOutcome,
) -> Result<(), OperationError> {
    journal.record_target_outcome(&plan.id, outcome)
}

fn target_payload(plan: &OperationPlan, target_id: &str) -> Result<String, OperationError> {
    let target = plan
        .targets
        .iter()
        .find(|target| target.target_id == target_id)
        .ok_or_else(|| {
            OperationError::validation(format!(
                "target {target_id} is not present in the confirmed process plan"
            ))
        })?;
    serde_json::to_string(target).map_err(|error| OperationError {
        code: OperationErrorCode::SerializationFailed,
        message: error.to_string(),
        target_id: Some(target_id.into()),
        retryable: false,
    })
}

pub fn journal_record(
    plan: &OperationPlan,
    execution: &ProcessExecution,
) -> OperationJournalRecord {
    let expected = decode(plan).expect("validated process plan");
    OperationJournalRecord {
        journal_id: uuid::Uuid::new_v4().to_string(),
        record_kind: JournalRecordKind::Operation,
        related_record_id: None,
        operation_id: plan.id.clone(),
        operation_schema_version: OPERATION_SCHEMA_VERSION,
        application_version: env!("CARGO_PKG_VERSION").into(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        terminal_state: execution.result.completion_state(),
        confirmed_plan: plan.clone(),
        requested_targets: plan.targets.clone(),
        captured_original_states: vec![CapturedOriginalState::TypedState {
            target_id: expected.pid.to_string(),
            state: state(&expected),
        }],
        target_outcomes: execution.result.outcomes.clone(),
        verification_results: vec![VerificationRecord {
            target_id: expected.pid.to_string(),
            status: execution.result.outcomes[0].verification,
            message: "PID/start-time identity checked after termination.".into(),
        }],
        restart_requirement: RebootPolicy::NotRequired,
        undo_eligibility: UndoEligibility {
            eligible: false,
            reason: "A terminated process cannot be restored safely.".into(),
            automated_restoration_test: None,
        },
        undo_execution_result: None,
        diagnostic: execution.result.outcomes[0].error.clone(),
    }
}

fn decode(plan: &OperationPlan) -> Result<ProcessIdentity, OperationError> {
    if plan.descriptor.id.0 != "process.terminate"
        || plan.descriptor.category != OperationCategory::ProcessManagement
        || plan.targets.len() != 1
        || plan.reversibility != Reversibility::NotReversible
        || plan.requires_elevation
            != (plan.descriptor.privilege == PrivilegeRequirement::Administrator)
    {
        return Err(OperationError::validation(
            "invalid process termination plan",
        ));
    }
    let identity: ProcessIdentity = serde_json::from_str(&plan.targets[0].requested_action)
        .map_err(|error| OperationError::validation(error.to_string()))?;
    if plan.targets[0].target_id != identity.pid.to_string() {
        return Err(OperationError::validation(
            "process plan target was modified",
        ));
    }
    Ok(identity)
}

fn identity(process: &ProcessInfo) -> ProcessIdentity {
    ProcessIdentity {
        pid: process.pid,
        start_time: process.start_time,
        name: process.name.clone(),
        path: process.path.clone(),
    }
}

fn validate_protection(identity: &ProcessIdentity) -> Result<(), OperationError> {
    if identity.pid <= 4
        || identity.pid == std::process::id()
        || crate::processes::kill::is_protected_process_name(&identity.name)
    {
        return Err(OperationError {
            code: OperationErrorCode::TargetProtected,
            message: "WinOptimizer, Defender, and critical/system processes cannot be terminated."
                .into(),
            target_id: Some(identity.pid.to_string()),
            retryable: false,
        });
    }
    Ok(())
}

fn not_found(identity: &ProcessIdentity) -> OperationError {
    OperationError {
        code: OperationErrorCode::TargetNotFound,
        message: "The confirmed process is no longer running.".into(),
        target_id: Some(identity.pid.to_string()),
        retryable: true,
    }
}

fn state(identity: &ProcessIdentity) -> StateSnapshot {
    StateSnapshot {
        summary: format!("{} (PID {})", identity.name, identity.pid),
        properties: BTreeMap::from([
            ("start_time".into(), identity.start_time.to_string()),
            ("path".into(), identity.path.clone().unwrap_or_default()),
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn journal() -> (tempfile::TempDir, OperationJournal) {
        let directory = tempfile::tempdir().unwrap();
        let journal = OperationJournal::open_at(&directory.path().join("journal.db")).unwrap();
        (directory, journal)
    }

    fn process(pid: u32, start_time: u64, name: &str) -> ProcessInfo {
        ProcessInfo {
            pid,
            start_time,
            name: name.into(),
            cpu_usage: 0.0,
            memory_bytes: 0,
            disk_read_bytes: 0,
            disk_write_bytes: 0,
            status: "Run".into(),
            path: Some(format!(r"C:\fixtures\{name}")),
        }
    }

    #[test]
    fn journal_target_helper_does_not_resolve_later_targets() {
        let (_directory, journal) = journal();
        let contract = crate::core::operation::synthetic_operation_contract().unwrap();
        let first = &contract.plan.targets[0];
        journal.begin_execution(&contract.plan).unwrap();
        mark_target_ready_and_mutation_started(&journal, &contract.plan, &first.target_id).unwrap();
        let outcome = TargetOutcome {
            target_id: first.target_id.clone(),
            requested_action: first.requested_action.clone(),
            before: None,
            after: None,
            execution: ExecutionStatus::Changed,
            verification: VerificationStatus::Verified,
            error: None,
        };
        record_target_outcome(&journal, &contract.plan, &outcome).unwrap();

        let unresolved = journal.list_unresolved_targets(&contract.plan.id).unwrap();
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].state, TargetExecutionState::Pending);
        assert_eq!(unresolved[0].target_id, contract.plan.targets[1].target_id);
    }

    #[test]
    fn self_is_protected() {
        let process = ProcessInfo {
            pid: std::process::id(),
            start_time: 1,
            name: "winoptimizer.exe".into(),
            cpu_usage: 0.0,
            memory_bytes: 0,
            disk_read_bytes: 0,
            disk_write_bytes: 0,
            status: "Run".into(),
            path: None,
        };
        assert_eq!(
            create_plan(&process).unwrap_err().code,
            OperationErrorCode::TargetProtected
        );
    }

    #[test]
    fn defender_processes_are_protected() {
        for name in [
            "MsMpEng.exe",
            "NisSrv.exe",
            "MsSense.exe",
            "MpDefenderCoreService.exe",
            "SecurityHealthService.exe",
            "SecurityHealthSystray.exe",
        ] {
            let error = create_plan(&process(42_424, 1, name)).unwrap_err();
            assert_eq!(error.code, OperationErrorCode::TargetProtected, "{name}");
        }
    }

    #[test]
    fn retry_rebuilds_from_the_same_current_process_identity() {
        let current = process(424_242, 7, "fixture.exe");
        let previous = create_plan(&current).unwrap();

        let retry = create_retry_plan(
            std::slice::from_ref(&current),
            &previous,
            std::slice::from_ref(&previous.targets[0].target_id),
        )
        .unwrap();

        assert_ne!(retry.id, previous.id);
        assert_eq!(retry.parent_operation_id, Some(previous.id.clone()));
        assert_eq!(retry.retry_of_operation_id, Some(previous.id));
        assert_eq!(retry.reversibility, Reversibility::NotReversible);
        assert!(retry.descriptor.mutates_system);
    }

    #[test]
    fn retry_rejects_pid_reuse_without_creating_a_plan() {
        let previous_process = process(424_243, 7, "fixture.exe");
        let previous = create_plan(&previous_process).unwrap();
        let reused_process = process(424_243, 8, "different.exe");

        let error = create_retry_plan(
            std::slice::from_ref(&reused_process),
            &previous,
            std::slice::from_ref(&previous.targets[0].target_id),
        )
        .unwrap_err();

        assert_eq!(error.code, OperationErrorCode::ValidationFailed);
        assert!(!error.retryable);
        assert!(error.message.contains("PID was reused"));
    }

    #[test]
    fn retry_rejects_missing_process_evidence_without_creating_a_plan() {
        let previous_process = process(424_244, 7, "fixture.exe");
        let previous = create_plan(&previous_process).unwrap();

        let error = create_retry_plan(
            &[],
            &previous,
            std::slice::from_ref(&previous.targets[0].target_id),
        )
        .unwrap_err();

        assert_eq!(error.code, OperationErrorCode::ValidationFailed);
        assert!(!error.retryable);
        assert!(error.message.contains("no retry plan was created"));
    }
}
