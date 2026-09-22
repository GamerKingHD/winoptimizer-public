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
use crate::startup::guidance::GuidanceInfo;
use crate::startup::scheduler::ScheduledTask;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub struct ScheduledTaskExecution {
    pub result: OperationResult,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ScheduledTaskTargetSnapshot {
    task: ScheduledTask,
    enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct ScheduledTaskGuidanceRetrySnapshot {
    task: ScheduledTask,
    guidance: GuidanceInfo,
    enabled: bool,
}

pub fn create_plan(task: &ScheduledTask, enabled: bool) -> Result<OperationPlan, OperationError> {
    if !task.can_modify || crate::startup::scheduler::is_protected(&task.name) {
        return Err(OperationError {
            code: OperationErrorCode::TargetProtected,
            message: format!(
                "{} is protected because Windows needs it enabled.",
                task.name
            ),
            target_id: Some(task.name.clone()),
            retryable: false,
        });
    }

    if task.definition.task_path != task.name || task.definition.definition_xml.trim().is_empty() {
        return Err(OperationError::validation(
            "scheduled task definition snapshot is missing or does not match its identity",
        ));
    }
    if !task.definition.exact_capture_available() {
        return Err(OperationError::validation(
            "exact Task Scheduler definition/security-descriptor capture is unavailable",
        ));
    }

    let currently_enabled = is_enabled(task);
    if task.definition.enabled != currently_enabled {
        return Err(OperationError::validation(
            "scheduled task enabled state is inconsistent with its native snapshot",
        ));
    }
    if currently_enabled == enabled {
        return Err(OperationError::validation(format!(
            "scheduled task is already {}",
            if enabled { "enabled" } else { "disabled" }
        )));
    }

    let snapshot = ScheduledTaskTargetSnapshot {
        task: task.clone(),
        enabled,
    };
    let requested_action = serde_json::to_string(&snapshot)
        .map_err(|error| validation(format!("failed to serialize scheduled task: {error}")))?;
    let action = if enabled { "Enable" } else { "Disable" };
    let risk = if task.classification == "recommended_disable" {
        RiskLevel::Medium
    } else {
        RiskLevel::High
    };

    Ok(OperationPlan {
        id: OperationId(uuid::Uuid::new_v4().to_string()),
        descriptor: OperationDescriptor {
            id: OperationId("startup.scheduled_task_change".into()),
            capability_id: "startup.tasks".into(),
            title: format!("{action} scheduled task"),
            category: OperationCategory::Startup,
            risk,
            privilege: PrivilegeRequirement::Administrator,
            reversibility: Reversibility::BestEffort,
            reboot_policy: RebootPolicy::NotRequired,
            experimental: true,
            proof_status: ProofStatus::Experimental,
            exposure: ExposureLevel::Advanced,
            recovery: RecoveryLevel::BestEffort,
            restore_point_policy: crate::core::operation::RestorePointPolicy::NotNeeded,
            mutates_system: true,
            requires_confirmation: true,
            requires_revalidation: true,
            requires_verification: true,
            requires_journal: true,
        },
        targets: vec![PlannedTarget {
            target_id: task.name.clone(),
            requested_action,
            description: format!("{action} {}", task.name),
        }],
        warnings: vec![
            OperationWarning {
                code: "scheduled_task_experimental".into(),
                message: "The native Task Scheduler XML and security descriptor are captured; the postcondition can be verified exactly, while restoration remains best-effort and is not an automatic undo path.".into(),
            },
            OperationWarning {
                code: "scheduled_task_dependency".into(),
                message: task.recommendation.clone(),
            },
        ],
        expected_effects: vec![ExpectedEffect {
            target_id: task.name.clone(),
            description: format!(
                "Task Scheduler reports the task {}.",
                if enabled { "enabled" } else { "disabled" }
            ),
        }],
        requires_elevation: true,
        requires_restart: false,
        reversibility: Reversibility::BestEffort,
        parent_operation_id: None,
        retry_of_operation_id: None,
    })
}

pub fn create_retry_plan(
    current: &[ScheduledTask],
    previous_plan: &OperationPlan,
    unresolved_target_ids: &[String],
) -> Result<OperationPlan, OperationError> {
    if previous_plan.descriptor.id.0 != "startup.scheduled_task_change" {
        return Err(validation(
            "retry requires domain-specific rediscovery for this operation",
        ));
    }
    if unresolved_target_ids.len() != 1 {
        return Err(validation(
            "scheduled-task retry requires exactly one target",
        ));
    }
    let previous = decode_plan(previous_plan)?;
    if unresolved_target_ids[0] != previous.task.name {
        return Err(validation(
            "scheduled-task retry target does not match the previous plan",
        ));
    }
    let current_task = current
        .iter()
        .find(|task| task.name == previous.task.name)
        .ok_or_else(|| {
            validation("scheduled task was not rediscovered; no retry plan was created")
        })?;

    let mut plan = create_plan(current_task, previous.enabled)?;
    plan.parent_operation_id = Some(previous_plan.id.clone());
    plan.retry_of_operation_id = Some(previous_plan.id.clone());
    plan.warnings.push(OperationWarning {
        code: "retry_of_operation".into(),
        message: previous_plan.id.0.clone(),
    });
    plan.validate_relationships()?;
    Ok(plan)
}

pub fn create_guidance_retry_plan(
    current: &[ScheduledTask],
    previous_plan: &OperationPlan,
    unresolved_target_ids: &[String],
) -> Result<OperationPlan, OperationError> {
    let requested_targets = decode_guidance_retry_targets(previous_plan, unresolved_target_ids)?;
    let mut plan =
        super::scheduled_tasks_guidance_operation::create_plan(&requested_targets, current)?;
    let previous_id = previous_plan.id.clone();
    plan.parent_operation_id = Some(previous_id.clone());
    plan.retry_of_operation_id = Some(previous_id.clone());
    plan.warnings.push(OperationWarning {
        code: "retry_of_operation".into(),
        message: previous_id.0,
    });
    plan.validate_relationships()?;
    Ok(plan)
}

fn decode_guidance_retry_targets(
    previous_plan: &OperationPlan,
    unresolved_target_ids: &[String],
) -> Result<
    Vec<super::scheduled_tasks_guidance_operation::ScheduledTaskGuidanceTarget>,
    OperationError,
> {
    if previous_plan.descriptor.id.0 != "startup.scheduled_tasks_guidance"
        || previous_plan.descriptor.category != OperationCategory::Startup
        || !previous_plan.descriptor.experimental
        || previous_plan.descriptor.reversibility != Reversibility::BestEffort
        || !previous_plan.descriptor.mutates_system
        || !previous_plan.descriptor.requires_confirmation
        || !previous_plan.descriptor.requires_revalidation
        || !previous_plan.descriptor.requires_verification
        || !previous_plan.descriptor.requires_journal
        || !previous_plan.requires_elevation
        || previous_plan.reversibility != Reversibility::BestEffort
        || previous_plan.targets.is_empty()
    {
        return Err(validation(
            "invalid scheduled-task guidance retry source plan",
        ));
    }
    previous_plan.descriptor.validate_contract()?;
    previous_plan.validate_relationships()?;
    if unresolved_target_ids.is_empty() {
        return Err(validation(
            "scheduled-task guidance retry requires at least one target",
        ));
    }

    let mut requested_targets = Vec::with_capacity(unresolved_target_ids.len());
    for target_id in unresolved_target_ids {
        if requested_targets.iter().any(
            |target: &super::scheduled_tasks_guidance_operation::ScheduledTaskGuidanceTarget| {
                target.task_name == *target_id
            },
        ) {
            return Err(validation(
                "scheduled-task guidance retry cannot contain duplicate targets",
            ));
        }
        let target = previous_plan
            .targets
            .iter()
            .find(|target| target.target_id == *target_id)
            .ok_or_else(|| {
                validation("scheduled-task guidance retry target is not in the source plan")
            })?;
        let snapshot: ScheduledTaskGuidanceRetrySnapshot =
            serde_json::from_str(&target.requested_action).map_err(|error| {
                validation(format!(
                    "invalid scheduled-task guidance retry target snapshot: {error}"
                ))
            })?;
        if snapshot.task.name != target.target_id || snapshot.guidance != snapshot.task.guidance {
            return Err(validation(
                "scheduled-task guidance retry target snapshot does not match the source plan",
            ));
        }
        requested_targets.push(
            super::scheduled_tasks_guidance_operation::ScheduledTaskGuidanceTarget {
                task_name: snapshot.task.name,
                enabled: snapshot.enabled,
            },
        );
    }
    Ok(requested_targets)
}

pub fn confirmed_action(
    plan: &OperationPlan,
    current: &ScheduledTask,
) -> Result<bool, OperationError> {
    let snapshot = decode_plan(plan)?;
    if !snapshot.task.state_matches(current) {
        return Err(OperationError {
            code: OperationErrorCode::TargetChangedSincePreview,
            message: "Scheduled task changed after confirmation; review it again.".into(),
            target_id: Some(current.name.clone()),
            retryable: true,
        });
    }
    Ok(snapshot.enabled)
}

pub fn finish(
    plan: &OperationPlan,
    before: &ScheduledTask,
    after: &ScheduledTask,
) -> Result<ScheduledTaskExecution, OperationError> {
    let enabled = confirmed_action(plan, before)?;
    if !after.definition.exact_capture_available()
        || before.name != after.name
        || before.path != after.path
        || is_enabled(after) != enabled
        || !crate::startup::scheduler::definition_matches_except_runtime_state(
            &before.definition,
            &after.definition,
        )
    {
        return Err(OperationError {
            code: OperationErrorCode::VerificationFailed,
            message: "Task Scheduler postcondition could not be verified against an exact native definition/security-descriptor snapshot.".into(),
            target_id: Some(before.name.clone()),
            retryable: true,
        });
    }

    let action = if enabled { "enabled" } else { "disabled" };
    let outcome = TargetOutcome {
        target_id: before.name.clone(),
        requested_action: action.into(),
        before: Some(state_snapshot(before)),
        after: Some(state_snapshot(after)),
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
    Ok(ScheduledTaskExecution {
        result,
        message: format!(
            "{} was {} and its exact native postcondition was verified; restoration remains best-effort.",
            before.name, action
        ),
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
    before: &ScheduledTask,
    enabled: bool,
    error: OperationError,
) -> TargetOutcome {
    TargetOutcome {
        target_id: before.name.clone(),
        requested_action: if enabled {
            "enable_task".into()
        } else {
            "disable_task".into()
        },
        before: Some(state_snapshot(before)),
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
                "target {target_id} is not present in the confirmed scheduled-task plan"
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
    before: &ScheduledTask,
    execution: &ScheduledTaskExecution,
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
        captured_original_states: vec![CapturedOriginalState::TypedState {
            target_id: before.name.clone(),
            state: state_snapshot(before),
        }],
        target_outcomes: execution.result.outcomes.clone(),
        verification_results: vec![VerificationRecord {
            target_id: before.name.clone(),
            status: VerificationStatus::Verified,
            message: "Exact native Task Scheduler postcondition was re-queried after the change; restoration remains best-effort.".into(),
        }],
        restart_requirement: RebootPolicy::NotRequired,
        undo_eligibility: UndoEligibility {
            eligible: false,
            reason: "Typed native Task Scheduler definition/state is retained in the target outcome; exact postcondition verification is available, but restoration remains best-effort.".into(),
            automated_restoration_test: None,
        },
        undo_execution_result: None,
        diagnostic: None,
    }
}

fn decode_plan(plan: &OperationPlan) -> Result<ScheduledTaskTargetSnapshot, OperationError> {
    if plan.descriptor.id.0 != "startup.scheduled_task_change"
        || plan.descriptor.category != OperationCategory::Startup
        || !plan.descriptor.experimental
        || plan.descriptor.capability_id != "startup.tasks"
        || plan.descriptor.privilege != PrivilegeRequirement::Administrator
        || plan.descriptor.reversibility != Reversibility::BestEffort
        || plan.descriptor.recovery != RecoveryLevel::BestEffort
        || plan.descriptor.proof_status != ProofStatus::Experimental
        || !plan.descriptor.mutates_system
        || !plan.descriptor.requires_confirmation
        || !plan.descriptor.requires_revalidation
        || !plan.descriptor.requires_verification
        || !plan.descriptor.requires_journal
        || !plan.requires_elevation
        || plan.requires_restart
        || plan.targets.len() != 1
        || plan.reversibility != Reversibility::BestEffort
    {
        return Err(validation("invalid scheduled-task operation plan"));
    }
    plan.descriptor.validate_contract()?;
    plan.validate_relationships()?;
    let snapshot: ScheduledTaskTargetSnapshot =
        serde_json::from_str(&plan.targets[0].requested_action)
            .map_err(|error| validation(format!("invalid scheduled task snapshot: {error}")))?;
    if snapshot.task.name != plan.targets[0].target_id
        || !snapshot.task.can_modify
        || crate::startup::scheduler::is_protected(&snapshot.task.name)
        || snapshot.task.definition.task_path != snapshot.task.name
        || snapshot.task.definition.definition_xml.trim().is_empty()
        || !snapshot.task.definition.exact_capture_available()
        || snapshot.task.definition.enabled != is_enabled(&snapshot.task)
        || is_enabled(&snapshot.task) == snapshot.enabled
    {
        return Err(validation(
            "scheduled task does not match the confirmed plan",
        ));
    }
    Ok(snapshot)
}

fn is_enabled(task: &ScheduledTask) -> bool {
    task.definition.enabled && !task.status.eq_ignore_ascii_case("disabled")
}

fn state_snapshot(task: &ScheduledTask) -> StateSnapshot {
    StateSnapshot {
        summary: format!("{} is {}", task.name, task.status),
        properties: BTreeMap::from([
            ("name".into(), task.name.clone()),
            ("path".into(), task.path.clone()),
            ("status".into(), task.status.clone()),
            ("classification".into(), task.classification.clone()),
            (
                "definition_xml".into(),
                task.definition.definition_xml.clone(),
            ),
            ("enabled".into(), task.definition.enabled.to_string()),
            ("task_path".into(), task.definition.task_path.clone()),
            (
                "current_state".into(),
                task.definition.current_state.clone(),
            ),
            (
                "next_run_time".into(),
                task.definition
                    .next_run_time
                    .as_deref()
                    .unwrap_or("unavailable")
                    .into(),
            ),
            ("triggers_xml".into(), task.definition.triggers_xml.clone()),
            ("actions_xml".into(), task.definition.actions_xml.clone()),
            (
                "principals_xml".into(),
                task.definition.principals_xml.clone(),
            ),
            ("run_level".into(), task.definition.run_level.clone()),
            (
                "security_descriptor".into(),
                task.definition
                    .security_descriptor
                    .as_deref()
                    .unwrap_or("unavailable")
                    .into(),
            ),
        ]),
    }
}

fn validation(message: impl Into<String>) -> OperationError {
    OperationError::validation(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::startup::scheduler::TaskDefinitionSnapshot;

    fn definition(name: &str, status: &str) -> TaskDefinitionSnapshot {
        TaskDefinitionSnapshot {
            task_path: name.into(),
            enabled: !status.eq_ignore_ascii_case("disabled"),
            current_state: status.into(),
            definition_xml: format!("<Task><URI>{name}</URI></Task>"),
            security_descriptor: Some("D:P(A;;FA;;;SY)".into()),
            ..Default::default()
        }
    }

    fn task(name: &str, status: &str) -> ScheduledTask {
        ScheduledTask {
            name: name.into(),
            path: String::new(),
            status: status.into(),
            trigger: String::new(),
            classification: "review".into(),
            can_modify: true,
            recommendation: "Review".into(),
            guidance: crate::startup::guidance::scheduled_task_guidance(name),
            definition: definition(name, status),
        }
    }

    #[test]
    fn protected_task_cannot_receive_a_plan() {
        let mut protected = task(r"\Microsoft\Windows\WindowsUpdate\Fixture", "Ready");
        protected.classification = "protected".into();
        protected.can_modify = false;
        assert_eq!(
            create_plan(&protected, false).unwrap_err().code,
            OperationErrorCode::TargetProtected
        );
    }

    #[test]
    fn task_without_an_exact_security_descriptor_cannot_receive_a_plan() {
        let mut incomplete = task(r"\Vendor\IncompleteFixture", "Ready");
        incomplete.definition.security_descriptor = None;

        assert_eq!(
            create_plan(&incomplete, false).unwrap_err().code,
            OperationErrorCode::ValidationFailed
        );
    }

    #[test]
    fn changed_task_state_is_rejected_after_preview() {
        let original = task(r"\Vendor\Fixture", "Ready");
        let plan = create_plan(&original, false).unwrap();
        let mut changed = original.clone();
        changed.status = "Disabled".into();
        assert_eq!(
            confirmed_action(&plan, &changed).unwrap_err().code,
            OperationErrorCode::TargetChangedSincePreview
        );
    }

    #[test]
    fn retry_rebuilds_from_current_task_state_and_links_the_source() {
        let previous_task = task(r"\Vendor\RetryFixture", "Ready");
        let previous = create_plan(&previous_task, false).unwrap();
        let mut current_task = previous_task;
        current_task.definition.next_run_time = Some("2026-08-11T12:00:00Z".into());
        let retry = create_retry_plan(
            std::slice::from_ref(&current_task),
            &previous,
            std::slice::from_ref(&previous.targets[0].target_id),
        )
        .unwrap();

        assert_ne!(retry.id, previous.id);
        assert_eq!(retry.parent_operation_id, Some(previous.id.clone()));
        assert_eq!(retry.retry_of_operation_id, Some(previous.id));
        assert!(!confirmed_action(&retry, &current_task).unwrap());
    }

    #[test]
    fn retry_rejects_a_missing_or_already_satisfied_task() {
        let previous_task = task(r"\Vendor\RetryFixture", "Ready");
        let previous = create_plan(&previous_task, false).unwrap();
        let target_id = previous.targets[0].target_id.clone();
        assert!(create_retry_plan(&[], &previous, std::slice::from_ref(&target_id)).is_err());

        let disabled = task(r"\Vendor\RetryFixture", "Disabled");
        let error = create_retry_plan(
            std::slice::from_ref(&disabled),
            &previous,
            std::slice::from_ref(&target_id),
        )
        .unwrap_err();
        assert_eq!(error.code, OperationErrorCode::ValidationFailed);
    }

    #[test]
    fn guidance_retry_rebuilds_from_current_task_snapshot_and_links_source() {
        let before = task(
            r"\Microsoft\Windows\Customer Experience Improvement Program\Consolidator",
            "Ready",
        );
        let previous = crate::core::scheduled_tasks_guidance_operation::create_plan(
            &[
                crate::core::scheduled_tasks_guidance_operation::ScheduledTaskGuidanceTarget {
                    task_name: before.name.clone(),
                    enabled: false,
                },
            ],
            std::slice::from_ref(&before),
        )
        .unwrap();
        let mut current = before.clone();
        current.definition.next_run_time = Some("2026-08-11T12:00:00Z".into());

        let retry = create_guidance_retry_plan(
            std::slice::from_ref(&current),
            &previous,
            std::slice::from_ref(&previous.targets[0].target_id),
        )
        .unwrap();

        assert_ne!(retry.id, previous.id);
        assert_eq!(retry.parent_operation_id, Some(previous.id.clone()));
        assert_eq!(retry.retry_of_operation_id, Some(previous.id));
        assert!(retry.descriptor.mutates_system);
        assert!(retry.descriptor.requires_confirmation);
        assert!(retry.descriptor.requires_revalidation);
        assert!(retry.descriptor.requires_verification);
        assert!(retry.descriptor.requires_journal);
        assert!(retry.requires_elevation);
        let snapshot: ScheduledTaskGuidanceRetrySnapshot =
            serde_json::from_str(&retry.targets[0].requested_action).unwrap();
        assert_eq!(
            snapshot.task.definition.next_run_time,
            current.definition.next_run_time
        );
        assert!(
            crate::core::scheduled_tasks_guidance_operation::confirm_current(
                &retry,
                std::slice::from_ref(&current),
            )
            .is_ok()
        );
    }

    #[test]
    fn enable_and_disable_plans_preserve_the_native_snapshot() {
        let ready = task(r"\Vendor\EnableFixture", "Ready");
        let disable_plan = create_plan(&ready, false).unwrap();
        assert!(!confirmed_action(&disable_plan, &ready).unwrap());

        let disabled = task(r"\Vendor\DisableFixture", "Disabled");
        let enable_plan = create_plan(&disabled, true).unwrap();
        assert!(confirmed_action(&enable_plan, &disabled).unwrap());

        let mut after = ready.clone();
        after.status = "Disabled".into();
        after.definition.enabled = false;
        after.definition.current_state = "Disabled".into();
        finish(&disable_plan, &ready, &after).unwrap();

        after.definition.definition_xml.push_str("<Changed/>");
        assert_eq!(
            finish(&disable_plan, &ready, &after).unwrap_err().code,
            OperationErrorCode::VerificationFailed
        );
    }

    #[test]
    fn incomplete_after_snapshot_cannot_be_reported_as_verified() {
        let ready = task(r"\Vendor\IncompleteAfterFixture", "Ready");
        let plan = create_plan(&ready, false).unwrap();
        let mut after = ready.clone();
        after.status = "Disabled".into();
        after.definition.enabled = false;
        after.definition.current_state = "Disabled".into();
        after.definition.security_descriptor = None;

        assert_eq!(
            finish(&plan, &ready, &after).unwrap_err().code,
            OperationErrorCode::VerificationFailed
        );
    }

    #[test]
    fn journal_snapshot_keeps_full_task_restore_evidence() {
        let mut original = task(r"\Vendor\Evidence", "Ready");
        original.definition.next_run_time = Some("2026-08-11T12:00:00Z".into());
        original.definition.triggers_xml = "<Triggers/>".into();
        original.definition.actions_xml = "<Actions/>".into();
        original.definition.principals_xml = "<Principals/>".into();
        original.definition.run_level = "LeastPrivilege".into();

        let evidence = state_snapshot(&original);
        assert_eq!(
            evidence.properties.get("task_path"),
            Some(&r"\Vendor\Evidence".to_string())
        );
        assert_eq!(
            evidence.properties.get("next_run_time"),
            Some(&"2026-08-11T12:00:00Z".to_string())
        );
        assert_eq!(
            evidence.properties.get("security_descriptor"),
            Some(&"D:P(A;;FA;;;SY)".to_string())
        );
    }

    #[test]
    fn journal_record_uses_typed_before_state_without_advertising_undo() {
        let before = task(r"\Vendor\JournalFixture", "Ready");
        let plan = create_plan(&before, false).unwrap();
        let mut after = before.clone();
        after.status = "Disabled".into();
        after.definition.enabled = false;
        after.definition.current_state = "Disabled".into();
        let execution = finish(&plan, &before, &after).unwrap();

        let record = journal_record(&plan, &before, &execution);

        assert_eq!(record.captured_original_states.len(), 1);
        assert_eq!(
            record.captured_original_states[0].target_id(),
            r"\Vendor\JournalFixture"
        );
        let snapshot = record.target_outcomes[0].before.as_ref().unwrap();
        assert_eq!(
            snapshot.properties.get("definition_xml"),
            Some(&format!("<Task><URI>{}</URI></Task>", before.name))
        );
        assert_eq!(
            snapshot.properties.get("security_descriptor"),
            Some(&"D:P(A;;FA;;;SY)".into())
        );
    }

    #[test]
    fn forged_plan_cannot_request_the_already_captured_task_state() {
        let ready = task(r"\Vendor\ForgedFixture", "Ready");
        let plan = create_plan(&ready, false).unwrap();
        let mut snapshot: ScheduledTaskTargetSnapshot =
            serde_json::from_str(&plan.targets[0].requested_action).unwrap();
        snapshot.enabled = true;
        let mut forged = plan;
        forged.targets[0].requested_action = serde_json::to_string(&snapshot).unwrap();

        assert_eq!(
            confirmed_action(&forged, &ready).unwrap_err().code,
            OperationErrorCode::ValidationFailed
        );
    }
}
