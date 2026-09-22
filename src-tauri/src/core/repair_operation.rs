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
use crate::repair::{
    self,
    troubleshoot::{SPOOLER_POSTCONDITION_TOKEN, WINDOWS_UPDATE_POSTCONDITION_TOKEN},
    RepairAction, RepairExecutionLog, RepairProcessTermination, RepairTreeContainment,
    RepairVerificationScope,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepairTarget {
    #[serde(flatten)]
    pub action: RepairAction,
    pub experimental: bool,
    pub execution_available: bool,
    pub verification_method: String,
    pub verification_scope: RepairVerificationScope,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepairGuide {
    pub id: String,
    pub title: String,
    pub symptom: String,
    pub description: String,
    pub evidence_action_ids: Vec<String>,
    pub related_action_ids: Vec<String>,
    pub limitations: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RepairTargetSnapshot {
    action: RepairAction,
    verification_method: String,
    verification_scope: RepairVerificationScope,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepairExecution {
    pub result: OperationResult,
    pub log: RepairExecutionLog,
    pub verification_message: String,
}

pub fn guides() -> Vec<RepairGuide> {
    vec![
        RepairGuide {
            id: "windows_integrity".into(),
            title: "Windows integrity".into(),
            symptom: "Windows reports corrupted or unreliable system files".into(),
            description:
                "Collect read-only SFC and component-store evidence before considering any repair."
                    .into(),
            evidence_action_ids: vec![
                "sfc_verify".into(),
                "dism_check_health".into(),
                "dism_check_health_api".into(),
            ],
            related_action_ids: vec![
                "sfc_scan".into(),
                "dism_restore_health".into(),
                "dism_restore_health_api".into(),
            ],
            limitations: "Evidence does not prove that a repair will succeed; repair actions remain manual and guarded until exact post-conditions exist.".into(),
        },
        RepairGuide {
            id: "windows_update".into(),
            title: "Windows Update".into(),
            symptom: "Windows Update is not starting or appears stuck".into(),
            description:
                "Check the Windows Update service identity and state without changing services."
                    .into(),
            evidence_action_ids: vec!["windows_update_status".into()],
            related_action_ids: vec!["rebuild_windows_update".into()],
            limitations: "The service check does not diagnose every Windows Update failure and does not rebuild or reset its cache.".into(),
        },
        RepairGuide {
            id: "networking".into(),
            title: "Networking".into(),
            symptom: "Network connectivity or local configuration looks incorrect".into(),
            description:
                "Collect read-only adapter, gateway, and DNS configuration evidence.".into(),
            evidence_action_ids: vec!["network_status".into()],
            related_action_ids: vec![
                "dns_flush".into(),
                "dns_flush_api".into(),
                "winsock_reset".into(),
                "tcpip_reset".into(),
            ],
            limitations: "The evidence describes local configuration only; it does not test reachability or automatically flush, reset, or otherwise change networking.".into(),
        },
        RepairGuide {
            id: "printing".into(),
            title: "Printing".into(),
            symptom: "Print jobs are not completing or the Print Spooler may be unavailable".into(),
            description: "Collect the Print Spooler service identity and state without changing it."
                .into(),
            evidence_action_ids: vec!["printing_status".into()],
            related_action_ids: vec!["recover_print_spooler".into()],
            limitations: "The service check does not inspect printer drivers, queues, ports, or perform spooler recovery.".into(),
        },
        RepairGuide {
            id: "defender_status".into(),
            title: "Defender status".into(),
            symptom: "You need to review Microsoft Defender protection status".into(),
            description: "Collect named Defender protection and signature fields for review."
                .into(),
            evidence_action_ids: vec!["defender_status".into()],
            related_action_ids: vec!["defender_update".into(), "defender_quick_scan".into()],
            limitations: "The collected fields are a point-in-time status check and do not replace Windows Security or an incident investigation.".into(),
        },
        RepairGuide {
            id: "microsoft_store_appx".into(),
            title: "Microsoft Store / AppX".into(),
            symptom: "Microsoft Store or its AppX registration appears unavailable".into(),
            description:
                "Check the current user's Microsoft Store package registration without changing AppX state."
                    .into(),
            evidence_action_ids: vec!["appx_status".into()],
            related_action_ids: vec![],
            limitations: "The evidence covers only the current user's package registration; it does not repair, reinstall, provision, or launch Microsoft Store.".into(),
        },
        RepairGuide {
            id: "search_indexing".into(),
            title: "Search / indexing".into(),
            symptom: "Windows Search is unavailable or indexing appears stalled".into(),
            description: "Collect the Windows Search service identity and state without changing indexing configuration."
                .into(),
            evidence_action_ids: vec!["search_indexing_status".into()],
            related_action_ids: vec![],
            limitations: "The service check does not inspect index contents, indexed locations, catalog health, or search-client behavior.".into(),
        },
    ]
}

pub fn targets() -> Vec<RepairTarget> {
    repair::list_actions()
        .into_iter()
        .map(|action| {
            let verification_method = repair::verification_method(&action.id);
            let verification_scope = repair::verification_scope(&action.id);
            RepairTarget {
                experimental: verification_method.is_none() || verification_scope.is_none(),
                execution_available: verification_method.is_some() && verification_scope.is_some(),
                verification_method: verification_method
                    .unwrap_or("No exact post-condition is implemented.")
                    .into(),
                verification_scope: verification_scope
                    .unwrap_or(RepairVerificationScope::UserSymptom),
                action,
            }
        })
        .collect()
}

pub fn find_target(id: &str) -> Result<RepairTarget, OperationError> {
    targets()
        .into_iter()
        .find(|target| target.action.id == id)
        .ok_or_else(|| OperationError {
            code: OperationErrorCode::TargetNotFound,
            message: "Repair action is no longer available.".into(),
            target_id: Some(id.into()),
            retryable: true,
        })
}

pub fn create_plan(target: &RepairTarget) -> Result<OperationPlan, OperationError> {
    let canonical_target = find_target(&target.action.id)?;
    if canonical_target != target.clone() {
        return Err(OperationError {
            code: OperationErrorCode::TargetChangedSincePreview,
            message: "Repair target is not the current canonical action.".into(),
            target_id: Some(target.action.id.clone()),
            retryable: true,
        });
    }
    if !target.execution_available {
        return Err(OperationError {
            code: OperationErrorCode::TargetProtected,
            message: format!(
                "{} remains Experimental until its execution and verification contract is implemented.",
                target.action.name
            ),
            target_id: Some(target.action.id.clone()),
            retryable: false,
        });
    }
    if target.action.mutates_system {
        return create_maintenance_plan(target);
    }

    create_diagnostic_plan(target)
}

fn create_diagnostic_plan(target: &RepairTarget) -> Result<OperationPlan, OperationError> {
    let snapshot = RepairTargetSnapshot {
        action: target.action.clone(),
        verification_method: target.verification_method.clone(),
        verification_scope: target.verification_scope,
    };
    let requested_action = serde_json::to_string(&snapshot)
        .map_err(|error| validation(format!("failed to serialize repair target: {error}")))?;

    let plan = OperationPlan {
        id: OperationId(uuid::Uuid::new_v4().to_string()),
        descriptor: OperationDescriptor {
            id: OperationId("repair.verified_diagnostic".into()),
            capability_id: "repair.guided".into(),
            title: target.action.name.clone(),
            category: OperationCategory::Repair,
            risk: risk(&target.action.risk),
            privilege: if target.action.requires_admin {
                PrivilegeRequirement::Administrator
            } else {
                PrivilegeRequirement::StandardUser
            },
            reversibility: Reversibility::NotReversible,
            reboot_policy: RebootPolicy::NotRequired,
            experimental: false,
            proof_status: ProofStatus::Proven,
            exposure: ExposureLevel::Supporting,
            recovery: RecoveryLevel::None,
            restore_point_policy: crate::core::operation::RestorePointPolicy::NotNeeded,
            mutates_system: false,
            requires_confirmation: false,
            requires_revalidation: false,
            requires_verification: false,
            requires_journal: false,
        },
        targets: vec![PlannedTarget {
            target_id: target.action.id.clone(),
            requested_action,
            description: target.action.description.clone(),
        }],
        warnings: vec![OperationWarning {
            code: "diagnostic_only".into(),
            message: "This verified repair action is read-only and does not mutate Windows.".into(),
        }],
        expected_effects: vec![ExpectedEffect {
            target_id: target.action.id.clone(),
            description: target.verification_method.clone(),
        }],
        requires_elevation: target.action.requires_admin,
        requires_restart: false,
        reversibility: Reversibility::NotReversible,
        parent_operation_id: None,
        retry_of_operation_id: None,
    };
    plan.descriptor.validate_contract()?;
    plan.validate_relationships()?;
    Ok(plan)
}

fn create_maintenance_plan(target: &RepairTarget) -> Result<OperationPlan, OperationError> {
    let snapshot = RepairTargetSnapshot {
        action: target.action.clone(),
        verification_method: target.verification_method.clone(),
        verification_scope: target.verification_scope,
    };
    let requested_action = serde_json::to_string(&snapshot)
        .map_err(|error| validation(format!("failed to serialize repair target: {error}")))?;
    let restore_point_policy = restore_point_policy(&target.action.id);
    let mut warnings = vec![OperationWarning {
        code: "maintenance_mutation".into(),
        message: "This action changes Windows state. WinOptimizer has no exact automatic undo for this repair.".into(),
    }];
    warnings.push(OperationWarning {
        code: "verification_scope".into(),
        message: format!(
            "Verification scope: {}. A successful command does not prove the user symptom is fixed.",
            verification_scope_name(target.verification_scope)
        ),
    });
    if target.action.restart_required {
        warnings.push(OperationWarning {
            code: "restart_required".into(),
            message: "Restart Windows before relying on the repaired configuration.".into(),
        });
    }
    if restore_point_policy.should_attempt() {
        warnings.push(OperationWarning {
            code: "restore_point".into(),
            message: format!(
                "A Windows restore point is {} before mutation; it is separate from exact WinOptimizer undo.",
                match restore_point_policy {
                    crate::core::operation::RestorePointPolicy::Recommended => "recommended",
                    crate::core::operation::RestorePointPolicy::Optional => "optional",
                    _ => "required when available",
                }
            ),
        });
    }

    let plan = OperationPlan {
        id: OperationId(uuid::Uuid::new_v4().to_string()),
        descriptor: OperationDescriptor {
            id: OperationId("repair.maintenance".into()),
            capability_id: "repair.guided".into(),
            title: target.action.name.clone(),
            category: OperationCategory::Repair,
            risk: risk(&target.action.risk),
            privilege: if target.action.requires_admin {
                PrivilegeRequirement::Administrator
            } else {
                PrivilegeRequirement::StandardUser
            },
            reversibility: Reversibility::NotReversible,
            reboot_policy: if target.action.restart_required {
                RebootPolicy::Required
            } else {
                RebootPolicy::NotRequired
            },
            experimental: false,
            proof_status: ProofStatus::PartiallyProven,
            exposure: ExposureLevel::Advanced,
            recovery: RecoveryLevel::None,
            restore_point_policy,
            mutates_system: true,
            requires_confirmation: true,
            requires_revalidation: true,
            requires_verification: true,
            requires_journal: true,
        },
        targets: vec![PlannedTarget {
            target_id: target.action.id.clone(),
            requested_action,
            description: target.action.description.clone(),
        }],
        warnings,
        expected_effects: vec![ExpectedEffect {
            target_id: target.action.id.clone(),
            description: target.verification_method.clone(),
        }],
        requires_elevation: target.action.requires_admin,
        requires_restart: target.action.restart_required,
        reversibility: Reversibility::NotReversible,
        parent_operation_id: None,
        retry_of_operation_id: None,
    };
    plan.descriptor.validate_contract()?;
    plan.validate_relationships()?;
    Ok(plan)
}

pub fn validate_plan(plan: &OperationPlan) -> Result<(), OperationError> {
    let snapshot = decode_plan(plan)?;
    let current = find_target(&snapshot.action.id)?;
    confirmed_action(plan, &current).map(|_| ())
}

pub fn create_retry_plan(
    current: &RepairTarget,
    previous_plan: &OperationPlan,
    unresolved_target_ids: &[String],
) -> Result<OperationPlan, OperationError> {
    if !matches!(
        previous_plan.descriptor.id.0.as_str(),
        "repair.verified_diagnostic" | "repair.maintenance"
    ) {
        return Err(OperationError::validation(
            "retry requires domain-specific rediscovery for this operation",
        ));
    }
    if unresolved_target_ids.len() != 1 {
        return Err(OperationError::validation(
            "repair retry requires exactly one target",
        ));
    }

    let previous = decode_plan(previous_plan)?;
    if unresolved_target_ids[0] != previous.action.id {
        return Err(OperationError::validation(
            "retry target does not match the previous repair plan",
        ));
    }
    if current.experimental || !current.execution_available {
        return Err(OperationError {
            code: OperationErrorCode::TargetProtected,
            message: "Repair retry requires a current, implemented verification contract.".into(),
            target_id: Some(previous.action.id.clone()),
            retryable: false,
        });
    }
    if repair::verification_method(&current.action.id) != Some(current.verification_method.as_str())
        || repair::verification_scope(&current.action.id) != Some(current.verification_scope)
    {
        return Err(OperationError::validation(
            "repair retry requires an implemented exact verification contract; no retry plan was created",
        ));
    }
    if previous.action != current.action
        || previous.verification_method != current.verification_method
        || previous.verification_scope != current.verification_scope
    {
        return Err(OperationError::validation(
            "repair action or verification evidence changed; no retry plan was created",
        ));
    }

    if current.action.mutates_system && !maintenance_retry_safe(&current.action.id) {
        return Err(OperationError {
            code: OperationErrorCode::TargetProtected,
            message: "This maintenance action is not safe to replay automatically; rediscover the current state and review a new plan.".into(),
            target_id: Some(previous.action.id.clone()),
            retryable: false,
        });
    }

    let mut plan = create_plan(current)?;
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
    current: &RepairTarget,
) -> Result<RepairAction, OperationError> {
    let snapshot = decode_plan(plan)?;
    if find_target(&current.action.id)? != current.clone() {
        return Err(OperationError {
            code: OperationErrorCode::TargetChangedSincePreview,
            message: "Repair target is not the current canonical diagnostic.".into(),
            target_id: Some(current.action.id.clone()),
            retryable: true,
        });
    }
    if snapshot.action != current.action
        || snapshot.verification_method != current.verification_method
        || !current.execution_available
    {
        return Err(OperationError {
            code: OperationErrorCode::TargetChangedSincePreview,
            message: "Repair action changed after confirmation; review it again.".into(),
            target_id: Some(current.action.id.clone()),
            retryable: true,
        });
    }
    Ok(snapshot.action)
}

pub fn finish(
    plan: &OperationPlan,
    log: RepairExecutionLog,
) -> Result<RepairExecution, OperationError> {
    let snapshot = decode_plan(plan)?;
    if log.action_id != snapshot.action.id
        || log.command != snapshot.action.command
        || log.args != snapshot.action.args
        || log.requires_admin != snapshot.action.requires_admin
        || log.mutates_system != snapshot.action.mutates_system
        || log.restart_required != snapshot.action.restart_required
    {
        return Err(validation(
            "repair execution log does not match the confirmed action",
        ));
    }
    // Cancellation and timeout are distinct terminal reasons with their own
    // durable error codes. They must never be recorded as ordinary success,
    // and when tree containment was unavailable the message must not imply the
    // whole process tree was terminated.
    let termination = match log.termination {
        RepairProcessTermination::Cancelled => Some((
            OperationErrorCode::OperationCancelled,
            ExecutionStatus::Skipped,
            "Repair command was cancelled before it completed.",
        )),
        RepairProcessTermination::TimedOut => Some((
            OperationErrorCode::OperationTimedOut,
            ExecutionStatus::Failed,
            "Repair command exceeded its execution bound and was terminated.",
        )),
        RepairProcessTermination::Completed | RepairProcessTermination::NonZeroExit => None,
    };
    let (execution_status, verification_status, error, message) = if let Some((
        code,
        execution,
        base,
    )) = termination
    {
        let message = match log.tree_containment {
                RepairTreeContainment::Unavailable => format!(
                    "{base} Process-tree containment was unavailable, so descendant processes may not have been terminated."
                ),
                RepairTreeContainment::Contained | RepairTreeContainment::NotApplicable => {
                    base.to_string()
                }
            };
        (
            execution,
            VerificationStatus::Failed,
            Some(OperationError {
                code,
                message: message.clone(),
                target_id: Some(snapshot.action.id.clone()),
                retryable: true,
            }),
            message,
        )
    } else {
        let verification =
            verify_log_with_scope(&snapshot.action.id, snapshot.verification_scope, &log);
        match verification {
            Ok(message) => (
                if snapshot.action.mutates_system {
                    ExecutionStatus::Changed
                } else {
                    ExecutionStatus::Unchanged
                },
                VerificationStatus::Verified,
                None,
                message,
            ),
            Err(error) => (
                if log.exit_code == Some(0) {
                    if snapshot.action.mutates_system {
                        ExecutionStatus::Changed
                    } else {
                        ExecutionStatus::Unchanged
                    }
                } else {
                    ExecutionStatus::Failed
                },
                VerificationStatus::Failed,
                Some(error.clone()),
                error.message,
            ),
        }
    };
    let outcome = TargetOutcome {
        target_id: snapshot.action.id.clone(),
        requested_action: if snapshot.action.mutates_system {
            "run_repair_maintenance".into()
        } else {
            "run_verified_diagnostic".into()
        },
        before: None,
        after: None,
        execution: execution_status,
        verification: verification_status,
        error,
    };
    let result = OperationResult::new(
        plan.id.clone(),
        vec![outcome],
        plan.requires_restart,
        UndoAvailability::NotReversible,
    )?;
    Ok(RepairExecution {
        result,
        log,
        verification_message: message,
    })
}

pub fn journal_record(plan: &OperationPlan, execution: &RepairExecution) -> OperationJournalRecord {
    let outcome = &execution.result.outcomes[0];
    OperationJournalRecord {
        journal_id: uuid::Uuid::new_v4().to_string(),
        record_kind: JournalRecordKind::Operation,
        related_record_id: None,
        operation_id: plan.id.clone(),
        operation_schema_version: OPERATION_SCHEMA_VERSION,
        application_version: env!("CARGO_PKG_VERSION").into(),
        timestamp: execution.log.ended_at.clone(),
        terminal_state: execution.result.completion_state(),
        confirmed_plan: plan.clone(),
        requested_targets: plan.targets.clone(),
        captured_original_states: Vec::new(),
        target_outcomes: execution.result.outcomes.clone(),
        verification_results: vec![VerificationRecord {
            target_id: outcome.target_id.clone(),
            status: outcome.verification,
            message: execution.verification_message.clone(),
        }],
        restart_requirement: plan.descriptor.reboot_policy,
        undo_eligibility: UndoEligibility {
            eligible: false,
            reason: if plan.descriptor.mutates_system {
                "This repair has no exact automatic undo; review the recorded verification and restart state.".into()
            } else {
                "Read-only diagnostic; no undo is needed.".into()
            },
            automated_restoration_test: None,
        },
        undo_execution_result: None,
        diagnostic: outcome.error.clone(),
    }
}

fn decode_plan(plan: &OperationPlan) -> Result<RepairTargetSnapshot, OperationError> {
    if plan.descriptor.capability_id != "repair.guided"
        || plan.descriptor.category != OperationCategory::Repair
        || plan.descriptor.title.is_empty()
        || plan.descriptor.experimental
        || plan.descriptor.recovery != RecoveryLevel::None
        || plan.descriptor.reversibility != Reversibility::NotReversible
        || plan.targets.len() != 1
        || plan.expected_effects.len() != 1
        || plan.reversibility != Reversibility::NotReversible
    {
        return Err(validation("invalid repair operation plan"));
    }
    plan.descriptor.validate_contract()?;
    plan.validate_relationships()?;
    let snapshot: RepairTargetSnapshot = serde_json::from_str(&plan.targets[0].requested_action)
        .map_err(|error| validation(format!("invalid repair target snapshot: {error}")))?;
    let maintenance = snapshot.action.mutates_system;
    let expected_method = if maintenance {
        repair::maintenance_verification_method(&snapshot.action.id)
    } else {
        repair::diagnostic_verification_method(&snapshot.action.id)
    };
    let expected_scope = repair::verification_scope(&snapshot.action.id);
    let descriptor_valid = if maintenance {
        plan.descriptor.id.0 == "repair.maintenance"
            && plan.descriptor.proof_status == ProofStatus::PartiallyProven
            && plan.descriptor.exposure == ExposureLevel::Advanced
            && plan.descriptor.mutates_system
            && plan.descriptor.requires_confirmation
            && plan.descriptor.requires_revalidation
            && plan.descriptor.requires_verification
            && plan.descriptor.requires_journal
            && plan.descriptor.reboot_policy
                == if snapshot.action.restart_required {
                    RebootPolicy::Required
                } else {
                    RebootPolicy::NotRequired
                }
            && plan.descriptor.restore_point_policy == restore_point_policy(&snapshot.action.id)
            && plan.requires_restart == snapshot.action.restart_required
    } else {
        plan.descriptor.id.0 == "repair.verified_diagnostic"
            && plan.descriptor.proof_status == ProofStatus::Proven
            && plan.descriptor.exposure == ExposureLevel::Supporting
            && !plan.descriptor.mutates_system
            && !plan.descriptor.requires_confirmation
            && !plan.descriptor.requires_revalidation
            && !plan.descriptor.requires_verification
            && !plan.descriptor.requires_journal
            && plan.descriptor.restore_point_policy
                == crate::core::operation::RestorePointPolicy::NotNeeded
            && plan.descriptor.reboot_policy == RebootPolicy::NotRequired
            && !plan.requires_restart
    };
    if !descriptor_valid
        || snapshot.action.id != plan.targets[0].target_id
        || expected_method != Some(snapshot.verification_method.as_str())
        || expected_scope != Some(snapshot.verification_scope)
        || plan.targets[0].description != snapshot.action.description
        || plan.expected_effects[0].target_id != snapshot.action.id
        || plan.expected_effects[0].description != snapshot.verification_method
        || plan.descriptor.title != snapshot.action.name
        || plan.descriptor.risk != risk(&snapshot.action.risk)
        || plan.descriptor.privilege
            != if snapshot.action.requires_admin {
                PrivilegeRequirement::Administrator
            } else {
                PrivilegeRequirement::StandardUser
            }
        || plan.requires_elevation != snapshot.action.requires_admin
        || plan.requires_restart != snapshot.action.restart_required
    {
        return Err(validation(
            "repair target does not match the confirmed plan",
        ));
    }
    Ok(snapshot)
}

#[cfg(test)]
fn verify_log(action_id: &str, log: &RepairExecutionLog) -> Result<String, OperationError> {
    let scope =
        repair::verification_scope(action_id).unwrap_or(RepairVerificationScope::UserSymptom);
    verify_log_with_scope(action_id, scope, log)
}

fn verify_log_with_scope(
    action_id: &str,
    scope: RepairVerificationScope,
    log: &RepairExecutionLog,
) -> Result<String, OperationError> {
    if log.exit_code != Some(0) {
        return Err(OperationError {
            code: OperationErrorCode::ExecutionFailed,
            message: format!(
                "Repair command failed with exit code {:?}: {}",
                log.exit_code, log.stderr
            ),
            target_id: Some(action_id.into()),
            retryable: true,
        });
    }
    let output = format!("{}\n{}", log.stdout, log.stderr).to_ascii_lowercase();
    let matched = match scope {
        RepairVerificationScope::HealthPostcondition if action_id == "sfc_scan" => {
            // `sfc /verifyonly` output is localized, so the structured
            // follow-up exit code is the locale-independent postcondition.
            // This proves the verify pass completed with exit code 0; Windows
            // does not document SFC exit codes, so it is not treated as proof
            // that every protected file was repaired.
            output.contains("verificationexitcode=some(0)")
        }
        RepairVerificationScope::HealthPostcondition if action_id == "sfc_verify" => {
            // The read-only action *is* `sfc /verifyonly`. Its process exit
            // status is the only locale-independent evidence available, so the
            // claim is limited to "verification completed and produced output".
            !log.stdout.trim().is_empty() || !log.stderr.trim().is_empty()
        }
        RepairVerificationScope::HealthPostcondition
            if matches!(action_id, "dism_restore_health" | "dism_restore_health_api") =>
        {
            output.contains("verificationcomponentstorehealth=healthy")
        }
        RepairVerificationScope::StatePostcondition if action_id == "recover_print_spooler" => {
            output.contains(SPOOLER_POSTCONDITION_TOKEN)
        }
        RepairVerificationScope::StatePostcondition if action_id == "rebuild_windows_update" => {
            output.contains(WINDOWS_UPDATE_POSTCONDITION_TOKEN)
        }
        RepairVerificationScope::ExecutionAcknowledgement => {
            output.contains("verificationscope=execution_acknowledgement")
        }
        _ => match action_id {
            "dism_check_health" => {
                output.contains("component store")
                    || output.contains("image is repairable")
                    || output.contains("no component store corruption")
            }
            "windows_update_status" => output.contains("service_name") && output.contains("state"),
            "network_status" => {
                output.contains("networkconfiguration=missing")
                    || (output.contains("networkconfiguration=present")
                        && output.contains("interfacealias=")
                        && output.contains("ipv4defaultgateway=")
                        && output.contains("dnsserver="))
            }
            "printing_status" => {
                output.contains("service_name: spooler") && output.contains("state")
            }
            "defender_status" => {
                output.contains("amserviceenabled") && output.contains("antivirusenabled")
            }
            "appx_status" => {
                output.contains("microsoftstorepackage=missing")
                    || (output.contains("microsoftstorepackage=present")
                        && output.contains("packagename=microsoft.windowsstore")
                        && output.contains("packagefullname=")
                        && output.contains("packagestatus="))
            }
            "search_indexing_status" => {
                output.contains("service_name: wsearch") && output.contains("state")
            }
            "dism_check_health_api" => output.contains("component store health:"),
            _ => false,
        },
    };
    if !matched {
        return Err(OperationError {
            code: OperationErrorCode::VerificationFailed,
            message: if matches!(
                scope,
                RepairVerificationScope::ExecutionAcknowledgement
                    | RepairVerificationScope::StatePostcondition
                    | RepairVerificationScope::HealthPostcondition
            ) {
                "Repair command exited successfully but its expected maintenance verification evidence was missing."
                    .into()
            } else {
                "Repair command exited successfully but its expected diagnostic evidence was missing."
                    .into()
            },
            target_id: Some(action_id.into()),
            retryable: true,
        });
    }
    Ok(repair::verification_method(action_id)
        .unwrap_or("Diagnostic evidence verified.")
        .into())
}

fn verification_scope_name(scope: RepairVerificationScope) -> &'static str {
    match scope {
        RepairVerificationScope::ExecutionAcknowledgement => "execution acknowledgement",
        RepairVerificationScope::StatePostcondition => "state postcondition",
        RepairVerificationScope::HealthPostcondition => "health postcondition",
        RepairVerificationScope::UserSymptom => "user symptom",
    }
}

fn restore_point_policy(action_id: &str) -> crate::core::operation::RestorePointPolicy {
    match action_id {
        "sfc_scan"
        | "dism_restore_health"
        | "dism_restore_health_api"
        | "winsock_reset"
        | "tcpip_reset"
        | "recover_print_spooler"
        | "rebuild_windows_update" => crate::core::operation::RestorePointPolicy::Recommended,
        _ => crate::core::operation::RestorePointPolicy::Optional,
    }
}

fn maintenance_retry_safe(action_id: &str) -> bool {
    matches!(
        action_id,
        "sfc_scan"
            | "dism_restore_health"
            | "dism_restore_health_api"
            | "dns_flush"
            | "dns_flush_api"
            | "defender_update"
            | "defender_quick_scan"
            | "recover_print_spooler"
    )
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

    #[test]
    fn supported_mutating_repairs_have_guarded_plans() {
        let target = find_target("winsock_reset").unwrap();
        assert!(!target.experimental);
        assert!(target.execution_available);
        let plan = create_plan(&target).unwrap();
        assert_eq!(plan.descriptor.id.0, "repair.maintenance");
        assert!(plan.descriptor.mutates_system);
        assert!(plan.descriptor.requires_confirmation);
        assert!(plan.descriptor.requires_revalidation);
        assert!(plan.descriptor.requires_verification);
        assert!(plan.descriptor.requires_journal);
        assert!(plan.descriptor.validate_contract().is_ok());
    }

    #[test]
    fn guided_workflows_use_only_verified_read_only_evidence() {
        for guide in guides() {
            assert!(!guide.evidence_action_ids.is_empty());
            for action_id in guide.evidence_action_ids {
                let target = find_target(&action_id).unwrap();
                assert!(!target.action.mutates_system);
                assert!(target.execution_available);
            }
            for action_id in guide.related_action_ids {
                let target = find_target(&action_id).unwrap();
                if target.action.mutates_system {
                    assert_eq!(
                        target.execution_available,
                        repair::maintenance_verification_method(&action_id).is_some()
                    );
                }
            }
        }
    }

    #[test]
    fn phase_21_guides_have_one_evidence_action_and_existing_related_actions() {
        let networking = guides()
            .into_iter()
            .find(|guide| guide.id == "networking")
            .unwrap();
        assert_eq!(networking.evidence_action_ids, vec!["network_status"]);
        assert_eq!(
            networking.related_action_ids,
            vec!["dns_flush", "dns_flush_api", "winsock_reset", "tcpip_reset"]
        );

        let printing = guides()
            .into_iter()
            .find(|guide| guide.id == "printing")
            .unwrap();
        assert_eq!(printing.evidence_action_ids, vec!["printing_status"]);
        assert_eq!(printing.related_action_ids, vec!["recover_print_spooler"]);

        let appx = guides()
            .into_iter()
            .find(|guide| guide.id == "microsoft_store_appx")
            .unwrap();
        assert_eq!(appx.evidence_action_ids, vec!["appx_status"]);
        assert!(appx.related_action_ids.is_empty());

        let search = guides()
            .into_iter()
            .find(|guide| guide.id == "search_indexing")
            .unwrap();
        assert_eq!(search.evidence_action_ids, vec!["search_indexing_status"]);
        assert!(search.related_action_ids.is_empty());
    }

    #[test]
    fn phase_21_diagnostics_require_exact_evidence_markers() {
        let log = |action_id: &str, stdout: &str| RepairExecutionLog {
            id: "test-log".into(),
            action_id: action_id.into(),
            command: "test".into(),
            args: vec![],
            started_at: "start".into(),
            ended_at: "end".into(),
            exit_code: Some(0),
            stdout: stdout.into(),
            stderr: String::new(),
            requires_admin: false,
            mutates_system: false,
            restart_required: false,
            termination: RepairProcessTermination::Completed,
            tree_containment: RepairTreeContainment::NotApplicable,
        };

        assert!(verify_log(
            "network_status",
            &log(
                "network_status",
                "NetworkConfiguration=Present\nInterfaceAlias=Ethernet\nIPv4DefaultGateway=192.0.2.1\nDNSServer=192.0.2.53"
            )
        )
        .is_ok());
        assert!(verify_log(
            "network_status",
            &log("network_status", "NetworkConfiguration=Present")
        )
        .is_err());

        assert!(verify_log(
            "printing_status",
            &log("printing_status", "SERVICE_NAME: Spooler\nSTATE: 4 RUNNING")
        )
        .is_ok());
        assert!(verify_log(
            "printing_status",
            &log("printing_status", "SERVICE_NAME: Other\nSTATE: 4 RUNNING")
        )
        .is_err());

        assert!(verify_log(
            "appx_status",
            &log(
                "appx_status",
                "MicrosoftStorePackage=Present\nPackageName=Microsoft.WindowsStore\nPackageFullName=Microsoft.WindowsStore_1.0\nPackageStatus=Ok"
            )
        )
        .is_ok());
        assert!(verify_log(
            "appx_status",
            &log("appx_status", "MicrosoftStorePackage=Missing")
        )
        .is_ok());
        assert!(verify_log(
            "appx_status",
            &log("appx_status", "MicrosoftStorePackage=Present")
        )
        .is_err());

        assert!(verify_log(
            "search_indexing_status",
            &log(
                "search_indexing_status",
                "SERVICE_NAME: WSearch\nSTATE: 4 RUNNING"
            )
        )
        .is_ok());
        assert!(verify_log(
            "search_indexing_status",
            &log(
                "search_indexing_status",
                "SERVICE_NAME: Search\nSTATE: 4 RUNNING"
            )
        )
        .is_err());
    }

    #[test]
    fn verified_diagnostic_has_a_read_only_plan() {
        let target = find_target("windows_update_status").unwrap();
        let plan = create_plan(&target).unwrap();

        assert!(!plan.descriptor.experimental);
        assert!(!plan.descriptor.requires_verification);
        assert!(plan.descriptor.validate_contract().is_ok());
        assert_eq!(plan.reversibility, Reversibility::NotReversible);
        assert_eq!(
            confirmed_action(&plan, &target).unwrap().id,
            target.action.id
        );
    }

    #[test]
    fn retry_rebuilds_an_unchanged_read_only_diagnostic_plan() {
        let target = find_target("windows_update_status").unwrap();
        let previous = create_plan(&target).unwrap();

        let retry = create_retry_plan(
            &target,
            &previous,
            std::slice::from_ref(&previous.targets[0].target_id),
        )
        .unwrap();

        assert_ne!(retry.id, previous.id);
        assert_eq!(retry.parent_operation_id, Some(previous.id.clone()));
        assert_eq!(retry.retry_of_operation_id, Some(previous.id));
        assert!(!retry.descriptor.mutates_system);
        assert_eq!(retry.reversibility, Reversibility::NotReversible);
    }

    #[test]
    fn retry_rejects_mutating_repairs_fail_closed() {
        let target = find_target("windows_update_status").unwrap();
        let previous = create_plan(&target).unwrap();
        let mut current = target.clone();
        current.action.mutates_system = true;
        current.execution_available = false;
        current.experimental = true;

        let error = create_retry_plan(
            &current,
            &previous,
            std::slice::from_ref(&previous.targets[0].target_id),
        )
        .unwrap_err();

        assert_eq!(error.code, OperationErrorCode::TargetProtected);
        assert!(!error.retryable);
        assert!(error
            .message
            .contains("current, implemented verification contract"));
    }

    #[test]
    fn retry_rejects_changed_repair_evidence_without_creating_a_plan() {
        let target = find_target("windows_update_status").unwrap();
        let previous = create_plan(&target).unwrap();
        let mut current = target.clone();
        current.verification_method = "unrecognized evidence".into();

        let error = create_retry_plan(
            &current,
            &previous,
            std::slice::from_ref(&previous.targets[0].target_id),
        )
        .unwrap_err();

        assert_eq!(error.code, OperationErrorCode::ValidationFailed);
        assert!(!error.retryable);
        assert!(error.message.contains("exact verification contract"));
    }

    #[test]
    fn zero_exit_without_expected_evidence_fails_verification() {
        let target = find_target("windows_update_status").unwrap();
        let plan = create_plan(&target).unwrap();
        let execution = finish(
            &plan,
            RepairExecutionLog {
                id: "log".into(),
                action_id: target.action.id,
                command: "SCM".into(),
                args: vec!["query".into(), "wuauserv".into()],
                started_at: "start".into(),
                ended_at: "end".into(),
                exit_code: Some(0),
                stdout: "success".into(),
                stderr: String::new(),
                requires_admin: false,
                mutates_system: false,
                restart_required: false,
                termination: RepairProcessTermination::Completed,
                tree_containment: RepairTreeContainment::NotApplicable,
            },
        )
        .unwrap();

        assert_eq!(execution.result.failed, 0);
        assert_eq!(execution.result.unverified, 1);
        assert_eq!(
            execution.result.outcomes[0].verification,
            VerificationStatus::Failed
        );
    }

    #[test]
    fn maintenance_finish_records_verified_restart_required_outcome() {
        let target = find_target("winsock_reset").unwrap();
        let plan = create_plan(&target).unwrap();
        let execution = finish(
            &plan,
            RepairExecutionLog {
                id: "maintenance-log".into(),
                action_id: target.action.id.clone(),
                command: target.action.command.clone(),
                args: target.action.args.clone(),
                started_at: "start".into(),
                ended_at: "end".into(),
                exit_code: Some(0),
                stdout: "VerificationScope=execution_acknowledgement".into(),
                stderr: String::new(),
                requires_admin: target.action.requires_admin,
                mutates_system: true,
                restart_required: target.action.restart_required,
                termination: RepairProcessTermination::Completed,
                tree_containment: RepairTreeContainment::NotApplicable,
            },
        )
        .unwrap();

        assert_eq!(execution.result.changed, 1);
        assert_eq!(execution.result.verified, 1);
        assert!(execution.result.reboot_required);
        assert_eq!(
            execution.result.outcomes[0].requested_action,
            "run_repair_maintenance"
        );
    }

    #[test]
    fn forged_mutating_descriptor_is_rejected_before_dispatch() {
        let target = find_target("windows_update_status").unwrap();
        let mut plan = create_plan(&target).unwrap();
        plan.descriptor.mutates_system = true;
        plan.descriptor.requires_confirmation = true;
        plan.descriptor.requires_revalidation = true;
        plan.descriptor.requires_verification = true;
        plan.descriptor.requires_journal = true;

        let error = validate_plan(&plan).unwrap_err();

        assert_eq!(error.code, OperationErrorCode::ValidationFailed);
        assert!(error.message.contains("does not match the confirmed plan"));
    }

    #[test]
    fn scriptless_postcondition_tokens_verify_end_to_end() {
        use crate::repair::troubleshoot::{
            SPOOLER_POSTCONDITION_TOKEN, WINDOWS_UPDATE_POSTCONDITION_TOKEN,
        };

        let log = |action_id: &str, stdout: &str| RepairExecutionLog {
            id: "test-log".into(),
            action_id: action_id.into(),
            command: "test".into(),
            args: vec![],
            started_at: "start".into(),
            ended_at: "end".into(),
            exit_code: Some(0),
            stdout: stdout.into(),
            stderr: String::new(),
            requires_admin: true,
            mutates_system: true,
            restart_required: false,
            termination: RepairProcessTermination::Completed,
            tree_containment: RepairTreeContainment::NotApplicable,
        };

        // Real tokens emitted by troubleshoot.rs must verify.
        assert!(verify_log(
            "recover_print_spooler",
            &log(
                "recover_print_spooler",
                &format!(
                    "Spooler started successfully.\n{}",
                    SPOOLER_POSTCONDITION_TOKEN
                )
            )
        )
        .is_ok());
        assert!(verify_log(
            "rebuild_windows_update",
            &log(
                "rebuild_windows_update",
                &format!("catroot2 renamed.\n{}", WINDOWS_UPDATE_POSTCONDITION_TOKEN)
            )
        )
        .is_ok());

        // Malformed / unrelated output must not verify.
        assert!(verify_log(
            "recover_print_spooler",
            &log("recover_print_spooler", "Spooler started successfully.")
        )
        .is_err());
        assert!(verify_log(
            "rebuild_windows_update",
            &log("rebuild_windows_update", "Postcondition=SomethingElse")
        )
        .is_err());
        // Legacy CamelCase token without underscores must not verify.
        assert!(verify_log(
            "recover_print_spooler",
            &log(
                "recover_print_spooler",
                "Postcondition=SpoolerStateRestored"
            )
        )
        .is_err());
    }

    #[test]
    fn production_repair_catalogue_does_not_overclaim_repair_results() {
        // Positive completion claims only; the truthful limitation wording
        // ("does not prove ...") is allowed and is asserted separately below.
        let overclaims = [
            "repaired successfully",
            "repairs protected",
            "integrity restored",
            "successfully repaired",
            "corruption fixed",
            "component store repair completed",
        ];
        for action in repair::list_actions() {
            let haystack = format!("{} {}", action.name, action.description).to_ascii_lowercase();
            for phrase in overclaims {
                assert!(
                    !haystack.contains(phrase),
                    "repair action {} overclaims '{}' in: {}",
                    action.id,
                    phrase,
                    haystack
                );
            }
        }
        let sfc_scan = repair::list_actions()
            .into_iter()
            .find(|action| action.id == "sfc_scan")
            .unwrap();
        assert!(sfc_scan
            .description
            .to_ascii_lowercase()
            .contains("does not prove"));
        let restore = repair::list_actions()
            .into_iter()
            .find(|action| action.id == "dism_restore_health")
            .unwrap();
        assert!(!restore.name.to_ascii_lowercase().contains("repair"));
    }

    #[test]
    fn cancelled_repair_is_recorded_as_cancelled_not_success() {
        use crate::core::operation::OperationState;
        let target = find_target("sfc_scan").unwrap();
        let plan = create_plan(&target).unwrap();
        let mut log = emitted_maintenance_log("sfc_scan", Some(0), |_, _| {
            Ok((Some(0), "ok".into(), String::new()))
        });
        log.exit_code = None;
        log.termination = RepairProcessTermination::Cancelled;
        log.tree_containment = RepairTreeContainment::Contained;

        let execution = finish(&plan, log).unwrap();
        let outcome = &execution.result.outcomes[0];
        assert_eq!(outcome.execution, ExecutionStatus::Skipped);
        assert_eq!(outcome.verification, VerificationStatus::Failed);
        assert_eq!(
            outcome.error.as_ref().unwrap().code,
            OperationErrorCode::OperationCancelled
        );
        assert_eq!(
            execution.result.completion_state(),
            OperationState::Cancelled
        );
    }

    #[test]
    fn timed_out_repair_records_timeout_and_containment_truth() {
        use crate::core::operation::OperationState;
        let target = find_target("sfc_scan").unwrap();
        let plan = create_plan(&target).unwrap();
        let mut log = emitted_maintenance_log("sfc_scan", Some(0), |_, _| {
            Ok((Some(0), "ok".into(), String::new()))
        });
        log.exit_code = None;
        log.termination = RepairProcessTermination::TimedOut;
        log.tree_containment = RepairTreeContainment::Unavailable;

        let execution = finish(&plan, log).unwrap();
        let outcome = &execution.result.outcomes[0];
        assert_eq!(outcome.execution, ExecutionStatus::Failed);
        assert_eq!(outcome.verification, VerificationStatus::Failed);
        let error = outcome.error.as_ref().unwrap();
        assert_eq!(error.code, OperationErrorCode::OperationTimedOut);
        assert!(
            error.message.contains("containment was unavailable"),
            "timeout with uncontained tree must not imply full tree termination: {}",
            error.message
        );
        assert_eq!(execution.result.completion_state(), OperationState::Failed);
    }

    #[test]
    fn timed_out_repair_with_contained_tree_does_not_claim_unavailable() {
        let target = find_target("sfc_scan").unwrap();
        let plan = create_plan(&target).unwrap();
        let mut log = emitted_maintenance_log("sfc_scan", Some(0), |_, _| {
            Ok((Some(0), "ok".into(), String::new()))
        });
        log.exit_code = None;
        log.termination = RepairProcessTermination::TimedOut;
        log.tree_containment = RepairTreeContainment::Contained;

        let execution = finish(&plan, log).unwrap();
        let error = execution.result.outcomes[0].error.as_ref().unwrap();
        assert!(!error.message.contains("containment was unavailable"));
    }

    #[test]
    fn sfc_scan_verification_is_locale_independent() {
        let log = |stdout: &str| RepairExecutionLog {
            id: "test-log".into(),
            action_id: "sfc_scan".into(),
            command: "sfc".into(),
            args: vec![],
            started_at: "start".into(),
            ended_at: "end".into(),
            exit_code: Some(0),
            stdout: stdout.into(),
            stderr: String::new(),
            requires_admin: true,
            mutates_system: true,
            restart_required: false,
            termination: RepairProcessTermination::Completed,
            tree_containment: RepairTreeContainment::NotApplicable,
        };

        // A localized `sfc /verifyonly` message must still verify on the
        // structured follow-up exit code alone.
        assert!(verify_log(
            "sfc_scan",
            &log("Der Windows-Ressourcenschutz hat keine Integritätsverletzungen gefunden.\nVerificationExitCode=Some(0)")
        )
        .is_ok());
        // A failed follow-up exit code must not verify, even with the English phrase.
        assert!(verify_log(
            "sfc_scan",
            &log("Windows Resource Protection did not find any integrity violations.\nVerificationExitCode=Some(1)")
        )
        .is_err());
    }

    /// Build the production maintenance log by running the real emitter with a
    /// stubbed verification command runner.
    fn emitted_maintenance_log(
        action_id: &str,
        repair_exit_code: Option<i32>,
        run: impl FnMut(&str, &[&str]) -> std::io::Result<crate::repair::MaintenanceVerificationOutput>,
    ) -> RepairExecutionLog {
        let (stdout, stderr) = crate::repair::append_maintenance_verification_with(
            action_id,
            repair_exit_code,
            "repair command output".into(),
            String::new(),
            run,
        );
        RepairExecutionLog {
            id: "emitted-log".into(),
            action_id: action_id.into(),
            command: "sfc".into(),
            args: vec!["/scannow".into()],
            started_at: "start".into(),
            ended_at: "end".into(),
            exit_code: repair_exit_code,
            stdout,
            stderr,
            requires_admin: true,
            mutates_system: true,
            restart_required: false,
            termination: RepairProcessTermination::Completed,
            tree_containment: RepairTreeContainment::NotApplicable,
        }
    }

    #[test]
    fn sfc_scan_emitter_and_verifier_agree_on_a_completed_verify_pass() {
        let log = emitted_maintenance_log("sfc_scan", Some(0), |_, args| {
            assert_eq!(args, ["/verifyonly"]);
            Ok((Some(0), "lokalisierter SFC-Text".into(), String::new()))
        });

        assert!(verify_log("sfc_scan", &log).is_ok());
    }

    #[test]
    fn sfc_scan_emitter_fails_closed_when_the_repair_command_fails() {
        let log = emitted_maintenance_log("sfc_scan", Some(1), |_, _| {
            panic!("verification must not run when the repair command failed")
        });

        assert!(verify_log("sfc_scan", &log).is_err());
    }

    #[test]
    fn sfc_scan_emitter_fails_when_the_verify_command_cannot_run() {
        let log = emitted_maintenance_log("sfc_scan", Some(0), |_, _| {
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "verify command timed out",
            ))
        });

        assert!(verify_log("sfc_scan", &log).is_err());
    }

    #[test]
    fn sfc_scan_rejects_an_execution_acknowledgement_instead_of_exit_evidence() {
        let mut log = emitted_maintenance_log("sfc_scan", Some(0), |_, _| {
            Ok((None, String::new(), String::new()))
        });
        log.stdout = "repair command output\nVerificationScope=execution_acknowledgement".into();

        assert!(verify_log("sfc_scan", &log).is_err());
    }

    #[test]
    fn sfc_verify_accepts_localized_output_and_rejects_empty_completion() {
        let localized = RepairExecutionLog {
            id: "verify-log".into(),
            action_id: "sfc_verify".into(),
            command: "sfc".into(),
            args: vec!["/verifyonly".into()],
            started_at: "start".into(),
            ended_at: "end".into(),
            exit_code: Some(0),
            stdout: "Der Windows-Ressourcenschutz hat keine Integritätsverletzungen gefunden."
                .into(),
            stderr: String::new(),
            requires_admin: true,
            mutates_system: false,
            restart_required: false,
            termination: RepairProcessTermination::Completed,
            tree_containment: RepairTreeContainment::NotApplicable,
        };
        assert!(verify_log("sfc_verify", &localized).is_ok());

        let empty = RepairExecutionLog {
            stdout: String::new(),
            stderr: String::new(),
            ..localized
        };
        assert!(verify_log("sfc_verify", &empty).is_err());
    }

    #[test]
    fn interrupted_repair_execution_reconciles_to_outcome_unknown_on_restart() {
        let target = find_target("sfc_scan").unwrap();
        let plan = create_plan(&target).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let journal = crate::core::operation_journal::OperationJournal::open_at(
            &directory.path().join("journal.db"),
        )
        .unwrap();
        journal.begin_confirmed_execution(&plan).unwrap();
        let before = journal.list_unresolved_operations().unwrap();
        assert_eq!(before.len(), 1);
        assert_eq!(
            before[0].state,
            crate::core::operation::OperationExecutionState::Executing
        );

        // A crash before the terminal append must not leave the repair active:
        // it becomes outcome-unknown for review instead of executing forever.
        assert_eq!(journal.reconcile_unresolved_operations().unwrap(), 1);
        let after = journal.list_unresolved_operations().unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(
            after[0].state,
            crate::core::operation::OperationExecutionState::OutcomeUnknown
        );
        assert!(after[0]
            .targets
            .iter()
            .all(|target| target.state
                == crate::core::operation::TargetExecutionState::OutcomeUnknown));
    }
}
