//! WinOptimizer - Windows system optimization utility.
//!
//! This library registers all Tauri IPC commands exposed to the frontend.
//! Each module handles a specific optimization domain.

pub mod analyzer;
pub mod capabilities;
pub mod cleaners;
pub mod core;
pub mod processes;
pub mod repair;
pub mod startup;
pub mod system_info;
pub mod tweaks;

use core::adapters::Adapters;
use core::backup::BackupManager;
use core::cache::CacheStore;
use core::error::Result;
use core::filesystem::{FileSystemAdapter, FixtureFileSystem, NativeFileSystem};
use core::operation::{OperationError, SyntheticOperationContract};
use core::operation_journal::{OperationJournal, OperationJournalRecord, UndoPreview};
use core::runtime::RuntimeContext;
use core::safe_paths::SafePaths;
use core::startup_recovery::StartupRecoveryStore;
use core::undo_log::UndoLog;

use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};
use tauri::Emitter;

/// Progress event emitted during long-running operations.
#[derive(Debug, Clone, Serialize)]
pub struct ProgressEvent {
    pub operation: String,
    pub phase: String, // "started", "progress", "completed", "error"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percent: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_processed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_total: Option<u64>,
}

/// Application state shared across all Tauri commands.
pub struct AppState {
    pub runtime: Arc<RuntimeContext>,
    /// Retained legacy history is opened lazily, read-only, and only when its
    /// database already exists. Initialization failures are retained.
    undo_log: OnceLock<std::result::Result<Option<Arc<UndoLog>>, OperationError>>,
    /// Current Startup Manager recovery records are initialized only when the
    /// Startup Manager is used. Existing UndoLog rows remain read-compatible.
    startup_recovery: OnceLock<std::result::Result<Arc<StartupRecoveryStore>, OperationError>>,
    operation_journal:
        OnceLock<std::result::Result<Arc<OperationJournal>, core::operation::OperationError>>,
    pub backup_mgr: Arc<BackupManager>,
    pub filesystem: Arc<dyn FileSystemAdapter>,
    pub adapters: Adapters,
    pub safe_paths: Arc<SafePaths>,
    pub session_id: String,
    /// Cancel flags for long-running operations. Keyed by operation name.
    pub cancel_flags: Arc<Mutex<std::collections::HashMap<String, Arc<AtomicBool>>>>,
    /// Short-lived cache for expensive read-only queries (30s TTL).
    pub cache: CacheStore,
    /// The most recent disk snapshots stay in memory so navigation does not
    /// reread and reparse a complete volume index for every click.
    pub disk_snapshots:
        Arc<Mutex<std::collections::HashMap<String, Arc<analyzer::snapshot::DiskSnapshotIndex>>>>,
}

impl AppState {
    fn ensure_operation_journal(
        &self,
    ) -> std::result::Result<Arc<OperationJournal>, core::operation::OperationError> {
        self.operation_journal
            .get_or_init(|| {
                OperationJournal::open_at(&self.runtime.paths.journal)
                    .map(Arc::new)
                    .map_err(operation_journal_open_error)
            })
            .clone()
    }

    /// Read retained legacy history when its database already exists.
    fn ensure_undo_log(&self) -> std::result::Result<Option<Arc<UndoLog>>, OperationError> {
        self.undo_log
            .get_or_init(|| {
                UndoLog::open_existing_at(&self.runtime.paths.undo_log())
                    .map(|log| log.map(Arc::new))
                    .map_err(undo_log_open_error)
            })
            .clone()
    }

    fn ensure_startup_recovery(
        &self,
    ) -> std::result::Result<&Arc<StartupRecoveryStore>, OperationError> {
        self.startup_recovery
            .get_or_init(|| {
                let store = StartupRecoveryStore::open_at(&self.runtime.paths.startup_recovery())
                    .map_err(startup_recovery_open_error)?;

                // Preserve existing startup recovery without creating the
                // legacy database when it does not already exist.
                let legacy_path = self.runtime.paths.undo_log();
                if legacy_path.is_file() {
                    let legacy = UndoLog::open_read_only_at(&legacy_path)
                        .map_err(startup_recovery_open_error)?;
                    let imported = store
                        .import_legacy_entries(
                            &legacy.list().map_err(startup_recovery_open_error)?,
                        )
                        .map_err(startup_recovery_open_error)?;
                    if imported > 0 {
                        log::info!(
                            "Imported {} retained startup recovery record(s) into the current store",
                            imported
                        );
                    }
                }

                Ok(Arc::new(store))
            })
            .as_ref()
            .map_err(|error| error.clone())
    }
}

fn operation_journal_open_error(error: impl std::fmt::Display) -> core::operation::OperationError {
    core::operation::OperationError {
        code: core::operation::OperationErrorCode::InternalError,
        message: format!(
            "WinOptimizer could not initialize its operation journal. No system changes were made. ({error})"
        ),
        target_id: None,
        retryable: true,
    }
}

fn undo_log_open_error(error: impl std::fmt::Display) -> OperationError {
    OperationError {
        code: core::operation::OperationErrorCode::InternalError,
        message: format!(
            "WinOptimizer could not initialize its undo log. No system changes were made. ({error})"
        ),
        target_id: None,
        retryable: true,
    }
}

fn startup_recovery_open_error(error: impl std::fmt::Display) -> OperationError {
    OperationError {
        code: core::operation::OperationErrorCode::InternalError,
        message: format!(
            "WinOptimizer could not initialize Startup Manager recovery. No system changes were made. ({error})"
        ),
        target_id: None,
        retryable: true,
    }
}

#[cfg(test)]
fn open_operation_journal_at(
    path: &std::path::Path,
) -> std::result::Result<Arc<OperationJournal>, core::operation::OperationError> {
    OperationJournal::open_at(path)
        .map(Arc::new)
        .map_err(operation_journal_open_error)
}

/// Run the read-only disk scanner without starting a WebView window.
///
/// This is intentionally a narrow internal entry point for the disposable VM
/// performance comparison, where the benchmark process itself runs elevated.
pub fn run_disk_benchmark(root: &str, report_path: &str) -> i32 {
    use std::time::Instant;

    let started = Instant::now();
    let cancel = Arc::new(AtomicBool::new(false));
    let progress = Arc::new(AtomicU64::new(0));
    let tier = analyzer::disk_usage::StorageTier::from_mount_point(root);
    let scan = analyzer::disk_usage::scan_fresh(root, 64, cancel, progress, tier);
    let elapsed_ms = started.elapsed().as_millis() as u64;

    let (exit_code, report) = match scan {
        Ok(mut output) => {
            output.result.diagnostics.elapsed_ms = elapsed_ms;
            (
                0,
                serde_json::json!({
                    "elevated": core::elevation::is_elevated(),
                    "volume": root,
                    "cache_policy": "bypass",
                    "wall_ms": elapsed_ms,
                    "result": output.result,
                }),
            )
        }
        Err(error) => (
            1,
            serde_json::json!({
                "elevated": core::elevation::is_elevated(),
                "volume": root,
                "wall_ms": elapsed_ms,
                "error": error,
            }),
        ),
    };

    let report_path = std::path::Path::new(report_path);
    if let Some(parent) = report_path.parent() {
        if let Err(error) = std::fs::create_dir_all(parent) {
            eprintln!("Failed to create benchmark report directory: {}", error);
            return 1;
        }
    }
    if let Err(error) = std::fs::write(
        report_path,
        serde_json::to_vec_pretty(&report).expect("benchmark report is serializable"),
    ) {
        eprintln!("Failed to write benchmark report: {}", error);
        return 1;
    }

    exit_code
}

pub(crate) mod commands;
use capabilities::get_capability_manifest;
use commands::appx::*;
use commands::cleanup::*;
use commands::disk_analysis::*;
use commands::elevation::*;
use commands::environment::*;
use commands::history::*;
use commands::hosts::*;
use commands::network::*;
use commands::operations::*;
use commands::performance_experiments::*;
use commands::processes::*;
use commands::repair::*;
use commands::settings::*;
use commands::startup::*;
use commands::system::*;
use commands::system_tweaks::*;
use commands::transaction::*;
use commands::update::*;

fn execute_captured_undo_confirmed(
    source: &OperationJournalRecord,
    confirmed_plan: &core::operation::OperationPlan,
    journal: &core::operation_journal::OperationJournal,
    adapters: &core::adapters::Adapters,
) -> std::result::Result<OperationJournalRecord, core::operation::OperationError> {
    if source.confirmed_plan.descriptor.id.0 == "system_tweak.change"
        || source.confirmed_plan.descriptor.id.0.starts_with("preset.")
    {
        let preview = core::operation_journal::preview_undo(source)?;
        if preview.plan != *confirmed_plan {
            return Err(core::operation::OperationError::validation(
                "confirmed undo plan does not match the preview",
            ));
        }
        let outcomes = source
            .captured_original_states
            .iter()
            .map(|capture| {
                let target_id = capture.target_id().to_owned();
                let payload = serde_json::json!({
                    "target_id": target_id,
                    "operation": "restore_captured_original_state",
                })
                .to_string();
                journal.record_target_state(
                    &confirmed_plan.id,
                    &target_id,
                    core::operation::TargetExecutionState::Ready,
                    &payload,
                )?;
                journal.record_target_state(
                    &confirmed_plan.id,
                    &target_id,
                    core::operation::TargetExecutionState::MutationStarted,
                    &payload,
                )?;
                match core::system_tweak_operation::restore_captured_state(capture) {
                    Ok(outcome) => {
                        journal.record_target_outcome(&confirmed_plan.id, &outcome)?;
                        Ok(outcome)
                    }
                    Err(error) => {
                        let failure = core::operation::TargetOutcome {
                            target_id,
                            requested_action: "restore_captured_original_state".into(),
                            before: None,
                            after: None,
                            execution: core::operation::ExecutionStatus::Failed,
                            verification: core::operation::VerificationStatus::Failed,
                            error: Some(error.clone()),
                        };
                        journal.record_target_outcome(&confirmed_plan.id, &failure)?;
                        Err(error)
                    }
                }
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        core::operation_journal::finish_captured_undo(source, confirmed_plan, outcomes)
    } else if source.confirmed_plan.descriptor.id.0 == "environment.change" {
        let preview = core::operation_journal::preview_undo(source)?;
        if preview.plan != *confirmed_plan {
            return Err(core::operation::OperationError::validation(
                "confirmed undo plan does not match the preview",
            ));
        }
        let outcomes = source
            .captured_original_states
            .iter()
            .map(|capture| {
                let target_id = capture.target_id().to_owned();
                let payload = serde_json::json!({
                    "target_id": target_id,
                    "operation": "restore_captured_original_state",
                })
                .to_string();
                journal.record_target_state(
                    &confirmed_plan.id,
                    &target_id,
                    core::operation::TargetExecutionState::Ready,
                    &payload,
                )?;
                journal.record_target_state(
                    &confirmed_plan.id,
                    &target_id,
                    core::operation::TargetExecutionState::MutationStarted,
                    &payload,
                )?;
                match core::env_operation::restore_captured_state_with_adapter(
                    capture,
                    adapters.registry.as_ref(),
                ) {
                    Ok(outcome) => {
                        journal.record_target_outcome(&confirmed_plan.id, &outcome)?;
                        Ok(outcome)
                    }
                    Err(error) => {
                        let failure = core::operation::TargetOutcome {
                            target_id,
                            requested_action: "restore_captured_original_state".into(),
                            before: None,
                            after: None,
                            execution: core::operation::ExecutionStatus::Failed,
                            verification: core::operation::VerificationStatus::Failed,
                            error: Some(error.clone()),
                        };
                        journal.record_target_outcome(&confirmed_plan.id, &failure)?;
                        Err(error)
                    }
                }
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        core::operation_journal::finish_captured_undo(source, confirmed_plan, outcomes)
    } else {
        let outcomes = source
            .captured_original_states
            .iter()
            .map(|capture| {
                let target_id = capture.target_id().to_owned();
                let payload = serde_json::json!({
                    "target_id": target_id,
                    "operation": "restore_captured_original_state",
                })
                .to_string();
                journal.record_target_state(
                    &confirmed_plan.id,
                    &target_id,
                    core::operation::TargetExecutionState::Ready,
                    &payload,
                )?;
                journal.record_target_state(
                    &confirmed_plan.id,
                    &target_id,
                    core::operation::TargetExecutionState::MutationStarted,
                    &payload,
                )?;
                let before = match capture {
                    core::operation_journal::CapturedOriginalState::Synthetic { state, .. } => {
                        state.clone()
                    }
                    _ => {
                        return Err(core::operation::OperationError::validation(
                            "this journal phase only executes tested synthetic restorations",
                        ))
                    }
                };
                let outcome = core::operation::TargetOutcome {
                    target_id,
                    requested_action: "restore_captured_original_state".into(),
                    before: None,
                    after: Some(before),
                    execution: core::operation::ExecutionStatus::Changed,
                    verification: core::operation::VerificationStatus::Verified,
                    error: None,
                };
                journal.record_target_outcome(&confirmed_plan.id, &outcome)?;
                Ok(outcome)
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        core::operation_journal::finish_captured_undo(source, confirmed_plan, outcomes)
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
fn create_app_state() -> Arc<AppState> {
    let runtime = RuntimeContext::production()
        .expect("WinOptimizer could not initialize its production data path");
    create_app_state_with_runtime(runtime)
        .expect("WinOptimizer could not initialize its backup data path")
}

fn create_app_state_with_runtime(runtime: RuntimeContext) -> std::io::Result<Arc<AppState>> {
    let filesystem: Arc<dyn FileSystemAdapter> = if runtime.is_fixture() {
        Arc::new(FixtureFileSystem::new(runtime.data_root())?)
    } else {
        Arc::new(NativeFileSystem)
    };
    let adapters = if runtime.is_fixture() {
        Adapters::fixture()
    } else {
        Adapters::native()
    };
    Ok(Arc::new(AppState {
        backup_mgr: Arc::new(BackupManager::new_at(runtime.paths.backups.clone())?),
        filesystem,
        adapters,
        runtime: Arc::new(runtime),
        undo_log: OnceLock::new(),
        startup_recovery: OnceLock::new(),
        operation_journal: OnceLock::new(),
        safe_paths: Arc::new(SafePaths::new()),
        session_id: uuid::Uuid::new_v4().to_string(),
        cancel_flags: Arc::new(Mutex::new(std::collections::HashMap::new())),
        cache: CacheStore::new(30),
        disk_snapshots: Arc::new(Mutex::new(std::collections::HashMap::new())),
    }))
}

fn prepare_app_state(state: &AppState) {
    match state
        .ensure_operation_journal()
        .and_then(|journal| journal.reconcile_unresolved_operations())
    {
        Ok(reconciled) if reconciled > 0 => log::warn!(
            "{} interrupted operation(s) were marked outcome-unknown and require current-state inspection",
            reconciled
        ),
        Ok(_) => {}
        Err(error) => log::error!("Operation journal recovery inspection unavailable: {error}"),
    }
}

pub fn run_elevated_resume() {
    env_logger::init();
    if !core::elevation::is_elevated() {
        log::error!("elevated resume requires an elevated process");
        std::process::exit(1);
    }
    let state = create_app_state();
    prepare_app_state(&state);
    match tauri::async_runtime::block_on(commands::elevation::execute_resumed_operation_with_state(
        state,
    )) {
        Ok(_) => {}
        Err(error) => {
            log::error!("elevated resume failed: {error}");
            std::process::exit(1);
        }
    }
}

pub fn run() {
    let runtime = RuntimeContext::production()
        .expect("WinOptimizer could not initialize its production data path");
    run_with_runtime(runtime);
}

/// Start the application with an explicitly selected runtime.
///
/// Fixture mode swaps host adapters for deterministic implementations while
/// retaining production command registration and the bundled frontend.
pub fn run_with_runtime(runtime: RuntimeContext) {
    let _ = env_logger::try_init();

    // UndoLog is lazily initialized on first use — the OnceLock inside
    // AppState defers SQLite open until the first mutation, so the
    // webview window appears faster.
    let state = create_app_state_with_runtime(runtime)
        .expect("WinOptimizer could not initialize its runtime data path");
    prepare_app_state(&state);

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            // Disk Cleaner
            scan_cleanup_targets,
            create_cleanup_plan,
            execute_cleanup_plan,
            cancel_cleanup,
            scan_temp_files,
            scan_browser_cache,
            detect_browsers,
            scan_windows_update_cache,
            scan_system_logs,
            // Startup Manager
            list_startup_programs,
            create_startup_program_plan,
            execute_startup_program_plan,
            create_startup_program_enable_plan,
            execute_startup_program_enable_plan,
            create_service_plan,
            execute_service_plan,
            create_services_manual_plan,
            execute_services_manual_plan,
            create_services_guidance_plan,
            execute_services_guidance_plan,
            list_services,
            list_startup_tasks,
            scan_scheduled_tasks,
            create_scheduled_task_plan,
            execute_scheduled_task_plan,
            create_scheduled_tasks_guidance_plan,
            execute_scheduled_tasks_guidance_plan,
            // System Tweaks
            list_system_tweak_targets,
            create_system_tweak_plan,
            execute_system_tweak_plan,
            list_tweak_presets,
            create_tweak_preset_plan,
            execute_tweak_preset_plan,
            list_context_menu_entries,
            // Hosts File Editor
            list_hosts_entries,
            create_hosts_plan,
            execute_hosts_plan,
            // AppX De-provisioning
            create_appx_plan,
            execute_appx_plan,
            list_appx_candidates,
            get_appx_policy_status,
            // Disk Analyzer
            scan_disk_usage,
            cancel_scan,
            cancel_repair,
            export_disk_usage_json,
            list_disk_children,
            get_disk_entry_details,
            export_disk_snapshot_json,
            list_largest_disk_files,
            usn_scan_volume,
            usn_find_files,
            // Process Manager
            process_snapshot,
            create_process_plan,
            execute_process_plan,
            // System Info
            get_hardware_info,
            get_os_info,
            get_installed_updates,
            get_battery_info,
            list_env_vars,
            create_env_plan,
            execute_env_plan,
            get_network_info,
            // Undo Log
            get_undo_log,
            // Application Settings
            get_settings,
            update_settings,
            record_telemetry_event,
            check_for_updates,
            install_update,
            // Performance experiments
            list_performance_experiments,
            save_performance_experiment,
            prepare_performance_experiment_revert,
            record_performance_experiment_decision,
            // Elevation
            check_elevated,
            has_resume_operation_args,
            confirm_and_request_elevation,
            execute_resumed_operation,
            // Repair Center
            list_repair_targets,
            list_repair_guides,
            create_repair_plan,
            execute_repair_plan,
            list_repair_actions,
            // Transactional Engine
            preview_transaction,
            backup_registry_key_hive,
            // Shared operation kernel
            get_operation_kernel_contract,
            get_capability_manifest,
            get_operation_journal,
            get_unresolved_operations,
            create_retry_plan,
            export_support_bundle,
            preview_operation_undo,
            execute_operation_undo,
        ])
        .run(tauri::generate_context!())
        .expect("Failed to start WinOptimizer");
}

#[cfg(test)]
mod tauri_integration_tests {
    use super::core::operation::{
        OperationError, OperationErrorCode, OperationErrorContext, OperationErrorPhase, OperationId,
    };
    use super::tweaks;

    #[tauri::command]
    fn structured_operation_error() -> std::result::Result<(), OperationError> {
        Err(OperationError {
            code: OperationErrorCode::TargetProtected,
            message: "Synthetic protected target.".into(),
            target_id: Some("synthetic.protected".into()),
            retryable: false,
        })
    }

    #[test]
    fn operation_error_survives_tauri_ipc_serialization() {
        let ipc_error: tauri::ipc::InvokeError = structured_operation_error().unwrap_err().into();
        let error = ipc_error.0;

        assert_eq!(error["code"], "target_protected");
        assert_eq!(error["category"], "target");
        assert_eq!(error["phase"], "revalidation");
        assert_eq!(error["target_id"], "synthetic.protected");
        assert!(error["operation_id"].is_null());
        assert!(error["native_error_code"].is_null());
        assert_eq!(error["human_message"], "Synthetic protected target.");
        assert_eq!(error["retryable"], false);
        assert_eq!(error["requires_elevation"], false);
        assert_eq!(error["requires_restart"], false);
        assert!(error["diagnostic_context"].is_object());
    }

    #[test]
    fn contextual_operation_error_survives_tauri_ipc_serialization() {
        let ipc_error: tauri::ipc::InvokeError = OperationError {
            code: OperationErrorCode::ExecutionFailed,
            message: "restore failed".into(),
            target_id: None,
            retryable: true,
        }
        .with_context(OperationErrorContext {
            operation_id: Some(OperationId::from("operation.one")),
            phase: Some(OperationErrorPhase::Execution),
            native_error_code: Some(5),
            requires_elevation: Some(true),
            requires_restart: Some(false),
            ..OperationErrorContext::default()
        })
        .into();
        let error = ipc_error.0;

        assert_eq!(error["operation_id"], "operation.one");
        assert!(error["target_id"].is_null());
        assert_eq!(error["phase"], "execution");
        assert_eq!(error["native_error_code"], 5);
        assert_eq!(error["retryable"], true);
        assert_eq!(error["requires_elevation"], true);
        assert_eq!(error["requires_restart"], false);
        assert!(error["diagnostic_context"].is_object());
    }

    #[test]
    fn journal_open_failure_does_not_create_a_fallback_database() {
        let directory = tempfile::tempdir().unwrap();
        let blocker = directory.path().join("not_a_directory");
        std::fs::write(&blocker, b"blocker").unwrap();
        let primary = blocker.join("operations.db");
        let fallback = directory.path().join("operation_journal_fallback.db");

        let error = match super::open_operation_journal_at(&primary) {
            Ok(_) => panic!("journal unexpectedly opened through a blocked parent"),
            Err(error) => error,
        };

        assert_eq!(error.code, OperationErrorCode::InternalError);
        assert!(error.message.contains("No system changes were made"));
        assert!(!fallback.exists());
    }

    /// Test that privacy tweaks list returns data correctly.
    #[test]
    fn list_privacy_tweaks_direct() {
        let tweaks_list = tweaks::privacy::list();
        assert!(
            !tweaks_list.is_empty(),
            "privacy tweaks should not be empty"
        );
        assert!(tweaks_list.iter().any(|t| t.id == "disable_telemetry"));
    }

    /// Test that performance tweaks list returns data correctly.
    #[test]
    fn list_performance_tweaks_direct() {
        let tweaks_list = tweaks::performance::list();
        assert!(
            !tweaks_list.is_empty(),
            "performance tweaks should not be empty"
        );
        assert!(tweaks_list.iter().any(|t| t.id == "adjust_visual_effects"));
    }
}

#[cfg(test)]
mod runtime_context_tests {
    use super::core::runtime::RuntimeContext;
    use super::core::settings::{AppSettings, SettingsStore};
    use super::core::undo_log::UndoLog;
    use super::create_app_state_with_runtime;
    use rusqlite::params;
    use std::fs;
    use std::path::Path;

    #[test]
    fn fixture_cleanup_scan_uses_only_fixture_filesystem() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("fixture");
        let cleanup_file = root.join("cleanup/temp/sample.tmp");
        fs::create_dir_all(cleanup_file.parent().unwrap()).unwrap();
        fs::write(&cleanup_file, b"fixture cleanup").unwrap();
        let state = create_app_state_with_runtime(RuntimeContext::fixture(&root)).unwrap();

        let scan = super::core::cleanup_operation::scan_fixture(
            state.runtime.data_root(),
            state.filesystem.as_ref(),
        );
        assert_eq!(scan.targets.len(), 1);
        assert!(scan.targets[0]
            .resource_id
            .ends_with("fixture\\cleanup\\temp\\sample.tmp"));
        assert!(state
            .adapters
            .registry
            .list("HKCU\\Environment")
            .unwrap()
            .is_empty());
        assert!(state.adapters.elevation.is_elevated());
    }

    #[test]
    fn fixture_app_state_keeps_durable_stores_under_fixture_root() {
        let directory = tempfile::tempdir().unwrap();
        assert_fixture_stores_under(directory.path());
    }

    /// The same check with the base directory given by its 8.3 short name,
    /// as GitHub's Windows runners report their temp directory.
    #[cfg(windows)]
    #[test]
    fn fixture_app_state_accepts_a_short_name_root() {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::GetShortPathNameW;

        let directory = tempfile::tempdir().unwrap();
        let long_base = directory.path().join("winoptimizer long base directory");
        fs::create_dir_all(&long_base).unwrap();
        let mut wide: Vec<u16> = long_base.as_os_str().encode_wide().collect();
        wide.push(0);
        let mut buffer = vec![0u16; 1024];
        let length =
            unsafe { GetShortPathNameW(PCWSTR(wide.as_ptr()), Some(&mut buffer)) } as usize;
        let short_base = std::path::PathBuf::from(String::from_utf16_lossy(&buffer[..length]));
        if length == 0 || length > buffer.len() || short_base == long_base {
            eprintln!("8.3 short names are not available on this volume; skipping");
            return;
        }
        assert_fixture_stores_under(&short_base);
    }

    fn assert_fixture_stores_under(base: &Path) {
        let root = base.join("winoptimizer-fixture");
        let source = base.join("source.txt");
        fs::write(&source, b"fixture backup").unwrap();

        // The runtime canonicalizes the fixture root, so a root given by its
        // 8.3 short name (GitHub's Windows runners report RUNNER~1) comes back
        // in long form.  Compare every store against the root as the runtime
        // spells it.
        fs::create_dir_all(&root).unwrap();
        let root = RuntimeContext::fixture(&root).data_root().to_path_buf();
        let legacy_path = root.join("undo/undo.db");
        fs::create_dir_all(legacy_path.parent().unwrap()).unwrap();
        drop(UndoLog::open_at(&legacy_path).unwrap());
        let legacy_connection = rusqlite::Connection::open(&legacy_path).unwrap();
        legacy_connection
            .execute(
                "INSERT INTO undo_log
                 (id, session_id, kind, target, reversal_command, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    "fixture-legacy-id",
                    "fixture-session",
                    "startup_change",
                    "fixture-target",
                    "{}",
                    "2026-08-14T00:00:00Z",
                ],
            )
            .unwrap();
        drop(legacy_connection);

        let state = create_app_state_with_runtime(RuntimeContext::fixture(&root)).unwrap();

        let journal = state.ensure_operation_journal().unwrap();
        let undo = state.ensure_undo_log().unwrap().unwrap();
        let recovery = state.ensure_startup_recovery().unwrap().clone();
        let backup = state.backup_mgr.backup_file(&source).unwrap();
        let settings = AppSettings::default();
        SettingsStore::at(&state.runtime.paths.settings)
            .save(&settings)
            .unwrap();

        assert_eq!(state.runtime.data_root(), root.as_path());
        assert_eq!(journal.db_path(), &root.join("journal/operations.db"));
        assert_eq!(undo.db_path(), &legacy_path);
        assert_eq!(undo.list().unwrap()[0].id, "fixture-legacy-id");
        assert_eq!(recovery.db_path(), &root.join("startup/recovery.db"));
        assert!(backup.starts_with(root.join("backups")));
        assert_eq!(fs::read(&backup).unwrap(), b"fixture backup");
        assert_eq!(
            SettingsStore::at(root.join("settings.json"))
                .load()
                .unwrap(),
            settings
        );
        assert!(root.join("journal/operations.db").is_file());
        assert!(root.join("startup/recovery.db").is_file());
        assert!(root.join("settings.json").is_file());
        assert!(root.join("undo/undo.db").is_file());
    }
}
