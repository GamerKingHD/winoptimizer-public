use super::operation::TargetExecutionState;
use super::operation::{
    ExecutionStatus, ExpectedEffect, ExposureLevel, OperationCategory, OperationDescriptor,
    OperationError, OperationErrorCode, OperationId, OperationPlan, OperationResult,
    OperationWarning, PlannedTarget, PrivilegeRequirement, ProofStatus, RebootPolicy,
    RecoveryLevel, Reversibility, RiskLevel, StateSnapshot, TargetOutcome, UndoAvailability,
    VerificationStatus,
};
use super::operation_journal::{
    CapturedOriginalState, JournalRecordKind, OperationJournal, OperationJournalRecord,
    UndoEligibility, VerificationRecord, OPERATION_SCHEMA_VERSION,
};
use crate::startup::programs::{StartupMutationResult, StartupProgram};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StartupTargetSnapshot {
    id: String,
    name: String,
    command: String,
    location: String,
    value_type: Option<String>,
    enabled: bool,
    #[serde(default)]
    recovery_action_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StartupExecution {
    pub result: OperationResult,
    pub recovery_action_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StartupEnableExecution {
    pub result: OperationResult,
    pub message: String,
}

pub fn create_plan(program: &StartupProgram) -> Result<OperationPlan, OperationError> {
    if crate::startup::programs::is_protected(program) {
        return Err(OperationError {
            code: OperationErrorCode::TargetProtected,
            message: format!(
                "Protected startup entry cannot be disabled: {}",
                program.name
            ),
            target_id: Some(program.id.clone()),
            retryable: false,
        });
    }
    if !program.enabled {
        return Err(validation("startup entry is already disabled"));
    }
    if !supported_location(&program.location) {
        return Err(validation("startup entry location is not supported"));
    }

    let snapshot = snapshot(program);
    let requested_action = serde_json::to_string(&snapshot)
        .map_err(|error| validation(format!("failed to serialize startup target: {error}")))?;
    let requires_elevation = program.location == "HKLM_Run";
    let target = PlannedTarget {
        target_id: program.id.clone(),
        requested_action,
        description: format!("Disable {} from {}", program.name, program.location),
    };

    Ok(OperationPlan {
        id: OperationId(uuid::Uuid::new_v4().to_string()),
        descriptor: OperationDescriptor {
            id: OperationId("startup.disable_program".into()),
            capability_id: "startup.programs".into(),
            title: "Disable startup program".into(),
            category: OperationCategory::Startup,
            risk: if requires_elevation {
                RiskLevel::Medium
            } else {
                RiskLevel::Low
            },
            privilege: if requires_elevation {
                PrivilegeRequirement::Administrator
            } else {
                PrivilegeRequirement::StandardUser
            },
            reversibility: Reversibility::FullyReversible,
            reboot_policy: RebootPolicy::NotRequired,
            experimental: false,
            proof_status: ProofStatus::Proven,
            exposure: ExposureLevel::Core,
            recovery: RecoveryLevel::Exact,
            restore_point_policy: crate::core::operation::RestorePointPolicy::NotNeeded,
            mutates_system: true,
            requires_confirmation: true,
            requires_revalidation: true,
            requires_verification: true,
            requires_journal: true,
        },
        targets: vec![target],
        warnings: vec![OperationWarning {
            code: "startup_next_sign_in".into(),
            message: "The program will stop starting automatically at the next sign-in.".into(),
        }],
        expected_effects: vec![ExpectedEffect {
            target_id: program.id.clone(),
            description: "The exact startup entry is absent after re-enumeration.".into(),
        }],
        requires_elevation,
        requires_restart: false,
        reversibility: Reversibility::FullyReversible,
        parent_operation_id: None,
        retry_of_operation_id: None,
    })
}

pub fn create_retry_plan(
    current: &[StartupProgram],
    previous_plan: &OperationPlan,
    unresolved_target_ids: &[String],
) -> Result<OperationPlan, OperationError> {
    if previous_plan.descriptor.id.0 == "startup.enable_program" {
        return create_enable_retry_plan(current, previous_plan, unresolved_target_ids);
    }
    if previous_plan.descriptor.id.0 != "startup.disable_program" {
        return Err(OperationError::validation(
            "retry requires domain-specific rediscovery for this operation",
        ));
    }
    if unresolved_target_ids.len() != 1 {
        return Err(OperationError::validation(
            "startup retry requires exactly one target",
        ));
    }

    let previous_target = decode_plan(previous_plan)?;
    if !previous_target.enabled || unresolved_target_ids[0] != previous_target.id {
        return Err(OperationError::validation(
            "retry target does not match the previous startup plan",
        ));
    }

    let current_target = current
        .iter()
        .find(|program| program.id == previous_target.id)
        .ok_or_else(|| {
            OperationError::validation(
                "startup target was not rediscovered as an active entry; no retry plan was created",
            )
        })?;
    if !current_target.enabled {
        return Err(OperationError::validation(
            "startup target is already disabled; no retry plan is needed",
        ));
    }

    let mut plan = create_plan(current_target)?;
    plan.parent_operation_id = Some(previous_plan.id.clone());
    plan.retry_of_operation_id = Some(previous_plan.id.clone());
    plan.warnings.push(OperationWarning {
        code: "retry_of_operation".into(),
        message: previous_plan.id.0.clone(),
    });
    plan.validate_relationships()?;
    Ok(plan)
}

pub fn create_enable_retry_plan(
    current: &[StartupProgram],
    previous_plan: &OperationPlan,
    unresolved_target_ids: &[String],
) -> Result<OperationPlan, OperationError> {
    if previous_plan.descriptor.id.0 != "startup.enable_program" {
        return Err(OperationError::validation(
            "retry requires domain-specific rediscovery for this operation",
        ));
    }
    if unresolved_target_ids.len() != 1 {
        return Err(OperationError::validation(
            "startup enable retry requires exactly one target",
        ));
    }

    let previous_target = decode_enable_plan(previous_plan)?;
    if unresolved_target_ids[0] != previous_target.id {
        return Err(OperationError::validation(
            "startup enable retry target does not match the previous plan",
        ));
    }

    let current_target = current
        .iter()
        .find(|program| program.id == previous_target.id)
        .ok_or_else(|| {
            OperationError::validation(
                "disabled startup target was not rediscovered; no retry plan was created",
            )
        })?;
    if current_target.enabled {
        return Err(OperationError::validation(
            "startup target is already enabled; no retry plan is needed",
        ));
    }

    let mut plan = create_enable_plan(current_target)?;
    plan.parent_operation_id = Some(previous_plan.id.clone());
    plan.retry_of_operation_id = Some(previous_plan.id.clone());
    plan.warnings.push(OperationWarning {
        code: "retry_of_operation".into(),
        message: previous_plan.id.0.clone(),
    });
    plan.validate_relationships()?;
    Ok(plan)
}

pub fn create_enable_plan(program: &StartupProgram) -> Result<OperationPlan, OperationError> {
    if crate::startup::programs::is_protected(program) {
        return Err(OperationError {
            code: OperationErrorCode::TargetProtected,
            message: format!(
                "Protected startup entry cannot be enabled: {}",
                program.name
            ),
            target_id: Some(program.id.clone()),
            retryable: false,
        });
    }
    if program.enabled {
        return Err(validation("startup entry is already enabled"));
    }
    if !supported_location(&program.location) {
        return Err(validation("startup entry location is not supported"));
    }
    if program.recovery_action_id.is_none() {
        return Err(validation("startup entry has no recovery record"));
    }

    let snapshot = snapshot(program);
    let requested_action = serde_json::to_string(&snapshot)
        .map_err(|error| validation(format!("failed to serialize startup target: {error}")))?;
    let requires_elevation = program.location == "HKLM_Run";
    Ok(OperationPlan {
        id: OperationId(uuid::Uuid::new_v4().to_string()),
        descriptor: OperationDescriptor {
            id: OperationId("startup.enable_program".into()),
            capability_id: "startup.programs".into(),
            title: "Enable startup program".into(),
            category: OperationCategory::Startup,
            risk: if requires_elevation {
                RiskLevel::Medium
            } else {
                RiskLevel::Low
            },
            privilege: if requires_elevation {
                PrivilegeRequirement::Administrator
            } else {
                PrivilegeRequirement::StandardUser
            },
            reversibility: Reversibility::NotReversible,
            reboot_policy: RebootPolicy::NotRequired,
            experimental: false,
            proof_status: ProofStatus::PartiallyProven,
            exposure: ExposureLevel::Core,
            recovery: RecoveryLevel::BestEffort,
            restore_point_policy: crate::core::operation::RestorePointPolicy::NotNeeded,
            mutates_system: true,
            requires_confirmation: true,
            requires_revalidation: true,
            requires_verification: true,
            requires_journal: true,
        },
        targets: vec![PlannedTarget {
            target_id: program.id.clone(),
            requested_action,
            description: format!("Enable {} from {}", program.name, program.location),
        }],
        warnings: vec![OperationWarning {
            code: "startup_next_sign_in".into(),
            message: "The original startup entry will be restored for the next sign-in.".into(),
        }],
        expected_effects: vec![ExpectedEffect {
            target_id: program.id.clone(),
            description: "The exact startup entry is present after re-enumeration.".into(),
        }],
        requires_elevation,
        requires_restart: false,
        reversibility: Reversibility::NotReversible,
        parent_operation_id: None,
        retry_of_operation_id: None,
    })
}

pub fn validate_current(
    plan: &OperationPlan,
    current: &StartupProgram,
) -> Result<(), OperationError> {
    let planned = decode_plan(plan)?;
    if &planned != current {
        return Err(OperationError {
            code: OperationErrorCode::TargetChangedSincePreview,
            message: "Startup entry changed after confirmation; review it again.".into(),
            target_id: Some(planned.id),
            retryable: true,
        });
    }
    Ok(())
}

pub fn validate_enable_current(
    plan: &OperationPlan,
    current: &StartupProgram,
) -> Result<(), OperationError> {
    let planned = decode_enable_plan(plan)?;
    if &planned != current {
        return Err(OperationError {
            code: OperationErrorCode::TargetChangedSincePreview,
            message: "Disabled startup entry changed after confirmation; review it again.".into(),
            target_id: Some(planned.id),
            retryable: true,
        });
    }
    Ok(())
}

pub fn finish(
    plan: &OperationPlan,
    before: &StartupProgram,
    mutation: StartupMutationResult,
) -> Result<StartupExecution, OperationError> {
    validate_current(plan, before)?;
    if !mutation.verified {
        return Err(OperationError {
            code: OperationErrorCode::VerificationFailed,
            message: "Startup entry removal could not be verified.".into(),
            target_id: Some(before.id.clone()),
            retryable: true,
        });
    }

    let outcome = TargetOutcome {
        target_id: before.id.clone(),
        requested_action: "disable_startup_program".into(),
        before: Some(state_snapshot(before, true)),
        after: Some(state_snapshot(before, false)),
        execution: ExecutionStatus::Changed,
        verification: VerificationStatus::Verified,
        error: None,
    };
    let result = OperationResult::new(
        plan.id.clone(),
        vec![outcome],
        false,
        UndoAvailability::NotReversible,
    )?;
    Ok(StartupExecution {
        result,
        recovery_action_id: mutation.action_id,
        message: mutation.message,
    })
}

pub fn finish_enable(
    plan: &OperationPlan,
    before: &StartupProgram,
    after: &StartupProgram,
) -> Result<StartupEnableExecution, OperationError> {
    validate_enable_current(plan, before)?;
    if before.id != after.id || !after.enabled {
        return Err(OperationError {
            code: OperationErrorCode::VerificationFailed,
            message: "Startup entry restoration could not be verified.".into(),
            target_id: Some(before.id.clone()),
            retryable: true,
        });
    }

    let outcome = TargetOutcome {
        target_id: before.id.clone(),
        requested_action: "enable_startup_program".into(),
        before: Some(state_snapshot(before, false)),
        after: Some(state_snapshot(after, true)),
        execution: ExecutionStatus::Changed,
        verification: VerificationStatus::Verified,
        error: None,
    };
    let result = OperationResult::new(
        plan.id.clone(),
        vec![outcome],
        false,
        UndoAvailability::NotReversible,
    )?;
    Ok(StartupEnableExecution {
        result,
        message: format!("{} was enabled and verified.", before.name),
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

pub(crate) fn failed_outcome(
    before: &StartupProgram,
    requested_action: &str,
    error: OperationError,
) -> TargetOutcome {
    TargetOutcome {
        target_id: before.id.clone(),
        requested_action: requested_action.into(),
        before: Some(state_snapshot(before, before.enabled)),
        after: None,
        execution: ExecutionStatus::Failed,
        verification: VerificationStatus::Failed,
        error: Some(error),
    }
}

fn target_payload(plan: &OperationPlan, target_id: &str) -> Result<String, OperationError> {
    let target = plan
        .targets
        .iter()
        .find(|target| target.target_id == target_id)
        .ok_or_else(|| {
            OperationError::validation(format!(
                "target {target_id} is not present in the confirmed startup plan"
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
    before: &StartupProgram,
    execution: &StartupExecution,
) -> OperationJournalRecord {
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
        captured_original_states: vec![CapturedOriginalState::StartupEntry {
            target_id: before.id.clone(),
            identifier: before.id.clone(),
            name: before.name.clone(),
            location: before.location.clone(),
            value_type: before.value_type.clone(),
            enabled: before.enabled,
            command: before.command.clone(),
        }],
        target_outcomes: execution.result.outcomes.clone(),
        verification_results: execution
            .result
            .outcomes
            .iter()
            .map(|outcome| VerificationRecord {
                target_id: outcome.target_id.clone(),
                status: outcome.verification,
                message: "Startup entry absence verified by re-enumeration.".into(),
            })
            .collect(),
        restart_requirement: RebootPolicy::NotRequired,
        undo_eligibility: UndoEligibility {
            eligible: false,
            reason: format!(
                "Use Startup Manager Enable through durable recovery record {}.",
                execution.recovery_action_id
            ),
            automated_restoration_test: None,
        },
        undo_execution_result: None,
        diagnostic: None,
    }
}

pub fn journal_enable_record(
    plan: &OperationPlan,
    before: &StartupProgram,
    execution: &StartupEnableExecution,
) -> OperationJournalRecord {
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
        captured_original_states: vec![CapturedOriginalState::StartupEntry {
            target_id: before.id.clone(),
            identifier: before.id.clone(),
            name: before.name.clone(),
            location: before.location.clone(),
            value_type: before.value_type.clone(),
            enabled: before.enabled,
            command: before.command.clone(),
        }],
        target_outcomes: execution.result.outcomes.clone(),
        verification_results: execution
            .result
            .outcomes
            .iter()
            .map(|outcome| VerificationRecord {
                target_id: outcome.target_id.clone(),
                status: outcome.verification,
                message: "Startup entry presence verified by re-enumeration.".into(),
            })
            .collect(),
        restart_requirement: RebootPolicy::NotRequired,
        undo_eligibility: UndoEligibility {
            eligible: false,
            reason: "Startup enable consumes the original recovery record and is not recursively reversible.".into(),
            automated_restoration_test: None,
        },
        undo_execution_result: None,
        diagnostic: None,
    }
}

fn decode_plan(plan: &OperationPlan) -> Result<StartupProgram, OperationError> {
    if plan.descriptor.id.0 != "startup.disable_program"
        || plan.descriptor.category != OperationCategory::Startup
        || plan.targets.len() != 1
        || plan.reversibility != Reversibility::FullyReversible
    {
        return Err(validation("invalid startup operation plan"));
    }
    let target = &plan.targets[0];
    let snapshot: StartupTargetSnapshot = serde_json::from_str(&target.requested_action)
        .map_err(|error| validation(format!("invalid startup target snapshot: {error}")))?;
    if snapshot.id != target.target_id
        || !supported_location(&snapshot.location)
        || plan.requires_elevation != (snapshot.location == "HKLM_Run")
    {
        return Err(validation(
            "startup target does not match the confirmed plan",
        ));
    }
    Ok(StartupProgram {
        id: snapshot.id,
        name: snapshot.name,
        command: snapshot.command,
        location: snapshot.location,
        value_type: snapshot.value_type,
        enabled: snapshot.enabled,
        protected: false,
        recovery_action_id: snapshot.recovery_action_id,
    })
}

fn decode_enable_plan(plan: &OperationPlan) -> Result<StartupProgram, OperationError> {
    if plan.descriptor.id.0 != "startup.enable_program"
        || plan.descriptor.category != OperationCategory::Startup
        || plan.targets.len() != 1
        || plan.reversibility != Reversibility::NotReversible
    {
        return Err(validation("invalid startup enable operation plan"));
    }
    let target = &plan.targets[0];
    let snapshot: StartupTargetSnapshot = serde_json::from_str(&target.requested_action)
        .map_err(|error| validation(format!("invalid startup target snapshot: {error}")))?;
    if snapshot.id != target.target_id
        || snapshot.enabled
        || snapshot.recovery_action_id.is_none()
        || !supported_location(&snapshot.location)
        || plan.requires_elevation != (snapshot.location == "HKLM_Run")
    {
        return Err(validation(
            "startup enable target does not match the confirmed plan",
        ));
    }
    Ok(StartupProgram {
        id: snapshot.id,
        name: snapshot.name,
        command: snapshot.command,
        location: snapshot.location,
        value_type: snapshot.value_type,
        enabled: snapshot.enabled,
        protected: false,
        recovery_action_id: snapshot.recovery_action_id,
    })
}

fn snapshot(program: &StartupProgram) -> StartupTargetSnapshot {
    StartupTargetSnapshot {
        id: program.id.clone(),
        name: program.name.clone(),
        command: program.command.clone(),
        location: program.location.clone(),
        value_type: program.value_type.clone(),
        enabled: program.enabled,
        recovery_action_id: program.recovery_action_id.clone(),
    }
}

fn state_snapshot(program: &StartupProgram, enabled: bool) -> StateSnapshot {
    StateSnapshot {
        summary: if enabled {
            "Startup entry enabled"
        } else {
            "Startup entry disabled"
        }
        .into(),
        properties: BTreeMap::from([
            ("identifier".into(), program.id.clone()),
            ("name".into(), program.name.clone()),
            ("command".into(), program.command.clone()),
            ("location".into(), program.location.clone()),
            (
                "value_type".into(),
                program.value_type.clone().unwrap_or_else(|| "file".into()),
            ),
            ("enabled".into(), enabled.to_string()),
        ]),
    }
}

fn supported_location(location: &str) -> bool {
    matches!(
        location,
        "HKCU_Run" | "HKCU_RunOnce" | "HKLM_Run" | "StartupFolder"
    )
}

fn validation(message: impl Into<String>) -> OperationError {
    OperationError::validation(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program() -> StartupProgram {
        StartupProgram {
            id: "a".repeat(64),
            name: "Fixture".into(),
            command: r"C:\fixture.exe".into(),
            location: "HKCU_Run".into(),
            value_type: Some("REG_SZ".into()),
            enabled: true,
            protected: false,
            recovery_action_id: None,
        }
    }

    fn disabled_program() -> StartupProgram {
        StartupProgram {
            enabled: false,
            recovery_action_id: Some("undo-1".into()),
            ..program()
        }
    }

    #[test]
    fn plan_contains_the_exact_confirmed_target() {
        let source = program();
        let plan = create_plan(&source).unwrap();

        assert_eq!(plan.targets.len(), 1);
        assert_eq!(plan.targets[0].target_id, source.id);
        assert_eq!(plan.descriptor.category, OperationCategory::Startup);
        assert_eq!(plan.reversibility, Reversibility::FullyReversible);
        validate_current(&plan, &source).unwrap();
    }

    #[test]
    fn retry_rebuilds_from_the_current_active_target_and_links_the_source() {
        let previous = create_plan(&program()).unwrap();
        let mut current = program();
        current.value_type = Some("REG_EXPAND_SZ".into());

        let retry = create_retry_plan(
            std::slice::from_ref(&current),
            &previous,
            std::slice::from_ref(&previous.targets[0].target_id),
        )
        .unwrap();

        assert_ne!(retry.id, previous.id);
        assert_eq!(retry.parent_operation_id, Some(previous.id.clone()));
        assert_eq!(retry.retry_of_operation_id, Some(previous.id));
        assert_eq!(retry.targets[0].target_id, current.id);
        validate_current(&retry, &current).unwrap();
    }

    #[test]
    fn retry_rejects_a_missing_or_already_disabled_target() {
        let previous = create_plan(&program()).unwrap();
        let target_id = previous.targets[0].target_id.clone();

        let missing =
            create_retry_plan(&[], &previous, std::slice::from_ref(&target_id)).unwrap_err();
        assert_eq!(missing.code, OperationErrorCode::ValidationFailed);

        let mut disabled = program();
        disabled.enabled = false;
        let already_disabled = create_retry_plan(
            std::slice::from_ref(&disabled),
            &previous,
            std::slice::from_ref(&target_id),
        )
        .unwrap_err();
        assert_eq!(already_disabled.code, OperationErrorCode::ValidationFailed);
    }

    #[test]
    fn enable_retry_rebuilds_changed_target_and_links_the_source() {
        let previous = create_enable_plan(&disabled_program()).unwrap();
        let mut current = disabled_program();
        current.command = r"C:\changed.exe".into();

        let retry = create_retry_plan(
            std::slice::from_ref(&current),
            &previous,
            std::slice::from_ref(&previous.targets[0].target_id),
        )
        .unwrap();

        assert_ne!(retry.id, previous.id);
        assert_eq!(retry.parent_operation_id, Some(previous.id.clone()));
        assert_eq!(retry.retry_of_operation_id, Some(previous.id));
        assert_eq!(retry.targets[0].target_id, current.id);
        validate_enable_current(&retry, &current).unwrap();
    }

    #[test]
    fn enable_retry_rejects_a_missing_or_already_enabled_target() {
        let previous = create_enable_plan(&disabled_program()).unwrap();
        let target_id = previous.targets[0].target_id.clone();

        let missing =
            create_retry_plan(&[], &previous, std::slice::from_ref(&target_id)).unwrap_err();
        assert_eq!(missing.code, OperationErrorCode::ValidationFailed);

        let mut enabled = disabled_program();
        enabled.enabled = true;
        let already_enabled = create_retry_plan(
            std::slice::from_ref(&enabled),
            &previous,
            std::slice::from_ref(&target_id),
        )
        .unwrap_err();
        assert_eq!(already_enabled.code, OperationErrorCode::ValidationFailed);
    }

    #[test]
    fn windows_security_startup_is_protected() {
        let mut source = program();
        source.name = "Windows Security notification icon".into();
        source.command = r"C:\Windows\System32\SecurityHealthSystray.exe".into();
        assert_eq!(
            create_plan(&source).unwrap_err().code,
            OperationErrorCode::TargetProtected
        );
    }

    #[test]
    fn changed_target_is_rejected() {
        let source = program();
        let plan = create_plan(&source).unwrap();
        let mut changed = source;
        changed.command = r"C:\changed.exe".into();

        let error = validate_current(&plan, &changed).unwrap_err();
        assert_eq!(error.code, OperationErrorCode::TargetChangedSincePreview);
    }

    #[test]
    fn enable_plan_requires_a_persisted_recovery_record() {
        let plan = create_enable_plan(&disabled_program()).unwrap();

        assert_eq!(plan.descriptor.id.0, "startup.enable_program");
        assert_eq!(plan.reversibility, Reversibility::NotReversible);
        validate_enable_current(&plan, &disabled_program()).unwrap();
    }

    #[test]
    fn enable_plan_rejects_active_or_untracked_entries() {
        assert_eq!(
            create_enable_plan(&program()).unwrap_err().code,
            OperationErrorCode::ValidationFailed
        );
        let mut untracked = disabled_program();
        untracked.recovery_action_id = None;
        assert_eq!(
            create_enable_plan(&untracked).unwrap_err().code,
            OperationErrorCode::ValidationFailed
        );
    }

    #[test]
    fn changed_disabled_target_is_rejected() {
        let source = disabled_program();
        let plan = create_enable_plan(&source).unwrap();
        let mut changed = source;
        changed.command = r"C:\changed.exe".into();

        assert_eq!(
            validate_enable_current(&plan, &changed).unwrap_err().code,
            OperationErrorCode::TargetChangedSincePreview
        );
    }

    #[test]
    fn tampered_target_identifier_is_rejected() {
        let source = program();
        let mut plan = create_plan(&source).unwrap();
        plan.targets[0].target_id = "b".repeat(64);

        assert_eq!(
            validate_current(&plan, &source).unwrap_err().code,
            OperationErrorCode::ValidationFailed
        );
    }

    #[test]
    fn verified_mutation_reconciles_with_durable_recovery() {
        let source = program();
        let plan = create_plan(&source).unwrap();
        let execution = finish(
            &plan,
            &source,
            StartupMutationResult {
                status: "completed".into(),
                action_id: "undo-1".into(),
                message: "disabled".into(),
                recovery_available: true,
                verified: true,
            },
        )
        .unwrap();

        assert_eq!(execution.result.requested, 1);
        assert_eq!(execution.result.changed, 1);
        assert_eq!(execution.result.verified, 1);
        assert_eq!(execution.result.undo, UndoAvailability::NotReversible);
        assert_eq!(execution.recovery_action_id, "undo-1");
    }
}
