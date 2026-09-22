use crate::*;

struct CleanupCancellationGuard {
    flags: Arc<Mutex<std::collections::HashMap<String, Arc<AtomicBool>>>>,
    operation_id: String,
}

impl Drop for CleanupCancellationGuard {
    fn drop(&mut self) {
        if let Ok(mut flags) = self.flags.lock() {
            flags.remove(&self.operation_id);
        }
    }
}

// ============================================================
// Tauri Commands - Disk Cleaner
// ============================================================

#[tauri::command]
pub(crate) async fn scan_cleanup_targets(
    state: tauri::State<'_, Arc<AppState>>,
) -> std::result::Result<core::cleanup_operation::CleanupScan, core::operation::OperationError> {
    let safe_paths = state.safe_paths.clone();
    let filesystem = state.filesystem.clone();
    let runtime = state.runtime.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if runtime.is_fixture() {
            core::cleanup_operation::scan_fixture(runtime.data_root(), filesystem.as_ref())
        } else {
            core::cleanup_operation::scan(&safe_paths)
        }
    })
    .await
    .map_err(|error| core::operation::OperationError {
        code: core::operation::OperationErrorCode::InternalError,
        message: error.to_string(),
        target_id: None,
        retryable: true,
    })
}

#[tauri::command]
pub(crate) fn create_cleanup_plan(
    scan: core::cleanup_operation::CleanupScan,
    selected_target_ids: Vec<String>,
    deletion_mode: Option<core::cleanup_operation::CleanupDeletionMode>,
) -> std::result::Result<core::operation::OperationPlan, core::operation::OperationError> {
    core::cleanup_operation::create_plan_with_mode(
        &scan,
        &selected_target_ids,
        deletion_mode.unwrap_or_default(),
    )
}

#[tauri::command]
pub(crate) async fn execute_cleanup_plan(
    state: tauri::State<'_, Arc<AppState>>,
    confirmed_plan: core::operation::OperationPlan,
) -> std::result::Result<
    core::cleanup_operation::CleanupExecution,
    core::operation::ContextualOperationError,
> {
    let error_context = core::operation::OperationErrorContext::for_plan(&confirmed_plan);
    let fresh_scan = {
        let safe_paths = state.safe_paths.clone();
        let filesystem = state.filesystem.clone();
        let runtime = state.runtime.clone();
        tauri::async_runtime::spawn_blocking(move || {
            if runtime.is_fixture() {
                core::cleanup_operation::scan_fixture(runtime.data_root(), filesystem.as_ref())
            } else {
                core::cleanup_operation::scan(&safe_paths)
            }
        })
        .await
        .map_err(|error| core::operation::OperationError {
            code: core::operation::OperationErrorCode::InternalError,
            message: error.to_string(),
            target_id: None,
            retryable: true,
        })
        .map_err(|error| error.with_context(error_context.clone()))?
    };
    let (allowed_target_ids, requires_elevation) =
        core::cleanup_operation::validate_authoritative_scan(&confirmed_plan, &fresh_scan)
            .map_err(|error| error.with_context(error_context.clone()))?;
    if requires_elevation && !state.adapters.elevation.is_elevated() {
        return Err(core::operation::OperationError {
            code: core::operation::OperationErrorCode::ElevationRequired,
            message: "Administrator approval is required for the selected cleanup targets.".into(),
            target_id: None,
            retryable: true,
        }
        .with_context(error_context.clone()));
    }
    let journal = state
        .ensure_operation_journal()
        .map_err(|error| error.with_context(error_context.clone()))?;
    journal
        .begin_confirmed_execution(&confirmed_plan)
        .map_err(|error| error.with_context(error_context.clone()))?;
    let operation_id = confirmed_plan.id.0.clone();
    let retry_of_operation_id = confirmed_plan.retry_of_operation_id.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    let recovery_safe_paths = state.safe_paths.clone();
    let recovery_filesystem = state.filesystem.clone();
    let recovery_runtime = state.runtime.clone();
    state
        .cancel_flags
        .lock()
        .map_err(|_| {
            core::operation::OperationError::validation("cleanup cancellation state is unavailable")
                .with_context(error_context.clone())
        })?
        .insert(operation_id.clone(), cancelled.clone());
    let _cancellation_guard = CleanupCancellationGuard {
        flags: state.cancel_flags.clone(),
        operation_id: operation_id.clone(),
    };
    let execution_journal = journal.clone();
    let filesystem = state.filesystem.clone();
    let fixture_mode = state.runtime.is_fixture();
    let execution_allowed_target_ids = allowed_target_ids.clone();
    let execution_safe_paths = state.safe_paths.clone();
    let execution = tauri::async_runtime::spawn_blocking(move || {
        if fixture_mode {
            return core::cleanup_operation::execute_with_recycle_bin_journaled_with_filesystem(
                &confirmed_plan,
                &cancelled,
                &execution_allowed_target_ids,
                filesystem.as_ref(),
                &execution_journal,
            );
        }
        core::cleanup_operation::execute_with_recycle_bin_journaled(
            &confirmed_plan,
            &cancelled,
            &execution_allowed_target_ids,
            &execution_journal,
            &execution_safe_paths,
        )
    })
    .await
    .map_err(|error| {
        core::operation::OperationError {
            code: core::operation::OperationErrorCode::InternalError,
            message: error.to_string(),
            target_id: None,
            retryable: true,
        }
        .with_context(error_context.clone())
    })?;
    state
        .cancel_flags
        .lock()
        .map_err(|_| {
            core::operation::OperationError::validation("cleanup cancellation state is unavailable")
                .with_context(error_context.clone())
        })?
        .remove(&operation_id);
    let (result, record) = execution.map_err(|error: core::operation::OperationError| {
        error.with_context(error_context.clone())
    })?;
    journal
        .append(&record)
        .map_err(|error| error.with_context(error_context.clone()))?;
    if let Some(retry_of_operation_id) = retry_of_operation_id {
        if let Some(previous) = journal
            .list_unresolved_operations()
            .map_err(|error| error.with_context(error_context.clone()))?
            .into_iter()
            .find(|operation| operation.operation_id == retry_of_operation_id)
        {
            let recovery_scan = if recovery_runtime.is_fixture() {
                core::cleanup_operation::scan_fixture(
                    recovery_runtime.data_root(),
                    recovery_filesystem.as_ref(),
                )
            } else {
                core::cleanup_operation::scan(&recovery_safe_paths)
            };
            let reconciliation = core::cleanup_operation::reconciliation_record(
                &previous.confirmed_plan,
                &recovery_scan,
            )
            .map_err(|error| error.with_context(error_context.clone()))?;
            journal
                .append(&reconciliation)
                .map_err(|error| error.with_context(error_context.clone()))?;
        }
    }
    state.cache.invalidate("scan_temp_files");
    state.cache.invalidate("scan_browser_cache");
    state.cache.invalidate("scan_windows_update_cache");
    state.cache.invalidate("scan_system_logs");
    Ok(result)
}

#[tauri::command]
pub(crate) fn cancel_cleanup(
    state: tauri::State<'_, Arc<AppState>>,
    operation_id: String,
) -> std::result::Result<(), core::operation::OperationError> {
    let flags = state.cancel_flags.lock().map_err(|_| {
        core::operation::OperationError::validation("cleanup cancellation state is unavailable")
    })?;
    let flag = flags.get(&operation_id).ok_or_else(|| {
        core::operation::OperationError::validation("cleanup operation is not running")
    })?;
    flag.store(true, Ordering::SeqCst);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_guard_removes_flag_on_normal_error_and_join_paths() {
        for path in ["normal", "error", "join"] {
            let flags = Arc::new(Mutex::new(std::collections::HashMap::new()));
            flags
                .lock()
                .unwrap()
                .insert(path.into(), Arc::new(AtomicBool::new(false)));
            {
                let _guard = CleanupCancellationGuard {
                    flags: flags.clone(),
                    operation_id: path.into(),
                };
                let outcome: std::result::Result<(), &str> = match path {
                    "normal" => Ok(()),
                    "error" => Err("execution failed"),
                    _ => Err("blocking task join failed"),
                };
                let _ = outcome;
            }
            assert!(flags.lock().unwrap().is_empty(), "stale {path} flag");
        }
    }

    #[test]
    fn fixture_browser_detection_returns_only_the_fixture_browser() {
        let fixture_root = tempfile::tempdir().expect("fixture root");
        let runtime = core::runtime::RuntimeContext::fixture(fixture_root.path());

        let browsers = fixture_browsers(runtime.data_root());

        assert_eq!(browsers.len(), 1);
        assert_eq!(browsers[0].name, "Fixture Browser");
        let expected_cache_path = runtime
            .data_root()
            .join("cleanup")
            .join("browser-cache")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            browsers[0].cache_path.as_deref(),
            Some(expected_cache_path.as_str())
        );
        assert!(!browsers[0].is_running);
    }
}

#[tauri::command]
pub(crate) async fn scan_temp_files(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<cleaners::CleanItem>> {
    // Return cached results when a previous scan is still fresh.
    if let Some(cached) = state
        .cache
        .get::<Vec<cleaners::CleanItem>>("scan_temp_files")
    {
        return Ok(cached);
    }
    if state.runtime.is_fixture() {
        return Ok(core::cleanup_operation::scan_fixture_items(
            state.runtime.data_root(),
            state.filesystem.as_ref(),
            "temp",
        ));
    }
    let safe_paths = state.safe_paths.clone();
    tauri::async_runtime::spawn_blocking(move || cleaners::temp_files::scan(&safe_paths))
        .await
        .map_err(|e| core::error::WinOptimizerError::Other(e.to_string()))?
}

#[tauri::command]
pub(crate) async fn scan_browser_cache(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<cleaners::CleanItem>> {
    if let Some(cached) = state
        .cache
        .get::<Vec<cleaners::CleanItem>>("scan_browser_cache")
    {
        return Ok(cached);
    }
    if state.runtime.is_fixture() {
        return Ok(core::cleanup_operation::scan_fixture_items(
            state.runtime.data_root(),
            state.filesystem.as_ref(),
            "browser_cache",
        ));
    }
    tauri::async_runtime::spawn_blocking(|| {
        let browsers = cleaners::browser_cache::detect_browsers()?;
        cleaners::browser_cache::scan(&browsers)
    })
    .await
    .map_err(|e| core::error::WinOptimizerError::Other(e.to_string()))?
}

fn fixture_browsers(data_root: &std::path::Path) -> Vec<cleaners::browser_cache::BrowserInfo> {
    vec![cleaners::browser_cache::BrowserInfo {
        name: "Fixture Browser".into(),
        cache_path: Some(
            data_root
                .join("cleanup")
                .join("browser-cache")
                .to_string_lossy()
                .into_owned(),
        ),
        is_running: false,
    }]
}

#[tauri::command]
pub(crate) async fn detect_browsers(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<cleaners::browser_cache::BrowserInfo>> {
    if state.runtime.is_fixture() {
        return Ok(fixture_browsers(state.runtime.data_root()));
    }
    tauri::async_runtime::spawn_blocking(cleaners::browser_cache::detect_browsers)
        .await
        .map_err(|e| core::error::WinOptimizerError::Other(e.to_string()))?
}

#[tauri::command]
pub(crate) async fn scan_windows_update_cache(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<cleaners::CleanItem>> {
    if let Some(cached) = state
        .cache
        .get::<Vec<cleaners::CleanItem>>("scan_windows_update_cache")
    {
        return Ok(cached);
    }
    if state.runtime.is_fixture() {
        return Ok(core::cleanup_operation::scan_fixture_items(
            state.runtime.data_root(),
            state.filesystem.as_ref(),
            "update_cache",
        ));
    }
    tauri::async_runtime::spawn_blocking(cleaners::windows_update::scan)
        .await
        .map_err(|e| core::error::WinOptimizerError::Other(e.to_string()))?
}

#[tauri::command]
pub(crate) async fn scan_system_logs(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<cleaners::CleanItem>> {
    if let Some(cached) = state
        .cache
        .get::<Vec<cleaners::CleanItem>>("scan_system_logs")
    {
        return Ok(cached);
    }
    if state.runtime.is_fixture() {
        return Ok(core::cleanup_operation::scan_fixture_items(
            state.runtime.data_root(),
            state.filesystem.as_ref(),
            "logs",
        ));
    }
    tauri::async_runtime::spawn_blocking(cleaners::system_logs::scan)
        .await
        .map_err(|e| core::error::WinOptimizerError::Other(e.to_string()))?
}
