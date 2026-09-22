use super::operation::{
    ExecutionStatus, ExpectedEffect, ExposureLevel, OperationCategory, OperationDescriptor,
    OperationError, OperationErrorCode, OperationId, OperationPlan, OperationResult,
    OperationWarning, PlannedTarget, PrivilegeRequirement, ProofStatus, RebootPolicy,
    RecoveryLevel, Reversibility, RiskLevel, StateSnapshot, TargetExecutionState, TargetOutcome,
    UndoAvailability, VerificationStatus,
};
use super::operation_journal::{
    CapturedOriginalState, EnvironmentScope, JournalRecordKind, OperationJournal,
    OperationJournalRecord, UndoEligibility, VerificationRecord, OPERATION_SCHEMA_VERSION,
};
use crate::core::adapters::{RegistryAdapter, RegistryValue};
use crate::system_info::env_vars::EnvVar;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EnvAction {
    Set { variable: EnvVar },
    Delete { name: String, scope: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct EnvPlanSnapshot {
    action: EnvAction,
    before: Option<EnvVar>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnvExecution {
    pub result: OperationResult,
    pub message: String,
}

fn registry_key(scope: &str) -> &'static str {
    if scope == "system" {
        crate::system_info::env_vars::SYSTEM_ENVIRONMENT_KEY
    } else {
        crate::system_info::env_vars::USER_ENVIRONMENT_KEY
    }
}

fn decode_registry_value(name: String, scope: &str, value: RegistryValue) -> Option<EnvVar> {
    let (var_type, text) = match value {
        RegistryValue::String(text) => ("REG_SZ", text),
        RegistryValue::Raw { value_type, bytes } => {
            if !matches!(value_type, 1 | 2) {
                return None;
            }
            let units = bytes
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .take_while(|c| *c != 0)
                .collect::<Vec<_>>();
            (
                if value_type == 2 {
                    "REG_EXPAND_SZ"
                } else {
                    "REG_SZ"
                },
                String::from_utf16_lossy(&units),
            )
        }
        RegistryValue::Dword(_) => return None,
    };
    Some(EnvVar {
        name,
        value: text,
        scope: scope.into(),
        var_type: var_type.into(),
    })
}

pub fn list_from_adapter(registry: &dyn RegistryAdapter) -> Result<Vec<EnvVar>, OperationError> {
    let mut vars = Vec::new();
    for (scope, key) in [
        ("user", registry_key("user")),
        ("system", registry_key("system")),
    ] {
        for (name, value) in registry.list(key).map_err(|e| internal(e.to_string()))? {
            if let Some(var) = decode_registry_value(name, scope, value) {
                vars.push(var);
            }
        }
    }
    vars.sort_by(|a, b| a.scope.cmp(&b.scope).then(a.name.cmp(&b.name)));
    Ok(vars)
}

pub fn read_from_adapter(
    registry: &dyn RegistryAdapter,
    name: &str,
    scope: &str,
) -> Result<Option<EnvVar>, OperationError> {
    registry
        .read(registry_key(scope), name)
        .map_err(|e| internal(e.to_string()))
        .map(|value| value.and_then(|v| decode_registry_value(name.into(), scope, v)))
}

pub fn mutate_with_adapter(
    registry: &dyn RegistryAdapter,
    action: &EnvAction,
) -> Result<(), OperationError> {
    match action {
        EnvAction::Set { variable } => {
            let value_type = if variable.var_type == "REG_EXPAND_SZ" {
                2
            } else {
                1
            };
            let mut bytes = variable
                .value
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>();
            bytes.extend_from_slice(&[0, 0]);
            registry
                .write(
                    registry_key(&variable.scope),
                    &variable.name,
                    RegistryValue::Raw { value_type, bytes },
                )
                .map_err(|e| execution(e.to_string()))
        }
        EnvAction::Delete { name, scope } => registry
            .delete_value(registry_key(scope), name)
            .map_err(|e| execution(e.to_string())),
    }
}

/// Run one already-revalidated environment target through its durable target
/// lifecycle. The mutation closure is not called unless both pre-mutation
/// journal states have been persisted.
pub(crate) fn execute_target_with_journal(
    journal: &OperationJournal,
    plan: &OperationPlan,
    target: &PlannedTarget,
    execute: impl FnOnce() -> Result<EnvExecution, OperationError>,
) -> Result<EnvExecution, OperationError> {
    let payload = serde_json::to_string(target).map_err(|error| OperationError {
        code: OperationErrorCode::SerializationFailed,
        message: error.to_string(),
        target_id: Some(target.target_id.clone()),
        retryable: false,
    })?;
    journal.record_target_state(
        &plan.id,
        &target.target_id,
        TargetExecutionState::Ready,
        &payload,
    )?;
    journal.record_target_state(
        &plan.id,
        &target.target_id,
        TargetExecutionState::MutationStarted,
        &payload,
    )?;

    let execution = match execute() {
        Ok(execution) => execution,
        Err(error) => {
            journal.record_target_state(
                &plan.id,
                &target.target_id,
                TargetExecutionState::OutcomeUnknown,
                &error.to_string(),
            )?;
            return Err(error);
        }
    };
    if execution.result.outcomes.len() != 1
        || execution.result.outcomes[0].target_id != target.target_id
    {
        let error = OperationError::validation(
            "environment target execution must yield exactly its confirmed target outcome",
        );
        journal.record_target_state(
            &plan.id,
            &target.target_id,
            TargetExecutionState::OutcomeUnknown,
            &error.to_string(),
        )?;
        return Err(error);
    }
    journal.record_target_outcome(&plan.id, &execution.result.outcomes[0])?;
    Ok(execution)
}

pub fn create_plan(current: &[EnvVar], action: EnvAction) -> Result<OperationPlan, OperationError> {
    let (name, scope) = identity(&action);
    validate_action(&action)?;
    let before = find(current, name, scope).cloned();
    if matches!(action, EnvAction::Delete { .. }) && before.is_none() {
        return Err(target_missing(name, scope));
    }
    let snapshot = EnvPlanSnapshot {
        action: action.clone(),
        before,
    };
    let requested_action = serde_json::to_string(&snapshot)
        .map_err(|error| OperationError::validation(error.to_string()))?;
    let system = scope == "system";
    Ok(OperationPlan {
        id: OperationId(uuid::Uuid::new_v4().to_string()),
        descriptor: OperationDescriptor {
            id: OperationId("environment.change".into()),
            capability_id: "environment.variables".into(),
            title: match action {
                EnvAction::Set { .. } => "Set Environment Variable",
                EnvAction::Delete { .. } => "Delete Environment Variable",
            }
            .into(),
            category: OperationCategory::SystemConfiguration,
            risk: RiskLevel::High,
            privilege: if system {
                PrivilegeRequirement::Administrator
            } else {
                PrivilegeRequirement::StandardUser
            },
            reversibility: Reversibility::FullyReversible,
            reboot_policy: RebootPolicy::NotRequired,
            experimental: true,
            proof_status: ProofStatus::PartiallyProven,
            exposure: ExposureLevel::Supporting,
            recovery: RecoveryLevel::Exact,
            restore_point_policy: crate::core::operation::RestorePointPolicy::NotNeeded,
            mutates_system: true,
            requires_confirmation: true,
            requires_revalidation: true,
            requires_verification: true,
            requires_journal: true,
        },
        targets: vec![PlannedTarget {
            target_id: format!("{scope}\\{name}"),
            requested_action,
            description: format!("Change only {name} in {scope} scope."),
        }],
        warnings: vec![OperationWarning {
            code: "environment_refresh".into(),
            message:
                "Existing processes keep their current environment; new processes see the change."
                    .into(),
        }],
        expected_effects: vec![ExpectedEffect {
            target_id: format!("{scope}\\{name}"),
            description: "Exact scope, type, and value are verified by registry re-read.".into(),
        }],
        requires_elevation: system,
        requires_restart: false,
        reversibility: Reversibility::FullyReversible,
        parent_operation_id: None,
        retry_of_operation_id: None,
    })
}

pub fn create_retry_plan(
    current: &[EnvVar],
    previous_plan: &OperationPlan,
    unresolved_target_ids: &[String],
) -> Result<OperationPlan, OperationError> {
    if previous_plan.descriptor.id.0 != "environment.change" {
        return Err(OperationError::validation(
            "retry requires domain-specific rediscovery for this operation",
        ));
    }
    if unresolved_target_ids.len() != 1 {
        return Err(OperationError::validation(
            "environment retry requires exactly one target",
        ));
    }

    let snapshot = decode(previous_plan)?;
    let action = snapshot.action;
    let (name, scope) = identity(&action);
    let expected_target_id = format!("{scope}\\{name}");
    if unresolved_target_ids[0] != expected_target_id {
        return Err(OperationError::validation(
            "retry target does not match the previous environment plan",
        ));
    }

    let current_target = find(current, name, scope);
    let postcondition_present = match &action {
        EnvAction::Set { variable } => current_target == Some(variable),
        EnvAction::Delete { .. } => current_target.is_none(),
    };
    if postcondition_present {
        return Err(OperationError::validation(
            "environment target already matches the requested postcondition; no retry plan is needed",
        ));
    }

    let mut plan = create_plan(current, action)?;
    plan.parent_operation_id = Some(previous_plan.id.clone());
    plan.retry_of_operation_id = Some(previous_plan.id.clone());
    plan.warnings.push(OperationWarning {
        code: "retry_of_operation".into(),
        message: previous_plan.id.0.clone(),
    });
    plan.validate_relationships()?;
    Ok(plan)
}

pub fn confirmed_action(
    plan: &OperationPlan,
    current: &[EnvVar],
) -> Result<EnvAction, OperationError> {
    let snapshot = decode(plan)?;
    let (name, scope) = identity(&snapshot.action);
    if find(current, name, scope) != snapshot.before.as_ref() {
        return Err(OperationError {
            code: OperationErrorCode::TargetChangedSincePreview,
            message: "The environment variable changed after confirmation; review it again.".into(),
            target_id: Some(format!("{scope}\\{name}")),
            retryable: true,
        });
    }
    validate_action(&snapshot.action)?;
    let authoritative_requires_elevation = action_requires_elevation(&snapshot.action);
    if plan.requires_elevation != authoritative_requires_elevation {
        return Err(OperationError::validation(
            "environment elevation metadata does not match the confirmed action scope",
        ));
    }
    Ok(snapshot.action)
}

pub fn action_requires_elevation(action: &EnvAction) -> bool {
    matches!(identity(action).1, "system")
}

pub(crate) fn mark_target_revalidation_failed(
    journal: &OperationJournal,
    plan: &OperationPlan,
    target_id: &str,
) -> Result<(), OperationError> {
    let target = plan
        .targets
        .iter()
        .find(|target| target.target_id == target_id)
        .ok_or_else(|| OperationError::validation("environment target is not in the plan"))?;
    let payload = serde_json::to_string(target).map_err(|error| OperationError {
        code: OperationErrorCode::SerializationFailed,
        message: error.to_string(),
        target_id: Some(target_id.into()),
        retryable: false,
    })?;
    journal.record_target_state(
        &plan.id,
        target_id,
        TargetExecutionState::RevalidationFailed,
        &payload,
    )
}

pub fn finish(plan: &OperationPlan, current: &[EnvVar]) -> Result<EnvExecution, OperationError> {
    let snapshot = decode(plan)?;
    let (name, scope) = identity(&snapshot.action);
    let after = find(current, name, scope).cloned();
    let verified = match &snapshot.action {
        EnvAction::Set { variable } => after.as_ref() == Some(variable),
        EnvAction::Delete { .. } => after.is_none(),
    };
    let changed = snapshot.before != after;
    let error = (!verified).then(|| OperationError {
        code: OperationErrorCode::VerificationFailed,
        message: "Registry read-back did not match the confirmed variable state.".into(),
        target_id: Some(format!("{scope}\\{name}")),
        retryable: true,
    });
    let result = OperationResult::new(
        plan.id.clone(),
        vec![TargetOutcome {
            target_id: format!("{scope}\\{name}"),
            requested_action: match snapshot.action {
                EnvAction::Set { .. } => "set",
                EnvAction::Delete { .. } => "delete",
            }
            .into(),
            before: snapshot.before.as_ref().map(state),
            after: after.as_ref().map(state),
            execution: if !verified {
                ExecutionStatus::Failed
            } else if changed {
                ExecutionStatus::Changed
            } else {
                ExecutionStatus::Unchanged
            },
            verification: if verified {
                VerificationStatus::Verified
            } else {
                VerificationStatus::Failed
            },
            error,
        }],
        false,
        if verified {
            UndoAvailability::UndoAvailable
        } else {
            UndoAvailability::NotReversible
        },
    )?;
    Ok(EnvExecution {
        message: if result.failed == 0 {
            "Environment variable changed and verified.".into()
        } else {
            "Environment variable failed verification.".into()
        },
        result,
    })
}

pub fn journal_record(plan: &OperationPlan, execution: &EnvExecution) -> OperationJournalRecord {
    let snapshot = decode(plan).expect("validated environment plan");
    let (name, scope) = identity(&snapshot.action);
    let scope_value = if scope == "system" {
        EnvironmentScope::System
    } else {
        EnvironmentScope::User
    };
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
        captured_original_states: vec![CapturedOriginalState::EnvironmentVariable {
            target_id: format!("{scope}\\{name}"),
            name: name.into(),
            scope: scope_value,
            existed: snapshot.before.is_some(),
            value_type: snapshot.before.as_ref().map(|var| var.var_type.clone()),
            value: snapshot.before.as_ref().map(|var| var.value.clone()),
        }],
        target_outcomes: execution.result.outcomes.clone(),
        verification_results: vec![VerificationRecord {
            target_id: format!("{scope}\\{name}"),
            status: execution.result.outcomes[0].verification,
            message: "Environment variable verified by scoped registry re-read.".into(),
        }],
        restart_requirement: RebootPolicy::NotRequired,
        undo_eligibility: UndoEligibility {
            eligible: execution.result.failed == 0,
            reason: "Exact prior existence, type, and raw string value were captured.".into(),
            automated_restoration_test: Some(
                "env_operation::captured_user_variable_is_restored_and_verified".into(),
            ),
        },
        undo_execution_result: None,
        diagnostic: execution.result.outcomes[0].error.clone(),
    }
}

pub fn restore_captured_state_with_adapter(
    capture: &CapturedOriginalState,
    registry: &dyn RegistryAdapter,
) -> Result<TargetOutcome, OperationError> {
    let CapturedOriginalState::EnvironmentVariable {
        target_id,
        name,
        scope,
        existed,
        value_type,
        value,
    } = capture
    else {
        return Err(OperationError::validation(
            "environment undo received a different capture type",
        ));
    };
    let scope_name = match scope {
        EnvironmentScope::User => "user",
        EnvironmentScope::System => "system",
    };
    let before = read_from_adapter(registry, name, scope_name)?;
    if *existed {
        let variable = EnvVar {
            name: name.clone(),
            value: value
                .clone()
                .ok_or_else(|| OperationError::validation("captured value is missing"))?,
            scope: scope_name.into(),
            var_type: value_type
                .clone()
                .ok_or_else(|| OperationError::validation("captured value type is missing"))?,
        };
        mutate_with_adapter(registry, &EnvAction::Set { variable })?;
    } else {
        mutate_with_adapter(
            registry,
            &EnvAction::Delete {
                name: name.clone(),
                scope: scope_name.into(),
            },
        )?;
    }
    let after = read_from_adapter(registry, name, scope_name)?;
    let expected = existed.then(|| EnvVar {
        name: name.clone(),
        value: value.clone().unwrap_or_default(),
        scope: scope_name.into(),
        var_type: value_type.clone().unwrap_or_default(),
    });
    let verified = after == expected;
    let error = (!verified).then(|| OperationError {
        code: OperationErrorCode::VerificationFailed,
        message: "Environment undo registry read-back did not match captured state.".into(),
        target_id: Some(target_id.clone()),
        retryable: true,
    });
    Ok(TargetOutcome {
        target_id: target_id.clone(),
        requested_action: "restore_captured_original_state".into(),
        before: before.as_ref().map(state),
        after: after.as_ref().map(state),
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
        error,
    })
}

pub fn restore_captured_state(
    capture: &CapturedOriginalState,
) -> Result<TargetOutcome, OperationError> {
    let CapturedOriginalState::EnvironmentVariable {
        target_id,
        name,
        scope,
        existed,
        value_type,
        value,
    } = capture
    else {
        return Err(OperationError::validation(
            "environment undo received a different capture type",
        ));
    };
    let scope_name = match scope {
        EnvironmentScope::User => "user",
        EnvironmentScope::System => "system",
    };
    let before = crate::system_info::env_vars::read_scoped(name, scope_name).map_err(internal)?;
    if *existed {
        let variable = EnvVar {
            name: name.clone(),
            value: value
                .clone()
                .ok_or_else(|| OperationError::validation("captured value is missing"))?,
            scope: scope_name.into(),
            var_type: value_type
                .clone()
                .ok_or_else(|| OperationError::validation("captured value type is missing"))?,
        };
        crate::system_info::env_vars::restore(&variable).map_err(execution)?;
    } else {
        crate::system_info::env_vars::delete_without_undo(name, scope_name).map_err(execution)?;
    }
    let after = crate::system_info::env_vars::read_scoped(name, scope_name).map_err(internal)?;
    let expected = existed.then(|| EnvVar {
        name: name.clone(),
        value: value.clone().unwrap_or_default(),
        scope: scope_name.into(),
        var_type: value_type.clone().unwrap_or_default(),
    });
    let verified = after == expected;
    Ok(TargetOutcome {
        target_id: target_id.clone(),
        requested_action: "restore_captured_original_state".into(),
        before: before.as_ref().map(state),
        after: after.as_ref().map(state),
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
            message: "Environment undo registry read-back did not match captured state.".into(),
            target_id: Some(target_id.clone()),
            retryable: true,
        }),
    })
}

fn decode(plan: &OperationPlan) -> Result<EnvPlanSnapshot, OperationError> {
    if plan.descriptor.id.0 != "environment.change"
        || plan.descriptor.category != OperationCategory::SystemConfiguration
        || plan.targets.len() != 1
    {
        return Err(OperationError::validation(
            "invalid environment operation plan",
        ));
    }
    let snapshot: EnvPlanSnapshot = serde_json::from_str(&plan.targets[0].requested_action)
        .map_err(|error| OperationError::validation(error.to_string()))?;
    let (name, scope) = identity(&snapshot.action);
    if plan.targets[0].target_id != format!("{scope}\\{name}") {
        return Err(OperationError::validation(
            "environment plan target was modified",
        ));
    }
    validate_action(&snapshot.action)?;
    Ok(snapshot)
}

fn identity(action: &EnvAction) -> (&str, &str) {
    match action {
        EnvAction::Set { variable } => (&variable.name, &variable.scope),
        EnvAction::Delete { name, scope } => (name, scope),
    }
}

fn find<'a>(current: &'a [EnvVar], name: &str, scope: &str) -> Option<&'a EnvVar> {
    current
        .iter()
        .find(|var| var.scope == scope && var.name.eq_ignore_ascii_case(name))
}

fn validate(name: &str, scope: &str) -> Result<(), OperationError> {
    if name.is_empty()
        || name.contains(['=', '\0', '%'])
        || !matches!(scope, "user" | "system")
        || crate::system_info::env_vars::is_protected_name(name)
    {
        return Err(OperationError {
            code: OperationErrorCode::TargetProtected,
            message: "The environment variable name or scope is invalid or protected.".into(),
            target_id: Some(format!("{scope}\\{name}")),
            retryable: false,
        });
    }
    Ok(())
}

fn validate_action(action: &EnvAction) -> Result<(), OperationError> {
    let (name, scope) = identity(action);
    validate(name, scope)?;
    if let EnvAction::Set { variable } = action {
        if !matches!(variable.var_type.as_str(), "REG_SZ" | "REG_EXPAND_SZ") {
            return Err(OperationError::validation(
                "Environment variables must use REG_SZ or REG_EXPAND_SZ.",
            ));
        }
        if variable.value.contains(['\0', '\r', '\n']) {
            return Err(OperationError::validation(
                "Environment variable values cannot contain control characters.",
            ));
        }
    }
    Ok(())
}

fn target_missing(name: &str, scope: &str) -> OperationError {
    OperationError {
        code: OperationErrorCode::TargetNotFound,
        message: "The environment variable does not exist.".into(),
        target_id: Some(format!("{scope}\\{name}")),
        retryable: true,
    }
}

fn state(var: &EnvVar) -> StateSnapshot {
    StateSnapshot {
        summary: format!("{} ({})", var.name, var.scope),
        properties: BTreeMap::from([
            ("value".into(), var.value.clone()),
            ("type".into(), var.var_type.clone()),
        ]),
    }
}

fn internal(message: String) -> OperationError {
    OperationError {
        code: OperationErrorCode::InternalError,
        message,
        target_id: None,
        retryable: true,
    }
}

fn execution(message: String) -> OperationError {
    OperationError {
        code: OperationErrorCode::ExecutionFailed,
        message,
        target_id: None,
        retryable: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::adapters::{FixtureRegistryAdapter, RegistryAdapter};

    #[test]
    fn unsupported_raw_registry_types_are_not_exposed_by_adapter_readback() {
        let registry = FixtureRegistryAdapter::new();
        registry
            .write(
                crate::system_info::env_vars::USER_ENVIRONMENT_KEY,
                "WINOPTIMIZER_UNSUPPORTED_RAW",
                RegistryValue::Raw {
                    value_type: 4,
                    bytes: vec![1, 0, 2, 0],
                },
            )
            .unwrap();

        assert!(list_from_adapter(&registry)
            .unwrap()
            .iter()
            .all(|variable| { variable.name != "WINOPTIMIZER_UNSUPPORTED_RAW" }));
        assert!(
            read_from_adapter(&registry, "WINOPTIMIZER_UNSUPPORTED_RAW", "user")
                .unwrap()
                .is_none()
        );
    }

    fn journal() -> (tempfile::TempDir, OperationJournal) {
        let directory = tempfile::tempdir().unwrap();
        let journal = OperationJournal::open_at(&directory.path().join("journal.db")).unwrap();
        (directory, journal)
    }

    fn synthetic_execution(plan: &OperationPlan, target: &PlannedTarget) -> EnvExecution {
        let state = || StateSnapshot {
            summary: "synthetic environment state".into(),
            properties: BTreeMap::new(),
        };
        let outcome = TargetOutcome {
            target_id: target.target_id.clone(),
            requested_action: target.requested_action.clone(),
            before: Some(state()),
            after: Some(state()),
            execution: ExecutionStatus::Changed,
            verification: VerificationStatus::Verified,
            error: None,
        };
        EnvExecution {
            message: "synthetic test execution".into(),
            result: OperationResult::new(
                plan.id.clone(),
                vec![outcome],
                false,
                UndoAvailability::NotReversible,
            )
            .unwrap(),
        }
    }

    #[test]
    fn journaled_partial_target_execution_leaves_later_targets_pending() {
        let (_directory, journal) = journal();
        let contract = crate::core::operation::synthetic_operation_contract().unwrap();
        let first = contract.plan.targets[0].clone();
        let second = contract.plan.targets[1].clone();
        journal.begin_execution(&contract.plan).unwrap();
        let mut mutation_count = 0;

        execute_target_with_journal(&journal, &contract.plan, &first, || {
            mutation_count += 1;
            Ok(synthetic_execution(&contract.plan, &first))
        })
        .unwrap();

        assert_eq!(mutation_count, 1);
        let unresolved = journal.list_unresolved_targets(&contract.plan.id).unwrap();
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].target_id, second.target_id);
        assert_eq!(unresolved[0].state, TargetExecutionState::Pending);

        assert_eq!(journal.reconcile_unresolved_operations().unwrap(), 1);
        let reconciled = journal.list_unresolved_targets(&contract.plan.id).unwrap();
        assert_eq!(reconciled.len(), 1);
        assert_eq!(reconciled[0].target_id, second.target_id);
        assert_eq!(reconciled[0].state, TargetExecutionState::OutcomeUnknown);
    }

    #[test]
    fn missing_durable_target_state_blocks_mutation_before_the_closure() {
        let (_directory, journal) = journal();
        let contract = crate::core::operation::synthetic_operation_contract().unwrap();
        let target = contract.plan.targets[0].clone();
        let mut mutation_called = false;

        let error = execute_target_with_journal(&journal, &contract.plan, &target, || {
            mutation_called = true;
            Ok(synthetic_execution(&contract.plan, &target))
        })
        .unwrap_err();

        assert_eq!(error.code, OperationErrorCode::ValidationFailed);
        assert!(error.message.contains("no durable state"));
        assert!(!mutation_called);
    }

    #[test]
    fn mutation_error_is_left_as_outcome_unknown_after_target_start() {
        let (_directory, journal) = journal();
        let contract = crate::core::operation::synthetic_operation_contract().unwrap();
        let target = contract.plan.targets[0].clone();
        journal.begin_execution(&contract.plan).unwrap();

        let error = execute_target_with_journal(&journal, &contract.plan, &target, || {
            Err(OperationError {
                code: OperationErrorCode::ExecutionFailed,
                message: "synthetic mutation failure".into(),
                target_id: Some(target.target_id.clone()),
                retryable: true,
            })
        })
        .unwrap_err();

        assert_eq!(error.code, OperationErrorCode::ExecutionFailed);
        let unresolved = journal.list_unresolved_targets(&contract.plan.id).unwrap();
        assert_eq!(unresolved[0].target_id, target.target_id);
        assert_eq!(unresolved[0].state, TargetExecutionState::OutcomeUnknown);
    }

    fn variable(name: &str, scope: &str, var_type: &str, value: &str) -> EnvVar {
        EnvVar {
            name: name.into(),
            value: value.into(),
            scope: scope.into(),
            var_type: var_type.into(),
        }
    }

    #[test]
    fn protected_names_are_rejected() {
        let error = create_plan(
            &[],
            EnvAction::Set {
                variable: EnvVar {
                    name: "SystemRoot".into(),
                    value: "x".into(),
                    scope: "system".into(),
                    var_type: "REG_SZ".into(),
                },
            },
        )
        .unwrap_err();
        assert_eq!(error.code, OperationErrorCode::TargetProtected);
    }

    #[test]
    fn source_contract_covers_both_scopes_and_supported_string_types() {
        for scope in ["user", "system"] {
            for var_type in ["REG_SZ", "REG_EXPAND_SZ"] {
                let name = format!("WINOPTIMIZER_ENV_{scope}_{var_type}");
                let absent = variable(&name, scope, var_type, "%TEMP%\\value");
                let create = create_plan(
                    &[],
                    EnvAction::Set {
                        variable: absent.clone(),
                    },
                )
                .unwrap();
                assert!(create.descriptor.requires_revalidation);
                assert!(create.descriptor.requires_verification);
                assert_eq!(create.descriptor.proof_status, ProofStatus::PartiallyProven);
                assert_eq!(create.descriptor.recovery, RecoveryLevel::Exact);

                let existing = variable(&name, scope, var_type, "%TEMP%\\old");
                create_plan(
                    std::slice::from_ref(&existing),
                    EnvAction::Set { variable: absent },
                )
                .unwrap();
                create_plan(
                    &[existing],
                    EnvAction::Delete {
                        name,
                        scope: scope.into(),
                    },
                )
                .unwrap();
            }
        }
    }

    #[test]
    fn forged_system_elevation_metadata_is_rejected_before_mutation() {
        let variable = variable(
            "WINOPTIMIZER_ENV_FORGED_SYSTEM_SCOPE",
            "system",
            "REG_SZ",
            "value",
        );
        let mut plan = create_plan(&[], EnvAction::Set { variable }).unwrap();
        plan.requires_elevation = false;

        let error = confirmed_action(&plan, &[]).unwrap_err();
        assert_eq!(error.code, OperationErrorCode::ValidationFailed);
        assert!(error.message.contains("elevation metadata"));
    }

    #[test]
    fn revalidation_is_scoped_and_readback_captures_exact_type_and_value() {
        let original = variable(
            "WINOPTIMIZER_ENV_SCOPED",
            "user",
            "REG_EXPAND_SZ",
            "%TEMP%\\old",
        );
        let changed = variable(
            "WINOPTIMIZER_ENV_SCOPED",
            "user",
            "REG_EXPAND_SZ",
            "%TEMP%\\new",
        );
        let plan = create_plan(
            std::slice::from_ref(&original),
            EnvAction::Set {
                variable: changed.clone(),
            },
        )
        .unwrap();

        let same_name_other_scope = variable(
            "WINOPTIMIZER_ENV_SCOPED",
            "system",
            "REG_SZ",
            "system-value",
        );
        let mut current = vec![original.clone(), same_name_other_scope];
        assert!(confirmed_action(&plan, &current).is_ok());
        current[0] = changed.clone();
        let error = confirmed_action(&plan, &current).unwrap_err();
        assert_eq!(error.code, OperationErrorCode::TargetChangedSincePreview);

        let execution = finish(&plan, &[changed]).unwrap();
        assert_eq!(
            execution.result.outcomes[0].verification,
            VerificationStatus::Verified
        );
        let record = journal_record(&plan, &execution);
        let CapturedOriginalState::EnvironmentVariable {
            scope,
            value_type,
            value,
            existed,
            ..
        } = &record.captured_original_states[0]
        else {
            panic!("unexpected capture type");
        };
        assert_eq!(*scope, EnvironmentScope::User);
        assert_eq!(value_type.as_deref(), Some("REG_EXPAND_SZ"));
        assert_eq!(value.as_deref(), Some("%TEMP%\\old"));
        assert!(*existed);
        assert!(record.undo_eligibility.eligible);

        let delete_plan = create_plan(
            &[original],
            EnvAction::Delete {
                name: "WINOPTIMIZER_ENV_SCOPED".into(),
                scope: "user".into(),
            },
        )
        .unwrap();
        let delete_execution = finish(&delete_plan, &[]).unwrap();
        assert_eq!(
            delete_execution.result.outcomes[0].verification,
            VerificationStatus::Verified
        );
        let delete_record = journal_record(&delete_plan, &delete_execution);
        let CapturedOriginalState::EnvironmentVariable {
            scope,
            value_type,
            value,
            existed,
            ..
        } = &delete_record.captured_original_states[0]
        else {
            panic!("unexpected delete capture type");
        };
        assert_eq!(*scope, EnvironmentScope::User);
        assert_eq!(value_type.as_deref(), Some("REG_EXPAND_SZ"));
        assert_eq!(value.as_deref(), Some("%TEMP%\\old"));
        assert!(*existed);
        assert!(delete_record.undo_eligibility.eligible);
    }

    #[test]
    fn failed_verification_does_not_advertise_undo() {
        let original = variable("WINOPTIMIZER_ENV_FAILED_UNDO", "user", "REG_SZ", "original");
        let desired = variable("WINOPTIMIZER_ENV_FAILED_UNDO", "user", "REG_SZ", "desired");
        let plan = create_plan(&[original], EnvAction::Set { variable: desired }).unwrap();

        let execution = finish(&plan, &[]).unwrap();
        assert_eq!(execution.result.undo, UndoAvailability::NotReversible);
        assert!(!journal_record(&plan, &execution).undo_eligibility.eligible);
    }

    #[test]
    fn retry_rebuilds_from_current_state_and_links_the_previous_operation() {
        let original = variable("WINOPTIMIZER_ENV_RETRY", "user", "REG_SZ", "old");
        let current = variable("WINOPTIMIZER_ENV_RETRY", "user", "REG_SZ", "interrupted");
        let desired = variable("WINOPTIMIZER_ENV_RETRY", "user", "REG_SZ", "new");
        let previous = create_plan(
            std::slice::from_ref(&original),
            EnvAction::Set {
                variable: desired.clone(),
            },
        )
        .unwrap();

        let retry = create_retry_plan(
            std::slice::from_ref(&current),
            &previous,
            std::slice::from_ref(&previous.targets[0].target_id),
        )
        .unwrap();

        assert_ne!(retry.id, previous.id);
        assert_eq!(retry.parent_operation_id, Some(previous.id.clone()));
        assert_eq!(retry.retry_of_operation_id, Some(previous.id));
        assert!(confirmed_action(&retry, std::slice::from_ref(&current)).is_ok());
        assert_eq!(retry.targets[0].target_id, "user\\WINOPTIMIZER_ENV_RETRY");
        assert!(retry.targets[0].requested_action.contains("interrupted"));

        let changed = variable("WINOPTIMIZER_ENV_RETRY", "user", "REG_SZ", "changed");
        assert_eq!(
            confirmed_action(&retry, std::slice::from_ref(&changed))
                .unwrap_err()
                .code,
            OperationErrorCode::TargetChangedSincePreview
        );
    }

    #[test]
    fn retry_rejects_a_satisfied_postcondition_without_replaying_it() {
        let desired = variable("WINOPTIMIZER_ENV_RETRY_SATISFIED", "user", "REG_SZ", "new");
        let previous = create_plan(
            &[],
            EnvAction::Set {
                variable: desired.clone(),
            },
        )
        .unwrap();

        let error = create_retry_plan(
            std::slice::from_ref(&desired),
            &previous,
            std::slice::from_ref(&previous.targets[0].target_id),
        )
        .unwrap_err();
        assert_eq!(error.code, OperationErrorCode::ValidationFailed);
        assert!(error.message.contains("postcondition"));
    }

    #[test]
    fn retry_rejects_multiple_environment_targets() {
        let variable = variable("WINOPTIMIZER_ENV_RETRY_MULTI", "user", "REG_SZ", "new");
        let previous = create_plan(
            &[],
            EnvAction::Set {
                variable: variable.clone(),
            },
        )
        .unwrap();
        let error = create_retry_plan(
            std::slice::from_ref(&variable),
            &previous,
            &[
                previous.targets[0].target_id.clone(),
                previous.targets[0].target_id.clone(),
            ],
        )
        .unwrap_err();
        assert_eq!(error.code, OperationErrorCode::ValidationFailed);
    }

    #[test]
    fn invalid_registry_type_and_control_value_are_rejected_before_execution() {
        for variable in [
            variable("WINOPTIMIZER_ENV_INVALID_TYPE", "user", "REG_DWORD", "1"),
            variable(
                "WINOPTIMIZER_ENV_INVALID_VALUE",
                "user",
                "REG_SZ",
                "line\nfeed",
            ),
        ] {
            let error = create_plan(&[], EnvAction::Set { variable }).unwrap_err();
            assert_eq!(error.code, OperationErrorCode::ValidationFailed);
        }
    }

    #[test]
    fn captured_user_variable_is_restored_and_verified() {
        let name = format!("WINOPTIMIZER_ENV_UNDO_{}", uuid::Uuid::new_v4());
        let original = EnvVar {
            name: name.clone(),
            value: "%TEMP%\\original".into(),
            scope: "user".into(),
            var_type: "REG_EXPAND_SZ".into(),
        };
        let changed = EnvVar {
            value: "changed".into(),
            var_type: "REG_SZ".into(),
            ..original.clone()
        };
        crate::system_info::env_vars::delete_without_undo(&name, "user").unwrap();
        crate::system_info::env_vars::restore(&original).unwrap();
        crate::system_info::env_vars::restore(&changed).unwrap();
        let capture = CapturedOriginalState::EnvironmentVariable {
            target_id: format!("user\\{name}"),
            name: name.clone(),
            scope: EnvironmentScope::User,
            existed: true,
            value_type: Some(original.var_type.clone()),
            value: Some(original.value.clone()),
        };

        let outcome = restore_captured_state(&capture).unwrap();
        let restored = crate::system_info::env_vars::list()
            .unwrap()
            .into_iter()
            .find(|var| var.scope == "user" && var.name == name);
        crate::system_info::env_vars::delete_without_undo(&name, "user").unwrap();

        assert_eq!(outcome.verification, VerificationStatus::Verified);
        assert_eq!(restored, Some(original));
    }
}
