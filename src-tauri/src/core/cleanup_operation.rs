use super::operation::{
    ExecutionStatus, ExpectedEffect, ExposureLevel, OperationCategory, OperationDescriptor,
    OperationError, OperationErrorCode, OperationId, OperationPlan, OperationResult,
    OperationState, OperationWarning, PlannedTarget, PrivilegeRequirement, ProofStatus,
    RebootPolicy, RecoveryLevel, Reversibility, RiskLevel, StateSnapshot, TargetExecutionState,
    TargetOutcome, UndoAvailability, VerificationStatus,
};
use super::operation_journal::{
    JournalRecordKind, OperationJournal, OperationJournalRecord, UndoEligibility,
    VerificationRecord, OPERATION_SCHEMA_VERSION,
};
use crate::cleaners::{self, CleanItem};
use crate::core::safe_paths::SafePaths;
use serde::{Deserialize, Serialize};
#[cfg(any(test, debug_assertions))]
use std::cell::Cell;
use std::collections::{BTreeMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

const FREE_SPACE_WARNING: &str = "cleanup_free_space_before";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum CleanupDeletionMode {
    #[default]
    RecycleBin,
    Permanent,
}

#[cfg(any(test, debug_assertions))]
#[derive(Clone, Copy, PartialEq, Eq)]
enum TargetLoopFailure {
    Before(usize),
    After(usize),
}

#[cfg(any(test, debug_assertions))]
thread_local! {
    static TARGET_LOOP_FAILURE: Cell<Option<TargetLoopFailure>> = const { Cell::new(None) };
}

#[cfg(any(test, debug_assertions))]
fn set_target_loop_failure(failure_to_inject: TargetLoopFailure) {
    TARGET_LOOP_FAILURE.with(|failure| failure.set(Some(failure_to_inject)));
}

#[cfg(any(test, debug_assertions))]
#[allow(dead_code)]
pub(crate) fn fail_before_target_n(target_number: usize) {
    assert!(target_number > 0, "target number is one-based");
    set_target_loop_failure(TargetLoopFailure::Before(target_number));
}

#[cfg(any(test, debug_assertions))]
#[allow(dead_code)]
pub(crate) fn fail_after_target_n(target_number: usize) {
    assert!(target_number > 0, "target number is one-based");
    set_target_loop_failure(TargetLoopFailure::After(target_number));
}

#[cfg(any(test, debug_assertions))]
fn take_target_loop_failure(expected: TargetLoopFailure) -> bool {
    TARGET_LOOP_FAILURE.with(|failure| {
        if failure.get() == Some(expected) {
            failure.set(None);
            true
        } else {
            false
        }
    })
}

#[cfg(any(test, debug_assertions))]
fn take_fail_before_target_n(target_number: usize) -> bool {
    take_target_loop_failure(TargetLoopFailure::Before(target_number))
}

#[cfg(any(test, debug_assertions))]
fn take_fail_after_target_n(target_number: usize) -> bool {
    take_target_loop_failure(TargetLoopFailure::After(target_number))
}

#[cfg(any(test, debug_assertions))]
fn target_loop_failure(hook: &str) -> OperationError {
    OperationError {
        code: OperationErrorCode::InternalError,
        message: format!("failure injection: {hook}"),
        target_id: None,
        retryable: true,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupTarget {
    pub target_id: String,
    pub category: String,
    pub resource_id: String,
    pub description: String,
    pub size_bytes: u64,
    pub last_modified: Option<String>,
    pub confidence: String,
    pub risk: RiskLevel,
    pub recommended: bool,
    pub recommendation_reason: String,
    pub accessibility: String,
    pub reversibility: Reversibility,
    pub affected_application: Option<String>,
    pub group_name: String,
    pub cleanup_type: String,
    pub requires_elevation: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<crate::core::file_identity::FileIdentity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupScan {
    pub scan_id: String,
    pub scanned_at: String,
    pub targets: Vec<CleanupTarget>,
    pub estimated_bytes: u64,
    pub free_space_before: Option<u64>,
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CleanupTargetSnapshot {
    category: String,
    path: String,
    size_bytes: u64,
    last_modified: Option<String>,
    #[serde(default)]
    deletion_mode: CleanupDeletionMode,
    #[serde(default)]
    canonical_path: Option<String>,
    #[serde(default)]
    identity: Option<crate::core::file_identity::FileIdentity>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CleanupExecution {
    pub result: OperationResult,
    pub estimated_bytes: u64,
    pub measured_disk_space_delta: Option<i64>,
    pub journal_id: String,
}

pub fn scan(safe_paths: &SafePaths) -> CleanupScan {
    let mut targets = Vec::new();
    let mut failures = Vec::new();

    collect_scan(
        "Temporary files",
        cleaners::temp_files::scan(safe_paths),
        safe_paths,
        &mut targets,
        &mut failures,
    );
    collect_scan(
        "Browser cache",
        cleaners::browser_cache::detect_browsers()
            .and_then(|browsers| cleaners::browser_cache::scan(&browsers)),
        safe_paths,
        &mut targets,
        &mut failures,
    );
    collect_scan(
        "Windows Update cache",
        cleaners::windows_update::scan(),
        safe_paths,
        &mut targets,
        &mut failures,
    );
    collect_scan(
        "System logs",
        cleaners::system_logs::scan(),
        safe_paths,
        &mut targets,
        &mut failures,
    );

    targets.sort_by(|left, right| left.resource_id.cmp(&right.resource_id));
    targets.dedup_by(|left, right| left.target_id == right.target_id);
    let estimated_bytes = targets.iter().map(|target| target.size_bytes).sum();
    let free_space_before = targets
        .first()
        .and_then(|target| available_space(Path::new(&target.resource_id)));

    CleanupScan {
        scan_id: uuid::Uuid::new_v4().to_string(),
        scanned_at: chrono::Utc::now().to_rfc3339(),
        targets,
        estimated_bytes,
        free_space_before,
        failures,
    }
}

/// Scan deterministic fixture cleanup roots without consulting host state.
pub fn scan_fixture(
    data_root: &Path,
    filesystem: &dyn crate::core::filesystem::FileSystemAdapter,
) -> CleanupScan {
    let definitions = [
        ("temp", "Temporary files", "temp", 3),
        ("prefetch", "Optional Prefetch space", "prefetch", 3),
        ("browser-cache", "Browser cache", "browser_cache", 5),
        ("update-cache", "Windows Update cache", "update_cache", 3),
        ("system-logs", "System logs", "logs", 2),
    ];
    let mut targets = Vec::new();
    let mut failures = Vec::new();
    for (directory, description, category, max_depth) in definitions {
        collect_fixture_files(
            &data_root.join("cleanup").join(directory),
            description,
            category,
            filesystem,
            &mut targets,
            &mut failures,
            0,
            max_depth,
        );
    }
    targets.sort_by(|left, right| left.resource_id.cmp(&right.resource_id));
    let estimated_bytes = targets.iter().map(|target| target.size_bytes).sum();
    CleanupScan {
        scan_id: uuid::Uuid::new_v4().to_string(),
        scanned_at: chrono::Utc::now().to_rfc3339(),
        targets,
        estimated_bytes,
        free_space_before: None,
        failures,
    }
}

/// Return legacy cleaner DTOs from the same isolated fixture scanner.
pub fn scan_fixture_items(
    data_root: &Path,
    filesystem: &dyn crate::core::filesystem::FileSystemAdapter,
    category: &str,
) -> Vec<CleanItem> {
    scan_fixture(data_root, filesystem)
        .targets
        .into_iter()
        .filter(|target| target.category == category)
        .map(|target| CleanItem {
            path: target.resource_id,
            size_bytes: target.size_bytes,
            last_accessed: target.last_modified,
            category: target.category,
            description: target.description,
            group_key: target.group_name,
            sub_group: target.cleanup_type,
        })
        .collect()
}
#[allow(clippy::too_many_arguments)]
fn collect_fixture_files(
    directory: &Path,
    description: &str,
    category: &str,
    filesystem: &dyn crate::core::filesystem::FileSystemAdapter,
    targets: &mut Vec<CleanupTarget>,
    failures: &mut Vec<String>,
    depth: usize,
    max_depth: usize,
) {
    let entries = match filesystem.list(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            failures.push(format!("{description}: {error}"));
            return;
        }
    };
    for path in entries {
        let metadata = match filesystem.metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                failures.push(format!(
                    "{description}: failed to inspect {}: {error}",
                    path.display()
                ));
                continue;
            }
        };
        if metadata.is_dir() {
            if depth < max_depth {
                collect_fixture_files(
                    &path,
                    description,
                    category,
                    filesystem,
                    targets,
                    failures,
                    depth + 1,
                    max_depth,
                );
            }
        } else if metadata.is_file() && metadata.len() > 0 {
            let path_string = path.to_string_lossy().into_owned();
            let last_modified = match metadata.modified().ok().map(system_time_key) {
                Some(value) => value,
                None => {
                    failures.push(format!(
                        "{description}: failed to capture modification time for {}",
                        path.display()
                    ));
                    continue;
                }
            };
            let (risk, recommended, recommendation_reason, requires_elevation) =
                category_policy(category);
            let fixture_identity = match filesystem.identity(&path) {
                Ok(value) => value,
                Err(error) => {
                    failures.push(format!(
                        "{description}: failed to capture identity for {}: {error}",
                        path.display()
                    ));
                    continue;
                }
            };
            targets.push(CleanupTarget {
                target_id: normalized_target_id(&path_string),
                category: category.into(),
                resource_id: path_string.clone(),
                description: description.into(),
                size_bytes: metadata.len(),
                last_modified: Some(last_modified),
                confidence: "high".into(),
                risk,
                recommended,
                recommendation_reason: recommendation_reason.into(),
                accessibility: "accessible".into(),
                reversibility: Reversibility::NotReversible,
                affected_application: None,
                group_name: description.into(),
                cleanup_type: "fixture".into(),
                requires_elevation,
                canonical_path: Some(path_string),
                identity: Some(crate::core::file_identity::FileIdentity {
                    volume_serial: fixture_identity.volume_serial,
                    file_reference: fixture_identity.file_reference,
                }),
            });
        }
    }
}

fn collect_scan(
    label: &str,
    result: crate::core::error::Result<Vec<CleanItem>>,
    safe_paths: &SafePaths,
    targets: &mut Vec<CleanupTarget>,
    failures: &mut Vec<String>,
) {
    match result {
        Ok(items) => {
            for item in items {
                // Scan and execution share one authorization model: never
                // advertise a candidate the executor would refuse.
                if !safe_paths.is_safe(Path::new(&item.path)) {
                    continue;
                }
                let target = enrich_target(item);
                if target.last_modified.is_some() {
                    targets.push(target);
                } else {
                    failures.push(format!(
                        "{label}: failed to capture modification time for {}",
                        target.resource_id
                    ));
                }
            }
        }
        Err(error) => failures.push(format!("{label}: {error}")),
    }
}

fn enrich_target(item: CleanItem) -> CleanupTarget {
    let path = Path::new(&item.path);
    let metadata = path.metadata().ok();
    let file_snapshot = crate::core::file_identity::snapshot_for_path(path).ok();
    let canonical_path = path
        .canonicalize()
        .ok()
        .map(|value| value.to_string_lossy().into_owned());
    let identity = file_snapshot.map(|value| value.identity);
    let last_modified = metadata
        .as_ref()
        .and_then(|value| value.modified().ok())
        .map(system_time_key);
    let (risk, recommended, recommendation_reason, requires_elevation) =
        category_policy(&item.category);
    let affected_application = (item.category == "browser_cache").then_some(item.group_key.clone());
    let group_name = item.group_key.clone();
    let cleanup_type = item.sub_group.clone();

    CleanupTarget {
        target_id: normalized_target_id(&item.path),
        category: item.category,
        resource_id: item.path,
        description: item.description,
        size_bytes: item.size_bytes,
        last_modified,
        confidence: "high".into(),
        risk,
        recommended,
        recommendation_reason: recommendation_reason.into(),
        accessibility: if metadata.is_some() {
            "accessible".into()
        } else {
            "unavailable".into()
        },
        reversibility: Reversibility::NotReversible,
        affected_application,
        group_name,
        cleanup_type,
        requires_elevation,
        canonical_path,
        identity,
    }
}

fn category_policy(category: &str) -> (RiskLevel, bool, &'static str, bool) {
    match category {
        "temp" => (
            RiskLevel::Low,
            true,
            "Temporary files are the only clearly safe default.",
            false,
        ),
        "prefetch" => (
            RiskLevel::Medium,
            false,
            "Prefetch cleanup is optional space recovery; no performance improvement is claimed.",
            true,
        ),
        "browser_cache" => (
            RiskLevel::Medium,
            false,
            "May affect signed-in applications; select explicitly.",
            false,
        ),
        "update_cache" => (
            RiskLevel::Medium,
            false,
            "Windows Update data requires explicit selection and elevation.",
            true,
        ),
        "logs" => (
            RiskLevel::Medium,
            false,
            "Logs may be needed for diagnosis; select explicitly.",
            true,
        ),
        _ => (
            RiskLevel::High,
            false,
            "Unknown targets are never selected automatically.",
            true,
        ),
    }
}

pub fn create_plan(
    scan: &CleanupScan,
    selected_target_ids: &[String],
) -> Result<OperationPlan, OperationError> {
    create_plan_with_mode(scan, selected_target_ids, CleanupDeletionMode::RecycleBin)
}

pub fn create_plan_with_mode(
    scan: &CleanupScan,
    selected_target_ids: &[String],
    deletion_mode: CleanupDeletionMode,
) -> Result<OperationPlan, OperationError> {
    if selected_target_ids.is_empty() {
        return Err(validation("select at least one cleanup target"));
    }
    let selected: HashSet<&str> = selected_target_ids.iter().map(String::as_str).collect();
    if selected.len() != selected_target_ids.len() {
        return Err(validation("cleanup selection contains duplicate targets"));
    }

    let targets: Vec<&CleanupTarget> = scan
        .targets
        .iter()
        .filter(|target| selected.contains(target.target_id.as_str()))
        .collect();
    if targets.len() != selected.len() {
        return Err(validation(
            "cleanup selection contains a target outside the current scan",
        ));
    }

    let requires_elevation = targets.iter().any(|target| target.requires_elevation);
    let risk = targets
        .iter()
        .map(|target| target.risk)
        .max_by_key(|risk| match risk {
            RiskLevel::None => 0,
            RiskLevel::Low => 0,
            RiskLevel::Medium => 1,
            RiskLevel::High => 2,
            RiskLevel::Critical => 3,
        })
        .unwrap_or(RiskLevel::Low);
    let estimated_bytes: u64 = targets.iter().map(|target| target.size_bytes).sum();
    let planned_targets = targets
        .iter()
        .map(|target| {
            let snapshot = CleanupTargetSnapshot {
                category: target.category.clone(),
                path: target.resource_id.clone(),
                size_bytes: target.size_bytes,
                last_modified: target.last_modified.clone(),
                deletion_mode,
                canonical_path: target.canonical_path.clone(),
                identity: target.identity.clone(),
            };
            Ok(PlannedTarget {
                target_id: target.target_id.clone(),
                requested_action: serde_json::to_string(&snapshot).map_err(|error| {
                    validation(format!("failed to serialize cleanup target: {error}"))
                })?,
                description: target.description.clone(),
            })
        })
        .collect::<Result<Vec<_>, OperationError>>()?;

    Ok(OperationPlan {
        id: OperationId(uuid::Uuid::new_v4().to_string()),
        descriptor: OperationDescriptor {
            id: OperationId("cleanup.reference_slice".into()),
            capability_id: "cleanup.temp".into(),
            title: "Clean Up selected targets".into(),
            category: OperationCategory::Cleanup,
            risk,
            privilege: if requires_elevation {
                PrivilegeRequirement::Administrator
            } else {
                PrivilegeRequirement::StandardUser
            },
            reversibility: Reversibility::NotReversible,
            reboot_policy: RebootPolicy::NotRequired,
            experimental: false,
            proof_status: ProofStatus::PartiallyProven,
            exposure: ExposureLevel::Core,
            recovery: RecoveryLevel::None,
            restore_point_policy: crate::core::operation::RestorePointPolicy::NotNeeded,
            mutates_system: true,
            requires_confirmation: true,
            requires_revalidation: true,
            requires_verification: true,
            requires_journal: true,
        },
        targets: planned_targets,
        warnings: vec![
            OperationWarning {
                code: FREE_SPACE_WARNING.into(),
                message: scan
                    .free_space_before
                    .map(|bytes| bytes.to_string())
                    .unwrap_or_default(),
            },
            OperationWarning {
                code: "cleanup_estimated_bytes".into(),
                message: estimated_bytes.to_string(),
            },
            OperationWarning {
                code: "cleanup_undo_unavailable".into(),
                message: format!(
                    "Deletion mode: {}. Exact WinOptimizer undo is unavailable.",
                    deletion_mode_label(deletion_mode)
                ),
            },
        ],
        expected_effects: targets
            .iter()
            .map(|target| ExpectedEffect {
                target_id: target.target_id.clone(),
                description: format!(
                    "The selected file is {} according to the confirmed deletion mode.",
                    if deletion_mode == CleanupDeletionMode::RecycleBin {
                        "moved to the Recycle Bin"
                    } else {
                        "permanently deleted"
                    }
                ),
            })
            .collect(),
        requires_elevation,
        requires_restart: false,
        reversibility: Reversibility::NotReversible,
        parent_operation_id: None,
        retry_of_operation_id: None,
    })
}

pub fn create_retry_plan(
    scan: &CleanupScan,
    previous_plan: &OperationPlan,
    unresolved_target_ids: &[String],
) -> Result<OperationPlan, OperationError> {
    if previous_plan.descriptor.id.0 != "cleanup.reference_slice" {
        return Err(validation(
            "retry requires domain-specific rediscovery for this operation",
        ));
    }
    let deletion_mode = plan_deletion_mode(previous_plan)?;
    let mut plan = create_plan_with_mode(scan, unresolved_target_ids, deletion_mode)?;
    plan.parent_operation_id = Some(previous_plan.id.clone());
    plan.retry_of_operation_id = Some(previous_plan.id.clone());
    plan.warnings.push(OperationWarning {
        code: "retry_of_operation".into(),
        message: previous_plan.id.0.clone(),
    });
    Ok(plan)
}

pub fn reconciliation_record(
    plan: &OperationPlan,
    scan: &CleanupScan,
) -> Result<OperationJournalRecord, OperationError> {
    if plan.descriptor.id.0 != "cleanup.reference_slice" || plan.targets.is_empty() {
        return Err(validation("invalid cleanup recovery plan"));
    }

    let target_outcomes = plan
        .targets
        .iter()
        .map(|target| match decode_snapshot(target) {
            Ok(snapshot) => {
                let path = Path::new(&snapshot.path);
                if !path.exists() {
                    Ok(TargetOutcome {
                        target_id: target.target_id.clone(),
                        requested_action: cleanup_requested_action(snapshot.deletion_mode).into(),
                        before: None,
                        after: Some(absent_snapshot()),
                        execution: ExecutionStatus::Unchanged,
                        verification: VerificationStatus::Verified,
                        error: None,
                    })
                } else {
                    let rediscovered = scan
                        .targets
                        .iter()
                        .find(|candidate| candidate.target_id == target.target_id);
                    let mut properties = BTreeMap::new();
                    properties.insert("path".into(), snapshot.path.clone());
                    if let Ok(metadata) = path.metadata() {
                        properties.insert("size_bytes".into(), metadata.len().to_string());
                        properties.insert(
                            "last_modified".into(),
                            metadata
                                .modified()
                                .ok()
                                .map(system_time_key)
                                .unwrap_or_default(),
                        );
                    }
                    properties.insert(
                        "rediscovered_in_scan".into(),
                        rediscovered.is_some().to_string(),
                    );
                    let error = target_error(
                        target,
                        OperationErrorCode::OperationInterrupted,
                        "Cleanup outcome is unknown; the target still exists during recovery inspection.",
                        true,
                    );
                    Ok(TargetOutcome {
                        target_id: target.target_id.clone(),
                        requested_action: cleanup_requested_action(snapshot.deletion_mode).into(),
                        before: None,
                        after: Some(StateSnapshot {
                            summary: "Target still exists during recovery inspection.".into(),
                            properties,
                        }),
                        execution: ExecutionStatus::Skipped,
                        verification: VerificationStatus::Unverified,
                        error: Some(error),
                    })
                }
            }
            Err(error) => Ok(TargetOutcome {
                target_id: target.target_id.clone(),
                requested_action: "cleanup_target".into(),
                before: None,
                after: None,
                execution: ExecutionStatus::Skipped,
                verification: VerificationStatus::Unverified,
                error: Some(target_error(
                    target,
                    OperationErrorCode::OperationInterrupted,
                    format!("Cleanup recovery could not decode the confirmed target: {}", error.message),
                    true,
                )),
            }),
        })
        .collect::<Result<Vec<_>, OperationError>>()?;
    let diagnostic = target_outcomes
        .iter()
        .find_map(|outcome| outcome.error.clone());
    let terminal_state = if diagnostic.is_some() {
        OperationState::Interrupted
    } else {
        OperationState::Succeeded
    };

    Ok(OperationJournalRecord {
        journal_id: uuid::Uuid::new_v4().to_string(),
        record_kind: JournalRecordKind::Operation,
        related_record_id: None,
        operation_id: plan.id.clone(),
        operation_schema_version: OPERATION_SCHEMA_VERSION,
        application_version: env!("CARGO_PKG_VERSION").into(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        terminal_state,
        confirmed_plan: plan.clone(),
        requested_targets: plan.targets.clone(),
        captured_original_states: Vec::new(),
        target_outcomes: target_outcomes.clone(),
        verification_results: target_outcomes
            .iter()
            .map(|outcome| VerificationRecord {
                target_id: outcome.target_id.clone(),
                status: outcome.verification,
                message: outcome
                    .error
                    .as_ref()
                    .map(|error| error.message.clone())
                    .unwrap_or_else(|| {
                        "Recovery inspection verified the target postcondition.".into()
                    }),
            })
            .collect(),
        restart_requirement: plan.descriptor.reboot_policy,
        undo_eligibility: UndoEligibility {
            eligible: false,
            reason: "Recovery reconciliation does not create an undo record.".into(),
            automated_restoration_test: None,
        },
        undo_execution_result: None,
        diagnostic,
    })
}

/// Bind a client-confirmed plan to the exact metadata from the fresh scan
/// performed by the server. Client relabeling cannot change coordination or
/// privilege requirements.
pub fn validate_authoritative_scan(
    plan: &OperationPlan,
    scan: &CleanupScan,
) -> Result<(HashSet<String>, bool), OperationError> {
    plan_deletion_mode(plan)?;
    let fresh = scan
        .targets
        .iter()
        .map(|target| (target.target_id.as_str(), target))
        .collect::<BTreeMap<_, _>>();
    let mut allowed = HashSet::with_capacity(plan.targets.len());
    let mut requires_elevation = false;
    for planned in &plan.targets {
        let snapshot = decode_snapshot(planned)?;
        let target = fresh
            .get(planned.target_id.as_str())
            .ok_or_else(|| validation("Target is outside the current server-side cleanup scan."))?;
        if normalized_target_id(&snapshot.path) != target.target_id
            || snapshot.category != target.category
            || normalized_target_id(&snapshot.path) != normalized_target_id(&target.resource_id)
            || snapshot.size_bytes != target.size_bytes
            || snapshot.last_modified.is_none()
            || target.last_modified.is_none()
            || snapshot.last_modified != target.last_modified
            || snapshot.canonical_path.as_deref().map(normalized_target_id)
                != target.canonical_path.as_deref().map(normalized_target_id)
            || snapshot.identity != target.identity
        {
            return Err(OperationError {
                code: OperationErrorCode::TargetChangedSincePreview,
                message: "cleanup target metadata changed since preview".into(),
                target_id: Some(planned.target_id.clone()),
                retryable: true,
            });
        }
        requires_elevation |= target.requires_elevation;
        allowed.insert(target.target_id.clone());
    }
    if plan.requires_elevation != requires_elevation {
        return Err(validation(
            "cleanup elevation metadata changed since preview",
        ));
    }
    Ok((allowed, requires_elevation))
}

pub fn execute(
    plan: &OperationPlan,
    cancelled: &AtomicBool,
    allowed_target_ids: &HashSet<String>,
    safe_paths: &crate::core::safe_paths::SafePaths,
) -> Result<(CleanupExecution, OperationJournalRecord), OperationError> {
    execute_with_recycle_bin_internal(plan, cancelled, allowed_target_ids, None, safe_paths)
}

pub fn execute_with_recycle_bin(
    plan: &OperationPlan,
    cancelled: &AtomicBool,
    allowed_target_ids: &HashSet<String>,
    safe_paths: &crate::core::safe_paths::SafePaths,
) -> Result<(CleanupExecution, OperationJournalRecord), OperationError> {
    execute_with_recycle_bin_internal(plan, cancelled, allowed_target_ids, None, safe_paths)
}

pub fn execute_with_recycle_bin_journaled_with_filesystem(
    plan: &OperationPlan,
    cancelled: &AtomicBool,
    allowed_target_ids: &HashSet<String>,
    filesystem: &dyn crate::core::filesystem::FileSystemAdapter,
    journal: &OperationJournal,
) -> Result<(CleanupExecution, OperationJournalRecord), OperationError> {
    if plan.descriptor.category != OperationCategory::Cleanup
        || plan.reversibility != Reversibility::NotReversible
    {
        return Err(validation("invalid cleanup operation plan"));
    }
    let deletion_mode = plan_deletion_mode(plan)?;
    let mut outcomes = Vec::with_capacity(plan.targets.len());
    for target in &plan.targets {
        #[cfg(any(test, debug_assertions))]
        if take_fail_before_target_n(outcomes.len() + 1) {
            return Err(target_loop_failure("fail_before_target_n"));
        }
        let payload = serde_json::to_string(target).map_err(|e| OperationError {
            code: OperationErrorCode::SerializationFailed,
            message: e.to_string(),
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
        let outcome = if cancelled.load(Ordering::SeqCst) {
            error_outcome(
                target,
                OperationErrorCode::OperationCancelled,
                "Cleanup was cancelled before this target.",
                ExecutionStatus::Skipped,
            )
        } else if !allowed_target_ids.contains(&target.target_id) {
            error_outcome(
                target,
                OperationErrorCode::TargetProtected,
                "Target is outside the current server-side cleanup scan.",
                ExecutionStatus::Skipped,
            )
        } else {
            let snapshot = decode_snapshot(target)?;
            let path = Path::new(&snapshot.path);
            match filesystem.metadata(path) {
                Ok(_metadata) => {
                    if let Err(error) = validate_fixture_target(target, &snapshot, filesystem) {
                        error_outcome(target, error.code, &error.message, ExecutionStatus::Skipped)
                    } else {
                        let before = StateSnapshot {
                            summary: "File existed at confirmation.".into(),
                            properties: BTreeMap::new(),
                        };
                        match filesystem.delete(
                            path,
                            snapshot.identity.as_ref(),
                            matches!(deletion_mode, CleanupDeletionMode::RecycleBin),
                        ) {
                            Ok(()) if !filesystem.exists(path) => TargetOutcome {
                                target_id: target.target_id.clone(),
                                requested_action: cleanup_requested_action(deletion_mode).into(),
                                before: Some(before),
                                after: Some(absent_snapshot()),
                                execution: ExecutionStatus::Changed,
                                verification: VerificationStatus::Verified,
                                error: None,
                            },
                            Ok(()) => error_outcome(
                                target,
                                OperationErrorCode::VerificationFailed,
                                "Target still exists after cleanup.",
                                ExecutionStatus::Failed,
                            ),
                            Err(e) => error_outcome(
                                target,
                                OperationErrorCode::ExecutionFailed,
                                &format!("Failed to remove target: {e}"),
                                ExecutionStatus::Failed,
                            ),
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => error_outcome(
                    target,
                    OperationErrorCode::TargetNotFound,
                    "snapshot path is missing",
                    ExecutionStatus::Skipped,
                ),
                Err(e) => error_outcome(
                    target,
                    OperationErrorCode::PermissionDenied,
                    &format!("Cannot inspect target: {e}"),
                    ExecutionStatus::Failed,
                ),
            }
        };
        journal.record_target_outcome(&plan.id, &outcome)?;
        outcomes.push(outcome);
        #[cfg(any(test, debug_assertions))]
        if take_fail_after_target_n(outcomes.len()) {
            return Err(target_loop_failure("fail_after_target_n"));
        }
    }
    let result = OperationResult::new(
        plan.id.clone(),
        outcomes,
        false,
        UndoAvailability::NotReversible,
    )?;
    let execution = CleanupExecution {
        estimated_bytes: warning_u64(plan, "cleanup_estimated_bytes").unwrap_or(0),
        measured_disk_space_delta: None,
        journal_id: uuid::Uuid::new_v4().to_string(),
        result,
    };
    let record = journal_record(plan, &execution.result, execution.journal_id.clone());
    Ok((execution, record))
}

pub fn execute_with_recycle_bin_journaled(
    plan: &OperationPlan,
    cancelled: &AtomicBool,
    allowed_target_ids: &HashSet<String>,
    journal: &OperationJournal,
    safe_paths: &crate::core::safe_paths::SafePaths,
) -> Result<(CleanupExecution, OperationJournalRecord), OperationError> {
    execute_with_recycle_bin_internal(
        plan,
        cancelled,
        allowed_target_ids,
        Some(journal),
        safe_paths,
    )
}

fn execute_with_recycle_bin_internal(
    plan: &OperationPlan,
    cancelled: &AtomicBool,
    allowed_target_ids: &HashSet<String>,
    journal: Option<&OperationJournal>,
    safe_paths: &crate::core::safe_paths::SafePaths,
) -> Result<(CleanupExecution, OperationJournalRecord), OperationError> {
    if plan.descriptor.category != OperationCategory::Cleanup
        || plan.reversibility != Reversibility::NotReversible
    {
        return Err(validation("invalid cleanup operation plan"));
    }
    if journal.is_none() {
        return Err(OperationError::validation(
            "cleanup execution requires an initialized OperationJournal",
        ));
    }
    let deletion_mode = plan_deletion_mode(plan)?;
    let coordinate_services =
        needs_windows_update_service_coordination(plan, cancelled, allowed_target_ids);
    let service_guard = if coordinate_services {
        Some(
            cleaners::windows_update::begin_cleanup_service_guard().map_err(|error| {
                service_coordination_error(format!(
                    "Windows Update service preparation failed: {error}"
                ))
            })?,
        )
    } else {
        None
    };

    let execute_body = || -> Result<(CleanupExecution, OperationJournalRecord), OperationError> {
        let before_free_space = warning_u64(plan, FREE_SPACE_WARNING);
        let estimated_bytes = warning_u64(plan, "cleanup_estimated_bytes").unwrap_or(0);
        let mut outcomes = Vec::with_capacity(plan.targets.len());

        for target in &plan.targets {
            #[cfg(any(test, debug_assertions))]
            if take_fail_before_target_n(outcomes.len() + 1) {
                return Err(target_loop_failure("fail_before_target_n"));
            }

            let target_payload = || {
                serde_json::to_string(target).map_err(|error| OperationError {
                    code: OperationErrorCode::SerializationFailed,
                    message: error.to_string(),
                    target_id: Some(target.target_id.clone()),
                    retryable: false,
                })
            };
            if let Some(journal) = journal {
                journal.record_target_state(
                    &plan.id,
                    &target.target_id,
                    TargetExecutionState::Ready,
                    &target_payload()?,
                )?;
                journal.record_target_state(
                    &plan.id,
                    &target.target_id,
                    TargetExecutionState::MutationStarted,
                    &target_payload()?,
                )?;
            }

            let target_is_absent = decode_snapshot(target)
                .map(|snapshot| !Path::new(&snapshot.path).exists())
                .unwrap_or(false);
            let outcome = if !allowed_target_ids.contains(&target.target_id) && !target_is_absent {
                error_outcome(
                    target,
                    OperationErrorCode::TargetProtected,
                    "Target is outside the current server-side cleanup scan.",
                    ExecutionStatus::Skipped,
                )
            } else if cancelled.load(Ordering::SeqCst) {
                error_outcome(
                    target,
                    OperationErrorCode::OperationCancelled,
                    "Cleanup was cancelled before this target.",
                    ExecutionStatus::Skipped,
                )
            } else {
                execute_target(target, deletion_mode, safe_paths)
            };
            if let Some(journal) = journal {
                journal.record_target_outcome(&plan.id, &outcome)?;
            }
            outcomes.push(outcome);

            #[cfg(any(test, debug_assertions))]
            if take_fail_after_target_n(outcomes.len()) {
                return Err(target_loop_failure("fail_after_target_n"));
            }
        }

        let result = OperationResult::new(
            plan.id.clone(),
            outcomes,
            false,
            UndoAvailability::NotReversible,
        )?;
        let after_free_space = plan
            .targets
            .first()
            .and_then(|target| decode_snapshot(target).ok())
            .and_then(|snapshot| available_space(Path::new(&snapshot.path)));
        let measured_disk_space_delta = before_free_space
            .zip(after_free_space)
            .map(|(before, after)| after as i128 - before as i128)
            .and_then(|delta| i64::try_from(delta).ok());
        let journal_id = uuid::Uuid::new_v4().to_string();
        let record = journal_record(plan, &result, journal_id.clone());

        Ok((
            CleanupExecution {
                result,
                estimated_bytes,
                measured_disk_space_delta,
                journal_id,
            },
            record,
        ))
    };

    match service_guard {
        Some(mut guard) => {
            let execution = catch_unwind(AssertUnwindSafe(execute_body));
            let restoration = catch_unwind(AssertUnwindSafe(|| guard.restore()))
                .map_err(|_| "service restoration panicked".to_string())
                .and_then(|result| result);
            finish_service_coordinated_execution(execution, restoration)
        }
        None => execute_body(),
    }
}

fn needs_windows_update_service_coordination(
    plan: &OperationPlan,
    cancelled: &AtomicBool,
    allowed_target_ids: &HashSet<String>,
) -> bool {
    !cancelled.load(Ordering::SeqCst)
        && plan.targets.iter().any(|target| {
            allowed_target_ids.contains(&target.target_id)
                && decode_snapshot(target)
                    .map(|snapshot| {
                        snapshot.category == "update_cache" && Path::new(&snapshot.path).exists()
                    })
                    .unwrap_or(false)
        })
}

fn finish_service_coordinated_execution(
    execution: std::thread::Result<
        Result<(CleanupExecution, OperationJournalRecord), OperationError>,
    >,
    restoration: std::result::Result<(), String>,
) -> Result<(CleanupExecution, OperationJournalRecord), OperationError> {
    match execution {
        Ok(result) => match restoration {
            Ok(()) => result,
            Err(error) => match result {
                Ok(_) => Err(service_restoration_error(error)),
                Err(mut operation_error) => {
                    operation_error.message = format!(
                        "{}; Windows Update service restoration unresolved: {error}",
                        operation_error.message
                    );
                    operation_error.retryable = true;
                    Err(operation_error)
                }
            },
        },
        Err(payload) => {
            let panic_message = payload
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("unknown panic");
            let message = match restoration {
                Ok(()) => format!("Windows Update cleanup panicked: {panic_message}"),
                Err(error) => format!(
                    "Windows Update cleanup panicked: {panic_message}; Windows Update service restoration unresolved: {error}"
                ),
            };
            Err(OperationError {
                code: OperationErrorCode::InternalError,
                message,
                target_id: None,
                retryable: true,
            })
        }
    }
}

fn service_coordination_error(message: String) -> OperationError {
    OperationError {
        code: OperationErrorCode::ExecutionFailed,
        message,
        target_id: None,
        retryable: true,
    }
}

fn service_restoration_error(error: String) -> OperationError {
    service_coordination_error(format!(
        "Windows Update service restoration unresolved: {error}"
    ))
}

fn reject_unsafe_cleanup_path(path: &Path) -> Result<(), String> {
    let raw = path.to_string_lossy();
    if raw.contains('*') || raw.contains('?') {
        return Err("cleanup path contains glob metacharacters".into());
    }
    Ok(())
}

fn path_or_ancestor_is_reparse(path: &Path) -> bool {
    path.ancestors().any(|ancestor| {
        if ancestor.as_os_str().is_empty() {
            return false;
        }
        std::fs::symlink_metadata(ancestor)
            .map(|metadata| {
                crate::analyzer::disk_usage::is_reparse_point(ancestor, &metadata.file_type())
            })
            .unwrap_or(false)
    })
}

fn execute_target(
    target: &PlannedTarget,
    deletion_mode: CleanupDeletionMode,
    safe_paths: &crate::core::safe_paths::SafePaths,
) -> TargetOutcome {
    let snapshot = match decode_snapshot(target) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return error_outcome(
                target,
                OperationErrorCode::ValidationFailed,
                &error.message,
                ExecutionStatus::Failed,
            )
        }
    };
    let path = PathBuf::from(&snapshot.path);
    if snapshot.deletion_mode != deletion_mode {
        return error_outcome(
            target,
            OperationErrorCode::TargetChangedSincePreview,
            "cleanup deletion mode changed since preview",
            ExecutionStatus::Skipped,
        );
    }
    if let Err(message) = reject_unsafe_cleanup_path(&path) {
        return error_outcome(
            target,
            OperationErrorCode::ValidationFailed,
            &message,
            ExecutionStatus::Failed,
        );
    }
    if normalized_target_id(&snapshot.path) != target.target_id
        || !known_cleanup_category(&snapshot.category)
    {
        return error_outcome(
            target,
            OperationErrorCode::TargetProtected,
            "Target is outside the confirmed cleanup scan.",
            ExecutionStatus::Skipped,
        );
    }
    let expected_path = match snapshot.canonical_path.as_deref() {
        Some(path) if !path.trim().is_empty() => PathBuf::from(path),
        _ => {
            return error_outcome(
                target,
                OperationErrorCode::TargetProtected,
                "cleanup target has no confirmed canonical path",
                ExecutionStatus::Skipped,
            )
        }
    };
    if cfg!(windows)
        && !snapshot.identity.as_ref().is_some_and(|identity| {
            identity.volume_serial.is_some() && identity.file_reference.is_some()
        })
    {
        return error_outcome(
            target,
            OperationErrorCode::TargetProtected,
            "cleanup target has no complete Windows file identity",
            ExecutionStatus::Skipped,
        );
    }
    let current = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return TargetOutcome {
                target_id: target.target_id.clone(),
                requested_action: cleanup_requested_action(deletion_mode).into(),
                before: None,
                after: Some(absent_snapshot()),
                execution: ExecutionStatus::Unchanged,
                verification: VerificationStatus::Failed,
                error: Some(target_error(
                    target,
                    OperationErrorCode::TargetNotFound,
                    "snapshot path is missing",
                    true,
                )),
            };
        }
        Err(error) => {
            return error_outcome(
                target,
                OperationErrorCode::PermissionDenied,
                &format!("Cannot inspect target: {error}"),
                ExecutionStatus::Failed,
            )
        }
    };
    if current.is_dir() {
        return error_outcome(
            target,
            OperationErrorCode::TargetProtected,
            "cleanup removes files only and refuses directory targets",
            ExecutionStatus::Skipped,
        );
    }
    if path_or_ancestor_is_reparse(&path) {
        return error_outcome(
            target,
            OperationErrorCode::TargetProtected,
            "cleanup refuses reparse or junction retargets",
            ExecutionStatus::Failed,
        );
    }
    let current_canonical = match path.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            return error_outcome(
                target,
                OperationErrorCode::TargetChangedSincePreview,
                &format!("cannot confirm target path: {error}"),
                ExecutionStatus::Skipped,
            )
        }
    };
    if normalized_target_id(&current_canonical.to_string_lossy())
        != normalized_target_id(&expected_path.to_string_lossy())
    {
        return error_outcome(
            target,
            OperationErrorCode::TargetChangedSincePreview,
            "target path changed since preview",
            ExecutionStatus::Skipped,
        );
    }
    let current_modified = current.modified().ok().map(system_time_key);
    if snapshot.last_modified.is_none() || current_modified.is_none() {
        return error_outcome(
            target,
            OperationErrorCode::TargetChangedSincePreview,
            "cleanup target lacks reliable modification-time identity",
            ExecutionStatus::Skipped,
        );
    }
    if current.len() != snapshot.size_bytes || current_modified != snapshot.last_modified {
        return error_outcome(
            target,
            OperationErrorCode::TargetChangedSincePreview,
            "target_changed_since_preview",
            ExecutionStatus::Skipped,
        );
    }
    if let Some(expected_identity) = snapshot.identity.as_ref() {
        let current_identity = crate::core::file_identity::snapshot_for_path(&path)
            .ok()
            .map(|value| value.identity);
        if current_identity.as_ref() != Some(expected_identity) {
            return error_outcome(
                target,
                OperationErrorCode::TargetChangedSincePreview,
                "target identity changed since preview",
                ExecutionStatus::Skipped,
            );
        }
    }
    if snapshot.category == "browser_cache" && browser_cache_is_running(&path) {
        return error_outcome(
            target,
            OperationErrorCode::TargetChangedSincePreview,
            "browser_running_at_execution",
            ExecutionStatus::Skipped,
        );
    }
    // Execution-side protected-path invariant. Scanner-side whitelisting is
    // defense-in-depth and does not substitute for re-checking the confirmed
    // path against the approved roots immediately before deletion.
    if !safe_paths.is_safe(&path) {
        return error_outcome(
            target,
            OperationErrorCode::TargetProtected,
            "cleanup refuses a target outside the approved protected-path roots",
            ExecutionStatus::Skipped,
        );
    }

    let mut properties = BTreeMap::new();
    properties.insert("path".into(), snapshot.path.clone());
    properties.insert("size_bytes".into(), snapshot.size_bytes.to_string());
    let before = Some(StateSnapshot {
        summary: "File existed at confirmation.".into(),
        properties,
    });
    let deletion = crate::core::race_safe_delete::delete_file(
        &path,
        &expected_path,
        snapshot.identity.as_ref(),
        deletion_mode == CleanupDeletionMode::RecycleBin,
    );
    if let Err(error) = deletion {
        return TargetOutcome {
            target_id: target.target_id.clone(),
            requested_action: cleanup_requested_action(deletion_mode).into(),
            before,
            after: None,
            execution: ExecutionStatus::Failed,
            verification: VerificationStatus::Failed,
            error: Some(target_error(
                target,
                OperationErrorCode::ExecutionFailed,
                format!("Failed to remove target: {error}"),
                true,
            )),
        };
    }
    let removed = !path.exists();
    TargetOutcome {
        target_id: target.target_id.clone(),
        requested_action: cleanup_requested_action(deletion_mode).into(),
        before,
        after: Some(if removed {
            absent_snapshot()
        } else {
            StateSnapshot {
                summary: "Target still exists after cleanup.".into(),
                properties: BTreeMap::new(),
            }
        }),
        execution: ExecutionStatus::Changed,
        verification: if removed {
            VerificationStatus::Verified
        } else {
            VerificationStatus::Failed
        },
        error: (!removed).then(|| {
            target_error(
                target,
                OperationErrorCode::VerificationFailed,
                "Target still exists after cleanup.",
                true,
            )
        }),
    }
}

fn browser_cache_is_running(path: &Path) -> bool {
    match cleaners::browser_cache::detect_browsers() {
        Ok(browsers) => browsers.into_iter().any(|browser| {
            browser.is_running
                && browser
                    .cache_path
                    .map(|cache_path| path.starts_with(Path::new(&cache_path)))
                    .unwrap_or(false)
        }),
        Err(_) => true,
    }
}

fn journal_record(
    plan: &OperationPlan,
    result: &OperationResult,
    journal_id: String,
) -> OperationJournalRecord {
    OperationJournalRecord {
        journal_id,
        record_kind: JournalRecordKind::Operation,
        related_record_id: None,
        operation_id: plan.id.clone(),
        operation_schema_version: OPERATION_SCHEMA_VERSION,
        application_version: env!("CARGO_PKG_VERSION").into(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        terminal_state: result.completion_state(),
        confirmed_plan: plan.clone(),
        requested_targets: plan.targets.clone(),
        captured_original_states: Vec::new(),
        target_outcomes: result.outcomes.clone(),
        verification_results: result
            .outcomes
            .iter()
            .map(|outcome| VerificationRecord {
                target_id: outcome.target_id.clone(),
                status: outcome.verification,
                message: outcome
                    .error
                    .as_ref()
                    .map(|error| error.message.clone())
                    .unwrap_or_else(|| format!("{:?}", outcome.verification)),
            })
            .collect(),
        restart_requirement: RebootPolicy::NotRequired,
        undo_eligibility: UndoEligibility {
            eligible: false,
            reason: "Cleanup deletion has no automated restoration test.".into(),
            automated_restoration_test: None,
        },
        undo_execution_result: None,
        diagnostic: result
            .outcomes
            .iter()
            .find_map(|outcome| outcome.error.clone()),
    }
}

fn decode_snapshot(target: &PlannedTarget) -> Result<CleanupTargetSnapshot, OperationError> {
    serde_json::from_str(&target.requested_action)
        .map_err(|error| validation(format!("invalid confirmed cleanup target: {error}")))
}

fn plan_deletion_mode(plan: &OperationPlan) -> Result<CleanupDeletionMode, OperationError> {
    let mut mode = None;
    for target in &plan.targets {
        let target_mode = decode_snapshot(target)?.deletion_mode;
        if let Some(previous) = mode {
            if previous != target_mode {
                return Err(validation(
                    "cleanup plan mixes deletion modes across targets",
                ));
            }
        } else {
            mode = Some(target_mode);
        }
    }
    mode.ok_or_else(|| validation("cleanup plan has no targets"))
}

fn deletion_mode_label(mode: CleanupDeletionMode) -> &'static str {
    match mode {
        CleanupDeletionMode::RecycleBin => "move to Recycle Bin",
        CleanupDeletionMode::Permanent => "permanent deletion",
    }
}

fn cleanup_requested_action(mode: CleanupDeletionMode) -> &'static str {
    match mode {
        CleanupDeletionMode::RecycleBin => "move_to_recycle_bin",
        CleanupDeletionMode::Permanent => "delete_permanently",
    }
}

fn validate_fixture_target(
    target: &PlannedTarget,
    snapshot: &CleanupTargetSnapshot,
    filesystem: &dyn crate::core::filesystem::FileSystemAdapter,
) -> Result<(), OperationError> {
    let path = Path::new(&snapshot.path);
    if normalized_target_id(&snapshot.path) != target.target_id
        || !known_cleanup_category(&snapshot.category)
    {
        return Err(validation("Target is outside the confirmed cleanup scan."));
    }
    let expected = snapshot
        .canonical_path
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .ok_or_else(|| validation("cleanup target has no confirmed canonical path"))?;
    let identity = filesystem
        .identity(path)
        .map_err(|error| validation(format!("cannot confirm target path: {error}")))?;
    if normalized_target_id(&identity.canonical_path.to_string_lossy())
        != normalized_target_id(expected)
    {
        return Err(OperationError {
            code: OperationErrorCode::TargetChangedSincePreview,
            message: "target path changed since preview".into(),
            target_id: Some(target.target_id.clone()),
            retryable: true,
        });
    }
    let metadata = filesystem
        .metadata(path)
        .map_err(|error| validation(error.to_string()))?;
    let modified = metadata.modified().ok().map(system_time_key);
    if snapshot.last_modified.is_none() || modified.is_none() {
        return Err(OperationError {
            code: OperationErrorCode::TargetChangedSincePreview,
            message: "cleanup target lacks reliable modification-time identity".into(),
            target_id: Some(target.target_id.clone()),
            retryable: true,
        });
    }
    if metadata.len() != snapshot.size_bytes || modified != snapshot.last_modified {
        return Err(OperationError {
            code: OperationErrorCode::TargetChangedSincePreview,
            message: "target_changed_since_preview".into(),
            target_id: Some(target.target_id.clone()),
            retryable: true,
        });
    }
    let expected_identity = snapshot.identity.as_ref().ok_or_else(|| OperationError {
        code: OperationErrorCode::TargetProtected,
        message: "cleanup target has no confirmed file identity".into(),
        target_id: Some(target.target_id.clone()),
        retryable: false,
    })?;
    let current_identity = crate::core::file_identity::FileIdentity {
        volume_serial: identity.volume_serial,
        file_reference: identity.file_reference,
    };
    if &current_identity != expected_identity {
        return Err(OperationError {
            code: OperationErrorCode::TargetChangedSincePreview,
            message: "target identity changed since preview".into(),
            target_id: Some(target.target_id.clone()),
            retryable: true,
        });
    }
    Ok(())
}

fn warning_u64(plan: &OperationPlan, code: &str) -> Option<u64> {
    plan.warnings
        .iter()
        .find(|warning| warning.code == code)
        .and_then(|warning| warning.message.parse().ok())
}

fn known_cleanup_category(category: &str) -> bool {
    matches!(
        category,
        "temp" | "prefetch" | "browser_cache" | "update_cache" | "logs"
    )
}

fn normalized_target_id(path: &str) -> String {
    path.replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn system_time_key(value: std::time::SystemTime) -> String {
    value
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string()
}

fn absent_snapshot() -> StateSnapshot {
    StateSnapshot {
        summary: "Target is absent.".into(),
        properties: BTreeMap::new(),
    }
}

fn error_outcome(
    target: &PlannedTarget,
    code: OperationErrorCode,
    message: &str,
    execution: ExecutionStatus,
) -> TargetOutcome {
    TargetOutcome {
        target_id: target.target_id.clone(),
        requested_action: decode_snapshot(target)
            .map(|snapshot| cleanup_requested_action(snapshot.deletion_mode))
            .unwrap_or("cleanup_target")
            .into(),
        before: None,
        after: None,
        execution,
        verification: VerificationStatus::Failed,
        error: Some(target_error(target, code, message, true)),
    }
}

fn target_error(
    target: &PlannedTarget,
    code: OperationErrorCode,
    message: impl Into<String>,
    retryable: bool,
) -> OperationError {
    OperationError {
        code,
        message: message.into(),
        target_id: Some(target.target_id.clone()),
        retryable,
    }
}

fn validation(message: impl Into<String>) -> OperationError {
    OperationError::validation(message)
}

#[cfg(windows)]
fn available_space(path: &Path) -> Option<u64> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let root = path.components().next().map(|component| {
        let mut value = component.as_os_str().to_string_lossy().into_owned();
        if value.ends_with(':') {
            value.push('\\');
        }
        value
    })?;
    let root = HSTRING::from(root);
    let mut available = 0u64;
    unsafe {
        GetDiskFreeSpaceExW(PCWSTR(root.as_ptr()), Some(&mut available), None, None)
            .ok()
            .map(|_| available)
    }
}

#[cfg(not(windows))]
fn available_space(_path: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::operation::{OperationExecutionState, TargetExecutionState};
    use crate::core::operation_journal::OperationJournal;
    use tempfile::tempdir;

    fn scanned_target(path: &Path, category: &str, recommended: bool) -> CleanupTarget {
        let metadata = path.metadata().unwrap();
        CleanupTarget {
            target_id: normalized_target_id(&path.to_string_lossy()),
            category: category.into(),
            resource_id: path.to_string_lossy().into_owned(),
            description: "test target".into(),
            size_bytes: metadata.len(),
            last_modified: metadata.modified().ok().map(system_time_key),
            confidence: "high".into(),
            risk: RiskLevel::Low,
            recommended,
            recommendation_reason: "test".into(),
            accessibility: "accessible".into(),
            reversibility: Reversibility::NotReversible,
            affected_application: None,
            group_name: "test group".into(),
            cleanup_type: "test type".into(),
            requires_elevation: false,
            canonical_path: path
                .canonicalize()
                .ok()
                .map(|value| value.to_string_lossy().into_owned()),
            identity: crate::core::file_identity::snapshot_for_path(path)
                .ok()
                .map(|value| value.identity),
        }
    }

    fn scan_with(targets: Vec<CleanupTarget>) -> CleanupScan {
        CleanupScan {
            scan_id: "scan".into(),
            scanned_at: "now".into(),
            estimated_bytes: targets.iter().map(|target| target.size_bytes).sum(),
            free_space_before: None,
            targets,
            failures: Vec::new(),
        }
    }

    #[test]
    fn enriched_target_preserves_program_and_cleanup_type() {
        let root = tempdir().unwrap();
        let path = root.path().join("cache.bin");
        std::fs::write(&path, b"cache").unwrap();

        let target = enrich_target(CleanItem {
            path: path.to_string_lossy().into_owned(),
            size_bytes: 5,
            last_accessed: None,
            category: "browser_cache".into(),
            description: "Chrome cache".into(),
            group_key: "Google Chrome (Default)".into(),
            sub_group: "Cache".into(),
        });

        assert_eq!(
            target.affected_application.as_deref(),
            Some("Google Chrome (Default)")
        );
        assert_eq!(target.group_name, "Google Chrome (Default)");
        assert_eq!(target.cleanup_type, "Cache");
    }

    fn allow(plan: &OperationPlan) -> HashSet<String> {
        plan.targets
            .iter()
            .map(|target| target.target_id.clone())
            .collect()
    }

    fn safe_paths_for(directory: &tempfile::TempDir) -> crate::core::safe_paths::SafePaths {
        crate::core::safe_paths::SafePaths::from_canonical_roots(vec![directory
            .path()
            .canonicalize()
            .unwrap()])
    }

    fn execute_with_journal(
        directory: &tempfile::TempDir,
        journal_name: &str,
        plan: &OperationPlan,
        cancelled: &AtomicBool,
        allowed_target_ids: &HashSet<String>,
    ) -> Result<(CleanupExecution, OperationJournalRecord), OperationError> {
        let journal = OperationJournal::open_at(&directory.path().join(journal_name)).unwrap();
        journal.begin_confirmed_execution(plan).unwrap();
        let safe_paths = crate::core::safe_paths::SafePaths::from_canonical_roots(vec![directory
            .path()
            .canonicalize()
            .unwrap()]);
        execute_with_recycle_bin_journaled(
            plan,
            cancelled,
            allowed_target_ids,
            &journal,
            &safe_paths,
        )
    }

    #[test]
    fn safe_defaults_only_recommend_temporary_files() {
        assert!(category_policy("temp").1);
        assert!(!category_policy("browser_cache").1);
        assert!(!category_policy("prefetch").1);
        assert!(!category_policy("update_cache").1);
        assert!(!category_policy("logs").1);
        assert!(!category_policy("unknown").1);
    }

    #[test]
    fn plan_rejects_targets_outside_the_scan() {
        let error = create_plan(&scan_with(Vec::new()), &["forged".into()]).unwrap_err();
        assert!(error.message.contains("outside the current scan"));
    }

    #[test]
    fn unjournaled_execution_fails_closed_before_mutation() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("unjournaled.tmp");
        std::fs::write(&path, b"keep").unwrap();
        let target = scanned_target(&path, "temp", true);
        let plan = create_plan(&scan_with(vec![target.clone()]), &[target.target_id]).unwrap();
        let safe_paths = crate::core::safe_paths::SafePaths::from_canonical_roots(vec![directory
            .path()
            .canonicalize()
            .unwrap()]);

        let error =
            execute(&plan, &AtomicBool::new(false), &allow(&plan), &safe_paths).unwrap_err();

        assert_eq!(error.code, OperationErrorCode::ValidationFailed);
        assert!(error
            .message
            .contains("cleanup execution requires an initialized OperationJournal"));
        assert!(path.exists());

        let error =
            execute_with_recycle_bin(&plan, &AtomicBool::new(false), &allow(&plan), &safe_paths)
                .unwrap_err();

        assert_eq!(error.code, OperationErrorCode::ValidationFailed);
        assert!(error
            .message
            .contains("cleanup execution requires an initialized OperationJournal"));
        assert!(path.exists());
    }

    #[test]
    fn generated_plan_preserves_exact_selected_targets() {
        let directory = tempdir().unwrap();
        let first = directory.path().join("first.tmp");
        let second = directory.path().join("second.tmp");
        std::fs::write(&first, b"one").unwrap();
        std::fs::write(&second, b"two").unwrap();
        let first = scanned_target(&first, "temp", true);
        let second = scanned_target(&second, "temp", true);
        let scan = scan_with(vec![first.clone(), second]);

        let plan = create_plan(&scan, std::slice::from_ref(&first.target_id)).unwrap();

        assert_eq!(plan.targets.len(), 1);
        assert_eq!(plan.targets[0].target_id, first.target_id);
        assert_eq!(plan.reversibility, Reversibility::NotReversible);
    }

    #[test]
    fn authoritative_scan_rejects_forged_size_and_canonical_path() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("authoritative.tmp");
        std::fs::write(&path, b"authoritative").unwrap();
        let target = scanned_target(&path, "temp", true);
        let scan = scan_with(vec![target.clone()]);
        let plan = create_plan(&scan, std::slice::from_ref(&target.target_id)).unwrap();

        let mut forged_size = scan.clone();
        forged_size.targets[0].size_bytes += 1;
        let error = validate_authoritative_scan(&plan, &forged_size).unwrap_err();
        assert_eq!(error.code, OperationErrorCode::TargetChangedSincePreview);

        let mut forged_path = scan;
        forged_path.targets[0].canonical_path = Some("C:\\forged\\target".into());
        let error = validate_authoritative_scan(&plan, &forged_path).unwrap_err();
        assert_eq!(error.code, OperationErrorCode::TargetChangedSincePreview);
    }

    #[test]
    fn fixture_validation_rejects_identity_replacement_with_matching_metadata() {
        let directory = tempdir().unwrap();
        let fixture_root = directory.path().join("fixture");
        let path = fixture_root.join("cleanup/temp/replaced.tmp");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"same metadata").unwrap();
        let filesystem = crate::core::filesystem::FixtureFileSystem::new(&fixture_root).unwrap();

        let scan = scan_fixture(&fixture_root, &filesystem);
        assert_eq!(scan.targets.len(), 1);
        let target = scan.targets[0].clone();
        let planned = create_plan(&scan, std::slice::from_ref(&target.target_id))
            .unwrap()
            .targets
            .remove(0);
        let mut snapshot = decode_snapshot(&planned).unwrap();
        snapshot.identity = Some(crate::core::file_identity::FileIdentity {
            volume_serial: Some(u64::MAX),
            file_reference: Some("replacement".into()),
        });

        let error = validate_fixture_target(&planned, &snapshot, &filesystem).unwrap_err();

        assert_eq!(error.code, OperationErrorCode::TargetChangedSincePreview);
        assert_eq!(error.message, "target identity changed since preview");
        assert!(path.exists());
    }

    #[test]
    fn retry_rebuilds_a_new_plan_from_current_targets() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("retry.tmp");
        std::fs::write(&path, b"retry").unwrap();
        let target = scanned_target(&path, "temp", true);
        let scan = scan_with(vec![target.clone()]);
        let previous = create_plan(&scan, std::slice::from_ref(&target.target_id)).unwrap();

        let retry =
            create_retry_plan(&scan, &previous, std::slice::from_ref(&target.target_id)).unwrap();

        assert_ne!(retry.id, previous.id);
        assert_eq!(retry.parent_operation_id, Some(previous.id.clone()));
        assert_eq!(retry.retry_of_operation_id, Some(previous.id));
        assert_eq!(
            retry.targets,
            scan.targets
                .iter()
                .map(|target| PlannedTarget {
                    target_id: target.target_id.clone(),
                    requested_action: serde_json::to_string(&CleanupTargetSnapshot {
                        category: target.category.clone(),
                        path: target.resource_id.clone(),
                        size_bytes: target.size_bytes,
                        last_modified: target.last_modified.clone(),
                        deletion_mode: CleanupDeletionMode::RecycleBin,
                        canonical_path: target.canonical_path.clone(),
                        identity: target.identity.clone(),
                    })
                    .unwrap(),
                    description: target.description.clone(),
                })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn recovery_reconciliation_verifies_an_already_absent_target() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("reconciled.tmp");
        std::fs::write(&path, b"reconciled").unwrap();
        let target = scanned_target(&path, "temp", true);
        let plan = create_plan(&scan_with(vec![target.clone()]), &[target.target_id]).unwrap();
        std::fs::remove_file(&path).unwrap();

        let record = reconciliation_record(&plan, &scan_with(Vec::new())).unwrap();

        assert_eq!(record.terminal_state, OperationState::Succeeded);
        assert_eq!(
            record.target_outcomes[0].execution,
            ExecutionStatus::Unchanged
        );
        assert_eq!(
            record.target_outcomes[0].verification,
            VerificationStatus::Verified
        );
        record.validate().unwrap();
    }

    #[test]
    fn recovery_reconciliation_marks_existing_target_unknown() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("unknown.tmp");
        std::fs::write(&path, b"unknown").unwrap();
        let target = scanned_target(&path, "temp", true);
        let scan = scan_with(vec![target.clone()]);
        let plan = create_plan(&scan, &[target.target_id]).unwrap();

        let record = reconciliation_record(&plan, &scan).unwrap();

        assert_eq!(record.terminal_state, OperationState::Interrupted);
        assert_eq!(
            record.target_outcomes[0].verification,
            VerificationStatus::Unverified
        );
        assert_eq!(
            record.target_outcomes[0].error.as_ref().unwrap().code,
            OperationErrorCode::OperationInterrupted
        );
        record.validate().unwrap();
    }

    #[test]
    fn changed_target_is_not_deleted() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("changed.tmp");
        std::fs::write(&path, b"before").unwrap();
        let target = scanned_target(&path, "temp", true);
        let plan = create_plan(&scan_with(vec![target.clone()]), &[target.target_id]).unwrap();
        std::fs::write(&path, b"changed content").unwrap();

        let (execution, _) = execute_with_journal(
            &directory,
            "journal.db",
            &plan,
            &AtomicBool::new(false),
            &allow(&plan),
        )
        .unwrap();

        assert!(path.exists());
        assert_eq!(execution.result.skipped, 1);
        assert_eq!(
            execution.result.outcomes[0].error.as_ref().unwrap().code,
            OperationErrorCode::TargetChangedSincePreview
        );
    }

    #[test]
    fn cancellation_reconciles_every_requested_target() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("cancelled.tmp");
        std::fs::write(&path, b"keep").unwrap();
        let target = scanned_target(&path, "temp", true);
        let plan = create_plan(&scan_with(vec![target.clone()]), &[target.target_id]).unwrap();

        let (execution, record) = execute_with_journal(
            &directory,
            "journal.db",
            &plan,
            &AtomicBool::new(true),
            &allow(&plan),
        )
        .unwrap();

        assert!(path.exists());
        assert_eq!(execution.result.requested, 1);
        assert_eq!(execution.result.skipped, 1);
        assert_eq!(record.target_outcomes.len(), 1);
        record.validate().unwrap();
    }

    #[test]
    fn fail_after_target_n_is_one_shot_and_leaves_execution_unresolved() {
        let directory = tempdir().unwrap();
        let first_path = directory.path().join("first.tmp");
        let second_path = directory.path().join("second.tmp");
        std::fs::write(&first_path, b"first").unwrap();
        std::fs::write(&second_path, b"second").unwrap();
        let first = scanned_target(&first_path, "temp", true);
        let second = scanned_target(&second_path, "temp", true);
        let plan = create_plan(
            &scan_with(vec![first.clone(), second.clone()]),
            &[first.target_id.clone(), second.target_id.clone()],
        )
        .unwrap();
        let journal = OperationJournal::open_at(&directory.path().join("journal.db")).unwrap();
        journal.begin_confirmed_execution(&plan).unwrap();

        fail_after_target_n(1);
        let error = execute_with_recycle_bin_journaled(
            &plan,
            &AtomicBool::new(false),
            &allow(&plan),
            &journal,
            &safe_paths_for(&directory),
        )
        .unwrap_err();

        assert_eq!(error.message, "failure injection: fail_after_target_n");
        assert!(!first_path.exists());
        assert!(second_path.exists());
        assert!(journal.list().unwrap().is_empty());
        let unresolved = journal.list_unresolved_operations().unwrap();
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].state, OperationExecutionState::Executing);
        assert!(unresolved[0]
            .targets
            .iter()
            .all(|target| target.state == TargetExecutionState::Pending));

        let (second_execution, _) = execute_with_journal(
            &directory,
            "retry-journal.db",
            &plan,
            &AtomicBool::new(false),
            &allow(&plan),
        )
        .unwrap();
        assert_eq!(second_execution.result.changed, 1);
        assert_eq!(second_execution.result.unchanged, 1);

        assert_eq!(journal.reconcile_unresolved_operations().unwrap(), 1);
        let reconciled = journal.list_unresolved_operations().unwrap();
        assert_eq!(reconciled[0].state, OperationExecutionState::OutcomeUnknown);
        assert!(reconciled[0]
            .targets
            .iter()
            .all(|target| target.state == TargetExecutionState::OutcomeUnknown));
    }

    #[test]
    fn fail_before_target_n_is_one_shot_and_preserves_prior_effects_and_unresolved_execution() {
        let directory = tempdir().unwrap();
        let first_path = directory.path().join("first.tmp");
        let second_path = directory.path().join("second.tmp");
        std::fs::write(&first_path, b"first").unwrap();
        std::fs::write(&second_path, b"second").unwrap();
        let first = scanned_target(&first_path, "temp", true);
        let second = scanned_target(&second_path, "temp", true);
        let plan = create_plan(
            &scan_with(vec![first.clone(), second.clone()]),
            &[first.target_id.clone(), second.target_id.clone()],
        )
        .unwrap();
        let journal = OperationJournal::open_at(&directory.path().join("journal.db")).unwrap();
        journal.begin_confirmed_execution(&plan).unwrap();

        fail_before_target_n(2);
        let error = execute_with_recycle_bin_journaled(
            &plan,
            &AtomicBool::new(false),
            &allow(&plan),
            &journal,
            &safe_paths_for(&directory),
        )
        .unwrap_err();

        assert_eq!(error.message, "failure injection: fail_before_target_n");
        assert!(!first_path.exists());
        assert!(second_path.exists());
        assert!(journal.list().unwrap().is_empty());
        let unresolved = journal.list_unresolved_operations().unwrap();
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].state, OperationExecutionState::Executing);
        assert!(unresolved[0]
            .targets
            .iter()
            .all(|target| target.state == TargetExecutionState::Pending));

        let (second_execution, _) = execute_with_journal(
            &directory,
            "retry-journal.db",
            &plan,
            &AtomicBool::new(false),
            &allow(&plan),
        )
        .unwrap();
        assert_eq!(second_execution.result.changed, 1);
        assert_eq!(second_execution.result.unchanged, 1);
        assert!(!second_path.exists());
    }

    #[test]
    fn execution_verifies_removal_and_reports_no_undo() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("remove.tmp");
        std::fs::write(&path, b"remove").unwrap();
        let target = scanned_target(&path, "temp", true);
        let plan = create_plan(&scan_with(vec![target.clone()]), &[target.target_id]).unwrap();

        let (execution, record) = execute_with_journal(
            &directory,
            "journal.db",
            &plan,
            &AtomicBool::new(false),
            &allow(&plan),
        )
        .unwrap();

        assert!(!path.exists());
        assert_eq!(execution.result.changed, 1);
        assert_eq!(execution.result.verified, 1);
        assert_eq!(execution.result.undo, UndoAvailability::NotReversible);
        record.validate().unwrap();
    }

    #[test]
    fn service_restoration_failure_never_returns_success() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("restore-failure.tmp");
        std::fs::write(&path, b"restore failure").unwrap();
        let target = scanned_target(&path, "temp", true);
        let plan = create_plan(&scan_with(vec![target.clone()]), &[target.target_id]).unwrap();
        let (execution, record) = execute_with_journal(
            &directory,
            "journal.db",
            &plan,
            &AtomicBool::new(false),
            &allow(&plan),
        )
        .unwrap();

        let error = finish_service_coordinated_execution(
            Ok(Ok((execution, record))),
            Err("BITS could not be started".into()),
        )
        .unwrap_err();

        assert_eq!(error.code, OperationErrorCode::ExecutionFailed);
        assert!(error
            .message
            .contains("Windows Update service restoration unresolved"));
    }

    #[test]
    fn protected_path_is_excluded_when_not_in_server_scan() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("forged.tmp");
        std::fs::write(&path, b"keep").unwrap();
        let target = scanned_target(&path, "temp", true);
        let plan = create_plan(&scan_with(vec![target.clone()]), &[target.target_id]).unwrap();

        let (execution, _) = execute_with_journal(
            &directory,
            "journal.db",
            &plan,
            &AtomicBool::new(false),
            &HashSet::new(),
        )
        .unwrap();

        assert!(path.exists());
        assert_eq!(execution.result.skipped, 1);
        assert_eq!(
            execution.result.outcomes[0].error.as_ref().unwrap().code,
            OperationErrorCode::TargetProtected
        );
    }

    #[test]
    fn partial_failure_reconciles_success_and_changed_target() {
        let directory = tempdir().unwrap();
        let removable = directory.path().join("remove.tmp");
        let changed = directory.path().join("changed.tmp");
        std::fs::write(&removable, b"remove").unwrap();
        std::fs::write(&changed, b"before").unwrap();
        let removable = scanned_target(&removable, "temp", true);
        let changed = scanned_target(&changed, "temp", true);
        let scan = scan_with(vec![removable.clone(), changed.clone()]);
        let plan = create_plan(
            &scan,
            &[removable.target_id.clone(), changed.target_id.clone()],
        )
        .unwrap();
        std::fs::write(&changed.resource_id, b"changed content").unwrap();

        let (execution, _) = execute_with_journal(
            &directory,
            "journal.db",
            &plan,
            &AtomicBool::new(false),
            &allow(&plan),
        )
        .unwrap();

        assert_eq!(execution.result.requested, 2);
        assert_eq!(execution.result.changed, 1);
        assert_eq!(execution.result.skipped, 1);
        assert_eq!(
            execution.result.changed
                + execution.result.unchanged
                + execution.result.skipped
                + execution.result.failed,
            execution.result.requested
        );
    }

    fn planned_temp_target(path: &Path) -> PlannedTarget {
        let target = scanned_target(path, "temp", true);
        create_plan(&scan_with(vec![target.clone()]), &[target.target_id])
            .unwrap()
            .targets
            .remove(0)
    }

    #[test]
    fn execute_target_refuses_junction_retarget_and_leaves_outside_canary() {
        let sandbox = tempdir().unwrap();
        let scan_root = sandbox.path().join("scan");
        let outside = sandbox.path().join("outside");
        std::fs::create_dir_all(&scan_root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let scanned = scan_root.join("file.tmp");
        let canary = outside.join("file.tmp");
        std::fs::write(&canary, b"canary").unwrap();
        std::fs::write(&scanned, b"canary").unwrap();
        let planned = planned_temp_target(&scanned);

        std::fs::remove_file(&scanned).unwrap();
        std::fs::remove_dir(&scan_root).unwrap();
        let status = std::process::Command::new("cmd")
            .args([
                "/C",
                "mklink",
                "/J",
                &scan_root.to_string_lossy(),
                &outside.to_string_lossy(),
            ])
            .status()
            .expect("mklink");
        assert!(status.success(), "mklink /J should succeed without admin");
        let outcome = execute_target(
            &planned,
            CleanupDeletionMode::RecycleBin,
            &safe_paths_for(&sandbox),
        );

        assert!(
            canary.exists(),
            "outside canary must stay untouched after a junction retarget"
        );
        assert_ne!(outcome.verification, VerificationStatus::Verified);
        assert_eq!(outcome.execution, ExecutionStatus::Failed);
        assert_eq!(
            outcome.error.as_ref().unwrap().code,
            OperationErrorCode::TargetProtected
        );
    }

    #[test]
    fn execute_target_does_not_verify_a_missing_snapshot_path() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("missing.tmp");
        std::fs::write(&path, b"gone").unwrap();
        let planned = planned_temp_target(&path);
        std::fs::remove_file(&path).unwrap();
        let outcome = execute_target(
            &planned,
            CleanupDeletionMode::RecycleBin,
            &safe_paths_for(&directory),
        );

        assert_ne!(outcome.verification, VerificationStatus::Verified);
        assert_eq!(outcome.execution, ExecutionStatus::Unchanged);
        assert_eq!(
            outcome.error.as_ref().unwrap().code,
            OperationErrorCode::TargetNotFound
        );
    }

    #[cfg(windows)]
    #[test]
    fn execute_target_rejects_a_snapshot_without_file_identity() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("identity-required.tmp");
        std::fs::write(&path, b"identity").unwrap();
        let planned = planned_temp_target(&path);
        let mut snapshot = decode_snapshot(&planned).unwrap();
        snapshot.identity = None;
        let planned = PlannedTarget {
            requested_action: serde_json::to_string(&snapshot).unwrap(),
            ..planned
        };
        let outcome = execute_target(
            &planned,
            CleanupDeletionMode::RecycleBin,
            &safe_paths_for(&directory),
        );

        assert_eq!(outcome.execution, ExecutionStatus::Skipped);
        assert_eq!(
            outcome.error.as_ref().unwrap().code,
            OperationErrorCode::TargetProtected
        );
        assert!(path.exists());
    }

    #[test]
    fn recycle_and_delete_reject_glob_metacharacters() {
        let star = PathBuf::from(r"C:\temp\foo*.tmp");
        let question = PathBuf::from(r"C:\temp\foo?.tmp");
        let star_error = reject_unsafe_cleanup_path(&star).unwrap_err();
        let question_error = reject_unsafe_cleanup_path(&question).unwrap_err();
        assert!(star_error.contains("glob"), "{star_error}");
        assert!(question_error.contains("glob"), "{question_error}");

        let planned = PlannedTarget {
            target_id: normalized_target_id(r"c:\temp\foo*.tmp"),
            requested_action: serde_json::to_string(&CleanupTargetSnapshot {
                category: "temp".into(),
                path: r"c:\temp\foo*.tmp".into(),
                size_bytes: 1,
                last_modified: None,
                deletion_mode: CleanupDeletionMode::RecycleBin,
                canonical_path: None,
                identity: None,
            })
            .unwrap(),
            description: "glob".into(),
        };
        let outcome = execute_target(
            &planned,
            CleanupDeletionMode::RecycleBin,
            &crate::core::safe_paths::SafePaths::from_canonical_roots(Vec::new()),
        );
        assert_eq!(outcome.execution, ExecutionStatus::Failed);
        assert!(outcome.error.as_ref().unwrap().message.contains("glob"));
    }

    #[test]
    fn fixture_recycle_mode_preserves_the_file_instead_of_deleting_it() {
        use crate::core::filesystem::FixtureFileSystem;
        let directory = tempdir().unwrap();
        let path = directory.path().join("fixture-recycle.tmp");
        std::fs::write(&path, b"recycle-me").unwrap();
        let target = scanned_target(&path, "temp", true);
        let plan = create_plan_with_mode(
            &scan_with(vec![target.clone()]),
            std::slice::from_ref(&target.target_id),
            CleanupDeletionMode::RecycleBin,
        )
        .unwrap();
        let journal = OperationJournal::open_at(&directory.path().join("journal.db")).unwrap();
        journal.begin_confirmed_execution(&plan).unwrap();
        let filesystem = FixtureFileSystem::new(directory.path()).unwrap();

        let (execution, _) = execute_with_recycle_bin_journaled_with_filesystem(
            &plan,
            &AtomicBool::new(false),
            &allow(&plan),
            &filesystem,
            &journal,
        )
        .unwrap();

        assert_eq!(execution.result.changed, 1);
        assert_eq!(
            execution.result.outcomes[0].requested_action,
            "move_to_recycle_bin"
        );
        assert!(!path.exists(), "recycled file must leave its original path");
        assert!(
            directory
                .path()
                .join(".recycle")
                .join("fixture-recycle.tmp")
                .exists(),
            "RecycleBin mode must preserve the file in the recycle area"
        );
    }

    #[test]
    fn fixture_permanent_mode_removes_without_a_recycle_artifact() {
        use crate::core::filesystem::FixtureFileSystem;
        let directory = tempdir().unwrap();
        let path = directory.path().join("fixture-permanent.tmp");
        std::fs::write(&path, b"delete-me").unwrap();
        let target = scanned_target(&path, "temp", true);
        let plan = create_plan_with_mode(
            &scan_with(vec![target.clone()]),
            std::slice::from_ref(&target.target_id),
            CleanupDeletionMode::Permanent,
        )
        .unwrap();
        let journal = OperationJournal::open_at(&directory.path().join("journal.db")).unwrap();
        journal.begin_confirmed_execution(&plan).unwrap();
        let filesystem = FixtureFileSystem::new(directory.path()).unwrap();

        let (execution, _) = execute_with_recycle_bin_journaled_with_filesystem(
            &plan,
            &AtomicBool::new(false),
            &allow(&plan),
            &filesystem,
            &journal,
        )
        .unwrap();

        assert_eq!(
            execution.result.outcomes[0].requested_action,
            "delete_permanently"
        );
        assert!(!path.exists());
        assert!(
            !directory.path().join(".recycle").exists(),
            "permanent mode must not create a recycle artifact"
        );
    }

    fn sandbox_environment(
        sandbox: &tempfile::TempDir,
    ) -> crate::core::safe_paths::CleanupEnvironment {
        crate::core::safe_paths::CleanupEnvironment {
            user_profile: sandbox.path().join("Users/Alice"),
            system_root: sandbox.path().join("Windows"),
            local_app_data: sandbox.path().join("Users/Alice/AppData/Local"),
            temp: sandbox.path().join("Users/Alice/AppData/Local/Temp"),
        }
    }

    fn write_candidate(path: &Path) -> String {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"cleanup-candidate").unwrap();
        path.to_string_lossy().into_owned()
    }

    /// Proves scan, plan, and execution agree on the same authorization model
    /// for every advertised cleanup category, using real scanner output.
    #[test]
    fn production_scan_plan_execute_agrees_across_cleanup_categories() {
        use crate::core::safe_paths::SafePaths;

        let sandbox = tempdir().unwrap();
        let environment = sandbox_environment(&sandbox);
        std::fs::create_dir_all(&environment.user_profile).unwrap();
        std::fs::create_dir_all(&environment.temp).unwrap();
        let safe_paths = SafePaths::from_environment(&environment);

        let temp_file = environment.temp.join("junk.tmp");
        let prefetch_file = environment.system_root.join("Prefetch/APP.pf");
        let update_file = environment
            .system_root
            .join("SoftwareDistribution/Download/pkg.tmp");
        let log_file = environment.system_root.join("Logs/CBS/cbs.log");
        let browser_profile = environment
            .local_app_data
            .join("Google/Chrome/User Data/Profile 1");
        let browser_cache = browser_profile.join("Cache");
        let browser_file = browser_cache.join("data_0");

        let candidates = [
            write_candidate(&temp_file),
            write_candidate(&prefetch_file),
            write_candidate(&update_file),
            write_candidate(&log_file),
            write_candidate(&browser_file),
        ];

        let mut items = Vec::new();
        items.extend(
            crate::cleaners::temp_files::scan_dirs(
                vec![
                    (
                        environment.temp.to_string_lossy().into_owned(),
                        "temp",
                        "User temporary files",
                    ),
                    (
                        environment
                            .system_root
                            .join("Prefetch")
                            .to_string_lossy()
                            .into_owned(),
                        "prefetch",
                        "Optional Prefetch space cleanup",
                    ),
                ],
                &safe_paths,
            )
            .unwrap(),
        );
        items.extend(
            crate::cleaners::windows_update::scan_dirs(vec![(
                environment
                    .system_root
                    .join("SoftwareDistribution/Download"),
                "Windows Update cache (Download folder)",
            )])
            .unwrap(),
        );
        items.extend(
            crate::cleaners::system_logs::scan_dirs(vec![(
                environment.system_root.join("Logs"),
                "Windows log files",
            )])
            .unwrap(),
        );
        items.extend(
            crate::cleaners::browser_cache::scan(&[crate::cleaners::browser_cache::BrowserInfo {
                name: "Google Chrome (Profile 1)".into(),
                cache_path: Some(browser_cache.to_string_lossy().into_owned()),
                is_running: false,
            }])
            .unwrap(),
        );

        let mut targets = Vec::new();
        let mut failures = Vec::new();
        collect_scan(
            "sandbox",
            Ok(items),
            &safe_paths,
            &mut targets,
            &mut failures,
        );
        assert!(failures.is_empty(), "scanner failures: {failures:?}");
        assert_eq!(
            targets.len(),
            5,
            "every advertised cleanup category must survive scan authorization: {targets:#?}"
        );

        let selected = targets
            .iter()
            .map(|target| target.target_id.clone())
            .collect::<Vec<_>>();
        let plan = create_plan_with_mode(
            &scan_with(targets),
            &selected,
            CleanupDeletionMode::Permanent,
        )
        .unwrap();

        for planned in &plan.targets {
            let outcome = execute_target(planned, CleanupDeletionMode::Permanent, &safe_paths);
            assert_eq!(
                outcome.execution,
                ExecutionStatus::Changed,
                "execution refused a legitimate candidate: {:?}",
                outcome.error
            );
            assert_eq!(outcome.verification, VerificationStatus::Verified);
        }
        for candidate in candidates {
            let path = Path::new(&candidate);
            assert!(!path.exists(), "{candidate} should have been removed");
        }
    }

    /// Proves the execution-side authorization boundary refuses protected
    /// neighbors even when a plan somehow contains them.
    #[test]
    fn execution_authorization_refuses_protected_neighbors() {
        use crate::core::safe_paths::SafePaths;

        let sandbox = tempdir().unwrap();
        let environment = sandbox_environment(&sandbox);
        std::fs::create_dir_all(&environment.user_profile).unwrap();
        let safe_paths = SafePaths::from_environment(&environment);

        let protected = [
            environment.system_root.join("System32/kernel32.dll"),
            environment.system_root.join("notepad.exe"),
            environment
                .local_app_data
                .join("Google/Chrome/User Data/Default/Preferences"),
        ];

        for path in protected {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"protected").unwrap();
            let target = scanned_target(&path, "temp", false);
            let scan = scan_with(vec![target.clone()]);
            let planned = create_plan_with_mode(
                &scan,
                std::slice::from_ref(&target.target_id),
                CleanupDeletionMode::Permanent,
            )
            .unwrap()
            .targets
            .remove(0);

            let outcome = execute_target(&planned, CleanupDeletionMode::Permanent, &safe_paths);

            assert_eq!(
                outcome.error.as_ref().map(|error| error.code),
                Some(OperationErrorCode::TargetProtected),
                "{} must be refused by execution authorization",
                path.display()
            );
            assert!(path.exists(), "{} must not be deleted", path.display());
        }
    }

    /// An approved root passes the prefix check (starts_with is true for the
    /// root itself), so execution must refuse directory targets explicitly
    /// instead of relying on the OS refusing to open a directory for deletion.
    #[test]
    fn execution_never_deletes_an_approved_root_directory() {
        use crate::core::safe_paths::SafePaths;

        let sandbox = tempdir().unwrap();
        let environment = sandbox_environment(&sandbox);
        std::fs::create_dir_all(&environment.user_profile).unwrap();
        std::fs::create_dir_all(&environment.temp).unwrap();
        let safe_paths = SafePaths::from_environment(&environment);

        let root = environment.temp.clone();
        let canary = root.join("canary.tmp");
        std::fs::write(&canary, b"canary").unwrap();

        let target = scanned_target(&root, "temp", false);
        let scan = scan_with(vec![target.clone()]);
        let planned = create_plan_with_mode(
            &scan,
            std::slice::from_ref(&target.target_id),
            CleanupDeletionMode::Permanent,
        )
        .ok()
        .and_then(|plan| plan.targets.into_iter().next());

        if let Some(planned) = planned {
            let outcome = execute_target(&planned, CleanupDeletionMode::Permanent, &safe_paths);
            assert_eq!(
                outcome.error.as_ref().map(|error| error.code),
                Some(OperationErrorCode::TargetProtected),
                "directory targets must be refused explicitly: {outcome:?}"
            );
        }

        assert!(root.is_dir(), "approved root directory must survive");
        assert!(canary.exists(), "files inside the root must survive");
    }
}
