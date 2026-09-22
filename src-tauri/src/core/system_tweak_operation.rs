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
use crate::tweaks::{customization, performance, privacy};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TweakKind {
    Privacy,
    Performance,
    Customization,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TweakPhysicalResource {
    RegistryDword {
        full_key: String,
        value_name: String,
        key_existed: bool,
        value_existed: bool,
        value: Option<u32>,
        #[serde(default)]
        registry_type: Option<u32>,
        #[serde(default)]
        raw_value: Option<Vec<u8>>,
    },
    Service {
        name: String,
        existed: bool,
        start_type: Option<String>,
        running: Option<bool>,
        delayed_auto_start: Option<bool>,
    },
    ScheduledTask {
        path: String,
        existed: bool,
        enabled: Option<bool>,
        current_state: Option<String>,
        #[serde(default)]
        definition: Option<crate::startup::scheduler::TaskDefinitionSnapshot>,
    },
    OptionalFeature {
        name: String,
        existed: bool,
        enabled: bool,
    },
    ActivePowerPlan {
        guid: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TweakPhysicalState {
    pub resources: Vec<TweakPhysicalResource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemTweakTarget {
    pub kind: TweakKind,
    pub id: String,
    pub name: String,
    pub description: String,
    pub risk: RiskLevel,
    pub is_applied: bool,
    pub experimental: bool,
    pub requires_elevation: bool,
    pub requires_restart: bool,
    pub evidence_status: String,
    pub exposure: ExposureLevel,
    pub reversibility: Reversibility,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TweakTargetSnapshot {
    target: SystemTweakTarget,
    apply: bool,
    physical_state: TweakPhysicalState,
}

#[derive(Debug, Clone, Serialize)]
pub struct SystemTweakExecution {
    pub result: OperationResult,
    pub message: String,
}

pub fn list_targets() -> Vec<SystemTweakTarget> {
    list_targets_with_labs(true)
}

pub fn list_targets_with_labs(labs_enabled: bool) -> Vec<SystemTweakTarget> {
    let privacy_targets = privacy::list().into_iter().map(|tweak| {
        let experimental = matches!(
            tweak.id.as_str(),
            "stop_diagtrack_service"
                | "disable_telemetry_autologger"
                | "disable_telemetry_tasks"
                | "disable_ai_service_autostart"
        );
        let requires_restart = matches!(
            tweak.id.as_str(),
            "disable_copilot" | "disable_recall" | "disable_modern_standby_networking"
        );
        let user_scoped = matches!(
            tweak.id.as_str(),
            "disable_advertising_id"
                | "disable_app_launch_tracking"
                | "disable_copilot"
                | "disable_click_to_do"
        );
        let conditional = matches!(
            tweak.id.as_str(),
            "disable_web_search"
                | "disable_copilot"
                | "disable_find_my_device"
                | "disable_recall"
                | "disable_click_to_do"
                | "disable_modern_standby_networking"
                | "disable_ai_service_autostart"
        );
        SystemTweakTarget {
            kind: TweakKind::Privacy,
            risk: risk(&tweak.risk),
            requires_restart,
            requires_elevation: !user_scoped,
            experimental,
            id: tweak.id,
            name: tweak.name,
            description: tweak.description,
            is_applied: tweak.is_applied,
            evidence_status: if experimental {
                "experimental".into()
            } else {
                "pending_native_validation".into()
            },
            exposure: if experimental {
                ExposureLevel::Labs
            } else if conditional {
                ExposureLevel::Supporting
            } else {
                ExposureLevel::Core
            },
            reversibility: if experimental {
                Reversibility::BestEffort
            } else {
                Reversibility::FullyReversible
            },
        }
    });
    let performance_targets = performance::list().into_iter().map(|tweak| {
        let experimental = matches!(
            tweak.id.as_str(),
            "processor_scheduling"
                | "high_performance_power"
                | "disable_indexing"
                | "ultimate_performance_power"
                | "disable_hpet"
                | "enable_tsc_sync"
                | "disable_dynamic_tick"
                | "force_tsc_calibration"
                | "disable_timer_coalescing"
        );
        let requires_restart = matches!(
            tweak.id.as_str(),
            "disable_hpet"
                | "enable_tsc_sync"
                | "disable_dynamic_tick"
                | "force_tsc_calibration"
                | "disable_timer_coalescing"
        );
        SystemTweakTarget {
            kind: TweakKind::Performance,
            risk: risk(&tweak.risk),
            requires_restart,
            requires_elevation: tweak.id != "adjust_visual_effects"
                && tweak.id != "disable_startup_delay"
                && tweak.id != "enable_game_mode"
                && tweak.id != "disable_game_dvr"
                && tweak.id != "disable_mouse_acceleration",
            experimental,
            id: tweak.id,
            name: tweak.name,
            description: tweak.description,
            is_applied: tweak.is_applied,
            evidence_status: if experimental {
                "experimental".into()
            } else {
                "pending_native_validation".into()
            },
            exposure: if experimental {
                ExposureLevel::Labs
            } else {
                ExposureLevel::Supporting
            },
            reversibility: if experimental {
                Reversibility::BestEffort
            } else {
                Reversibility::FullyReversible
            },
        }
    });
    let customization_targets = customization::list()
        .into_iter()
        .map(|tweak| SystemTweakTarget {
            kind: TweakKind::Customization,
            risk: risk(&tweak.risk),
            requires_restart: false,
            requires_elevation: matches!(
                tweak.id.as_str(),
                "enable_long_paths" | "disable_windows_tips"
            ),
            experimental: false,
            id: tweak.id,
            name: tweak.name,
            description: tweak.description,
            is_applied: tweak.is_applied,
            evidence_status: "pending_native_validation".into(),
            exposure: ExposureLevel::Supporting,
            reversibility: Reversibility::FullyReversible,
        });
    privacy_targets
        .chain(performance_targets)
        .chain(customization_targets)
        .filter(|target| labs_enabled || !target.experimental)
        .collect()
}

pub fn find_target(kind: &TweakKind, id: &str) -> Result<SystemTweakTarget, OperationError> {
    find_target_with_labs(kind, id, true)
}

pub fn find_target_with_labs(
    kind: &TweakKind,
    id: &str,
    labs_enabled: bool,
) -> Result<SystemTweakTarget, OperationError> {
    list_targets_with_labs(labs_enabled)
        .into_iter()
        .find(|target| &target.kind == kind && target.id == id)
        .ok_or_else(|| OperationError {
            code: OperationErrorCode::TargetNotFound,
            message: "System tweak is no longer available.".into(),
            target_id: Some(id.into()),
            retryable: true,
        })
}

pub fn ensure_labs_enabled(
    plan: &OperationPlan,
    settings_path: impl AsRef<std::path::Path>,
) -> Result<(), OperationError> {
    if plan.descriptor.exposure != ExposureLevel::Labs {
        return Ok(());
    }
    let labs_enabled = crate::core::settings::SettingsStore::at(settings_path)
        .load()
        .map(|settings| settings.labs_enabled)
        .unwrap_or(false);
    if labs_enabled {
        Ok(())
    } else {
        Err(OperationError {
            code: OperationErrorCode::TargetProtected,
            message: "Experimental system tweaks require Labs to be enabled in Settings.".into(),
            target_id: plan.targets.first().map(|target| target.target_id.clone()),
            retryable: false,
        })
    }
}

pub fn create_plan(target: &SystemTweakTarget) -> Result<OperationPlan, OperationError> {
    let apply = !target.is_applied;
    let physical_state = capture_physical_state(target)?;
    let snapshot = TweakTargetSnapshot {
        target: target.clone(),
        apply,
        physical_state,
    };
    let requested_action = serde_json::to_string(&snapshot)
        .map_err(|error| validation(format!("failed to serialize tweak target: {error}")))?;
    let reversibility = if target.experimental {
        Reversibility::BestEffort
    } else {
        Reversibility::FullyReversible
    };
    let reboot_policy = if target.requires_restart {
        RebootPolicy::Required
    } else {
        RebootPolicy::NotRequired
    };
    let target_id = format!("{:?}:{}", target.kind, target.id).to_lowercase();

    Ok(OperationPlan {
        id: OperationId(uuid::Uuid::new_v4().to_string()),
        descriptor: OperationDescriptor {
            id: OperationId("system_tweak.change".into()),
            capability_id: "tweaks.system".into(),
            title: if apply {
                format!("Enable {}", target.name)
            } else {
                format!("Disable {}", target.name)
            },
            category: OperationCategory::SystemTweak,
            risk: target.risk,
            privilege: if target.requires_elevation {
                PrivilegeRequirement::Administrator
            } else {
                PrivilegeRequirement::StandardUser
            },
            reversibility,
            reboot_policy,
            experimental: target.experimental,
            proof_status: if target.experimental {
                ProofStatus::Experimental
            } else if target.evidence_status == "pending_native_validation" {
                ProofStatus::PartiallyProven
            } else {
                ProofStatus::Proven
            },
            exposure: if target.experimental {
                ExposureLevel::Labs
            } else {
                ExposureLevel::Advanced
            },
            recovery: if target.experimental {
                RecoveryLevel::BestEffort
            } else {
                RecoveryLevel::Exact
            },
            restore_point_policy: if target.experimental {
                crate::core::operation::RestorePointPolicy::Recommended
            } else {
                crate::core::operation::RestorePointPolicy::NotNeeded
            },
            mutates_system: true,
            requires_confirmation: true,
            requires_revalidation: true,
            requires_verification: true,
            requires_journal: true,
        },
        targets: vec![PlannedTarget {
            target_id: target_id.clone(),
            requested_action,
            description: target.description.clone(),
        }],
        warnings: warnings(target),
        expected_effects: vec![ExpectedEffect {
            target_id,
            description: format!("Applied state becomes {apply} and is verified by read-back."),
        }],
        requires_elevation: target.requires_elevation,
        requires_restart: target.requires_restart,
        reversibility,
        parent_operation_id: None,
        retry_of_operation_id: None,
    })
}

fn capture_physical_state(
    target: &SystemTweakTarget,
) -> Result<TweakPhysicalState, OperationError> {
    #[cfg(test)]
    if target.id == "fixture" {
        return Ok(TweakPhysicalState { resources: vec![] });
    }
    let result = match target.kind {
        TweakKind::Privacy => crate::tweaks::privacy::capture_state(&target.id),
        TweakKind::Performance => crate::tweaks::performance::capture_state(&target.id),
        TweakKind::Customization => crate::tweaks::customization::capture_state(&target.id),
    };
    result.map_err(|message| OperationError {
        code: OperationErrorCode::InternalError,
        message: format!("Could not capture {} pre-state: {message}", target.name),
        target_id: Some(target.id.clone()),
        retryable: true,
    })
}

pub fn restore_captured_state(
    capture: &CapturedOriginalState,
) -> Result<TargetOutcome, OperationError> {
    let (target_id, state) = match capture {
        CapturedOriginalState::TypedState { target_id, state } => (target_id, state),
        _ => {
            return Err(validation(
                "system tweak recovery requires a typed captured state",
            ))
        }
    };
    let physical_state = state
        .properties
        .get("physical_state")
        .ok_or_else(|| validation("system tweak capture has no physical pre-state"))
        .and_then(|value| {
            serde_json::from_str::<TweakPhysicalState>(value).map_err(|error| {
                validation(format!("invalid system tweak physical state: {error}"))
            })
        })?;
    let (kind, id) = target_id
        .split_once(':')
        .ok_or_else(|| validation("system tweak capture target identifier is invalid"))?;
    let kind = match kind {
        "privacy" => TweakKind::Privacy,
        "performance" => TweakKind::Performance,
        "customization" => TweakKind::Customization,
        _ => return Err(validation("system tweak capture kind is invalid")),
    };
    let before = find_target(&kind, id)?;
    let before_physical_state = capture_physical_state(&before)?;
    match kind {
        TweakKind::Privacy => crate::tweaks::privacy::restore_state(id, &physical_state),
        TweakKind::Performance => crate::tweaks::performance::restore_state(id, &physical_state),
        TweakKind::Customization => {
            crate::tweaks::customization::restore_state(id, &physical_state)
        }
    }
    .map_err(|message| OperationError {
        code: OperationErrorCode::ExecutionFailed,
        message,
        target_id: Some(target_id.clone()),
        retryable: true,
    })?;
    let after = find_target(&kind, id)?;
    let after_physical_state = capture_physical_state(&after)?;
    if after_physical_state != physical_state {
        return Err(OperationError {
            code: OperationErrorCode::VerificationFailed,
            message: "System tweak recovery did not restore the captured physical state.".into(),
            target_id: Some(target_id.clone()),
            retryable: true,
        });
    }
    Ok(TargetOutcome {
        target_id: target_id.clone(),
        requested_action: "restore_captured_original_state".into(),
        before: Some(state_snapshot(&before, &before_physical_state)),
        after: Some(state_snapshot(&after, &after_physical_state)),
        execution: ExecutionStatus::Changed,
        verification: VerificationStatus::Verified,
        error: None,
    })
}

pub fn create_retry_plan(
    current_targets: &[SystemTweakTarget],
    previous_plan: &OperationPlan,
    unresolved_target_ids: &[String],
) -> Result<OperationPlan, OperationError> {
    previous_plan.validate_relationships()?;
    if previous_plan.descriptor.id.0 != "system_tweak.change" {
        return Err(OperationError::validation(
            "retry requires domain-specific rediscovery for this operation",
        ));
    }
    if unresolved_target_ids.len() != 1 {
        return Err(validation("system tweak retry requires exactly one target"));
    }

    let snapshot = decode_plan(previous_plan)?;
    let expected_target_id = previous_plan.targets[0].target_id.clone();
    if unresolved_target_ids[0] != expected_target_id {
        return Err(validation(
            "retry target does not match the previous system tweak plan",
        ));
    }

    let current_target = current_targets
        .iter()
        .find(|target| target.kind == snapshot.target.kind && target.id == snapshot.target.id)
        .ok_or_else(|| {
            validation("system tweak target was not rediscovered; no retry plan was created")
        })?;
    if current_target.is_applied == snapshot.apply {
        return Err(validation(
            "system tweak target already matches the requested postcondition; no retry plan is needed",
        ));
    }
    if current_target.experimental != snapshot.target.experimental {
        return Err(validation(
            "system tweak target classification changed; no retry plan was created",
        ));
    }

    let mut plan = create_plan(current_target)?;
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

pub fn confirmed_target(
    plan: &OperationPlan,
    current: &SystemTweakTarget,
) -> Result<bool, OperationError> {
    let snapshot = decode_plan(plan)?;
    if &snapshot.target != current {
        return Err(OperationError {
            code: OperationErrorCode::TargetChangedSincePreview,
            message: "System tweak state changed after confirmation; review it again.".into(),
            target_id: Some(current.id.clone()),
            retryable: true,
        });
    }
    let current_physical_state = capture_physical_state(current)?;
    if current_physical_state != snapshot.physical_state {
        return Err(OperationError {
            code: OperationErrorCode::TargetChangedSincePreview,
            message: "System tweak physical state changed after confirmation; review it again."
                .into(),
            target_id: Some(current.id.clone()),
            retryable: true,
        });
    }
    Ok(snapshot.apply)
}

pub fn finish(
    plan: &OperationPlan,
    before: &SystemTweakTarget,
    after: &SystemTweakTarget,
) -> Result<SystemTweakExecution, OperationError> {
    let apply = confirmed_target(plan, before)?;
    let snapshot = decode_plan(plan)?;
    if before.kind != after.kind || before.id != after.id || after.is_applied != apply {
        return Err(OperationError {
            code: OperationErrorCode::VerificationFailed,
            message: "System tweak state did not match the confirmed result.".into(),
            target_id: Some(before.id.clone()),
            retryable: true,
        });
    }
    let outcome = TargetOutcome {
        target_id: plan.targets[0].target_id.clone(),
        requested_action: if apply { "apply" } else { "revert" }.into(),
        before: Some(state_snapshot(before, &snapshot.physical_state)),
        after: Some(state_snapshot(after, &snapshot.physical_state)),
        execution: ExecutionStatus::Changed,
        verification: VerificationStatus::Verified,
        error: None,
    };
    let result = OperationResult::new(
        plan.id.clone(),
        vec![outcome],
        plan.requires_restart,
        if plan.descriptor.recovery == RecoveryLevel::Exact {
            UndoAvailability::UndoAvailable
        } else {
            UndoAvailability::NotReversible
        },
    )?;
    Ok(SystemTweakExecution {
        result,
        message: format!(
            "{} was {} and verified.",
            before.name,
            if apply { "enabled" } else { "disabled" }
        ),
    })
}

pub(crate) fn mark_target_ready_and_mutation_started_for_operation(
    journal: &OperationJournal,
    operation_id: &OperationId,
    plan: &OperationPlan,
    target_id: &str,
) -> Result<(), OperationError> {
    mark_target_ready_for_operation(journal, operation_id, plan, target_id)?;
    let payload = target_payload(plan, target_id)?;
    journal.record_target_state(
        operation_id,
        target_id,
        TargetExecutionState::MutationStarted,
        &payload,
    )
}

pub(crate) fn mark_target_ready_for_operation(
    journal: &OperationJournal,
    operation_id: &OperationId,
    plan: &OperationPlan,
    target_id: &str,
) -> Result<(), OperationError> {
    let payload = target_payload(plan, target_id)?;
    journal.record_target_state(
        operation_id,
        target_id,
        TargetExecutionState::Ready,
        &payload,
    )
}

pub(crate) fn mark_target_revalidation_failed_for_operation(
    journal: &OperationJournal,
    operation_id: &OperationId,
    plan: &OperationPlan,
    target_id: &str,
) -> Result<(), OperationError> {
    let payload = target_payload(plan, target_id)?;
    journal.record_target_state(
        operation_id,
        target_id,
        TargetExecutionState::RevalidationFailed,
        &payload,
    )
}

pub(crate) fn record_target_outcome_for_operation(
    journal: &OperationJournal,
    operation_id: &OperationId,
    outcome: &TargetOutcome,
) -> Result<(), OperationError> {
    journal.record_target_outcome(operation_id, outcome)
}

fn target_payload(plan: &OperationPlan, target_id: &str) -> Result<String, OperationError> {
    let target = plan
        .targets
        .iter()
        .find(|target| target.target_id == target_id)
        .ok_or_else(|| {
            OperationError::validation(format!(
                "target {target_id} is not present in the confirmed system tweak plan"
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
    before: &SystemTweakTarget,
    execution: &SystemTweakExecution,
) -> OperationJournalRecord {
    let snapshot = decode_plan(plan).expect("validated system tweak plan must decode");
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
            state: state_snapshot(before, &snapshot.physical_state),
        }],
        target_outcomes: execution.result.outcomes.clone(),
        verification_results: execution
            .result
            .outcomes
            .iter()
            .map(|outcome| VerificationRecord {
                target_id: outcome.target_id.clone(),
                status: outcome.verification,
                message: "Tweak state verified by domain read-back.".into(),
            })
            .collect(),
        restart_requirement: plan.descriptor.reboot_policy,
        undo_eligibility: if plan.descriptor.recovery == RecoveryLevel::Exact {
            UndoEligibility::exact(
                "system_tweak_operation::captured_state_is_restored_and_verified",
            )
        } else {
            UndoEligibility::unavailable("Experimental tweaks do not advertise exact undo.")
        },
        undo_execution_result: None,
        diagnostic: None,
    }
}

fn decode_plan(plan: &OperationPlan) -> Result<TweakTargetSnapshot, OperationError> {
    if plan.descriptor.id.0 != "system_tweak.change"
        || plan.descriptor.category != OperationCategory::SystemTweak
        || plan.targets.len() != 1
    {
        return Err(validation("invalid system tweak operation plan"));
    }
    let snapshot: TweakTargetSnapshot = serde_json::from_str(&plan.targets[0].requested_action)
        .map_err(|error| validation(format!("invalid system tweak snapshot: {error}")))?;
    let expected_id = format!("{:?}:{}", snapshot.target.kind, snapshot.target.id).to_lowercase();
    if plan.targets[0].target_id != expected_id
        || plan.descriptor.experimental != snapshot.target.experimental
    {
        return Err(validation(
            "system tweak target does not match the confirmed plan",
        ));
    }
    Ok(snapshot)
}

fn state_snapshot(
    target: &SystemTweakTarget,
    physical_state: &TweakPhysicalState,
) -> StateSnapshot {
    let physical_state =
        serde_json::to_string(physical_state).expect("system tweak physical state must serialize");
    StateSnapshot {
        summary: format!(
            "{} is {}",
            target.name,
            if target.is_applied {
                "enabled"
            } else {
                "disabled"
            }
        ),
        properties: BTreeMap::from([
            ("kind".into(), format!("{:?}", target.kind).to_lowercase()),
            ("id".into(), target.id.clone()),
            ("is_applied".into(), target.is_applied.to_string()),
            ("experimental".into(), target.experimental.to_string()),
            ("physical_state".into(), physical_state),
        ]),
    }
}

fn warnings(target: &SystemTweakTarget) -> Vec<OperationWarning> {
    let mut warnings = Vec::new();
    if target.experimental {
        warnings.push(OperationWarning {
            code: "experimental_workflow".into(),
            message: "This target remains Experimental because exact automated restoration is not yet proven.".into(),
        });
    }
    if target.evidence_status == "pending_native_validation" {
        warnings.push(OperationWarning {
            code: "native_validation_pending".into(),
            message: "Source review is complete, but native Windows validation is still pending; verify the resulting state on the target machine.".into(),
        });
    }
    if target.requires_restart {
        warnings.push(OperationWarning {
            code: "restart_required".into(),
            message: "Windows must restart before this change is fully active.".into(),
        });
    }
    warnings
}

fn risk(value: &str) -> RiskLevel {
    match value {
        "low" => RiskLevel::Low,
        "medium" => RiskLevel::Medium,
        "high" => RiskLevel::High,
        _ => RiskLevel::Critical,
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

    fn target(experimental: bool) -> SystemTweakTarget {
        SystemTweakTarget {
            kind: TweakKind::Privacy,
            id: "fixture".into(),
            name: "Fixture".into(),
            description: "Fixture tweak".into(),
            risk: RiskLevel::Low,
            is_applied: false,
            experimental,
            requires_elevation: false,
            requires_restart: false,
            evidence_status: if experimental {
                "experimental".into()
            } else {
                "confirmed_working".into()
            },
            exposure: if experimental {
                ExposureLevel::Labs
            } else {
                ExposureLevel::Core
            },
            reversibility: if experimental {
                Reversibility::BestEffort
            } else {
                Reversibility::FullyReversible
            },
        }
    }

    #[test]
    fn journal_target_helper_leaves_unstarted_targets_pending() {
        let (_directory, journal) = journal();
        let contract = crate::core::operation::synthetic_operation_contract().unwrap();
        let first = &contract.plan.targets[0];
        journal.begin_execution(&contract.plan).unwrap();
        mark_target_ready_and_mutation_started_for_operation(
            &journal,
            &contract.plan.id,
            &contract.plan,
            &first.target_id,
        )
        .unwrap();
        let outcome = TargetOutcome {
            target_id: first.target_id.clone(),
            requested_action: first.requested_action.clone(),
            before: None,
            after: None,
            execution: ExecutionStatus::Changed,
            verification: VerificationStatus::Verified,
            error: None,
        };
        journal
            .record_target_outcome(&contract.plan.id, &outcome)
            .unwrap();

        let unresolved = journal.list_unresolved_targets(&contract.plan.id).unwrap();
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].state, TargetExecutionState::Pending);
        assert_eq!(unresolved[0].target_id, contract.plan.targets[1].target_id);
    }

    #[test]
    fn stable_registry_target_is_reversible() {
        let source = target(false);
        let plan = create_plan(&source).unwrap();

        assert_eq!(plan.reversibility, Reversibility::FullyReversible);
        assert_eq!(plan.descriptor.recovery, RecoveryLevel::Exact);
        assert!(!plan.descriptor.experimental);
        assert!(confirmed_target(&plan, &source).unwrap());
    }

    #[test]
    fn stable_execution_advertises_exact_undo() {
        let source = target(false);
        let plan = create_plan(&source).unwrap();
        let mut after = source.clone();
        after.is_applied = true;

        let execution = finish(&plan, &source, &after).unwrap();

        assert_eq!(execution.result.undo, UndoAvailability::UndoAvailable);
    }

    #[test]
    fn experimental_target_never_claims_exact_undo() {
        let source = target(true);
        let plan = create_plan(&source).unwrap();

        assert_eq!(plan.reversibility, Reversibility::BestEffort);
        assert!(plan.descriptor.experimental);
        assert!(plan
            .warnings
            .iter()
            .any(|warning| warning.code == "experimental_workflow"));
    }

    #[test]
    fn labs_filter_hides_experimental_system_targets_when_disabled() {
        assert!(list_targets_with_labs(false)
            .iter()
            .all(|target| !target.experimental));
        assert!(list_targets_with_labs(true)
            .iter()
            .any(|target| target.experimental));
    }

    #[test]
    fn changed_state_is_rejected_before_execution() {
        let source = target(false);
        let plan = create_plan(&source).unwrap();
        let mut changed = source;
        changed.is_applied = true;

        assert_eq!(
            confirmed_target(&plan, &changed).unwrap_err().code,
            OperationErrorCode::TargetChangedSincePreview
        );
    }

    #[test]
    fn retry_rebuilds_from_current_target_and_links_the_previous_operation() {
        let previous_target = target(false);
        let previous = create_plan(&previous_target).unwrap();
        let mut current_target = previous_target;
        current_target.name = "Current Fixture".into();

        let retry = create_retry_plan(
            std::slice::from_ref(&current_target),
            &previous,
            std::slice::from_ref(&previous.targets[0].target_id),
        )
        .unwrap();

        assert_ne!(retry.id, previous.id);
        assert_eq!(retry.parent_operation_id, Some(previous.id.clone()));
        assert_eq!(retry.retry_of_operation_id, Some(previous.id.clone()));
        assert_eq!(retry.descriptor.title, "Enable Current Fixture");
        assert!(confirmed_target(&retry, &current_target).unwrap());
    }

    #[test]
    fn retry_rejects_missing_changed_or_satisfied_targets() {
        let previous_target = target(false);
        let previous = create_plan(&previous_target).unwrap();
        let target_id = previous.targets[0].target_id.clone();

        assert_eq!(
            create_retry_plan(&[], &previous, std::slice::from_ref(&target_id))
                .unwrap_err()
                .code,
            OperationErrorCode::ValidationFailed
        );

        let mut changed_target = target(false);
        changed_target.id = "changed_fixture".into();
        assert_eq!(
            create_retry_plan(
                std::slice::from_ref(&changed_target),
                &previous,
                std::slice::from_ref(&target_id),
            )
            .unwrap_err()
            .code,
            OperationErrorCode::ValidationFailed
        );

        let mut changed_classification = target(true);
        changed_classification.is_applied = false;
        let error = create_retry_plan(
            std::slice::from_ref(&changed_classification),
            &previous,
            std::slice::from_ref(&target_id),
        )
        .unwrap_err();
        assert!(error.message.contains("classification changed"));

        let mut satisfied_target = target(false);
        satisfied_target.is_applied = true;
        let error = create_retry_plan(
            std::slice::from_ref(&satisfied_target),
            &previous,
            std::slice::from_ref(&target_id),
        )
        .unwrap_err();
        assert_eq!(error.code, OperationErrorCode::ValidationFailed);
        assert!(error.message.contains("no retry plan"));
    }

    #[test]
    fn read_back_must_match_confirmed_effect() {
        let source = target(false);
        let plan = create_plan(&source).unwrap();
        let after = source.clone();

        assert_eq!(
            finish(&plan, &source, &after).unwrap_err().code,
            OperationErrorCode::VerificationFailed
        );
    }
}
