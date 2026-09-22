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
use crate::startup::services::{self, ServiceInfo};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServiceAction {
    Start,
    Stop,
    SetStartType { start_type: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ServiceTargetSnapshot {
    service: ServiceInfo,
    action: ServiceAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServiceGuidanceRetrySnapshot {
    service: ServiceInfo,
    guidance: GuidanceInfo,
    desired_start_type: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceExecution {
    pub result: OperationResult,
    pub message: String,
}

pub fn create_plan(
    service: &ServiceInfo,
    action: ServiceAction,
) -> Result<OperationPlan, OperationError> {
    if services::is_protected(&service.name) {
        return Err(OperationError {
            code: OperationErrorCode::TargetProtected,
            message: format!("{} is a protected Windows service.", service.name),
            target_id: Some(service.name.clone()),
            retryable: false,
        });
    }
    if !service.exact_configuration_available() {
        return Err(validation(
            "exact native Service Control Manager configuration is unavailable",
        ));
    }
    validate_action(service, &action)?;
    let snapshot = ServiceTargetSnapshot {
        service: service.clone(),
        action: action.clone(),
    };
    let requested_action = serde_json::to_string(&snapshot)
        .map_err(|error| validation(format!("failed to serialize service target: {error}")))?;

    Ok(OperationPlan {
        id: OperationId(uuid::Uuid::new_v4().to_string()),
        descriptor: OperationDescriptor {
            id: OperationId("startup.service_change".into()),
            capability_id: "startup.services".into(),
            title: action_title(service, &action),
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
        targets: vec![PlannedTarget {
            target_id: service.name.clone(),
            requested_action,
            description: format!("{} ({})", service.display_name, service.name),
        }],
        warnings: vec![
            OperationWarning {
                code: "experimental_workflow".into(),
                message: "Service mutation remains Experimental and does not advertise exact undo."
                    .into(),
            },
            OperationWarning {
                code: "service_dependencies".into(),
                message:
                    "Changing a service can affect dependent Windows features and applications."
                        .into(),
            },
        ],
        expected_effects: vec![ExpectedEffect {
            target_id: service.name.clone(),
            description: expected_description(&action),
        }],
        requires_elevation: true,
        requires_restart: false,
        reversibility: Reversibility::BestEffort,
        parent_operation_id: None,
        retry_of_operation_id: None,
    })
}

pub fn create_retry_plan(
    current: &[ServiceInfo],
    previous_plan: &OperationPlan,
    unresolved_target_ids: &[String],
) -> Result<OperationPlan, OperationError> {
    if previous_plan.descriptor.id.0 != "startup.service_change" {
        return Err(validation(
            "retry requires domain-specific rediscovery for this operation",
        ));
    }
    if unresolved_target_ids.len() != 1 {
        return Err(validation("service retry requires exactly one target"));
    }
    let previous = decode_plan(previous_plan)?;
    if unresolved_target_ids[0] != previous.service.name {
        return Err(validation(
            "service retry target does not match the previous plan",
        ));
    }
    let current_service = current
        .iter()
        .find(|service| service.name == previous.service.name)
        .ok_or_else(|| validation("service was not rediscovered; no retry plan was created"))?;

    let mut plan = create_plan(current_service, previous.action)?;
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
    previous_plan: &OperationPlan,
    unresolved_target_ids: &[String],
) -> Result<OperationPlan, OperationError> {
    let requested_targets = decode_guidance_retry_targets(previous_plan, unresolved_target_ids)?;
    let mut plan = super::services_guidance_operation::create_plan(&requested_targets)?;
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
) -> Result<Vec<super::services_guidance_operation::ServiceGuidanceTarget>, OperationError> {
    if previous_plan.descriptor.id.0 != "startup.services_guidance"
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
        return Err(validation("invalid service guidance retry source plan"));
    }
    previous_plan.descriptor.validate_contract()?;
    previous_plan.validate_relationships()?;
    if unresolved_target_ids.is_empty() {
        return Err(validation(
            "service guidance retry requires at least one target",
        ));
    }

    let mut requested_targets = Vec::with_capacity(unresolved_target_ids.len());
    for target_id in unresolved_target_ids {
        if requested_targets.iter().any(
            |target: &super::services_guidance_operation::ServiceGuidanceTarget| {
                target.service_name == *target_id
            },
        ) {
            return Err(validation(
                "service guidance retry cannot contain duplicate targets",
            ));
        }
        let target = previous_plan
            .targets
            .iter()
            .find(|target| target.target_id == *target_id)
            .ok_or_else(|| validation("service guidance retry target is not in the source plan"))?;
        let snapshot: ServiceGuidanceRetrySnapshot = serde_json::from_str(&target.requested_action)
            .map_err(|error| {
                validation(format!(
                    "invalid service guidance retry target snapshot: {error}"
                ))
            })?;
        if snapshot.service.name != target.target_id
            || snapshot.guidance != snapshot.service.guidance
        {
            return Err(validation(
                "service guidance retry target snapshot does not match the source plan",
            ));
        }
        requested_targets.push(super::services_guidance_operation::ServiceGuidanceTarget {
            service_name: snapshot.service.name,
            desired_start_type: snapshot.desired_start_type,
        });
    }
    Ok(requested_targets)
}

pub fn confirmed_action(
    plan: &OperationPlan,
    current: &ServiceInfo,
) -> Result<ServiceAction, OperationError> {
    let snapshot = decode_plan(plan)?;
    if !snapshot.service.state_matches(current) {
        return Err(OperationError {
            code: OperationErrorCode::TargetChangedSincePreview,
            message: "Service state changed after confirmation; review it again.".into(),
            target_id: Some(current.name.clone()),
            retryable: true,
        });
    }
    Ok(snapshot.action)
}

pub fn finish(
    plan: &OperationPlan,
    before: &ServiceInfo,
    after: &ServiceInfo,
) -> Result<ServiceExecution, OperationError> {
    let action = confirmed_action(plan, before)?;
    if !postcondition_matches(&action, before, after) || !action_matches(&action, after) {
        return Err(OperationError {
            code: OperationErrorCode::VerificationFailed,
            message: "Service postcondition could not be verified against an exact native configuration snapshot.".into(),
            target_id: Some(before.name.clone()),
            retryable: true,
        });
    }
    let outcome = TargetOutcome {
        target_id: before.name.clone(),
        requested_action: action_label(&action),
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
    Ok(ServiceExecution {
        result,
        message: format!(
            "{} was updated and its exact native postcondition was verified; restoration remains best-effort.",
            before.display_name
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
    before: &ServiceInfo,
    action: &ServiceAction,
    error: OperationError,
) -> TargetOutcome {
    TargetOutcome {
        target_id: before.name.clone(),
        requested_action: action_label(action),
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
                "target {target_id} is not present in the confirmed service plan"
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
    before: &ServiceInfo,
    execution: &ServiceExecution,
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
            message: "Exact native Service Control Manager postcondition verified; restoration remains best-effort.".into(),
        }],
        restart_requirement: RebootPolicy::NotRequired,
        undo_eligibility: UndoEligibility {
            eligible: false,
            reason: "Typed native service before/after state is retained in the target outcome; postcondition verification is exact for captured fields, but dependent-state restoration remains best-effort.".into(),
            automated_restoration_test: None,
        },
        undo_execution_result: None,
        diagnostic: None,
    }
}

fn decode_plan(plan: &OperationPlan) -> Result<ServiceTargetSnapshot, OperationError> {
    if plan.descriptor.id.0 != "startup.service_change"
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
        || plan.targets.len() != 1
        || plan.reversibility != Reversibility::BestEffort
    {
        return Err(validation("invalid service operation plan"));
    }
    plan.descriptor.validate_contract()?;
    plan.validate_relationships()?;
    let snapshot: ServiceTargetSnapshot =
        serde_json::from_str(&plan.targets[0].requested_action)
            .map_err(|error| validation(format!("invalid service target snapshot: {error}")))?;
    if snapshot.service.name != plan.targets[0].target_id
        || services::is_protected(&snapshot.service.name)
        || !snapshot.service.exact_configuration_available()
    {
        return Err(validation(
            "service target does not match the confirmed plan",
        ));
    }
    validate_action(&snapshot.service, &snapshot.action)?;
    Ok(snapshot)
}

fn postcondition_matches(
    action: &ServiceAction,
    before: &ServiceInfo,
    after: &ServiceInfo,
) -> bool {
    if !after.exact_configuration_available() || !before.configuration_matches(after) {
        return false;
    }

    match action {
        ServiceAction::Start | ServiceAction::Stop => {
            before.start_type.eq_ignore_ascii_case(&after.start_type)
        }
        ServiceAction::SetStartType { .. } => before.status.eq_ignore_ascii_case(&after.status),
    }
}

fn validate_action(service: &ServiceInfo, action: &ServiceAction) -> Result<(), OperationError> {
    match action {
        ServiceAction::Start if service.status.eq_ignore_ascii_case("running") => {
            Err(validation("service is already running"))
        }
        ServiceAction::Stop if service.status.eq_ignore_ascii_case("stopped") => {
            Err(validation("service is already stopped"))
        }
        ServiceAction::SetStartType { start_type } => {
            if !matches!(start_type.as_str(), "auto" | "demand" | "disabled") {
                return Err(validation("unsupported service start type"));
            }
            if service.start_type.eq_ignore_ascii_case(start_type) {
                return Err(validation("service already uses that start type"));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn action_matches(action: &ServiceAction, service: &ServiceInfo) -> bool {
    match action {
        ServiceAction::Start => service.status.eq_ignore_ascii_case("running"),
        ServiceAction::Stop => service.status.eq_ignore_ascii_case("stopped"),
        ServiceAction::SetStartType { start_type } => {
            service.start_type.eq_ignore_ascii_case(start_type)
        }
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

fn action_title(service: &ServiceInfo, action: &ServiceAction) -> String {
    match action {
        ServiceAction::Start => format!("Start {}", service.display_name),
        ServiceAction::Stop => format!("Stop {}", service.display_name),
        ServiceAction::SetStartType { start_type } => {
            format!("Set {} startup to {}", service.display_name, start_type)
        }
    }
}

fn expected_description(action: &ServiceAction) -> String {
    match action {
        ServiceAction::Start => "Service Control Manager reports Running.".into(),
        ServiceAction::Stop => "Service Control Manager reports Stopped.".into(),
        ServiceAction::SetStartType { start_type } => {
            format!("Service Control Manager reports {start_type} startup.")
        }
    }
}

fn action_label(action: &ServiceAction) -> String {
    match action {
        ServiceAction::Start => "start".into(),
        ServiceAction::Stop => "stop".into(),
        ServiceAction::SetStartType { start_type } => format!("set_start_type:{start_type}"),
    }
}

fn validation(message: impl Into<String>) -> OperationError {
    OperationError::validation(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(name: &str) -> ServiceInfo {
        service_state(name, "Stopped", "demand")
    }

    fn service_state(name: &str, status: &str, start_type: &str) -> ServiceInfo {
        ServiceInfo {
            name: name.into(),
            display_name: name.into(),
            status: status.into(),
            start_type: start_type.into(),
            description: None,
            binary_path: None,
            delayed_auto_start: Some(false),
            dependencies: Vec::new(),
            guidance: crate::startup::guidance::service_guidance(name),
        }
    }

    fn guidance_source_plan(service: &ServiceInfo, desired_start_type: &str) -> OperationPlan {
        let mut plan = create_plan(service, ServiceAction::Start).unwrap();
        plan.descriptor.id = OperationId("startup.services_guidance".into());
        plan.descriptor.capability_id = "startup.services".into();
        plan.targets[0].requested_action = serde_json::to_string(&ServiceGuidanceRetrySnapshot {
            service: service.clone(),
            guidance: service.guidance.clone(),
            desired_start_type: desired_start_type.into(),
        })
        .unwrap();
        plan
    }

    #[test]
    fn protected_service_is_rejected() {
        assert_eq!(
            create_plan(&service("RpcSs"), ServiceAction::Stop)
                .unwrap_err()
                .code,
            OperationErrorCode::TargetProtected
        );
    }

    #[test]
    fn plan_is_explicitly_experimental_and_non_reversible() {
        let plan = create_plan(&service("FixtureSvc"), ServiceAction::Start).unwrap();

        assert!(plan.descriptor.experimental);
        assert_eq!(plan.reversibility, Reversibility::BestEffort);
        assert!(confirmed_action(&plan, &service("FixtureSvc")).is_ok());
    }

    #[test]
    fn incomplete_native_configuration_snapshot_is_rejected() {
        let mut incomplete = service("IncompleteFixture");
        incomplete.delayed_auto_start = None;

        assert_eq!(
            create_plan(&incomplete, ServiceAction::Start)
                .unwrap_err()
                .code,
            OperationErrorCode::ValidationFailed
        );
    }

    #[test]
    fn changed_service_state_is_rejected() {
        let source = service("FixtureSvc");
        let plan = create_plan(&source, ServiceAction::Start).unwrap();
        let mut changed = source;
        changed.status = "Running".into();

        assert_eq!(
            confirmed_action(&plan, &changed).unwrap_err().code,
            OperationErrorCode::TargetChangedSincePreview
        );
    }

    #[test]
    fn retry_rebuilds_from_current_service_state_and_links_the_source() {
        let previous_service = service_state("RetryFixture", "Stopped", "auto");
        let previous = create_plan(&previous_service, ServiceAction::Start).unwrap();
        let mut current_service = previous_service;
        current_service.start_type = "demand".into();
        let retry = create_retry_plan(
            std::slice::from_ref(&current_service),
            &previous,
            std::slice::from_ref(&previous.targets[0].target_id),
        )
        .unwrap();

        assert_ne!(retry.id, previous.id);
        assert_eq!(retry.parent_operation_id, Some(previous.id.clone()));
        assert_eq!(retry.retry_of_operation_id, Some(previous.id));
        assert!(confirmed_action(&retry, &current_service).is_ok());
    }

    #[test]
    fn retry_rejects_a_missing_or_already_satisfied_service() {
        let previous_service = service_state("RetryFixture", "Stopped", "auto");
        let previous = create_plan(&previous_service, ServiceAction::Start).unwrap();
        let target_id = previous.targets[0].target_id.clone();
        assert!(create_retry_plan(&[], &previous, std::slice::from_ref(&target_id)).is_err());

        let running = service_state("RetryFixture", "Running", "auto");
        let error = create_retry_plan(
            std::slice::from_ref(&running),
            &previous,
            std::slice::from_ref(&target_id),
        )
        .unwrap_err();
        assert_eq!(error.code, OperationErrorCode::ValidationFailed);
    }

    #[test]
    fn guidance_retry_decodes_only_selected_targets_and_rejects_unknown_targets() {
        let previous_service = service_state("GuidanceRetryFixture", "Stopped", "auto");
        let previous = guidance_source_plan(&previous_service, "demand");
        let target_id = previous.targets[0].target_id.clone();

        let requested =
            decode_guidance_retry_targets(&previous, std::slice::from_ref(&target_id)).unwrap();
        assert_eq!(requested[0].service_name, target_id);
        assert_eq!(requested[0].desired_start_type, "demand");

        let error =
            create_guidance_retry_plan(&previous, &["missing-guidance-target".into()]).unwrap_err();
        assert_eq!(error.code, OperationErrorCode::ValidationFailed);
    }

    #[test]
    fn start_stop_and_start_type_results_require_native_state_match() {
        let cases = [
            (
                ServiceAction::Start,
                service_state("StartFixture", "Stopped", "auto"),
                service_state("StartFixture", "Running", "auto"),
            ),
            (
                ServiceAction::Stop,
                service_state("StopFixture", "Running", "auto"),
                service_state("StopFixture", "Stopped", "auto"),
            ),
            (
                ServiceAction::SetStartType {
                    start_type: "disabled".into(),
                },
                service_state("ConfigFixture", "Running", "auto"),
                service_state("ConfigFixture", "Running", "disabled"),
            ),
        ];

        for (action, before, after) in cases {
            let plan = create_plan(&before, action).unwrap();
            let execution = finish(&plan, &before, &after).unwrap();
            assert_eq!(execution.result.outcomes.len(), 1);
            assert_eq!(
                execution.result.outcomes[0].verification,
                VerificationStatus::Verified
            );
            assert_eq!(
                execution.result.outcomes[0].execution,
                ExecutionStatus::Changed
            );
        }
    }

    #[test]
    fn wrong_postcondition_is_reported_as_verification_failure() {
        let before = service_state("VerificationFixture", "Stopped", "auto");
        let plan = create_plan(&before, ServiceAction::Start).unwrap();
        let after = service_state("VerificationFixture", "Stopped", "auto");

        assert_eq!(
            finish(&plan, &before, &after).unwrap_err().code,
            OperationErrorCode::VerificationFailed
        );
    }

    #[test]
    fn incomplete_after_snapshot_cannot_be_reported_as_verified() {
        let before = service_state("IncompleteAfterFixture", "Stopped", "auto");
        let plan = create_plan(&before, ServiceAction::Start).unwrap();
        let mut after = service_state("IncompleteAfterFixture", "Running", "auto");
        after.delayed_auto_start = None;

        assert_eq!(
            finish(&plan, &before, &after).unwrap_err().code,
            OperationErrorCode::VerificationFailed
        );
    }

    #[test]
    fn journal_snapshot_keeps_native_service_configuration_evidence() {
        let mut service = service("EvidenceFixture");
        service.binary_path = Some(r#"C:\Fixture\service.exe"#.into());
        service.delayed_auto_start = Some(true);
        service.dependencies = vec!["RpcSs".into()];

        let snapshot = state_snapshot(&service);
        assert_eq!(
            snapshot.properties.get("binary_path"),
            Some(&r#"C:\Fixture\service.exe"#.to_string())
        );
        assert_eq!(
            snapshot.properties.get("delayed_auto_start"),
            Some(&"true".to_string())
        );
        assert_eq!(
            snapshot.properties.get("dependencies"),
            Some(&r#"["RpcSs"]"#.to_string())
        );
        assert_eq!(
            snapshot.properties.get("description"),
            Some(&"unavailable".to_string())
        );
    }

    #[test]
    fn journal_record_uses_typed_before_state_without_advertising_undo() {
        let before = service_state("JournalFixture", "Stopped", "demand");
        let mut after = before.clone();
        after.status = "Running".into();
        let plan = create_plan(&before, ServiceAction::Start).unwrap();
        let execution = finish(&plan, &before, &after).unwrap();

        let record = journal_record(&plan, &before, &execution);

        assert_eq!(record.captured_original_states.len(), 1);
        assert_eq!(
            record.captured_original_states[0].target_id(),
            "JournalFixture"
        );
        let snapshot = record.target_outcomes[0].before.as_ref().unwrap();
        assert_eq!(snapshot.properties.get("status"), Some(&"Stopped".into()));
        assert_eq!(
            snapshot.properties.get("description"),
            Some(&"unavailable".into())
        );
    }

    #[test]
    fn verification_rejects_unrequested_service_state_changes() {
        let before = service_state("VerificationFixture", "Stopped", "auto");
        let plan = create_plan(&before, ServiceAction::Start).unwrap();
        let mut after = service_state("VerificationFixture", "Running", "disabled");
        assert_eq!(
            finish(&plan, &before, &after).unwrap_err().code,
            OperationErrorCode::VerificationFailed
        );

        let before = service_state("StartTypeVerificationFixture", "Running", "auto");
        let plan = create_plan(
            &before,
            ServiceAction::SetStartType {
                start_type: "disabled".into(),
            },
        )
        .unwrap();
        after = service_state("StartTypeVerificationFixture", "Stopped", "disabled");
        assert_eq!(
            finish(&plan, &before, &after).unwrap_err().code,
            OperationErrorCode::VerificationFailed
        );
    }
}
