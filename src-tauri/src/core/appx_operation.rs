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
use crate::tweaks::appx::{AppxPackage, AppxPolicySnapshot};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppxAction {
    Mark { family_names: Vec<String> },
    Unmark { family_name: String },
    DisablePolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AppxPlanSnapshot {
    policy: AppxPolicySnapshot,
    candidates: Vec<AppxPackage>,
    action: AppxAction,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppxExecution {
    pub result: OperationResult,
    pub message: String,
}

pub fn create_plan(
    policy: &AppxPolicySnapshot,
    candidates: &[AppxPackage],
    action: AppxAction,
) -> Result<OperationPlan, OperationError> {
    validate_action(policy, candidates, &action)?;
    let snapshot = AppxPlanSnapshot {
        policy: policy.clone(),
        candidates: candidates.to_vec(),
        action: action.clone(),
    };
    let requested_action = serde_json::to_string(&snapshot)
        .map_err(|error| validation(format!("failed to serialize AppX target: {error}")))?;
    let target_ids = action_target_ids(&action);
    let targets = target_ids
        .iter()
        .map(|target_id| PlannedTarget {
            target_id: target_id.clone(),
            requested_action: requested_action.clone(),
            description: action_description(&action, target_id),
        })
        .collect::<Vec<_>>();

    Ok(OperationPlan {
        id: OperationId(uuid::Uuid::new_v4().to_string()),
        descriptor: OperationDescriptor {
            id: OperationId("appx.policy_change".into()),
            capability_id: "appx.manager".into(),
            title: action_title(&action),
            category: OperationCategory::AppManagement,
            risk: RiskLevel::High,
            privilege: PrivilegeRequirement::Administrator,
            reversibility: Reversibility::BestEffort,
            reboot_policy: RebootPolicy::MayBeRequired,
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
        expected_effects: targets
            .iter()
            .map(|target| ExpectedEffect {
                target_id: target.target_id.clone(),
                description: expected_description(&action),
            })
            .collect(),
        warnings: vec![
            OperationWarning {
                code: "experimental_workflow".into(),
                message: "AppX policy mutation remains Experimental and does not advertise exact undo."
                    .into(),
            },
            OperationWarning {
                code: "appx_sign_in_cycle".into(),
                message: "Windows may apply provisioning changes at the next sign-in or profile creation."
                    .into(),
            },
        ],
        targets,
        requires_elevation: true,
        requires_restart: true,
        reversibility: Reversibility::BestEffort,
        parent_operation_id: None,
        retry_of_operation_id: None,
    })
}

pub fn create_retry_plan(
    current_policy: &AppxPolicySnapshot,
    current_candidates: &[AppxPackage],
    previous_plan: &OperationPlan,
    unresolved_target_ids: &[String],
) -> Result<OperationPlan, OperationError> {
    previous_plan.validate_relationships()?;
    if previous_plan.descriptor.id.0 != "appx.policy_change" {
        return Err(OperationError::validation(
            "retry requires domain-specific rediscovery for this operation",
        ));
    }
    if unresolved_target_ids.is_empty() {
        return Err(validation("select at least one unresolved AppX target"));
    }

    let snapshot = decode_plan(previous_plan)?;
    let previous_target_ids = action_target_ids(&snapshot.action);
    let selected = unresolved_target_ids
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    if selected.len() != unresolved_target_ids.len()
        || unresolved_target_ids
            .iter()
            .any(|target_id| !previous_target_ids.contains(target_id))
    {
        return Err(validation(
            "retry targets do not match the previous AppX plan",
        ));
    }

    let retry_target_ids = unresolved_target_ids
        .iter()
        .filter(|target_id| !postcondition_satisfied(current_policy, &snapshot.action, target_id))
        .cloned()
        .collect::<Vec<_>>();
    if retry_target_ids.is_empty() {
        return Err(validation(
            "AppX target already matches the requested postcondition; no retry plan is needed",
        ));
    }

    let action = match snapshot.action {
        AppxAction::Mark { family_names } => AppxAction::Mark {
            family_names: family_names
                .into_iter()
                .filter(|family_name| retry_target_ids.contains(family_name))
                .collect(),
        },
        AppxAction::Unmark { family_name } => AppxAction::Unmark { family_name },
        AppxAction::DisablePolicy => AppxAction::DisablePolicy,
    };
    let mut plan = create_plan(current_policy, current_candidates, action)?;
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

pub fn confirmed_action(
    plan: &OperationPlan,
    current_policy: &AppxPolicySnapshot,
    current_candidates: &[AppxPackage],
) -> Result<AppxAction, OperationError> {
    let snapshot = decode_plan(plan)?;
    if &snapshot.policy != current_policy || snapshot.candidates != current_candidates {
        return Err(OperationError {
            code: OperationErrorCode::TargetChangedSincePreview,
            message: "AppX policy or candidate state changed after confirmation; review it again."
                .into(),
            target_id: None,
            retryable: true,
        });
    }
    validate_action(current_policy, current_candidates, &snapshot.action)?;
    Ok(snapshot.action)
}

pub fn finish(
    plan: &OperationPlan,
    before: &AppxPolicySnapshot,
    after: &AppxPolicySnapshot,
) -> Result<AppxExecution, OperationError> {
    let action = decode_plan(plan)?.action;
    let outcomes = action_target_ids(&action)
        .iter()
        .map(|target_id| target_outcome(plan, target_id, before, after))
        .collect::<Result<Vec<_>, _>>()?;
    let result = OperationResult::new(
        plan.id.clone(),
        outcomes,
        true,
        UndoAvailability::NotReversible,
    )?;
    Ok(AppxExecution {
        message: format!(
            "{} AppX targets changed; {} failed verification.",
            result.changed, result.failed
        ),
        result,
    })
}

pub fn target_outcome(
    plan: &OperationPlan,
    target_id: &str,
    before: &AppxPolicySnapshot,
    after: &AppxPolicySnapshot,
) -> Result<TargetOutcome, OperationError> {
    let action = decode_plan(plan)?.action;
    if !action_target_ids(&action).iter().any(|id| id == target_id) {
        return Err(OperationError::validation(
            "AppX target is not part of the confirmed action",
        ));
    }
    let (requested_action, verified) = match &action {
        AppxAction::Mark { .. } => (
            "mark_for_removal",
            after.marked_packages.iter().any(|name| name == target_id),
        ),
        AppxAction::Unmark { .. } => (
            "unmark",
            !after.marked_packages.iter().any(|name| name == target_id),
        ),
        AppxAction::DisablePolicy => ("disable_policy", !after.enabled),
    };
    Ok(outcome(
        target_id,
        requested_action,
        verified,
        before,
        after,
    ))
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
                "target {target_id} is not present in the confirmed AppX plan"
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
    before: &AppxPolicySnapshot,
    execution: &AppxExecution,
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
            target_id: plan.targets[0].target_id.clone(),
            state: policy_state(before),
        }],
        target_outcomes: execution.result.outcomes.clone(),
        verification_results: execution
            .result
            .outcomes
            .iter()
            .map(|outcome| VerificationRecord {
                target_id: outcome.target_id.clone(),
                status: outcome.verification,
                message: outcome
                    .error
                    .as_ref()
                    .map(|error| error.message.clone())
                    .unwrap_or_else(|| "AppX policy state verified by registry read-back.".into()),
            })
            .collect(),
        restart_requirement: RebootPolicy::MayBeRequired,
        undo_eligibility: UndoEligibility {
            eligible: false,
            reason: "AppX mutation remains Experimental; exact policy existence/type restoration is not proven.".into(),
            automated_restoration_test: None,
        },
        undo_execution_result: None,
        diagnostic: execution
            .result
            .outcomes
            .iter()
            .find_map(|outcome| outcome.error.clone()),
    }
}

fn decode_plan(plan: &OperationPlan) -> Result<AppxPlanSnapshot, OperationError> {
    if plan.descriptor.id.0 != "appx.policy_change"
        || plan.descriptor.category != OperationCategory::AppManagement
        || !plan.descriptor.experimental
        || plan.targets.is_empty()
        || plan.reversibility != Reversibility::BestEffort
    {
        return Err(validation("invalid AppX operation plan"));
    }
    let first: AppxPlanSnapshot = serde_json::from_str(&plan.targets[0].requested_action)
        .map_err(|error| validation(format!("invalid AppX target snapshot: {error}")))?;
    let expected_ids = action_target_ids(&first.action);
    if expected_ids.len() != plan.targets.len()
        || plan
            .targets
            .iter()
            .zip(expected_ids)
            .any(|(target, expected)| {
                target.target_id != expected
                    || target.requested_action != plan.targets[0].requested_action
            })
    {
        return Err(validation("AppX targets do not match the confirmed action"));
    }
    Ok(first)
}

fn validate_action(
    policy: &AppxPolicySnapshot,
    candidates: &[AppxPackage],
    action: &AppxAction,
) -> Result<(), OperationError> {
    match action {
        AppxAction::Mark { family_names } => {
            if family_names.is_empty() {
                return Err(validation("select at least one AppX package"));
            }
            let unique = family_names.iter().collect::<HashSet<_>>();
            if unique.len() != family_names.len() {
                return Err(validation("AppX selection contains duplicates"));
            }
            for family_name in family_names {
                let candidate = candidates
                    .iter()
                    .find(|candidate| candidate.family_name == *family_name)
                    .ok_or_else(|| OperationError {
                        code: OperationErrorCode::TargetProtected,
                        message: "AppX package is outside the current candidate list.".into(),
                        target_id: Some(family_name.clone()),
                        retryable: false,
                    })?;
                if candidate.is_protected {
                    return Err(OperationError {
                        code: OperationErrorCode::TargetProtected,
                        message: "Protected AppX package cannot be marked.".into(),
                        target_id: Some(family_name.clone()),
                        retryable: false,
                    });
                }
                if candidate.installed_package_full_name.is_none()
                    && candidate.provisioned_package_name.is_none()
                {
                    return Err(OperationError {
                        code: OperationErrorCode::TargetNotFound,
                        message: "AppX package is not installed or provisioned on this machine."
                            .into(),
                        target_id: Some(family_name.clone()),
                        retryable: true,
                    });
                }
                if policy.marked_packages.contains(family_name) {
                    return Err(validation(format!(
                        "{family_name} is already marked for removal"
                    )));
                }
            }
        }
        AppxAction::Unmark { family_name } => {
            if !policy.marked_packages.contains(family_name) {
                return Err(validation("AppX package is not currently marked"));
            }
        }
        AppxAction::DisablePolicy if !policy.enabled => {
            return Err(validation("AppX removal policy is already disabled"));
        }
        AppxAction::DisablePolicy => {}
    }
    Ok(())
}

fn action_target_ids(action: &AppxAction) -> Vec<String> {
    match action {
        AppxAction::Mark { family_names } => family_names.clone(),
        AppxAction::Unmark { family_name } => vec![family_name.clone()],
        AppxAction::DisablePolicy => vec!["appx_policy".into()],
    }
}

fn postcondition_satisfied(
    policy: &AppxPolicySnapshot,
    action: &AppxAction,
    target_id: &str,
) -> bool {
    match action {
        AppxAction::Mark { .. } => policy.marked_packages.iter().any(|name| name == target_id),
        AppxAction::Unmark { .. } => !policy.marked_packages.iter().any(|name| name == target_id),
        AppxAction::DisablePolicy => !policy.enabled,
    }
}

fn outcome(
    target_id: &str,
    requested_action: &str,
    verified: bool,
    before: &AppxPolicySnapshot,
    after: &AppxPolicySnapshot,
) -> TargetOutcome {
    TargetOutcome {
        target_id: target_id.into(),
        requested_action: requested_action.into(),
        before: Some(policy_state(before)),
        after: Some(policy_state(after)),
        execution: if verified {
            ExecutionStatus::Changed
        } else {
            ExecutionStatus::Failed
        },
        verification: if verified {
            VerificationStatus::Verified
        } else {
            VerificationStatus::Failed
        },
        error: (!verified).then(|| OperationError {
            code: OperationErrorCode::VerificationFailed,
            message: "AppX policy state did not match the confirmed effect.".into(),
            target_id: Some(target_id.into()),
            retryable: true,
        }),
    }
}

fn policy_state(policy: &AppxPolicySnapshot) -> StateSnapshot {
    StateSnapshot {
        summary: format!(
            "AppX policy {} with {} marked packages",
            if policy.enabled {
                "enabled"
            } else {
                "disabled"
            },
            policy.marked_packages.len()
        ),
        properties: BTreeMap::from([
            ("enabled".into(), policy.enabled.to_string()),
            ("marked_packages".into(), policy.marked_packages.join("\n")),
        ]),
    }
}

fn action_title(action: &AppxAction) -> String {
    match action {
        AppxAction::Mark { family_names } => {
            format!("Mark {} AppX packages for removal", family_names.len())
        }
        AppxAction::Unmark { .. } => "Unmark AppX package".into(),
        AppxAction::DisablePolicy => "Disable AppX removal policy".into(),
    }
}

fn action_description(action: &AppxAction, target_id: &str) -> String {
    match action {
        AppxAction::Mark { .. } => format!("Mark {target_id} for de-provisioning"),
        AppxAction::Unmark { .. } => format!("Remove {target_id} from the policy"),
        AppxAction::DisablePolicy => "Disable policy while preserving its package list".into(),
    }
}

fn expected_description(action: &AppxAction) -> String {
    match action {
        AppxAction::Mark { .. } => "Package family appears in the marked policy set.".into(),
        AppxAction::Unmark { .. } => "Package family is absent from the marked policy set.".into(),
        AppxAction::DisablePolicy => "AppX removal policy reports disabled.".into(),
    }
}

fn validation(message: impl Into<String>) -> OperationError {
    OperationError::validation(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn journal() -> (tempfile::TempDir, OperationJournal) {
        let directory = tempfile::tempdir().unwrap();
        let journal = OperationJournal::open_at(&directory.path().join("journal.db")).unwrap();
        (directory, journal)
    }

    fn package(name: &str, protected: bool, marked: bool) -> AppxPackage {
        AppxPackage {
            family_name: name.into(),
            display_name: name.into(),
            is_protected: protected,
            is_marked_for_removal: marked,
            installed_package_full_name: Some(format!("{name}_1.0.0.0_neutral__test")),
            provisioned_package_name: None,
            risk: "safe".into(),
        }
    }

    #[test]
    fn journal_target_helper_resolves_only_the_started_target() {
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
        assert_eq!(unresolved[0].target_id, contract.plan.targets[1].target_id);
        assert_eq!(unresolved[0].state, TargetExecutionState::Pending);
    }

    #[test]
    fn protected_package_is_rejected() {
        let policy = AppxPolicySnapshot {
            enabled: false,
            marked_packages: vec![],
        };
        let candidates = vec![package("Protected", true, false)];
        assert_eq!(
            create_plan(
                &policy,
                &candidates,
                AppxAction::Mark {
                    family_names: vec!["Protected".into()]
                }
            )
            .unwrap_err()
            .code,
            OperationErrorCode::TargetProtected
        );
    }

    #[test]
    fn changed_policy_is_rejected() {
        let policy = AppxPolicySnapshot {
            enabled: false,
            marked_packages: vec![],
        };
        let candidates = vec![package("Fixture", false, false)];
        let plan = create_plan(
            &policy,
            &candidates,
            AppxAction::Mark {
                family_names: vec!["Fixture".into()],
            },
        )
        .unwrap();
        let changed = AppxPolicySnapshot {
            enabled: true,
            marked_packages: vec![],
        };

        assert_eq!(
            confirmed_action(&plan, &changed, &candidates)
                .unwrap_err()
                .code,
            OperationErrorCode::TargetChangedSincePreview
        );
    }

    #[test]
    fn appx_policy_changes_do_not_require_system_restore() {
        let policy = AppxPolicySnapshot {
            enabled: false,
            marked_packages: vec![],
        };
        let candidates = vec![package("Fixture", false, false)];

        let plan = create_plan(
            &policy,
            &candidates,
            AppxAction::Mark {
                family_names: vec!["Fixture".into()],
            },
        )
        .unwrap();

        assert_eq!(
            plan.descriptor.restore_point_policy,
            crate::core::operation::RestorePointPolicy::NotNeeded
        );
    }

    #[test]
    fn retry_rebuilds_selected_target_and_links_the_previous_operation() {
        let policy = AppxPolicySnapshot {
            enabled: false,
            marked_packages: vec![],
        };
        let candidates = vec![package("Fixture", false, false)];
        let previous = create_plan(
            &policy,
            &candidates,
            AppxAction::Mark {
                family_names: vec!["Fixture".into()],
            },
        )
        .unwrap();
        let mut current_package = package("Fixture", false, false);
        current_package.display_name = "Current Fixture".into();
        let current_candidates = vec![current_package];

        let retry = create_retry_plan(
            &policy,
            &current_candidates,
            &previous,
            std::slice::from_ref(&previous.targets[0].target_id),
        )
        .unwrap();

        assert_ne!(retry.id, previous.id);
        assert_eq!(retry.parent_operation_id, Some(previous.id.clone()));
        assert_eq!(retry.retry_of_operation_id, Some(previous.id.clone()));
        assert!(retry
            .warnings
            .iter()
            .any(|warning| warning.code == "retry_of_operation"));
        assert_eq!(
            confirmed_action(&retry, &policy, &current_candidates).unwrap(),
            AppxAction::Mark {
                family_names: vec!["Fixture".into()]
            }
        );
    }

    #[test]
    fn retry_rejects_missing_protected_or_satisfied_targets() {
        let policy = AppxPolicySnapshot {
            enabled: false,
            marked_packages: vec![],
        };
        let candidates = vec![package("Fixture", false, false)];
        let previous = create_plan(
            &policy,
            &candidates,
            AppxAction::Mark {
                family_names: vec!["Fixture".into()],
            },
        )
        .unwrap();
        let target_id = previous.targets[0].target_id.clone();

        assert_eq!(
            create_retry_plan(&policy, &[], &previous, std::slice::from_ref(&target_id))
                .unwrap_err()
                .code,
            OperationErrorCode::TargetProtected
        );
        assert_eq!(
            create_retry_plan(
                &policy,
                &[package("Fixture", true, false)],
                &previous,
                std::slice::from_ref(&target_id),
            )
            .unwrap_err()
            .code,
            OperationErrorCode::TargetProtected
        );

        let satisfied = AppxPolicySnapshot {
            enabled: false,
            marked_packages: vec!["Fixture".into()],
        };
        let error = create_retry_plan(
            &satisfied,
            &candidates,
            &previous,
            std::slice::from_ref(&target_id),
        )
        .unwrap_err();
        assert_eq!(error.code, OperationErrorCode::ValidationFailed);
        assert!(error.message.contains("no retry plan"));
    }
}
