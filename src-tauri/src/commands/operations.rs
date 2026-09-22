use crate::*;

// ============================================================
// Application Entry Point
// ============================================================

#[tauri::command]
pub(crate) fn get_operation_kernel_contract(
) -> std::result::Result<SyntheticOperationContract, core::operation::OperationError> {
    core::operation::synthetic_operation_contract()
}

#[tauri::command]
pub(crate) async fn get_operation_journal(
    state: tauri::State<'_, Arc<AppState>>,
) -> std::result::Result<Vec<OperationJournalRecord>, core::operation::OperationError> {
    let journal = state.ensure_operation_journal()?;
    tauri::async_runtime::spawn_blocking(move || journal.list())
        .await
        .map_err(|error| core::operation::OperationError {
            code: core::operation::OperationErrorCode::InternalError,
            message: error.to_string(),
            target_id: None,
            retryable: true,
        })?
}

#[tauri::command]
pub(crate) async fn get_unresolved_operations(
    state: tauri::State<'_, Arc<AppState>>,
) -> std::result::Result<
    Vec<core::operation_journal::UnresolvedOperation>,
    core::operation::OperationError,
> {
    let journal = state.ensure_operation_journal()?;
    tauri::async_runtime::spawn_blocking(move || journal.list_unresolved_operations())
        .await
        .map_err(|error| core::operation::OperationError {
            code: core::operation::OperationErrorCode::InternalError,
            message: error.to_string(),
            target_id: None,
            retryable: true,
        })?
}

#[tauri::command]
pub(crate) async fn create_retry_plan(
    state: tauri::State<'_, Arc<AppState>>,
    operation_id: String,
    target_ids: Vec<String>,
) -> std::result::Result<core::operation::OperationPlan, core::operation::OperationError> {
    if !state.runtime.allows_host_operations() {
        return Err(core::operation::OperationError::validation(
            "operation retry is unavailable in fixture runtime",
        ));
    }
    let journal = state.ensure_operation_journal()?;
    let safe_paths = state.safe_paths.clone();
    let filesystem = state.filesystem.clone();
    let runtime = state.runtime.clone();
    let registry = state.adapters.registry.clone();
    let services = state.adapters.services.clone();
    let tasks = state.adapters.tasks.clone();
    let startup_programs = state.adapters.startup_programs.clone();
    let settings_path = state.runtime.paths.settings.clone();
    let requested_operation_id = operation_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let previous = journal
            .list_unresolved_operations()?
            .into_iter()
            .find(|operation| operation.operation_id.0 == requested_operation_id)
            .ok_or_else(|| {
                core::operation::OperationError::validation(
                    "the operation is no longer unresolved or was not found",
                )
            })?;
        if target_ids.is_empty() {
            return Err(core::operation::OperationError::validation(
                "select at least one unresolved target",
            ));
        }
        let unresolved_target_ids = previous
            .targets
            .iter()
            .map(|target| target.target_id.clone())
            .collect::<Vec<_>>();
        if target_ids
            .iter()
            .any(|target_id| !unresolved_target_ids.contains(target_id))
        {
            return Err(core::operation::OperationError::validation(
                "retry selection contains a target that is not unresolved",
            ));
        }
        if unresolved_target_ids.is_empty() {
            return Err(core::operation::OperationError::validation(
                "no unresolved targets remain for a new plan",
            ));
        }
        match previous.confirmed_plan.descriptor.id.0.as_str() {
            "cleanup.reference_slice" => {
                let scan = if runtime.is_fixture() {
                    core::cleanup_operation::scan_fixture(runtime.data_root(), filesystem.as_ref())
                } else {
                    core::cleanup_operation::scan(&safe_paths)
                };
                core::cleanup_operation::create_retry_plan(
                    &scan,
                    &previous.confirmed_plan,
                    &target_ids,
                )
            }
            "environment.change" => {
                let current = core::env_operation::list_from_adapter(registry.as_ref())?;
                core::env_operation::create_retry_plan(
                    &current,
                    &previous.confirmed_plan,
                    &target_ids,
                )
            }
            "startup.disable_program" | "startup.enable_program" => {
                let current = startup_programs.list().map_err(|error| {
                    core::operation::OperationError::validation(error.to_string())
                })?;
                core::startup_operation::create_retry_plan(
                    &current,
                    &previous.confirmed_plan,
                    &target_ids,
                )
            }
            "hosts.save" => {
                let current = crate::tweaks::hosts::snapshot().map_err(|error| {
                    core::operation::OperationError {
                        code: core::operation::OperationErrorCode::InternalError,
                        message: format!("failed to rediscover hosts file: {error}"),
                        target_id: None,
                        retryable: true,
                    }
                })?;
                core::hosts_operation::create_retry_plan(
                    &current,
                    &previous.confirmed_plan,
                    &target_ids,
                )
            }
            "startup.service_change" => {
                let current = services
                    .list()
                    .map(|items| items.into_iter().map(Into::into).collect::<Vec<_>>())
                    .map_err(|error| {
                        core::operation::OperationError::validation(error.to_string())
                    })?;
                core::service_operation::create_retry_plan(
                    &current,
                    &previous.confirmed_plan,
                    &target_ids,
                )
            }
            "startup.services_guidance" => core::service_operation::create_guidance_retry_plan(
                &previous.confirmed_plan,
                &target_ids,
            ),
            "startup.scheduled_task_change" => {
                let current = tasks
                    .list()
                    .map(|items| items.into_iter().map(Into::into).collect::<Vec<_>>())
                    .map_err(|error| {
                        core::operation::OperationError::validation(error.to_string())
                    })?;
                core::scheduled_task_operation::create_retry_plan(
                    &current,
                    &previous.confirmed_plan,
                    &target_ids,
                )
            }
            "startup.scheduled_tasks_guidance" => {
                let current = tasks
                    .list()
                    .map(|items| items.into_iter().map(Into::into).collect::<Vec<_>>())
                    .map_err(|error| {
                        core::operation::OperationError::validation(error.to_string())
                    })?;
                core::scheduled_task_operation::create_guidance_retry_plan(
                    &current,
                    &previous.confirmed_plan,
                    &target_ids,
                )
            }
            "startup.services_set_manual" => {
                let current = services
                    .list()
                    .map(|items| items.into_iter().map(Into::into).collect::<Vec<_>>())
                    .map_err(|error| {
                        core::operation::OperationError::validation(error.to_string())
                    })?;
                core::services_bulk_operation::create_retry_plan(
                    &current,
                    &previous.confirmed_plan,
                    &target_ids,
                )
            }
            "appx.policy_change" => {
                let current_policy = crate::tweaks::appx::policy_snapshot().map_err(|message| {
                    core::operation::OperationError {
                        code: core::operation::OperationErrorCode::InternalError,
                        message: format!("failed to rediscover AppX policy: {message}"),
                        target_id: None,
                        retryable: true,
                    }
                })?;
                let current_candidates = crate::tweaks::appx::list_candidates();
                core::appx_operation::create_retry_plan(
                    &current_policy,
                    &current_candidates,
                    &previous.confirmed_plan,
                    &target_ids,
                )
            }
            "system_tweak.change" => {
                let labs_enabled = core::settings::SettingsStore::at(&settings_path)
                    .load()
                    .map(|settings| settings.labs_enabled)
                    .unwrap_or(false);
                let current = core::system_tweak_operation::list_targets_with_labs(labs_enabled);
                core::system_tweak_operation::create_retry_plan(
                    &current,
                    &previous.confirmed_plan,
                    &target_ids,
                )
            }
            descriptor_id if descriptor_id.starts_with("preset.") => {
                let current = core::system_tweak_operation::list_targets();
                core::preset_operation::create_retry_plan(
                    &current,
                    &previous.confirmed_plan,
                    &target_ids,
                )
            }
            "process.terminate" => {
                let current = crate::processes::snapshots::snapshot();
                core::process_operation::create_retry_plan(
                    &current,
                    &previous.confirmed_plan,
                    &target_ids,
                )
            }
            "repair.verified_diagnostic" | "repair.maintenance" => {
                let target_id = target_ids.first().ok_or_else(|| {
                    core::operation::OperationError::validation(
                        "repair retry requires exactly one target",
                    )
                })?;
                let current = core::repair_operation::find_target(target_id)?;
                core::repair_operation::create_retry_plan(
                    &current,
                    &previous.confirmed_plan,
                    &target_ids,
                )
            }
            _ => Err(core::operation::OperationError::validation(
                "retry requires domain-specific rediscovery for this operation",
            )),
        }
    })
    .await
    .map_err(|error| core::operation::OperationError {
        code: core::operation::OperationErrorCode::InternalError,
        message: error.to_string(),
        target_id: Some(operation_id),
        retryable: true,
    })?
}

#[tauri::command]
pub(crate) async fn export_support_bundle(
    state: tauri::State<'_, Arc<AppState>>,
    options: Option<core::support_bundle::SupportBundleOptions>,
) -> std::result::Result<String, core::operation::OperationError> {
    let journal = state.ensure_operation_journal()?;
    let options = options.unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || {
        core::support_bundle::export(journal.as_ref(), options)
            .map(|path| path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|error| core::operation::OperationError {
        code: core::operation::OperationErrorCode::InternalError,
        message: error.to_string(),
        target_id: None,
        retryable: true,
    })?
}

#[tauri::command]
pub(crate) async fn preview_operation_undo(
    state: tauri::State<'_, Arc<AppState>>,
    journal_id: String,
) -> std::result::Result<UndoPreview, core::operation::OperationError> {
    let journal = state.ensure_operation_journal()?;
    let requested_journal_id = journal_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let record = journal.get(&requested_journal_id)?;
        core::operation_journal::preview_undo(&record)
    })
    .await
    .map_err(|error| core::operation::OperationError {
        code: core::operation::OperationErrorCode::InternalError,
        message: error.to_string(),
        target_id: Some(journal_id),
        retryable: true,
    })?
}

#[tauri::command]
pub(crate) async fn execute_operation_undo(
    state: tauri::State<'_, Arc<AppState>>,
    journal_id: String,
    confirmed_plan: core::operation::OperationPlan,
) -> std::result::Result<OperationJournalRecord, core::operation::ContextualOperationError> {
    let error_context = core::operation::OperationErrorContext::for_plan(&confirmed_plan);
    if !state.runtime.allows_host_operations() {
        return Err(core::operation::OperationError::validation(
            "operation undo is unavailable in fixture runtime",
        )
        .with_context(error_context));
    }
    if confirmed_plan.requires_elevation && !state.adapters.elevation.is_elevated() {
        return Err(core::operation::OperationError {
            code: core::operation::OperationErrorCode::ElevationRequired,
            message: "Administrator approval is required to restore this operation.".into(),
            target_id: Some(journal_id.clone()),
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
    let requested_journal_id = journal_id.clone();
    let adapters = state.adapters.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let source = journal.get(&requested_journal_id)?;
        let undo_record =
            execute_captured_undo_confirmed(&source, &confirmed_plan, &journal, &adapters)?;
        journal.append(&undo_record)?;
        Ok(undo_record)
    })
    .await
    .map_err(|error| {
        core::operation::OperationError {
            code: core::operation::OperationErrorCode::InternalError,
            message: error.to_string(),
            target_id: Some(journal_id),
            retryable: true,
        }
        .with_context(error_context.clone())
    })?
    .map_err(|error: core::operation::OperationError| error.with_context(error_context))
}
