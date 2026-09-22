use super::evidence::{
    EvidenceConfidence, EvidenceConflict, EvidenceMatchingRule, EvidenceMetadata,
    EvidenceSourceType,
};
use super::operation::{
    ExecutionStatus, ExpectedEffect, ExposureLevel, OperationCategory, OperationDescriptor,
    OperationError, OperationErrorCode, OperationId, OperationPlan, OperationResult,
    OperationWarning, PlannedTarget, PrivilegeRequirement, ProofStatus, RebootPolicy,
    RecoveryLevel, Reversibility, RiskLevel, TargetOutcome, UndoAvailability, VerificationStatus,
};
use super::operation_journal::{
    JournalRecordKind, OperationJournalRecord, UndoEligibility, VerificationRecord,
    OPERATION_SCHEMA_VERSION,
};
use super::system_tweak_operation::{self, SystemTweakTarget, TweakKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PresetDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub target_ids: Vec<String>,
    pub finding: String,
    pub mechanism: String,
    pub tradeoffs: Vec<String>,
    pub confidence: EvidenceConfidence,
    pub provenance: EvidenceMetadata,
}

#[derive(Debug, Clone, Serialize)]
pub struct PresetExecution {
    pub result: OperationResult,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PresetTarget {
    kind: TweakKind,
    id: String,
}

pub fn definitions() -> Vec<PresetDefinition> {
    vec![
        PresetDefinition {
            id: "balanced".into(),
            name: "Balanced control".into(),
            description: "Applies low-risk, reversible controls for clearer Explorer behavior, reduced promotion, and predictable desktop startup.".into(),
            target_ids: vec![
                "privacy:disable_telemetry".into(),
                "privacy:disable_activity_history".into(),
                "privacy:disable_advertising_id".into(),
                "privacy:disable_tailored_experiences".into(),
                "privacy:disable_app_launch_tracking".into(),
                "privacy:disable_delivery_optimization_p2p".into(),
                "customization:show_known_file_extensions".into(),
                "customization:enable_end_task".into(),
                "customization:disable_promotional_content".into(),
                "customization:disable_lock_screen_spotlight".into(),
                "customization:hide_widgets".into(),
                "customization:disable_windows_tips".into(),
                "performance:disable_startup_delay".into(),
            ],
            finding: "Balanced controls are not currently applied on this machine.".into(),
            mechanism: "Writes only the selected low-risk policy and usability values; every constituent operation is shown before confirmation.".into(),
            tradeoffs: vec!["Windows promotions, activity history, and peer-to-peer delivery features may be reduced.".into()],
            confidence: EvidenceConfidence::Unknown,
            provenance: provenance(
                "The candidate uses exact local applied-state reads; WinOptimizer has not measured a benefit for this machine.",
                &["Windows promotions, activity history, and peer-to-peer delivery features may be reduced."],
            ),
        },
        PresetDefinition {
            id: "privacy_essentials".into(),
            name: "Privacy essentials".into(),
            description:
                "Applies the source-reviewed privacy policies shown in System Tweaks, with capability-gated AI and device-location controls available when supported.".into(),
            target_ids: vec![
                "privacy:disable_telemetry".into(),
                "privacy:disable_cortana".into(),
                "privacy:disable_activity_history".into(),
                "privacy:disable_advertising_id".into(),
                "privacy:disable_location_tracking".into(),
                "privacy:disable_web_search".into(),
                "privacy:disable_feedback".into(),
                "privacy:disable_copilot".into(),
                "privacy:disable_find_my_device".into(),
                "privacy:disable_recall".into(),
                "privacy:disable_click_to_do".into(),
            ],
                finding: "Source-reviewed privacy-policy targets are not currently applied on this machine."
                .into(),
            mechanism: "Writes the selected Windows privacy policy registry values; the exact constituent targets are shown before confirmation.".into(),
            tradeoffs: vec![
                "Some Windows privacy features or related convenience behavior may be reduced."
                    .into(),
            ],
            confidence: EvidenceConfidence::Unknown,
            provenance: provenance(
                "The candidate is based on exact local applied-state reads; WinOptimizer has not measured a benefit for this machine.",
                &["Some Windows privacy features or related convenience behavior may be reduced."],
            ),
        },
        PresetDefinition {
            id: "desktop_responsiveness".into(),
            name: "Desktop responsiveness".into(),
            description:
                "Applies visible-effects and startup-delay registry tweaks."
                    .into(),
            target_ids: vec![
                "performance:adjust_visual_effects".into(),
                "performance:disable_startup_delay".into(),
            ],
                finding: "Source-reviewed responsiveness targets are not currently applied on this machine."
                .into(),
            mechanism: "Writes the Visual Effects and startup-delay registry values; the exact constituent targets are shown before confirmation.".into(),
            tradeoffs: vec![
                "Visual effects may be reduced and startup behavior may change.".into(),
            ],
            confidence: EvidenceConfidence::Unknown,
            provenance: provenance(
                "The candidate is based on exact local applied-state reads; WinOptimizer has not measured a benefit for this machine.",
                &["Visual effects may be reduced and startup behavior may change."],
            ),
        },
        PresetDefinition {
            id: "gaming".into(),
            name: "Gaming controls".into(),
            description:
                "Applies reversible gaming preferences for Game Mode, background capture, pointer response, visual effects, and startup delay.".into(),
            target_ids: vec![
                "performance:enable_game_mode".into(),
                "performance:disable_game_dvr".into(),
                "performance:disable_mouse_acceleration".into(),
                "performance:adjust_visual_effects".into(),
                "performance:disable_startup_delay".into(),
            ],
            finding: "Gaming controls are not currently applied on this machine.".into(),
            mechanism: "Writes the selected Windows gaming and desktop-preference values; the exact constituent targets are shown before confirmation.".into(),
            tradeoffs: vec![
                "Background game capture is disabled and pointer/visual behavior changes.".into(),
            ],
            confidence: EvidenceConfidence::Unknown,
            provenance: provenance(
                "The candidate is based on exact local applied-state reads; WinOptimizer has not measured a benefit for this machine.",
                &["Background game capture is disabled and pointer/visual behavior changes."],
            ),
        },
    ]
}

/// Return only source-reviewed, currently applicable preset members. Unknown or
/// contradictory evidence is intentionally excluded instead of being guessed.
pub fn machine_aware_definitions() -> Vec<PresetDefinition> {
    let targets = system_tweak_operation::list_targets();
    definitions()
        .into_iter()
        .filter_map(|mut definition| {
            let target_ids = eligible_target_ids(&definition, &targets).ok()?;
            if target_ids.is_empty() {
                return None;
            }
            let count = target_ids.len();
            definition.finding = finding_for_count(count);
            definition.target_ids = target_ids;
            Some(definition)
        })
        .collect()
}

pub fn create_plan(preset_id: &str) -> Result<OperationPlan, OperationError> {
    create_plan_with_targets(preset_id, None)
}

pub fn create_plan_with_targets(
    preset_id: &str,
    selected_target_ids: Option<&[String]>,
) -> Result<OperationPlan, OperationError> {
    let mut definition = definitions()
        .into_iter()
        .find(|preset| preset.id == preset_id)
        .ok_or_else(|| OperationError {
            code: OperationErrorCode::TargetNotFound,
            message: "Preset is no longer available.".into(),
            target_id: Some(preset_id.into()),
            retryable: true,
        })?;
    let current_targets = system_tweak_operation::list_targets();
    let eligible_ids = eligible_target_ids(&definition, &current_targets)?;
    let selected_ids = match selected_target_ids {
        Some(selected) => {
            let requested = selected.iter().collect::<BTreeSet<_>>();
            if requested.len() != selected.len() {
                return Err(validation("preset selection contains duplicate targets"));
            }
            if selected.is_empty() {
                return Err(validation("select at least one preset target"));
            }
            if selected
                .iter()
                .any(|target_id| !eligible_ids.contains(target_id))
            {
                return Err(validation(
                    "preset selection contains a target outside the current eligible preset",
                ));
            }
            eligible_ids
                .iter()
                .filter(|target_id| requested.contains(target_id))
                .cloned()
                .collect::<Vec<_>>()
        }
        None => eligible_ids,
    };
    if selected_ids.is_empty() {
        return Err(validation("preset has no applicable targets"));
    }
    definition.finding = finding_for_count(selected_ids.len());
    definition.target_ids = selected_ids.clone();
    let mut plans = Vec::new();
    for target_id in selected_ids {
        let target = parse_target(&target_id)?;
        let current = find_target(&current_targets, &target)
            .ok_or_else(|| validation("preset target is no longer available"))?;
        plans.push(system_tweak_operation::create_plan(current)?);
    }

    build_plan(&definition, &plans)
}

pub fn create_retry_plan(
    current_targets: &[SystemTweakTarget],
    previous_plan: &OperationPlan,
    unresolved_target_ids: &[String],
) -> Result<OperationPlan, OperationError> {
    previous_plan.validate_relationships()?;
    if !previous_plan.descriptor.id.0.starts_with("preset.") {
        return Err(OperationError::validation(
            "retry requires domain-specific rediscovery for this operation",
        ));
    }
    if unresolved_target_ids.is_empty() {
        return Err(validation("select at least one unresolved preset target"));
    }
    let selected = unresolved_target_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if selected.len() != unresolved_target_ids.len() {
        return Err(validation("preset retry selection contains duplicates"));
    }

    let preset_id = previous_plan
        .descriptor
        .id
        .0
        .strip_prefix("preset.")
        .ok_or_else(|| validation("invalid preset operation plan"))?;
    let mut definition = definitions()
        .into_iter()
        .find(|definition| definition.id == preset_id)
        .ok_or_else(|| validation("preset is no longer available; no retry plan was created"))?;
    let previous_children = child_plans(previous_plan)?;
    let previous_target_ids = previous_plan
        .targets
        .iter()
        .map(|target| target.target_id.clone())
        .collect::<BTreeSet<_>>();
    if previous_target_ids.len() != previous_plan.targets.len()
        || unresolved_target_ids
            .iter()
            .any(|target_id| !previous_target_ids.contains(target_id))
        || previous_plan
            .targets
            .iter()
            .any(|target| !definition.target_ids.contains(&target.target_id))
    {
        return Err(validation(
            "retry targets do not match the previous preset plan",
        ));
    }

    let mut retry_children = Vec::new();
    let mut retry_target_ids = Vec::new();
    for (target, child) in previous_plan.targets.iter().zip(previous_children.iter()) {
        if !selected.contains(&target.target_id) {
            continue;
        }
        retry_children.push(system_tweak_operation::create_retry_plan(
            current_targets,
            child,
            std::slice::from_ref(&target.target_id),
        )?);
        retry_target_ids.push(target.target_id.clone());
    }
    if retry_children.is_empty() {
        return Err(validation("preset retry selection contains no targets"));
    }

    definition.finding = finding_for_count(retry_children.len());
    definition.target_ids = retry_target_ids;
    let mut plan = build_plan(&definition, &retry_children)?;
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

fn build_plan(
    definition: &PresetDefinition,
    plans: &[OperationPlan],
) -> Result<OperationPlan, OperationError> {
    if plans.is_empty() {
        return Err(validation("preset has no applicable targets"));
    }
    let requires_elevation = plans.iter().any(|plan| plan.requires_elevation);
    let requires_restart = plans.iter().any(|plan| plan.requires_restart);
    let risk = plans
        .iter()
        .map(|plan| plan.descriptor.risk)
        .max_by_key(risk_rank)
        .unwrap_or(RiskLevel::Low);
    let exact_recovery = plans.iter().all(|plan| {
        plan.descriptor.recovery == RecoveryLevel::Exact
            && plan.reversibility == Reversibility::FullyReversible
    });
    let targets = plans
        .iter()
        .map(|plan| {
            let target = &plan.targets[0];
            Ok(PlannedTarget {
                target_id: target.target_id.clone(),
                requested_action: serde_json::to_string(plan).map_err(|error| {
                    validation(format!("failed to serialize preset target: {error}"))
                })?,
                description: target.description.clone(),
            })
        })
        .collect::<Result<Vec<_>, OperationError>>()?;
    let native_validation_pending = plans
        .iter()
        .any(|plan| plan.descriptor.proof_status == ProofStatus::PartiallyProven);

    Ok(OperationPlan {
        id: OperationId(uuid::Uuid::new_v4().to_string()),
        descriptor: OperationDescriptor {
            id: OperationId(format!("preset.{}", definition.id)),
            capability_id: "tweaks.system".into(),
            title: format!("Apply {}", definition.name),
            category: OperationCategory::SystemTweak,
            risk,
            privilege: if requires_elevation {
                PrivilegeRequirement::Administrator
            } else {
                PrivilegeRequirement::StandardUser
            },
            reversibility: if exact_recovery {
                Reversibility::FullyReversible
            } else {
                Reversibility::NotReversible
            },
            reboot_policy: if requires_restart {
                RebootPolicy::Required
            } else {
                RebootPolicy::NotRequired
            },
            experimental: false,
            proof_status: if native_validation_pending {
                ProofStatus::PartiallyProven
            } else {
                ProofStatus::Proven
            },
            exposure: ExposureLevel::Core,
            recovery: if exact_recovery {
                RecoveryLevel::Exact
            } else {
                RecoveryLevel::None
            },
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
                description: "The visible constituent tweak is applied and verified.".into(),
            })
            .collect(),
        warnings: {
            let mut warnings = vec![OperationWarning {
                code: "transparent_preset".into(),
                message: "Only the targets listed in this confirmation will execute.".into(),
            }];
            if native_validation_pending {
                warnings.push(OperationWarning {
                    code: "native_validation_pending".into(),
                    message: "One or more constituent targets still require native Windows validation; verify the resulting state on the target machine.".into(),
                });
            }
            warnings.push(recommendation_warning(definition));
            warnings
        },
        targets,
        requires_elevation,
        requires_restart,
        reversibility: if exact_recovery {
            Reversibility::FullyReversible
        } else {
            Reversibility::NotReversible
        },
        parent_operation_id: None,
        retry_of_operation_id: None,
    })
}

pub fn child_plans(plan: &OperationPlan) -> Result<Vec<OperationPlan>, OperationError> {
    if !plan.descriptor.id.0.starts_with("preset.")
        || plan.descriptor.category != OperationCategory::SystemTweak
        || plan.descriptor.experimental
        || plan.targets.is_empty()
    {
        return Err(validation("invalid preset operation plan"));
    }
    plan.targets
        .iter()
        .map(|target| {
            let child: OperationPlan = serde_json::from_str(&target.requested_action)
                .map_err(|error| validation(format!("invalid preset target plan: {error}")))?;
            if child.descriptor.id.0 != "system_tweak.change"
                || child.descriptor.experimental
                || child.targets.len() != 1
                || child.targets[0].target_id != target.target_id
            {
                return Err(validation(
                    "preset target does not match its confirmed child plan",
                ));
            }
            Ok(child)
        })
        .collect()
}

pub fn failed_outcome(plan: &OperationPlan, error: OperationError) -> TargetOutcome {
    TargetOutcome {
        target_id: plan.targets[0].target_id.clone(),
        requested_action: "apply".into(),
        before: None,
        after: None,
        execution: ExecutionStatus::Failed,
        verification: VerificationStatus::Failed,
        error: Some(error),
    }
}

pub fn finish(
    plan: &OperationPlan,
    outcomes: Vec<TargetOutcome>,
) -> Result<PresetExecution, OperationError> {
    if outcomes.len() != plan.targets.len() {
        return Err(validation(
            "preset outcomes do not reconcile with confirmed targets",
        ));
    }
    let result = OperationResult::new(
        plan.id.clone(),
        outcomes,
        plan.requires_restart,
        if plan.descriptor.recovery == RecoveryLevel::Exact {
            UndoAvailability::UndoAvailable
        } else {
            UndoAvailability::NotReversible
        },
    )?;
    let message = if result.failed == 0 {
        format!("All {} preset targets were verified.", result.changed)
    } else {
        format!(
            "{} preset targets changed; {} failed.",
            result.changed, result.failed
        )
    };
    Ok(PresetExecution { result, message })
}

pub fn journal_record(
    plan: &OperationPlan,
    execution: &PresetExecution,
    child_records: &[OperationJournalRecord],
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
        captured_original_states: child_records
            .iter()
            .flat_map(|record| record.captured_original_states.clone())
            .collect(),
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
                    .unwrap_or_else(|| "Constituent tweak verified by read-back.".into()),
            })
            .collect(),
        restart_requirement: plan.descriptor.reboot_policy,
        undo_eligibility: if plan.descriptor.recovery == RecoveryLevel::Exact {
            UndoEligibility::exact(
                "system_tweak_operation::captured_state_is_restored_and_verified",
            )
        } else {
            UndoEligibility::unavailable(
                "Composite preset recovery is unavailable for one or more constituent targets.",
            )
        },
        undo_execution_result: None,
        diagnostic: execution
            .result
            .outcomes
            .iter()
            .find_map(|outcome| outcome.error.clone()),
    }
}

fn parse_target(value: &str) -> Result<PresetTarget, OperationError> {
    let (kind, id) = value
        .split_once(':')
        .ok_or_else(|| validation("invalid preset target identifier"))?;
    let kind = match kind {
        "privacy" => TweakKind::Privacy,
        "performance" => TweakKind::Performance,
        "customization" => TweakKind::Customization,
        _ => return Err(validation("invalid preset target kind")),
    };
    Ok(PresetTarget {
        kind,
        id: id.into(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecommendationEligibility {
    Eligible,
    AlreadyApplied,
    Unsupported,
    Irrelevant,
    Conflicting,
    GuidanceConflict,
}

fn eligible_target_ids(
    definition: &PresetDefinition,
    current_targets: &[SystemTweakTarget],
) -> Result<Vec<String>, OperationError> {
    if definition.provenance.conflict != EvidenceConflict::None
        || definition.confidence != definition.provenance.confidence
    {
        return Ok(Vec::new());
    }

    let parsed_targets = definition
        .target_ids
        .iter()
        .map(|target_id| Ok((target_id.clone(), parse_target(target_id)?)))
        .collect::<Result<Vec<_>, OperationError>>()?;
    let candidates = parsed_targets
        .iter()
        .filter_map(|(target_id, target)| {
            let current = find_target(current_targets, target)?;
            if target_conflicts_with_applied(target, current_targets) {
                return None;
            }
            if recommendation_eligibility(Some(current), definition)
                == RecommendationEligibility::Eligible
            {
                Some((target_id, target, current))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    let mut conflicting_ids = BTreeSet::new();
    for (target_id, target, _) in &candidates {
        for (other_id, other, _) in &candidates {
            if target_id != other_id && targets_conflict(target, other) {
                conflicting_ids.insert((*target_id).clone());
            }
        }
    }

    let mut seen = BTreeSet::new();
    Ok(parsed_targets
        .into_iter()
        .filter_map(|(target_id, target)| {
            if !seen.insert(target_id.clone()) {
                return None;
            }
            let current = find_target(current_targets, &target);
            let eligibility = if conflicting_ids.contains(&target_id)
                || target_conflicts_with_applied(&target, current_targets)
            {
                RecommendationEligibility::Conflicting
            } else {
                recommendation_eligibility(current, definition)
            };
            (eligibility == RecommendationEligibility::Eligible).then_some(target_id)
        })
        .collect())
}

fn recommendation_eligibility(
    current: Option<&SystemTweakTarget>,
    definition: &PresetDefinition,
) -> RecommendationEligibility {
    if definition.provenance.conflict != EvidenceConflict::None {
        return RecommendationEligibility::GuidanceConflict;
    }
    let Some(target) = current else {
        return RecommendationEligibility::Unsupported;
    };
    if target.is_applied {
        return RecommendationEligibility::AlreadyApplied;
    }
    if target.experimental {
        return RecommendationEligibility::Unsupported;
    }
    if !is_relevant_target(target) {
        return RecommendationEligibility::Irrelevant;
    }
    RecommendationEligibility::Eligible
}

fn is_relevant_target(target: &SystemTweakTarget) -> bool {
    // This is a deliberately small recommendation scope. Advanced or newly
    // added tweak IDs are not inferred to be relevant without explicit rules.
    matches!(
        (&target.kind, target.id.as_str()),
        (&TweakKind::Privacy, "disable_telemetry")
            | (&TweakKind::Privacy, "disable_cortana")
            | (&TweakKind::Privacy, "disable_activity_history")
            | (&TweakKind::Privacy, "disable_advertising_id")
            | (&TweakKind::Privacy, "disable_location_tracking")
            | (&TweakKind::Privacy, "disable_web_search")
            | (&TweakKind::Privacy, "disable_feedback")
            | (&TweakKind::Privacy, "disable_tailored_experiences")
            | (&TweakKind::Privacy, "disable_app_launch_tracking")
            | (&TweakKind::Privacy, "disable_delivery_optimization_p2p")
            | (&TweakKind::Privacy, "disable_copilot")
            | (&TweakKind::Privacy, "disable_find_my_device")
            | (&TweakKind::Privacy, "disable_recall")
            | (&TweakKind::Privacy, "disable_click_to_do")
            | (&TweakKind::Customization, "show_known_file_extensions")
            | (&TweakKind::Customization, "enable_end_task")
            | (&TweakKind::Customization, "disable_promotional_content")
            | (&TweakKind::Customization, "disable_lock_screen_spotlight")
            | (&TweakKind::Customization, "hide_widgets")
            | (&TweakKind::Customization, "disable_windows_tips")
            | (&TweakKind::Customization, "enable_long_paths")
            | (&TweakKind::Performance, "adjust_visual_effects")
            | (&TweakKind::Performance, "disable_startup_delay")
            | (&TweakKind::Performance, "enable_game_mode")
            | (&TweakKind::Performance, "disable_game_dvr")
            | (&TweakKind::Performance, "disable_mouse_acceleration")
            | (&TweakKind::Performance, "high_performance_power")
            | (&TweakKind::Performance, "ultimate_performance_power")
    )
}

fn targets_conflict(left: &PresetTarget, right: &PresetTarget) -> bool {
    matches!(
        (&left.kind, left.id.as_str(), &right.kind, right.id.as_str()),
        (
            &TweakKind::Performance,
            "high_performance_power",
            &TweakKind::Performance,
            "ultimate_performance_power"
        ) | (
            &TweakKind::Performance,
            "ultimate_performance_power",
            &TweakKind::Performance,
            "high_performance_power"
        ) | (
            &TweakKind::Performance,
            "disable_hibernation",
            &TweakKind::Performance,
            "disable_fast_startup"
        ) | (
            &TweakKind::Performance,
            "disable_fast_startup",
            &TweakKind::Performance,
            "disable_hibernation"
        )
    )
}

fn target_conflicts_with_applied(
    target: &PresetTarget,
    current_targets: &[SystemTweakTarget],
) -> bool {
    current_targets.iter().any(|current| {
        if !current.is_applied {
            return false;
        }
        targets_conflict(
            target,
            &PresetTarget {
                kind: current.kind.clone(),
                id: current.id.clone(),
            },
        )
    })
}

fn find_target<'a>(
    current_targets: &'a [SystemTweakTarget],
    target: &PresetTarget,
) -> Option<&'a SystemTweakTarget> {
    current_targets
        .iter()
        .find(|current| current.kind == target.kind && current.id == target.id)
}

fn provenance(rationale: &str, tradeoffs: &[&str]) -> EvidenceMetadata {
    EvidenceMetadata {
        source_type: EvidenceSourceType::WinOptimizerDeterministicRule,
        source_name: "WinOptimizer local system-tweak state".into(),
        source_version: Some("system-tweak-rules-1".into()),
        scope: "This Windows desktop machine".into(),
        matching_rule: EvidenceMatchingRule::DeterministicRule,
        confidence: EvidenceConfidence::Unknown,
        rationale: rationale.into(),
        tradeoffs: tradeoffs.iter().map(|value| (*value).into()).collect(),
        conflict: EvidenceConflict::None,
        last_bundled_update: None,
    }
}

fn finding_for_count(count: usize) -> String {
    format!(
        "{} source-reviewed constituent {} not currently applied on this machine.",
        count,
        if count == 1 {
            "setting is"
        } else {
            "settings are"
        }
    )
}

fn recommendation_warning(definition: &PresetDefinition) -> OperationWarning {
    OperationWarning {
        code: "recommendation_evidence".into(),
        message: format!(
            "Finding: {} Mechanism: {} Provenance: {} ({}). Confidence: unknown; no machine-specific benefit has been measured.",
            definition.finding,
            definition.mechanism,
            definition.provenance.source_name,
            definition.provenance.scope,
        ),
    }
}

fn risk_rank(risk: &RiskLevel) -> u8 {
    match risk {
        RiskLevel::None => 0,
        RiskLevel::Low => 0,
        RiskLevel::Medium => 1,
        RiskLevel::High => 2,
        RiskLevel::Critical => 3,
    }
}

fn validation(message: impl Into<String>) -> OperationError {
    OperationError::validation(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn child_plan(experimental: bool) -> OperationPlan {
        child_plan_with("fixture", experimental)
    }

    fn child_plan_with(id: &str, experimental: bool) -> OperationPlan {
        let target = system_tweak_operation::SystemTweakTarget {
            kind: TweakKind::Privacy,
            id: id.into(),
            name: "Fixture".into(),
            description: "Fixture".into(),
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
        };
        system_tweak_operation::create_plan(&target).unwrap()
    }

    fn preset_with(child: &OperationPlan) -> OperationPlan {
        preset_with_id(child, "preset.fixture")
    }

    fn preset_with_id(child: &OperationPlan, preset_id: &str) -> OperationPlan {
        OperationPlan {
            id: OperationId("preset-plan".into()),
            descriptor: OperationDescriptor {
                id: OperationId(preset_id.into()),
                capability_id: "test.preset_fixture".into(),
                title: "Fixture".into(),
                category: OperationCategory::SystemTweak,
                risk: RiskLevel::None,
                privilege: PrivilegeRequirement::StandardUser,
                reversibility: Reversibility::FullyReversible,
                reboot_policy: RebootPolicy::NotRequired,
                experimental: false,
                proof_status: ProofStatus::Proven,
                exposure: ExposureLevel::Internal,
                recovery: RecoveryLevel::None,
                restore_point_policy: crate::core::operation::RestorePointPolicy::NotNeeded,
                mutates_system: false,
                requires_confirmation: false,
                requires_revalidation: false,
                requires_verification: false,
                requires_journal: false,
            },
            targets: vec![PlannedTarget {
                target_id: child.targets[0].target_id.clone(),
                requested_action: serde_json::to_string(child).unwrap(),
                description: "fixture".into(),
            }],
            warnings: vec![],
            expected_effects: vec![],
            requires_elevation: false,
            requires_restart: false,
            reversibility: Reversibility::FullyReversible,
            parent_operation_id: None,
            retry_of_operation_id: None,
        }
    }

    #[test]
    fn child_plans_are_visible_and_exact() {
        let child = child_plan(false);
        assert_eq!(child_plans(&preset_with(&child)).unwrap(), vec![child]);
    }

    #[test]
    fn experimental_child_is_rejected() {
        let child = child_plan(true);
        assert_eq!(
            child_plans(&preset_with(&child)).unwrap_err().code,
            OperationErrorCode::ValidationFailed
        );
    }

    #[test]
    fn retry_rebuilds_selected_child_and_links_the_previous_operation() {
        let child = child_plan_with("disable_telemetry", false);
        let previous = preset_with_id(&child, "preset.privacy_essentials");
        let mut current = fixture_target(TweakKind::Privacy, "disable_telemetry", false, false);
        current.name = "Current Fixture".into();

        let retry = create_retry_plan(
            std::slice::from_ref(&current),
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
        let retry_child = child_plans(&retry).unwrap().pop().unwrap();
        assert!(system_tweak_operation::confirmed_target(&retry_child, &current).unwrap());
    }

    #[test]
    fn retry_rejects_missing_or_satisfied_constituents() {
        let child = child_plan_with("disable_telemetry", false);
        let previous = preset_with_id(&child, "preset.privacy_essentials");
        let target_id = previous.targets[0].target_id.clone();

        assert_eq!(
            create_retry_plan(&[], &previous, std::slice::from_ref(&target_id))
                .unwrap_err()
                .code,
            OperationErrorCode::ValidationFailed
        );

        let satisfied = fixture_target(TweakKind::Privacy, "disable_telemetry", true, false);
        let error = create_retry_plan(
            std::slice::from_ref(&satisfied),
            &previous,
            std::slice::from_ref(&target_id),
        )
        .unwrap_err();
        assert_eq!(error.code, OperationErrorCode::ValidationFailed);
        assert!(error.message.contains("no retry plan"));
    }

    #[test]
    fn retry_fails_closed_for_an_unknown_preset() {
        let child = child_plan(false);
        let previous = preset_with(&child);
        let current = fixture_target(TweakKind::Privacy, "fixture", false, false);
        let error = create_retry_plan(
            std::slice::from_ref(&current),
            &previous,
            std::slice::from_ref(&previous.targets[0].target_id),
        )
        .unwrap_err();

        assert_eq!(error.code, OperationErrorCode::ValidationFailed);
        assert!(error.message.contains("no longer available"));
    }

    fn fixture_target(
        kind: TweakKind,
        id: &str,
        is_applied: bool,
        experimental: bool,
    ) -> SystemTweakTarget {
        SystemTweakTarget {
            kind,
            id: id.into(),
            name: id.into(),
            description: "Fixture".into(),
            risk: RiskLevel::Low,
            is_applied,
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
    fn hibernation_and_fast_startup_are_explicitly_conflicting_targets() {
        assert!(targets_conflict(
            &PresetTarget {
                kind: TweakKind::Performance,
                id: "disable_hibernation".into(),
            },
            &PresetTarget {
                kind: TweakKind::Performance,
                id: "disable_fast_startup".into(),
            },
        ));
    }

    #[test]
    fn machine_aware_filter_fails_closed_for_state_and_conflicts() {
        let mut definition = definitions()
            .into_iter()
            .find(|definition| definition.id == "privacy_essentials")
            .unwrap();
        definition.target_ids = vec![
            "privacy:disable_telemetry".into(),
            "privacy:disable_cortana".into(),
            "privacy:not_detected".into(),
            "performance:disable_timer_coalescing".into(),
            "performance:high_performance_power".into(),
            "performance:ultimate_performance_power".into(),
            "performance:processor_scheduling".into(),
        ];
        let current_targets = vec![
            fixture_target(TweakKind::Privacy, "disable_telemetry", true, false),
            fixture_target(TweakKind::Privacy, "disable_cortana", false, true),
            fixture_target(
                TweakKind::Performance,
                "disable_timer_coalescing",
                false,
                false,
            ),
            fixture_target(
                TweakKind::Performance,
                "high_performance_power",
                true,
                false,
            ),
            fixture_target(
                TweakKind::Performance,
                "ultimate_performance_power",
                false,
                false,
            ),
            fixture_target(TweakKind::Performance, "processor_scheduling", false, false),
        ];

        assert!(eligible_target_ids(&definition, &current_targets)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn unresolved_guidance_conflict_and_unmeasured_benefit_fail_closed() {
        let mut definition = definitions().into_iter().next().unwrap();
        assert_eq!(definition.confidence, EvidenceConfidence::Unknown);
        assert_eq!(
            definition.provenance.matching_rule,
            EvidenceMatchingRule::DeterministicRule
        );
        assert!(!definition.mechanism.is_empty());
        assert!(!definition.provenance.source_name.is_empty());

        definition.provenance.conflict = EvidenceConflict::Conflicting;
        let current_targets = vec![fixture_target(
            TweakKind::Privacy,
            "disable_telemetry",
            false,
            false,
        )];
        assert!(eligible_target_ids(&definition, &current_targets)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn composite_preset_does_not_advertise_generic_undo() {
        let child = child_plan(false);
        let plan = preset_with(&child);
        let outcome = TargetOutcome {
            target_id: plan.targets[0].target_id.clone(),
            requested_action: "apply".into(),
            before: None,
            after: None,
            execution: ExecutionStatus::Changed,
            verification: VerificationStatus::Verified,
            error: None,
        };

        let execution = finish(&plan, vec![outcome]).unwrap();
        assert_eq!(execution.result.undo, UndoAvailability::NotReversible);
    }

    #[test]
    fn stable_composite_preset_plan_claims_exact_recovery() {
        let child = child_plan(false);
        let definition = definitions().into_iter().next().unwrap();
        let plan = build_plan(&definition, std::slice::from_ref(&child)).unwrap();

        assert_eq!(
            plan.descriptor.reversibility,
            Reversibility::FullyReversible
        );
        assert_eq!(plan.descriptor.recovery, RecoveryLevel::Exact);
        assert_eq!(plan.reversibility, Reversibility::FullyReversible);
    }
}
