use super::operation::{
    ExecutionStatus, ExpectedEffect, ExposureLevel, OperationCategory, OperationDescriptor,
    OperationError, OperationErrorCode, OperationId, OperationPlan, OperationResult,
    OperationWarning, PlannedTarget, PrivilegeRequirement, ProofStatus, RebootPolicy,
    RecoveryLevel, Reversibility, RiskLevel, StateSnapshot, TargetOutcome, UndoAvailability,
    VerificationStatus,
};
use super::operation_journal::{
    JournalRecordKind, OperationJournal, OperationJournalRecord, UndoEligibility,
    VerificationRecord, OPERATION_SCHEMA_VERSION,
};
use crate::startup::services::{self, ServiceInfo};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize)]
pub struct ServicesBulkExecution {
    pub result: OperationResult,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ServiceTargetSnapshot {
    service: ServiceInfo,
}

pub fn create_manual_plan(services: &[ServiceInfo]) -> Result<OperationPlan, OperationError> {
    let candidates: Vec<&ServiceInfo> = services
        .iter()
        .filter(|service| {
            !services::is_protected(&service.name)
                && !service.start_type.eq_ignore_ascii_case("demand")
        })
        .collect();
    if candidates.is_empty() {
        return Err(OperationError::validation(
            "All eligible services already use Manual startup.",
        ));
    }

    let targets = candidates
        .iter()
        .map(|service| {
            let snapshot = ServiceTargetSnapshot {
                service: (*service).clone(),
            };
            let requested_action = serde_json::to_string(&snapshot).map_err(|error| {
                OperationError::validation(format!("failed to serialize service target: {error}"))
            })?;
            Ok(PlannedTarget {
                target_id: service.name.clone(),
                requested_action,
                description: format!(
                    "Set {} ({}) to Manual startup",
                    service.display_name, service.name
                ),
            })
        })
        .collect::<Result<Vec<_>, OperationError>>()?;

    let expected_effects = targets
        .iter()
        .map(|target| ExpectedEffect {
            target_id: target.target_id.clone(),
            description: "Service Control Manager reports demand/manual startup.".into(),
        })
        .collect();

    Ok(OperationPlan {
        id: OperationId(uuid::Uuid::new_v4().to_string()),
        descriptor: OperationDescriptor {
            id: OperationId("startup.services_set_manual".into()),
            capability_id: "startup.services".into(),
            title: "Set eligible services to Manual".into(),
            category: OperationCategory::Startup,
            risk: RiskLevel::High,
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
        targets,
        warnings: vec![
            OperationWarning {
                code: "services_bulk_experimental".into(),
                message: format!(
                    "{} eligible services will change. Protected Windows services are excluded.",
                    candidates.len()
                ),
            },
            OperationWarning {
                code: "service_dependencies".into(),
                message: "Manual startup can affect applications that expect a service to start automatically.".into(),
            },
        ],
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
    current: &[ServiceInfo],
) -> Result<Vec<ServiceInfo>, OperationError> {
    let snapshots = decode_plan(plan)?;
    let mut confirmed = Vec::with_capacity(snapshots.len());
    for snapshot in snapshots {
        let service = current
            .iter()
            .find(|candidate| candidate.name.eq_ignore_ascii_case(&snapshot.service.name))
            .ok_or_else(|| OperationError {
                code: OperationErrorCode::TargetNotFound,
                message: format!("Service not found: {}", snapshot.service.name),
                target_id: Some(snapshot.service.name.clone()),
                retryable: true,
            })?;
        if !service.state_matches(&snapshot.service) {
            return Err(OperationError {
                code: OperationErrorCode::TargetChangedSincePreview,
                message: format!(
                    "{} changed after confirmation; review the bulk action again.",
                    snapshot.service.name
                ),
                target_id: Some(snapshot.service.name.clone()),
                retryable: true,
            });
        }
        confirmed.push(service.clone());
    }
    Ok(confirmed)
}

pub fn create_retry_plan(
    current: &[ServiceInfo],
    previous_plan: &OperationPlan,
    unresolved_target_ids: &[String],
) -> Result<OperationPlan, OperationError> {
    if previous_plan.descriptor.id.0 != "startup.services_set_manual" {
        return Err(OperationError::validation(
            "retry requires domain-specific rediscovery for this operation",
        ));
    }
    if unresolved_target_ids.is_empty() {
        return Err(OperationError::validation(
            "bulk service retry requires at least one target",
        ));
    }
    let previous = decode_plan(previous_plan)?;
    let mut retry_services = Vec::with_capacity(unresolved_target_ids.len());
    let mut seen_targets = BTreeSet::new();
    for target_id in unresolved_target_ids {
        if !seen_targets.insert(target_id.to_ascii_lowercase()) {
            return Err(OperationError::validation(
                "bulk service retry cannot contain duplicate targets",
            ));
        }
        let planned = previous
            .iter()
            .find(|snapshot| snapshot.service.name.eq_ignore_ascii_case(target_id))
            .ok_or_else(|| {
                OperationError::validation(
                    "bulk service retry target does not match the previous plan",
                )
            })?;
        let current_service = current
            .iter()
            .find(|service| service.name.eq_ignore_ascii_case(&planned.service.name))
            .ok_or_else(|| OperationError {
                code: OperationErrorCode::TargetNotFound,
                message: format!("Service not found: {}", planned.service.name),
                target_id: Some(planned.service.name.clone()),
                retryable: true,
            })?;
        if current_service.start_type.eq_ignore_ascii_case("demand") {
            continue;
        }
        retry_services.push(current_service.clone());
    }
    if retry_services.is_empty() {
        return Err(OperationError::validation(
            "all bulk service retry targets already use Manual startup",
        ));
    }

    let mut plan = create_manual_plan(&retry_services)?;
    plan.parent_operation_id = Some(previous_plan.id.clone());
    plan.retry_of_operation_id = Some(previous_plan.id.clone());
    plan.warnings.push(OperationWarning {
        code: "retry_of_operation".into(),
        message: previous_plan.id.0.clone(),
    });
    plan.validate_relationships()?;
    Ok(plan)
}

pub fn execute(
    plan: &OperationPlan,
    current: &[ServiceInfo],
) -> Result<(ServicesBulkExecution, OperationJournalRecord), OperationError> {
    let _ = (plan, current);
    Err(OperationError::validation(
        "mutating bulk service execution requires an initialized OperationJournal",
    ))
}

pub fn execute_with_journal(
    plan: &OperationPlan,
    current: &[ServiceInfo],
    journal: &OperationJournal,
) -> Result<(ServicesBulkExecution, OperationJournalRecord), OperationError> {
    let before = match confirm_current(plan, current) {
        Ok(before) => before,
        Err(error) => {
            if let Some(target_id) = error.target_id.as_deref() {
                super::service_operation::mark_target_revalidation_failed(
                    journal, plan, target_id,
                )?;
            }
            return Err(error);
        }
    };
    let mut outcomes = Vec::with_capacity(before.len());

    for service in &before {
        crate::core::service_operation::mark_target_ready_and_mutation_started(
            journal,
            plan,
            &service.name,
        )?;
        let outcome = match services::set_start_type(&service.name, "demand") {
            Ok(()) => match services::inspect_state(&service.name, service) {
                Ok(after)
                    if after.start_type.eq_ignore_ascii_case("demand")
                        && service.configuration_matches(&after) =>
                {
                    TargetOutcome {
                        target_id: service.name.clone(),
                        requested_action: "set_start_type:demand".into(),
                        before: Some(state_snapshot(service)),
                        after: Some(state_snapshot(&after)),
                        execution: ExecutionStatus::Changed,
                        verification: VerificationStatus::Verified,
                        error: None,
                    }
                }
                Ok(after) => TargetOutcome {
                    target_id: service.name.clone(),
                    requested_action: "set_start_type:demand".into(),
                    before: Some(state_snapshot(service)),
                    after: Some(state_snapshot(&after)),
                    execution: ExecutionStatus::Failed,
                    verification: VerificationStatus::Failed,
                    error: Some(execution_error(
                        &service.name,
                        "Service Control Manager did not report Manual startup.",
                    )),
                },
                Err(error) => failed_outcome(service, error),
            },
            Err(error) => failed_outcome(service, error),
        };
        super::service_operation::record_target_outcome(journal, plan, &outcome)?;
        outcomes.push(outcome);
    }

    let result = OperationResult::new(
        plan.id.clone(),
        outcomes,
        false,
        UndoAvailability::NotReversible,
    )?;
    let changed = result.changed;
    let failed = result.failed;
    let execution = ServicesBulkExecution {
        result,
        message: format!(
            "{} service{} set to Manual; {} failed.",
            changed,
            if changed == 1 { "" } else { "s" },
            failed
        ),
    };
    let record = journal_record(plan, &execution);
    Ok((execution, record))
}

fn decode_plan(plan: &OperationPlan) -> Result<Vec<ServiceTargetSnapshot>, OperationError> {
    if plan.descriptor.id.0 != "startup.services_set_manual"
        || plan.descriptor.category != OperationCategory::Startup
        || !plan.descriptor.experimental
        || plan.targets.is_empty()
        || plan.reversibility != Reversibility::BestEffort
    {
        return Err(OperationError::validation(
            "invalid bulk service operation plan",
        ));
    }

    plan.targets
        .iter()
        .map(|target| {
            let snapshot: ServiceTargetSnapshot = serde_json::from_str(&target.requested_action)
                .map_err(|error| {
                    OperationError::validation(format!(
                        "invalid bulk service target snapshot: {error}"
                    ))
                })?;
            if snapshot.service.name != target.target_id
                || services::is_protected(&snapshot.service.name)
            {
                return Err(OperationError::validation(
                    "bulk service target does not match the confirmed plan",
                ));
            }
            Ok(snapshot)
        })
        .collect()
}

fn failed_outcome(service: &ServiceInfo, message: String) -> TargetOutcome {
    TargetOutcome {
        target_id: service.name.clone(),
        requested_action: "set_start_type:demand".into(),
        before: Some(state_snapshot(service)),
        after: None,
        execution: ExecutionStatus::Failed,
        verification: VerificationStatus::Failed,
        error: Some(execution_error(&service.name, &message)),
    }
}

fn execution_error(target_id: &str, message: &str) -> OperationError {
    OperationError {
        code: OperationErrorCode::ExecutionFailed,
        message: message.into(),
        target_id: Some(target_id.into()),
        retryable: true,
    }
}

fn state_snapshot(service: &ServiceInfo) -> StateSnapshot {
    StateSnapshot {
        summary: format!(
            "{} is {} with {} startup",
            service.display_name, service.status, service.start_type
        ),
        properties: BTreeMap::from([
            ("name".into(), service.name.clone()),
            ("display_name".into(), service.display_name.clone()),
            ("status".into(), service.status.clone()),
            ("start_type".into(), service.start_type.clone()),
            (
                "description".into(),
                service
                    .description
                    .as_deref()
                    .unwrap_or("unavailable")
                    .into(),
            ),
            (
                "binary_path".into(),
                service
                    .binary_path
                    .as_deref()
                    .unwrap_or("unavailable")
                    .into(),
            ),
            (
                "delayed_auto_start".into(),
                service
                    .delayed_auto_start
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "unavailable".into()),
            ),
            (
                "dependencies".into(),
                serde_json::to_string(&service.dependencies).unwrap_or_else(|_| "[]".into()),
            ),
        ]),
    }
}

fn journal_record(
    plan: &OperationPlan,
    execution: &ServicesBulkExecution,
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
        captured_original_states: Vec::new(),
        target_outcomes: execution.result.outcomes.clone(),
        verification_results: execution
            .result
            .outcomes
            .iter()
            .map(|outcome| VerificationRecord {
                target_id: outcome.target_id.clone(),
                status: outcome.verification,
                message: "Service Control Manager state was re-queried.".into(),
            })
            .collect(),
        restart_requirement: RebootPolicy::NotRequired,
        undo_eligibility: UndoEligibility {
            eligible: false,
            reason: "Typed native service before/after state is retained in target outcomes; exact dependent-state restoration is not available.".into(),
            automated_restoration_test: None,
        },
        undo_execution_result: None,
        diagnostic: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(name: &str, start_type: &str) -> ServiceInfo {
        ServiceInfo {
            name: name.into(),
            display_name: name.into(),
            status: "Stopped".into(),
            start_type: start_type.into(),
            description: None,
            binary_path: None,
            delayed_auto_start: None,
            dependencies: Vec::new(),
            guidance: crate::startup::guidance::service_guidance(name),
        }
    }

    #[test]
    fn bulk_plan_excludes_protected_and_manual_services() {
        let services = vec![
            service("RpcSs", "auto"),
            service("FixtureAuto", "auto"),
            service("FixtureManual", "demand"),
        ];
        let plan = create_manual_plan(&services).unwrap();
        assert_eq!(plan.targets.len(), 1);
        assert_eq!(plan.targets[0].target_id, "FixtureAuto");
    }

    #[test]
    fn retry_rebuilds_only_unresolved_services_and_links_the_source() {
        let previous_services = vec![service("RetryAuto", "auto")];
        let previous = create_manual_plan(&previous_services).unwrap();
        let mut current_service = service("RetryAuto", "auto");
        current_service.binary_path = Some(r"C:\Fixture\service.exe".into());
        let retry = create_retry_plan(
            std::slice::from_ref(&current_service),
            &previous,
            std::slice::from_ref(&previous.targets[0].target_id),
        )
        .unwrap();

        assert_ne!(retry.id, previous.id);
        assert_eq!(retry.parent_operation_id, Some(previous.id.clone()));
        assert_eq!(retry.retry_of_operation_id, Some(previous.id));
        assert_eq!(retry.targets.len(), 1);
        assert_eq!(retry.targets[0].target_id, "RetryAuto");
    }

    #[test]
    fn retry_rejects_missing_or_satisfied_bulk_service_targets() {
        let previous_service = service("RetryAuto", "auto");
        let previous = create_manual_plan(std::slice::from_ref(&previous_service)).unwrap();
        let target_id = previous.targets[0].target_id.clone();

        assert!(create_retry_plan(&[], &previous, std::slice::from_ref(&target_id)).is_err());
        let duplicate = create_retry_plan(
            std::slice::from_ref(&previous_service),
            &previous,
            &[target_id.clone(), target_id.clone()],
        )
        .unwrap_err();
        assert_eq!(duplicate.code, OperationErrorCode::ValidationFailed);

        let satisfied = service("RetryAuto", "demand");
        let error = create_retry_plan(
            std::slice::from_ref(&satisfied),
            &previous,
            std::slice::from_ref(&target_id),
        )
        .unwrap_err();
        assert_eq!(error.code, OperationErrorCode::ValidationFailed);
    }

    #[test]
    fn journal_record_uses_typed_before_state_without_synthetic_capture() {
        let before = service("JournalFixture", "auto");
        let plan = create_manual_plan(std::slice::from_ref(&before)).unwrap();
        let outcome = TargetOutcome {
            target_id: before.name.clone(),
            requested_action: "guidance:set_start_type:demand".into(),
            before: Some(state_snapshot(&before)),
            after: Some(state_snapshot(&before)),
            execution: ExecutionStatus::Unchanged,
            verification: VerificationStatus::Verified,
            error: None,
        };
        let result = OperationResult::new(
            plan.id.clone(),
            vec![outcome],
            false,
            UndoAvailability::NotReversible,
        )
        .unwrap();
        let execution = ServicesBulkExecution {
            result,
            message: "unchanged".into(),
        };

        let record = journal_record(&plan, &execution);

        assert!(record.captured_original_states.is_empty());
        assert_eq!(
            record.target_outcomes[0]
                .before
                .as_ref()
                .unwrap()
                .properties
                .get("description"),
            Some(&"unavailable".into())
        );
    }

    #[test]
    fn journaled_execution_keeps_targets_not_started_pending() {
        let first = service("FirstAuto", "auto");
        let second = service("SecondAuto", "auto");
        let plan = create_manual_plan(&[first, second]).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let journal = OperationJournal::open_at(&directory.path().join("journal.db")).unwrap();
        journal.begin_confirmed_execution(&plan).unwrap();

        crate::core::service_operation::mark_target_ready_and_mutation_started(
            &journal,
            &plan,
            &plan.targets[0].target_id,
        )
        .unwrap();

        let unresolved = journal.list_unresolved_targets(&plan.id).unwrap();
        assert_eq!(unresolved.len(), 2);
        assert_eq!(unresolved[0].target_id, plan.targets[0].target_id);
        assert_eq!(
            unresolved[0].state,
            super::super::operation::TargetExecutionState::MutationStarted
        );
        assert_eq!(unresolved[1].target_id, plan.targets[1].target_id);
        assert_eq!(
            unresolved[1].state,
            super::super::operation::TargetExecutionState::Pending
        );
    }
}
