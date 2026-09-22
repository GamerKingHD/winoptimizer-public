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
use crate::core::scheduled_task_operation::ScheduledTaskExecution;
use crate::startup::guidance::{GuidanceAction, GuidanceInfo, GuidanceSourceType};
use crate::startup::scheduler::{self, ScheduledTask};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledTaskGuidanceTarget {
    #[serde(rename = "taskName", alias = "task_name")]
    pub task_name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ScheduledTaskGuidanceTargetSnapshot {
    task: ScheduledTask,
    guidance: GuidanceInfo,
    enabled: bool,
}

pub fn create_plan(
    requested_targets: &[ScheduledTaskGuidanceTarget],
    current: &[ScheduledTask],
) -> Result<OperationPlan, OperationError> {
    if requested_targets.is_empty() {
        return Err(OperationError::validation(
            "select at least one scheduled-task recommendation",
        ));
    }

    let mut snapshots = Vec::with_capacity(requested_targets.len());
    let mut targets = Vec::with_capacity(requested_targets.len());
    let mut expected_effects = Vec::with_capacity(requested_targets.len());
    let mut has_community_source = false;

    for requested in requested_targets {
        if scheduler::is_protected(&requested.task_name) {
            return Err(target_error(
                OperationErrorCode::TargetProtected,
                format!(
                    "Protected scheduled task cannot receive guidance changes: {}",
                    requested.task_name
                ),
                &requested.task_name,
            ));
        }
        if snapshots
            .iter()
            .any(|snapshot: &ScheduledTaskGuidanceTargetSnapshot| {
                snapshot
                    .task
                    .name
                    .eq_ignore_ascii_case(&requested.task_name)
            })
        {
            return Err(OperationError::validation(format!(
                "scheduled task selected more than once: {}",
                requested.task_name
            )));
        }
        let task = current
            .iter()
            .find(|candidate| candidate.name.eq_ignore_ascii_case(&requested.task_name))
            .ok_or_else(|| {
                target_error(
                    OperationErrorCode::TargetNotFound,
                    format!("Scheduled task not found: {}", requested.task_name),
                    &requested.task_name,
                )
            })?;
        validate_requested_action(task, requested.enabled)?;
        let snapshot = ScheduledTaskGuidanceTargetSnapshot {
            task: task.clone(),
            guidance: task.guidance.clone(),
            enabled: requested.enabled,
        };
        has_community_source |= snapshot.guidance.source_type == GuidanceSourceType::Community;
        let requested_action = serde_json::to_string(&snapshot).map_err(|error| {
            OperationError::validation(format!("failed to serialize task guidance: {error}"))
        })?;
        targets.push(PlannedTarget {
            target_id: task.name.clone(),
            requested_action,
            description: format!(
                "{}: {}",
                task.name,
                if requested.enabled {
                    "Keep enabled"
                } else {
                    "Disable if unused"
                }
            ),
        });
        expected_effects.push(ExpectedEffect {
            target_id: task.name.clone(),
            description: format!(
                "Task Scheduler reports the task {}.",
                if requested.enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            ),
        });
        snapshots.push(snapshot);
    }

    let mut warnings = vec![
        OperationWarning {
            code: "scheduled_tasks_guidance_experimental".into(),
            message: "Guided scheduled-task changes remain Experimental; native XML and security-descriptor snapshots are captured, while exact restoration remains guarded.".into(),
        },
        OperationWarning {
            code: "scheduled_task_maintenance".into(),
            message: "Scheduled maintenance can include updates and security work; do not disable tasks without reviewing their purpose.".into(),
        },
    ];
    if has_community_source {
        warnings.push(OperationWarning {
            code: "community_guidance".into(),
            message: "At least one selected recommendation comes from a community source and requires explicit review.".into(),
        });
    }

    Ok(OperationPlan {
        id: OperationId(uuid::Uuid::new_v4().to_string()),
        descriptor: OperationDescriptor {
            id: OperationId("startup.scheduled_tasks_guidance".into()),
            capability_id: "startup.tasks".into(),
            title: "Apply selected scheduled-task guidance".into(),
            category: OperationCategory::Startup,
            risk: RiskLevel::High,
            privilege: PrivilegeRequirement::Administrator,
            reversibility: Reversibility::BestEffort,
            reboot_policy: RebootPolicy::NotRequired,
            experimental: true,
            proof_status: ProofStatus::Experimental,
            exposure: ExposureLevel::Labs,
            recovery: RecoveryLevel::BestEffort,
            restore_point_policy: crate::core::operation::RestorePointPolicy::NotNeeded,
            mutates_system: true,
            requires_confirmation: true,
            requires_revalidation: true,
            requires_verification: true,
            requires_journal: true,
        },
        targets,
        warnings,
        expected_effects,
        requires_elevation: true,
        requires_restart: false,
        reversibility: Reversibility::BestEffort,
        parent_operation_id: None,
        retry_of_operation_id: None,
    })
}

pub fn confirm_current(
    plan: &OperationPlan,
    current: &[ScheduledTask],
) -> Result<Vec<ScheduledTask>, OperationError> {
    let snapshots = decode_plan(plan)?;
    let mut confirmed = Vec::with_capacity(snapshots.len());
    for snapshot in snapshots {
        let task = current
            .iter()
            .find(|candidate| candidate.name.eq_ignore_ascii_case(&snapshot.task.name))
            .ok_or_else(|| {
                target_error(
                    OperationErrorCode::TargetNotFound,
                    format!("Scheduled task not found: {}", snapshot.task.name),
                    &snapshot.task.name,
                )
            })?;
        ensure_current_matches(&snapshot, task)?;
        confirmed.push(task.clone());
    }
    Ok(confirmed)
}

pub fn execute(
    plan: &OperationPlan,
    current: &[ScheduledTask],
) -> Result<(ScheduledTaskExecution, OperationJournalRecord), OperationError> {
    let _ = (plan, current);
    Err(OperationError::validation(
        "mutating scheduled-task guidance execution requires an initialized OperationJournal",
    ))
}

pub fn execute_with_journal(
    plan: &OperationPlan,
    current: &[ScheduledTask],
    journal: &OperationJournal,
) -> Result<(ScheduledTaskExecution, OperationJournalRecord), OperationError> {
    let snapshots = decode_plan(plan)?;
    let mut before = Vec::new();
    let mut outcomes = Vec::with_capacity(snapshots.len());

    for snapshot in snapshots {
        let target_id = snapshot.task.name.clone();
        let Some(task) = current
            .iter()
            .find(|candidate| candidate.name.eq_ignore_ascii_case(&target_id))
        else {
            super::scheduled_task_operation::mark_target_revalidation_failed(
                journal, plan, &target_id,
            )?;
            outcomes.push(failed_outcome(
                &target_id,
                snapshot.enabled,
                None,
                target_error(
                    OperationErrorCode::TargetNotFound,
                    format!("Scheduled task not found: {target_id}"),
                    &target_id,
                ),
            ));
            continue;
        };

        if let Err(error) = ensure_current_matches(&snapshot, task) {
            super::scheduled_task_operation::mark_target_revalidation_failed(
                journal, plan, &target_id,
            )?;
            outcomes.push(failed_outcome(
                &target_id,
                snapshot.enabled,
                Some(task),
                error,
            ));
            continue;
        }
        before.push(task.clone());

        if is_enabled(task) == snapshot.enabled {
            let outcome = TargetOutcome {
                target_id: target_id.clone(),
                requested_action: requested_action(snapshot.enabled),
                before: Some(state_snapshot(task)),
                after: Some(state_snapshot(task)),
                execution: ExecutionStatus::Unchanged,
                verification: VerificationStatus::Verified,
                error: None,
            };
            outcomes.push(outcome);
            continue;
        }

        super::scheduled_task_operation::mark_target_ready_and_mutation_started(
            journal, plan, &target_id,
        )?;
        let outcome = match scheduler::set_task_state_checked(task, snapshot.enabled) {
            Ok(after) if is_enabled(&after) == snapshot.enabled => TargetOutcome {
                target_id: target_id.clone(),
                requested_action: requested_action(snapshot.enabled),
                before: Some(state_snapshot(task)),
                after: Some(state_snapshot(&after)),
                execution: ExecutionStatus::Changed,
                verification: VerificationStatus::Verified,
                error: None,
            },
            Ok(_) => failed_outcome(
                &target_id,
                snapshot.enabled,
                Some(task),
                target_error(
                    OperationErrorCode::VerificationFailed,
                    "Task Scheduler did not report the requested state.",
                    &target_id,
                ),
            ),
            Err(error) => failed_outcome(
                &target_id,
                snapshot.enabled,
                Some(task),
                target_error(
                    OperationErrorCode::ExecutionFailed,
                    error.to_string(),
                    &target_id,
                ),
            ),
        };
        super::scheduled_task_operation::record_target_outcome(journal, plan, &outcome)?;
        outcomes.push(outcome);
    }

    let result = OperationResult::new(
        plan.id.clone(),
        outcomes,
        false,
        UndoAvailability::NotReversible,
    )?;
    let execution = ScheduledTaskExecution {
        message: format!(
            "{} guided scheduled-task change{} applied; {} failed.",
            result.changed,
            if result.changed == 1 {
                " was"
            } else {
                "s were"
            },
            result.failed
        ),
        result,
    };
    let record = journal_record(plan, &execution);
    Ok((execution, record))
}

fn decode_plan(
    plan: &OperationPlan,
) -> Result<Vec<ScheduledTaskGuidanceTargetSnapshot>, OperationError> {
    if plan.descriptor.id.0 != "startup.scheduled_tasks_guidance"
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
        || plan.targets.is_empty()
        || plan.reversibility != Reversibility::BestEffort
    {
        return Err(OperationError::validation(
            "invalid scheduled-task guidance operation plan",
        ));
    }
    plan.descriptor.validate_contract()?;
    plan.validate_relationships()?;

    plan.targets
        .iter()
        .map(|target| {
            let snapshot: ScheduledTaskGuidanceTargetSnapshot =
                serde_json::from_str(&target.requested_action).map_err(|error| {
                    OperationError::validation(format!(
                        "invalid scheduled-task guidance target snapshot: {error}"
                    ))
                })?;
            if snapshot.task.name != target.target_id {
                return Err(OperationError::validation(
                    "scheduled-task guidance target does not match its plan target",
                ));
            }
            if snapshot.guidance != snapshot.task.guidance {
                return Err(OperationError::validation(
                    "scheduled-task guidance snapshot does not match its native task snapshot",
                ));
            }
            validate_requested_action(&snapshot.task, snapshot.enabled)?;
            Ok(snapshot)
        })
        .collect()
}

fn validate_requested_action(task: &ScheduledTask, enabled: bool) -> Result<(), OperationError> {
    if task.guidance.protected || !task.can_modify || scheduler::is_protected(&task.name) {
        return Err(target_error(
            OperationErrorCode::TargetProtected,
            format!(
                "Protected scheduled task cannot receive guidance changes: {}",
                task.name
            ),
            &task.name,
        ));
    }
    if task.definition.task_path != task.name || task.definition.definition_xml.trim().is_empty() {
        return Err(target_error(
            OperationErrorCode::ValidationFailed,
            format!(
                "{} is missing its native Task Scheduler definition snapshot",
                task.name
            ),
            &task.name,
        ));
    }
    if !task.definition.exact_capture_available() {
        return Err(target_error(
            OperationErrorCode::ValidationFailed,
            format!(
                "{} is missing its exact native Task Scheduler definition/security descriptor",
                task.name
            ),
            &task.name,
        ));
    }
    if task.definition.enabled != is_enabled(task) {
        return Err(target_error(
            OperationErrorCode::ValidationFailed,
            format!(
                "{} has inconsistent native Task Scheduler enabled state",
                task.name
            ),
            &task.name,
        ));
    }
    let allowed = match task.guidance.action {
        GuidanceAction::DisableIfUnused => !enabled,
        GuidanceAction::KeepEnabled | GuidanceAction::SetManual | GuidanceAction::ReviewFirst => {
            false
        }
    };
    if !allowed {
        return Err(target_error(
            OperationErrorCode::ValidationFailed,
            format!(
                "{} is guidance group {}; review it as an individual scheduled-task action",
                task.name,
                action_label(task.guidance.action)
            ),
            &task.name,
        ));
    }
    Ok(())
}

fn ensure_current_matches(
    snapshot: &ScheduledTaskGuidanceTargetSnapshot,
    current: &ScheduledTask,
) -> Result<(), OperationError> {
    if !snapshot.task.state_matches(current) {
        return Err(target_error(
            OperationErrorCode::TargetChangedSincePreview,
            format!(
                "{} changed after confirmation; review the guidance again.",
                current.name
            ),
            &current.name,
        ));
    }
    if snapshot.guidance != crate::startup::guidance::scheduled_task_guidance(&current.name) {
        return Err(target_error(
            OperationErrorCode::TargetChangedSincePreview,
            format!(
                "{} guidance changed after confirmation; review it again.",
                current.name
            ),
            &current.name,
        ));
    }
    Ok(())
}

fn is_enabled(task: &ScheduledTask) -> bool {
    task.definition.enabled && !task.status.eq_ignore_ascii_case("disabled")
}

fn action_label(action: GuidanceAction) -> &'static str {
    match action {
        GuidanceAction::KeepEnabled => "Keep enabled",
        GuidanceAction::SetManual => "Set to Manual",
        GuidanceAction::DisableIfUnused => "Disable if unused",
        GuidanceAction::ReviewFirst => "Review first",
    }
}

fn requested_action(enabled: bool) -> String {
    format!("guidance:{}", if enabled { "enable" } else { "disable" })
}

fn state_snapshot(task: &ScheduledTask) -> StateSnapshot {
    StateSnapshot {
        summary: format!("{} is {}", task.name, task.status),
        properties: BTreeMap::from([
            ("name".into(), task.name.clone()),
            ("path".into(), task.path.clone()),
            ("status".into(), task.status.clone()),
            (
                "guidance_group".into(),
                action_label(task.guidance.action).into(),
            ),
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

fn failed_outcome(
    target_id: &str,
    enabled: bool,
    before: Option<&ScheduledTask>,
    error: OperationError,
) -> TargetOutcome {
    TargetOutcome {
        target_id: target_id.into(),
        requested_action: requested_action(enabled),
        before: before.map(state_snapshot),
        after: None,
        execution: ExecutionStatus::Failed,
        verification: VerificationStatus::Failed,
        error: Some(error),
    }
}

fn target_error(
    code: OperationErrorCode,
    message: impl Into<String>,
    target_id: &str,
) -> OperationError {
    OperationError {
        code,
        message: message.into(),
        target_id: Some(target_id.into()),
        retryable: matches!(
            code,
            OperationErrorCode::TargetNotFound
                | OperationErrorCode::TargetChangedSincePreview
                | OperationErrorCode::ExecutionFailed
                | OperationErrorCode::VerificationFailed
        ),
    }
}

fn journal_record(
    plan: &OperationPlan,
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
        captured_original_states: execution
            .result
            .outcomes
            .iter()
            .filter_map(|outcome| {
                outcome.before.clone().map(|state| {
                    CapturedOriginalState::TypedState {
                        target_id: outcome.target_id.clone(),
                        state,
                    }
                })
            })
            .collect(),
        target_outcomes: execution.result.outcomes.clone(),
        verification_results: execution
            .result
            .outcomes
            .iter()
            .map(|outcome| VerificationRecord {
                target_id: outcome.target_id.clone(),
                status: outcome.verification,
                message: "Task Scheduler state was re-queried where execution was attempted.".into(),
            })
            .collect(),
        restart_requirement: RebootPolicy::NotRequired,
        undo_eligibility: UndoEligibility {
            eligible: false,
            reason: "Typed native Task Scheduler definition/state is retained in target outcomes; exact XML/security-descriptor restoration is not available.".into(),
            automated_restoration_test: None,
        },
        undo_execution_result: None,
        diagnostic: None,
    }
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
    fn review_first_and_protected_tasks_cannot_enter_guidance_plan() {
        let review = task(r"\Vendor\Task", "Ready");
        assert_eq!(
            validate_requested_action(&review, false).unwrap_err().code,
            OperationErrorCode::ValidationFailed
        );

        let mut protected = task(r"\Microsoft\Windows\WindowsUpdate\Fixture", "Ready");
        protected.can_modify = false;
        protected.classification = "protected".into();
        assert_eq!(
            validate_requested_action(&protected, false)
                .unwrap_err()
                .code,
            OperationErrorCode::TargetProtected
        );
    }

    #[test]
    fn guidance_plan_rejects_incomplete_native_task_capture() {
        let mut incomplete = task(
            r"\Microsoft\Windows\Customer Experience Improvement Program\Consolidator",
            "Ready",
        );
        incomplete.definition.security_descriptor = None;

        assert_eq!(
            validate_requested_action(&incomplete, false)
                .unwrap_err()
                .code,
            OperationErrorCode::ValidationFailed
        );
    }

    #[test]
    fn stale_task_state_is_rejected_after_preview() {
        let before = task(
            r"\Microsoft\Windows\Customer Experience Improvement Program\Consolidator",
            "Ready",
        );
        let plan = create_plan(
            &[ScheduledTaskGuidanceTarget {
                task_name: before.name.clone(),
                enabled: false,
            }],
            std::slice::from_ref(&before),
        )
        .unwrap();
        let mut changed = before.clone();
        changed.status = "Running".into();
        assert_eq!(
            confirm_current(&plan, &[changed]).unwrap_err().code,
            OperationErrorCode::TargetChangedSincePreview
        );
    }

    #[test]
    fn execution_reports_stale_and_unchanged_targets_together() {
        let first = task(
            r"\Microsoft\Windows\Customer Experience Improvement Program\Consolidator",
            "Ready",
        );
        let second = task(
            r"\Microsoft\Windows\Customer Experience Improvement Program\UsbCeipTask",
            "Disabled",
        );
        let plan = create_plan(
            &[
                ScheduledTaskGuidanceTarget {
                    task_name: first.name.clone(),
                    enabled: false,
                },
                ScheduledTaskGuidanceTarget {
                    task_name: second.name.clone(),
                    enabled: false,
                },
            ],
            &[first.clone(), second.clone()],
        )
        .unwrap();
        let mut stale_first = first;
        stale_first.status = "Running".into();

        let directory = tempfile::tempdir().unwrap();
        let journal = OperationJournal::open_at(&directory.path().join("journal.db")).unwrap();
        journal.begin_confirmed_execution(&plan).unwrap();
        let (execution, record) =
            execute_with_journal(&plan, &[stale_first, second], &journal).unwrap();
        assert_eq!(execution.result.failed, 1);
        assert_eq!(execution.result.unchanged, 1);
        assert_eq!(
            execution.result.completion_state(),
            super::super::operation::OperationState::PartiallySucceeded
        );
        assert_eq!(record.captured_original_states.len(), 2);
        assert!(record.captured_original_states.iter().any(|capture| {
            capture.target_id()
                == r"\Microsoft\Windows\Customer Experience Improvement Program\UsbCeipTask"
        }));
        assert!(record
            .target_outcomes
            .iter()
            .find(|outcome| outcome.target_id
                == r"\Microsoft\Windows\Customer Experience Improvement Program\UsbCeipTask")
            .and_then(|outcome| outcome.before.as_ref())
            .and_then(|snapshot| snapshot.properties.get("definition_xml"))
            .is_some());
    }
}
