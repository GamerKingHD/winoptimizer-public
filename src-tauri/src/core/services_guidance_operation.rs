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
use super::services_bulk_operation::ServicesBulkExecution;
use crate::startup::guidance::{GuidanceAction, GuidanceInfo};
use crate::startup::services::{self, ServiceInfo};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceGuidanceTarget {
    #[serde(rename = "serviceName", alias = "service_name")]
    pub service_name: String,
    #[serde(rename = "desiredStartType", alias = "desired_start_type")]
    pub desired_start_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ServiceGuidanceTargetSnapshot {
    service: ServiceInfo,
    guidance: GuidanceInfo,
    desired_start_type: String,
}

pub fn create_plan(
    requested_targets: &[ServiceGuidanceTarget],
) -> Result<OperationPlan, OperationError> {
    if requested_targets.is_empty() {
        return Err(OperationError::validation(
            "select at least one service recommendation",
        ));
    }

    let current = services::list();
    let mut snapshots = Vec::with_capacity(requested_targets.len());
    let mut targets = Vec::with_capacity(requested_targets.len());
    let mut expected_effects = Vec::with_capacity(requested_targets.len());
    let mut has_community_source = false;

    for requested in requested_targets {
        if services::is_protected(&requested.service_name) {
            return Err(target_error(
                OperationErrorCode::TargetProtected,
                format!(
                    "Protected Windows service cannot receive guidance changes: {}",
                    requested.service_name
                ),
                &requested.service_name,
            ));
        }
        if snapshots
            .iter()
            .any(|snapshot: &ServiceGuidanceTargetSnapshot| {
                snapshot
                    .service
                    .name
                    .eq_ignore_ascii_case(&requested.service_name)
            })
        {
            return Err(OperationError::validation(format!(
                "service selected more than once: {}",
                requested.service_name
            )));
        }
        let service = current
            .iter()
            .find(|candidate| candidate.name.eq_ignore_ascii_case(&requested.service_name))
            .ok_or_else(|| {
                target_error(
                    OperationErrorCode::TargetNotFound,
                    format!("Service not found: {}", requested.service_name),
                    &requested.service_name,
                )
            })?;
        validate_requested_action(service, &requested.desired_start_type)?;
        let snapshot = ServiceGuidanceTargetSnapshot {
            service: service.clone(),
            guidance: service.guidance.clone(),
            desired_start_type: requested.desired_start_type.clone(),
        };
        has_community_source |= matches!(
            snapshot.guidance.source_type,
            crate::startup::guidance::GuidanceSourceType::Community
        );
        let requested_action = serde_json::to_string(&snapshot).map_err(|error| {
            OperationError::validation(format!("failed to serialize service guidance: {error}"))
        })?;
        targets.push(PlannedTarget {
            target_id: service.name.clone(),
            requested_action,
            description: format!(
                "{}: {}",
                service.display_name,
                action_description(&service.guidance.action, &requested.desired_start_type)
            ),
        });
        expected_effects.push(ExpectedEffect {
            target_id: service.name.clone(),
            description: format!(
                "Service Control Manager reports {} startup.",
                requested.desired_start_type
            ),
        });
        snapshots.push(snapshot);
    }

    let mut warnings = vec![
        OperationWarning {
            code: "services_guidance_experimental".into(),
            message: "Guided service changes remain Experimental; exact dependent-state undo is not available.".into(),
        },
        OperationWarning {
            code: "service_dependencies".into(),
            message: "Service dependencies and on-demand starts can affect Windows features and applications.".into(),
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
            id: OperationId("startup.services_guidance".into()),
            capability_id: "startup.services".into(),
            title: "Apply selected service guidance".into(),
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

/// Strict confirmation helper used by tests and callers that want an all-target
/// stale-state rejection before executing a plan.
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
            .ok_or_else(|| {
                target_error(
                    OperationErrorCode::TargetNotFound,
                    format!("Service not found: {}", snapshot.service.name),
                    &snapshot.service.name,
                )
            })?;
        ensure_current_matches(&snapshot, service)?;
        confirmed.push(service.clone());
    }
    Ok(confirmed)
}

pub fn execute(
    plan: &OperationPlan,
    current: &[ServiceInfo],
) -> Result<(ServicesBulkExecution, OperationJournalRecord), OperationError> {
    let _ = (plan, current);
    Err(OperationError::validation(
        "mutating service guidance execution requires an initialized OperationJournal",
    ))
}

pub fn execute_with_journal(
    plan: &OperationPlan,
    current: &[ServiceInfo],
    journal: &OperationJournal,
) -> Result<(ServicesBulkExecution, OperationJournalRecord), OperationError> {
    let snapshots = decode_plan(plan)?;
    let mut before = Vec::new();
    let mut outcomes = Vec::with_capacity(snapshots.len());

    for snapshot in snapshots {
        let target_id = snapshot.service.name.clone();
        let Some(service) = current
            .iter()
            .find(|candidate| candidate.name.eq_ignore_ascii_case(&target_id))
        else {
            super::service_operation::mark_target_revalidation_failed(journal, plan, &target_id)?;
            outcomes.push(failed_outcome(
                &target_id,
                &snapshot.desired_start_type,
                None,
                target_error(
                    OperationErrorCode::TargetNotFound,
                    format!("Service not found: {target_id}"),
                    &target_id,
                ),
            ));
            continue;
        };

        if let Err(error) = ensure_current_matches(&snapshot, service) {
            super::service_operation::mark_target_revalidation_failed(journal, plan, &target_id)?;
            outcomes.push(failed_outcome(
                &target_id,
                &snapshot.desired_start_type,
                Some(service),
                error,
            ));
            continue;
        }

        if service
            .start_type
            .eq_ignore_ascii_case(&snapshot.desired_start_type)
        {
            let outcome = TargetOutcome {
                target_id: target_id.clone(),
                requested_action: requested_action(&snapshot.desired_start_type),
                before: Some(state_snapshot(service)),
                after: Some(state_snapshot(service)),
                execution: ExecutionStatus::Unchanged,
                verification: VerificationStatus::Verified,
                error: None,
            };
            outcomes.push(outcome);
            continue;
        }

        let service = match services::inspect_state(&target_id, service) {
            Ok(latest) => {
                if let Err(error) = ensure_current_matches(&snapshot, &latest) {
                    super::service_operation::mark_target_revalidation_failed(
                        journal, plan, &target_id,
                    )?;
                    outcomes.push(failed_outcome(
                        &target_id,
                        &snapshot.desired_start_type,
                        Some(&latest),
                        error,
                    ));
                    continue;
                }
                latest
            }
            Err(error) => {
                super::service_operation::mark_target_revalidation_failed(
                    journal, plan, &target_id,
                )?;
                outcomes.push(failed_outcome(
                    &target_id,
                    &snapshot.desired_start_type,
                    Some(service),
                    target_error(OperationErrorCode::VerificationFailed, error, &target_id),
                ));
                continue;
            }
        };
        before.push(service.clone());
        super::service_operation::mark_target_ready_and_mutation_started(
            journal, plan, &target_id,
        )?;

        let outcome = match services::set_start_type(&target_id, &snapshot.desired_start_type) {
            Ok(()) => match services::inspect_state(&target_id, &service) {
                Ok(after)
                    if after.exact_configuration_available()
                        && after.name == service.name
                        && after.status.eq_ignore_ascii_case(&service.status)
                        && after
                            .start_type
                            .eq_ignore_ascii_case(&snapshot.desired_start_type)
                        && service.configuration_matches(&after) =>
                {
                    TargetOutcome {
                        target_id: target_id.clone(),
                        requested_action: requested_action(&snapshot.desired_start_type),
                        before: Some(state_snapshot(&service)),
                        after: Some(state_snapshot(&after)),
                        execution: ExecutionStatus::Changed,
                        verification: VerificationStatus::Verified,
                        error: None,
                    }
                }
                Ok(_after) => failed_outcome(
                    &target_id,
                    &snapshot.desired_start_type,
                    Some(&service),
                    target_error(
                        OperationErrorCode::VerificationFailed,
                        format!(
                            "Service Control Manager did not report {} startup.",
                            snapshot.desired_start_type
                        ),
                        &target_id,
                    ),
                ),
                Err(error) => failed_outcome(
                    &target_id,
                    &snapshot.desired_start_type,
                    Some(&service),
                    target_error(OperationErrorCode::VerificationFailed, error, &target_id),
                ),
            },
            Err(error) => failed_outcome(
                &target_id,
                &snapshot.desired_start_type,
                Some(&service),
                target_error(OperationErrorCode::ExecutionFailed, error, &target_id),
            ),
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
    let execution = ServicesBulkExecution {
        message: format!(
            "{} guided service change{} applied; {} failed.",
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

fn decode_plan(plan: &OperationPlan) -> Result<Vec<ServiceGuidanceTargetSnapshot>, OperationError> {
    if plan.descriptor.id.0 != "startup.services_guidance"
        || plan.descriptor.category != OperationCategory::Startup
        || !plan.descriptor.experimental
        || plan.descriptor.capability_id != "startup.services"
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
            "invalid service guidance operation plan",
        ));
    }
    plan.descriptor.validate_contract()?;
    plan.validate_relationships()?;

    plan.targets
        .iter()
        .map(|target| {
            let snapshot: ServiceGuidanceTargetSnapshot =
                serde_json::from_str(&target.requested_action).map_err(|error| {
                    OperationError::validation(format!(
                        "invalid service guidance target snapshot: {error}"
                    ))
                })?;
            if snapshot.service.name != target.target_id {
                return Err(OperationError::validation(
                    "service guidance target does not match its plan target",
                ));
            }
            if snapshot.guidance != snapshot.service.guidance {
                return Err(OperationError::validation(
                    "service guidance snapshot does not match its native service snapshot",
                ));
            }
            validate_requested_action(&snapshot.service, &snapshot.desired_start_type)?;
            Ok(snapshot)
        })
        .collect()
}

fn validate_requested_action(
    service: &ServiceInfo,
    desired_start_type: &str,
) -> Result<(), OperationError> {
    if !matches!(desired_start_type, "auto" | "demand" | "disabled") {
        return Err(OperationError::validation(format!(
            "unsupported service start type: {desired_start_type}"
        )));
    }
    if service.guidance.protected || services::is_protected(&service.name) {
        return Err(target_error(
            OperationErrorCode::TargetProtected,
            format!(
                "Protected Windows service cannot receive guidance changes: {}",
                service.name
            ),
            &service.name,
        ));
    }
    if !service.exact_configuration_available() {
        return Err(target_error(
            OperationErrorCode::ValidationFailed,
            format!(
                "{} is missing its exact native Service Control Manager configuration",
                service.name
            ),
            &service.name,
        ));
    }
    let allowed = match service.guidance.action {
        GuidanceAction::SetManual => desired_start_type == "demand",
        GuidanceAction::DisableIfUnused => desired_start_type == "disabled",
        GuidanceAction::KeepEnabled | GuidanceAction::ReviewFirst => false,
    };
    if !allowed {
        return Err(target_error(
            OperationErrorCode::ValidationFailed,
            format!(
                "{} is guidance group {}; review it as an individual service action",
                service.name,
                action_label(service.guidance.action)
            ),
            &service.name,
        ));
    }
    Ok(())
}

fn ensure_current_matches(
    snapshot: &ServiceGuidanceTargetSnapshot,
    current: &ServiceInfo,
) -> Result<(), OperationError> {
    if !snapshot.service.state_matches(current) {
        return Err(target_error(
            OperationErrorCode::TargetChangedSincePreview,
            format!(
                "{} changed after confirmation; review the guidance again.",
                current.name
            ),
            &current.name,
        ));
    }
    if snapshot.guidance != crate::startup::guidance::service_guidance(&current.name) {
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

fn action_label(action: GuidanceAction) -> &'static str {
    match action {
        GuidanceAction::KeepEnabled => "Keep enabled",
        GuidanceAction::SetManual => "Set to Manual",
        GuidanceAction::DisableIfUnused => "Disable if unused",
        GuidanceAction::ReviewFirst => "Review first",
    }
}

fn action_description(action: &GuidanceAction, desired_start_type: &str) -> String {
    format!("{} ({desired_start_type})", action_label(*action))
}

fn requested_action(desired_start_type: &str) -> String {
    format!("guidance:set_start_type:{desired_start_type}")
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
            (
                "guidance_group".into(),
                action_label(service.guidance.action).into(),
            ),
        ]),
    }
}

fn failed_outcome(
    target_id: &str,
    desired_start_type: &str,
    before: Option<&ServiceInfo>,
    error: OperationError,
) -> TargetOutcome {
    TargetOutcome {
        target_id: target_id.into(),
        requested_action: requested_action(desired_start_type),
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
                message: "Service Control Manager state was re-queried where execution was attempted.".into(),
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
            delayed_auto_start: Some(false),
            dependencies: Vec::new(),
            guidance: crate::startup::guidance::service_guidance(name),
        }
    }

    #[test]
    fn protected_service_is_rejected_by_guidance_plans() {
        let error = create_plan(&[ServiceGuidanceTarget {
            service_name: "RpcSs".into(),
            desired_start_type: "disabled".into(),
        }])
        .unwrap_err();
        assert_eq!(error.code, OperationErrorCode::TargetProtected);
    }

    #[test]
    fn guidance_action_validation_rejects_review_first_and_wrong_target_state() {
        let review = service("AcmeVendorService", "auto");
        assert_eq!(
            validate_requested_action(&review, "disabled")
                .unwrap_err()
                .code,
            OperationErrorCode::ValidationFailed
        );
        let manual = service("BITS", "auto");
        assert!(validate_requested_action(&manual, "demand").is_ok());
        assert!(validate_requested_action(&manual, "disabled").is_err());
    }

    #[test]
    fn guidance_action_validation_rejects_incomplete_native_configuration() {
        let mut incomplete = service("BITS", "auto");
        incomplete.delayed_auto_start = None;

        assert_eq!(
            validate_requested_action(&incomplete, "demand")
                .unwrap_err()
                .code,
            OperationErrorCode::ValidationFailed
        );
    }

    #[test]
    fn stale_state_is_rejected_before_guidance_execution() {
        let before = service("BITS", "auto");
        let plan = create_plan(&[ServiceGuidanceTarget {
            service_name: "BITS".into(),
            desired_start_type: "demand".into(),
        }]);
        // The local test machine does not need to enumerate BITS: decode and
        // strict stale checks are covered using a synthetic plan below.
        let plan = plan.unwrap_or_else(|_| OperationPlan {
            id: OperationId("test".into()),
            descriptor: OperationDescriptor {
                id: OperationId("startup.services_guidance".into()),
                capability_id: "startup.services".into(),
                title: "test".into(),
                category: OperationCategory::Startup,
                risk: RiskLevel::High,
                privilege: PrivilegeRequirement::Administrator,
                reversibility: Reversibility::BestEffort,
                reboot_policy: RebootPolicy::NotRequired,
                experimental: true,
                proof_status: ProofStatus::Experimental,
                exposure: ExposureLevel::Internal,
                recovery: RecoveryLevel::BestEffort,
                restore_point_policy: crate::core::operation::RestorePointPolicy::NotNeeded,
                mutates_system: true,
                requires_confirmation: true,
                requires_revalidation: true,
                requires_verification: true,
                requires_journal: true,
            },
            targets: vec![PlannedTarget {
                target_id: before.name.clone(),
                requested_action: serde_json::to_string(&ServiceGuidanceTargetSnapshot {
                    service: before.clone(),
                    guidance: before.guidance.clone(),
                    desired_start_type: "demand".into(),
                })
                .unwrap(),
                description: "test".into(),
            }],
            warnings: vec![],
            expected_effects: vec![],
            requires_elevation: true,
            requires_restart: false,
            reversibility: Reversibility::BestEffort,
            parent_operation_id: None,
            retry_of_operation_id: None,
        });
        let mut changed = before.clone();
        changed.status = "Running".into();
        assert_eq!(
            confirm_current(&plan, &[changed]).unwrap_err().code,
            OperationErrorCode::TargetChangedSincePreview
        );
    }

    #[test]
    fn journal_record_uses_typed_before_state_without_advertising_undo() {
        let before = service("BITS", "auto");
        let plan = OperationPlan {
            id: OperationId("guidance-journal-test".into()),
            descriptor: OperationDescriptor {
                id: OperationId("startup.services_guidance".into()),
                capability_id: "startup.services".into(),
                title: "test".into(),
                category: OperationCategory::Startup,
                risk: RiskLevel::High,
                privilege: PrivilegeRequirement::Administrator,
                reversibility: Reversibility::BestEffort,
                reboot_policy: RebootPolicy::NotRequired,
                experimental: true,
                proof_status: ProofStatus::Experimental,
                exposure: ExposureLevel::Internal,
                recovery: RecoveryLevel::BestEffort,
                restore_point_policy: crate::core::operation::RestorePointPolicy::NotNeeded,
                mutates_system: true,
                requires_confirmation: true,
                requires_revalidation: true,
                requires_verification: true,
                requires_journal: true,
            },
            targets: vec![PlannedTarget {
                target_id: before.name.clone(),
                requested_action: serde_json::to_string(&ServiceGuidanceTargetSnapshot {
                    service: before.clone(),
                    guidance: before.guidance.clone(),
                    desired_start_type: "demand".into(),
                })
                .unwrap(),
                description: "test".into(),
            }],
            warnings: vec![],
            expected_effects: vec![],
            requires_elevation: true,
            requires_restart: false,
            reversibility: Reversibility::BestEffort,
            parent_operation_id: None,
            retry_of_operation_id: None,
        };
        let outcome = TargetOutcome {
            target_id: before.name.clone(),
            requested_action: requested_action("demand"),
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

        assert_eq!(record.captured_original_states.len(), 1);
        assert_eq!(record.captured_original_states[0].target_id(), "BITS");
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
}
