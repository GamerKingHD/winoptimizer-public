use crate::*;

/// Maps a blocking task that panicked or was cancelled to the shared internal
/// error returned by every startup command.
fn internal_task_error(error: impl std::fmt::Display) -> core::operation::OperationError {
    core::operation::OperationError {
        code: core::operation::OperationErrorCode::InternalError,
        message: error.to_string(),
        target_id: None,
        retryable: true,
    }
}

// ============================================================
// Tauri Commands - Startup Manager
// ============================================================

#[tauri::command]
pub(crate) async fn create_startup_program_plan(
    state: tauri::State<'_, Arc<AppState>>,
    program_id: String,
) -> std::result::Result<core::operation::OperationPlan, core::operation::OperationError> {
    let startup_programs = state.adapters.startup_programs.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let program = startup_programs.find_by_id(&program_id).map_err(|error| {
            core::operation::OperationError {
                code: core::operation::OperationErrorCode::TargetNotFound,
                message: error.to_string(),
                target_id: Some(program_id),
                retryable: true,
            }
        })?;
        core::startup_operation::create_plan(&program)
    })
    .await
    .map_err(internal_task_error)?
}

#[tauri::command]
pub(crate) async fn create_startup_program_enable_plan(
    state: tauri::State<'_, Arc<AppState>>,
    program_id: String,
) -> std::result::Result<core::operation::OperationPlan, core::operation::OperationError> {
    let recovery_store = state.ensure_startup_recovery()?.clone();
    let startup_programs = state.adapters.startup_programs.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let program = startup_programs
            .find_by_id_with_recovery(&program_id, &recovery_store)
            .map_err(|error| core::operation::OperationError {
                code: core::operation::OperationErrorCode::TargetNotFound,
                message: error.to_string(),
                target_id: Some(program_id),
                retryable: true,
            })?;
        core::startup_operation::create_enable_plan(&program)
    })
    .await
    .map_err(internal_task_error)?
}

#[tauri::command]
pub(crate) async fn execute_startup_program_plan(
    state: tauri::State<'_, Arc<AppState>>,
    confirmed_plan: core::operation::OperationPlan,
) -> std::result::Result<
    core::startup_operation::StartupExecution,
    core::operation::ContextualOperationError,
> {
    let error_context = core::operation::OperationErrorContext::for_plan(&confirmed_plan);
    let target_id = confirmed_plan
        .targets
        .first()
        .map(|target| target.target_id.clone())
        .ok_or_else(|| core::operation::OperationError::validation("startup plan has no target"))
        .map_err(|error| error.with_context(error_context.clone()))?;
    if confirmed_plan.requires_elevation && !state.adapters.elevation.is_elevated() {
        return Err(core::operation::OperationError {
            code: core::operation::OperationErrorCode::ElevationRequired,
            message: "Administrator approval is required for this startup entry.".into(),
            target_id: Some(target_id),
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
    let recovery_store = state
        .ensure_startup_recovery()
        .map_err(|error| error.with_context(error_context.clone()))?
        .clone();
    let session_id = state.session_id.clone();
    let backup_mgr = state.backup_mgr.clone();
    let startup_programs = state.adapters.startup_programs.clone();
    let journal_for_execution = journal.clone();
    let execution = tauri::async_runtime::spawn_blocking(move || {
        let current = match startup_programs.find_by_id(&target_id) {
            Ok(current) => current,
            Err(error) => {
                let operation_error = core::operation::OperationError {
                    code: core::operation::OperationErrorCode::TargetNotFound,
                    message: error.to_string(),
                    target_id: Some(target_id.clone()),
                    retryable: true,
                };
                core::startup_operation::mark_target_revalidation_failed(
                    &journal_for_execution,
                    &confirmed_plan,
                    &target_id,
                )?;
                return Err(operation_error);
            }
        };
        if let Err(error) = core::startup_operation::validate_current(&confirmed_plan, &current) {
            core::startup_operation::mark_target_revalidation_failed(
                &journal_for_execution,
                &confirmed_plan,
                &target_id,
            )?;
            return Err(error);
        }
        core::startup_operation::mark_target_ready_and_mutation_started(
            &journal_for_execution,
            &confirmed_plan,
            &target_id,
        )?;
        let mutation =
            match startup_programs.disable(&current, &recovery_store, &session_id, &backup_mgr) {
                Ok(mutation) => mutation,
                Err(error) => {
                    let operation_error = core::operation::OperationError {
                        code: core::operation::OperationErrorCode::ExecutionFailed,
                        message: error.to_string(),
                        target_id: Some(target_id.clone()),
                        retryable: true,
                    };
                    let outcome = core::startup_operation::failed_outcome(
                        &current,
                        "disable_startup_program",
                        operation_error.clone(),
                    );
                    core::startup_operation::record_target_outcome(
                        &journal_for_execution,
                        &confirmed_plan,
                        &outcome,
                    )?;
                    return Err(operation_error);
                }
            };
        let execution = match core::startup_operation::finish(&confirmed_plan, &current, mutation) {
            Ok(execution) => execution,
            Err(error) => {
                let outcome = core::startup_operation::failed_outcome(
                    &current,
                    "disable_startup_program",
                    error.clone(),
                );
                core::startup_operation::record_target_outcome(
                    &journal_for_execution,
                    &confirmed_plan,
                    &outcome,
                )?;
                return Err(error);
            }
        };
        let outcome = execution.result.outcomes.first().ok_or_else(|| {
            core::operation::OperationError::validation(
                "startup execution produced no target outcome",
            )
        })?;
        core::startup_operation::record_target_outcome(
            &journal_for_execution,
            &confirmed_plan,
            outcome,
        )?;
        let record = core::startup_operation::journal_record(&confirmed_plan, &current, &execution);
        Ok((execution, record))
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
    })?
    .map_err(|error: core::operation::OperationError| error.with_context(error_context.clone()))?;
    let (execution, record) = execution;
    journal
        .append(&record)
        .map_err(|error| error.with_context(error_context))?;
    state.cache.invalidate("list_startup_programs");
    Ok(execution)
}

pub(crate) fn execute_startup_program_enable_confirmed_with_journal(
    confirmed_plan: core::operation::OperationPlan,
    recovery_store: Arc<core::startup_recovery::StartupRecoveryStore>,
    backup_mgr: Arc<core::backup::BackupManager>,
    journal: Arc<core::operation_journal::OperationJournal>,
    startup_programs: Arc<dyn core::adapters::StartupProgramAdapter>,
) -> std::result::Result<
    (
        core::startup_operation::StartupEnableExecution,
        OperationJournalRecord,
    ),
    core::operation::OperationError,
> {
    let target_id = confirmed_plan
        .targets
        .first()
        .map(|target| target.target_id.clone())
        .ok_or_else(|| {
            core::operation::OperationError::validation("startup enable plan has no target")
        })?;
    let current = match startup_programs.find_by_id_with_recovery(&target_id, &recovery_store) {
        Ok(current) => current,
        Err(error) => {
            let operation_error = core::operation::OperationError {
                code: core::operation::OperationErrorCode::TargetNotFound,
                message: error.to_string(),
                target_id: Some(target_id.clone()),
                retryable: true,
            };
            core::startup_operation::mark_target_revalidation_failed(
                &journal,
                &confirmed_plan,
                &target_id,
            )?;
            return Err(operation_error);
        }
    };
    if let Err(error) = core::startup_operation::validate_enable_current(&confirmed_plan, &current)
    {
        core::startup_operation::mark_target_revalidation_failed(
            &journal,
            &confirmed_plan,
            &target_id,
        )?;
        return Err(error);
    }
    let entry = match startup_programs.find_recovery_entry_for_program(&current, &recovery_store) {
        Ok(entry) => entry,
        Err(error) => {
            let operation_error = core::operation::OperationError {
                code: core::operation::OperationErrorCode::TargetChangedSincePreview,
                message: error.to_string(),
                target_id: Some(target_id.clone()),
                retryable: true,
            };
            core::startup_operation::mark_target_revalidation_failed(
                &journal,
                &confirmed_plan,
                &target_id,
            )?;
            return Err(operation_error);
        }
    };
    let reversal: serde_json::Value = match serde_json::from_str(&entry.reversal_command) {
        Ok(reversal) => reversal,
        Err(error) => {
            let operation_error = core::operation::OperationError {
                code: core::operation::OperationErrorCode::ValidationFailed,
                message: format!("invalid startup recovery record: {error}"),
                target_id: Some(target_id.clone()),
                retryable: false,
            };
            core::startup_operation::mark_target_revalidation_failed(
                &journal,
                &confirmed_plan,
                &target_id,
            )?;
            return Err(operation_error);
        }
    };
    core::startup_operation::mark_target_ready_and_mutation_started(
        &journal,
        &confirmed_plan,
        &target_id,
    )?;
    if let Err(error) = super::history::restore_startup_undo_entry(&backup_mgr, &entry, &reversal) {
        let operation_error = core::operation::OperationError {
            code: core::operation::OperationErrorCode::ExecutionFailed,
            message: error.to_string(),
            target_id: Some(target_id.clone()),
            retryable: true,
        };
        let outcome = core::startup_operation::failed_outcome(
            &current,
            "enable_startup_program",
            operation_error.clone(),
        );
        core::startup_operation::record_target_outcome(&journal, &confirmed_plan, &outcome)?;
        return Err(operation_error);
    }
    let restored = match startup_programs
        .list()
        .map_err(|error| core::operation::OperationError::validation(error.to_string()))?
        .into_iter()
        .find(|program| program.id == current.id && program.enabled)
    {
        Some(restored) => restored,
        None => {
            let operation_error = core::operation::OperationError {
                code: core::operation::OperationErrorCode::VerificationFailed,
                message: "Startup entry restoration could not be verified by re-enumeration."
                    .into(),
                target_id: Some(target_id.clone()),
                retryable: true,
            };
            let outcome = core::startup_operation::failed_outcome(
                &current,
                "enable_startup_program",
                operation_error.clone(),
            );
            core::startup_operation::record_target_outcome(&journal, &confirmed_plan, &outcome)?;
            return Err(operation_error);
        }
    };
    let execution =
        match core::startup_operation::finish_enable(&confirmed_plan, &current, &restored) {
            Ok(execution) => execution,
            Err(error) => {
                let outcome = core::startup_operation::failed_outcome(
                    &current,
                    "enable_startup_program",
                    error.clone(),
                );
                core::startup_operation::record_target_outcome(
                    &journal,
                    &confirmed_plan,
                    &outcome,
                )?;
                return Err(error);
            }
        };
    let outcome = execution.result.outcomes.first().ok_or_else(|| {
        core::operation::OperationError::validation(
            "startup enable execution produced no target outcome",
        )
    })?;
    core::startup_operation::record_target_outcome(&journal, &confirmed_plan, outcome)?;
    if let Err(error) = recovery_store.mark_rolled_back(&entry.id) {
        return Err(core::operation::OperationError {
            code: core::operation::OperationErrorCode::InternalError,
            message: format!("failed to complete startup recovery record: {error}"),
            target_id: Some(target_id.clone()),
            retryable: true,
        });
    }
    let record =
        core::startup_operation::journal_enable_record(&confirmed_plan, &current, &execution);
    Ok((execution, record))
}

#[tauri::command]
pub(crate) async fn execute_startup_program_enable_plan(
    state: tauri::State<'_, Arc<AppState>>,
    confirmed_plan: core::operation::OperationPlan,
) -> std::result::Result<
    core::startup_operation::StartupEnableExecution,
    core::operation::ContextualOperationError,
> {
    let error_context = core::operation::OperationErrorContext::for_plan(&confirmed_plan);
    if confirmed_plan.requires_elevation && !state.adapters.elevation.is_elevated() {
        return Err(core::operation::OperationError {
            code: core::operation::OperationErrorCode::ElevationRequired,
            message: "Administrator approval is required to enable this startup entry.".into(),
            target_id: confirmed_plan
                .targets
                .first()
                .map(|target| target.target_id.clone()),
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
    let recovery_store = state
        .ensure_startup_recovery()
        .map_err(|error| error.with_context(error_context.clone()))?
        .clone();
    let backup_mgr = state.backup_mgr.clone();
    let startup_programs = state.adapters.startup_programs.clone();
    let journal_for_execution = journal.clone();
    let (execution, record) = tauri::async_runtime::spawn_blocking(move || {
        execute_startup_program_enable_confirmed_with_journal(
            confirmed_plan,
            recovery_store,
            backup_mgr,
            journal_for_execution,
            startup_programs,
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
    })?
    .map_err(|error: core::operation::OperationError| error.with_context(error_context.clone()))?;
    journal
        .append(&record)
        .map_err(|error| error.with_context(error_context))?;
    state.cache.invalidate("list_startup_programs");
    Ok(execution)
}

#[tauri::command]
pub(crate) async fn create_service_plan(
    state: tauri::State<'_, Arc<AppState>>,
    service_name: String,
    action: core::service_operation::ServiceAction,
) -> std::result::Result<core::operation::OperationPlan, core::operation::OperationError> {
    let services = state.adapters.services.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let service = services
            .inspect(&service_name)
            .map(Into::into)
            .map_err(|message| core::operation::OperationError {
                code: core::operation::OperationErrorCode::TargetNotFound,
                message: message.to_string(),
                target_id: Some(service_name),
                retryable: true,
            })?;
        core::service_operation::create_plan(&service, action)
    })
    .await
    .map_err(internal_task_error)?
}

#[tauri::command]
pub(crate) async fn create_services_manual_plan(
    state: tauri::State<'_, Arc<AppState>>,
) -> std::result::Result<core::operation::OperationPlan, core::operation::OperationError> {
    let adapter = state.adapters.services.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let services = adapter
            .list()
            .map_err(|error| core::operation::OperationError::validation(error.to_string()))?;
        let services = services.into_iter().map(Into::into).collect::<Vec<_>>();
        core::services_bulk_operation::create_manual_plan(&services)
    })
    .await
    .map_err(internal_task_error)?
}

#[tauri::command]
pub(crate) async fn create_services_guidance_plan(
    targets: Vec<core::services_guidance_operation::ServiceGuidanceTarget>,
) -> std::result::Result<core::operation::OperationPlan, core::operation::OperationError> {
    tauri::async_runtime::spawn_blocking(move || {
        core::services_guidance_operation::create_plan(&targets)
    })
    .await
    .map_err(internal_task_error)?
}

pub(crate) fn execute_service_confirmed_with_journal(
    confirmed_plan: core::operation::OperationPlan,
    journal: Arc<core::operation_journal::OperationJournal>,
    services: Arc<dyn core::adapters::ServiceAdapter>,
) -> std::result::Result<
    (
        core::service_operation::ServiceExecution,
        OperationJournalRecord,
    ),
    core::operation::OperationError,
> {
    let service_name = confirmed_plan
        .targets
        .first()
        .map(|target| target.target_id.clone())
        .ok_or_else(|| core::operation::OperationError::validation("service plan has no target"))?;
    let before = match services.inspect(&service_name).map(|info| {
        let info: crate::core::adapters::ServiceInfo = info;
        crate::startup::services::ServiceInfo::from(info)
    }) {
        Ok(before) => before,
        Err(message) => {
            let error = core::operation::OperationError {
                code: core::operation::OperationErrorCode::TargetNotFound,
                message: message.to_string(),
                target_id: Some(service_name.clone()),
                retryable: true,
            };
            core::service_operation::mark_target_revalidation_failed(
                &journal,
                &confirmed_plan,
                &service_name,
            )?;
            return Err(error);
        }
    };
    let action = match core::service_operation::confirmed_action(&confirmed_plan, &before) {
        Ok(action) => action,
        Err(error) => {
            core::service_operation::mark_target_revalidation_failed(
                &journal,
                &confirmed_plan,
                &service_name,
            )?;
            return Err(error);
        }
    };
    core::service_operation::mark_target_ready_and_mutation_started(
        &journal,
        &confirmed_plan,
        &service_name,
    )?;
    let fail = |error: core::operation::OperationError| {
        let outcome = core::service_operation::failed_outcome(&before, &action, error.clone());
        core::service_operation::record_target_outcome(&journal, &confirmed_plan, &outcome)?;
        Err(error)
    };
    let after = match &action {
        core::service_operation::ServiceAction::Start => {
            if let Err(error) =
                services.control(&service_name, crate::core::adapters::ServiceAction::Start)
            {
                return fail(core::operation::OperationError {
                    code: core::operation::OperationErrorCode::ExecutionFailed,
                    message: error.to_string(),
                    target_id: Some(service_name.clone()),
                    retryable: true,
                });
            }
            if let Err(error) = services.wait_for_status(&service_name, "Running", 20) {
                return fail(core::operation::OperationError {
                    code: core::operation::OperationErrorCode::VerificationFailed,
                    message: error.to_string(),
                    target_id: Some(service_name.clone()),
                    retryable: true,
                });
            }
            let info: crate::core::adapters::ServiceInfo = services
                .inspect(&service_name)
                .map_err(|error| core::operation::OperationError {
                    code: core::operation::OperationErrorCode::VerificationFailed,
                    message: error.to_string(),
                    target_id: Some(service_name.clone()),
                    retryable: true,
                })?;
            crate::startup::services::ServiceInfo::from(info)
        }
        core::service_operation::ServiceAction::Stop => {
            if let Err(error) =
                services.control(&service_name, crate::core::adapters::ServiceAction::Stop)
            {
                return fail(core::operation::OperationError {
                    code: core::operation::OperationErrorCode::ExecutionFailed,
                    message: error.to_string(),
                    target_id: Some(service_name.clone()),
                    retryable: true,
                });
            }
            if let Err(error) = services.wait_for_status(&service_name, "Stopped", 20) {
                return fail(core::operation::OperationError {
                    code: core::operation::OperationErrorCode::VerificationFailed,
                    message: error.to_string(),
                    target_id: Some(service_name.clone()),
                    retryable: true,
                });
            }
            let info: crate::core::adapters::ServiceInfo = services
                .inspect(&service_name)
                .map_err(|error| core::operation::OperationError {
                    code: core::operation::OperationErrorCode::VerificationFailed,
                    message: error.to_string(),
                    target_id: Some(service_name.clone()),
                    retryable: true,
                })?;
            crate::startup::services::ServiceInfo::from(info)
        }
        core::service_operation::ServiceAction::SetStartType { start_type } => {
            if let Err(error) = services.set_start_type(
                &service_name,
                match start_type.as_str() {
                    "auto" | "Auto" => crate::core::adapters::ServiceStartType::Auto,
                    "demand" | "Demand" => crate::core::adapters::ServiceStartType::Demand,
                    "manual" | "Manual" => crate::core::adapters::ServiceStartType::Manual,
                    "disabled" | "Disabled" => crate::core::adapters::ServiceStartType::Disabled,
                    _ => {
                        return fail(core::operation::OperationError::validation(
                            "invalid service start type",
                        ))
                    }
                },
            ) {
                return fail(core::operation::OperationError {
                    code: core::operation::OperationErrorCode::ExecutionFailed,
                    message: error.to_string(),
                    target_id: Some(service_name.clone()),
                    retryable: true,
                });
            }
            let info: crate::core::adapters::ServiceInfo = services
                .inspect(&service_name)
                .map_err(|error| core::operation::OperationError {
                    code: core::operation::OperationErrorCode::VerificationFailed,
                    message: error.to_string(),
                    target_id: Some(service_name.clone()),
                    retryable: true,
                })?;
            crate::startup::services::ServiceInfo::from(info)
        }
    };
    let execution = match core::service_operation::finish(&confirmed_plan, &before, &after) {
        Ok(execution) => execution,
        Err(error) => return fail(error),
    };
    let outcome = execution.result.outcomes.first().ok_or_else(|| {
        core::operation::OperationError::validation("service execution produced no target outcome")
    })?;
    core::service_operation::record_target_outcome(&journal, &confirmed_plan, outcome)?;
    let record = core::service_operation::journal_record(&confirmed_plan, &before, &execution);
    Ok((execution, record))
}

pub(crate) fn execute_services_manual_confirmed_with_journal(
    confirmed_plan: core::operation::OperationPlan,
    journal: Arc<core::operation_journal::OperationJournal>,
    services: Arc<dyn core::adapters::ServiceAdapter>,
) -> std::result::Result<
    (
        core::services_bulk_operation::ServicesBulkExecution,
        OperationJournalRecord,
    ),
    core::operation::OperationError,
> {
    let current = services
        .list()
        .map_err(|error| core::operation::OperationError::validation(error.to_string()))?
        .into_iter()
        .map(Into::into)
        .collect::<Vec<_>>();
    core::services_bulk_operation::execute_with_journal(&confirmed_plan, &current, &journal)
}

pub(crate) fn execute_services_guidance_confirmed_with_journal(
    confirmed_plan: core::operation::OperationPlan,
    journal: Arc<core::operation_journal::OperationJournal>,
    services: Arc<dyn core::adapters::ServiceAdapter>,
) -> std::result::Result<
    (
        core::services_bulk_operation::ServicesBulkExecution,
        OperationJournalRecord,
    ),
    core::operation::OperationError,
> {
    let current = services
        .list()
        .map_err(|error| core::operation::OperationError::validation(error.to_string()))?
        .into_iter()
        .map(Into::into)
        .collect::<Vec<_>>();
    core::services_guidance_operation::execute_with_journal(&confirmed_plan, &current, &journal)
}

#[tauri::command]
pub(crate) async fn execute_service_plan(
    state: tauri::State<'_, Arc<AppState>>,
    confirmed_plan: core::operation::OperationPlan,
) -> std::result::Result<
    core::service_operation::ServiceExecution,
    core::operation::ContextualOperationError,
> {
    let error_context = core::operation::OperationErrorContext::for_plan(&confirmed_plan);
    if !state.adapters.elevation.is_elevated() {
        return Err(core::operation::OperationError {
            code: core::operation::OperationErrorCode::ElevationRequired,
            message: "Administrator approval is required for service changes.".into(),
            target_id: confirmed_plan
                .targets
                .first()
                .map(|target| target.target_id.clone()),
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
    let journal_for_execution = journal.clone();
    let services = state.adapters.services.clone();
    let (execution, record) = tauri::async_runtime::spawn_blocking(move || {
        execute_service_confirmed_with_journal(confirmed_plan, journal_for_execution, services)
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
    })?
    .map_err(|error: core::operation::OperationError| error.with_context(error_context.clone()))?;
    journal
        .append(&record)
        .map_err(|error| error.with_context(error_context))?;
    state.cache.invalidate("list_services");
    Ok(execution)
}

#[tauri::command]
pub(crate) async fn execute_services_manual_plan(
    state: tauri::State<'_, Arc<AppState>>,
    confirmed_plan: core::operation::OperationPlan,
) -> std::result::Result<
    core::services_bulk_operation::ServicesBulkExecution,
    core::operation::ContextualOperationError,
> {
    let error_context = core::operation::OperationErrorContext::for_plan(&confirmed_plan);
    if !state.adapters.elevation.is_elevated() {
        return Err(core::operation::OperationError {
            code: core::operation::OperationErrorCode::ElevationRequired,
            message: "Administrator approval is required for service changes.".into(),
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
    let journal_for_execution = journal.clone();
    let services = state.adapters.services.clone();
    let (execution, record) = tauri::async_runtime::spawn_blocking(move || {
        execute_services_manual_confirmed_with_journal(
            confirmed_plan,
            journal_for_execution,
            services,
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
    })?
    .map_err(|error: core::operation::OperationError| error.with_context(error_context.clone()))?;
    journal
        .append(&record)
        .map_err(|error| error.with_context(error_context))?;
    state.cache.invalidate("list_services");
    Ok(execution)
}

#[tauri::command]
pub(crate) async fn execute_services_guidance_plan(
    state: tauri::State<'_, Arc<AppState>>,
    confirmed_plan: core::operation::OperationPlan,
) -> std::result::Result<
    core::services_bulk_operation::ServicesBulkExecution,
    core::operation::ContextualOperationError,
> {
    let error_context = core::operation::OperationErrorContext::for_plan(&confirmed_plan);
    if !state.adapters.elevation.is_elevated() {
        return Err(core::operation::OperationError {
            code: core::operation::OperationErrorCode::ElevationRequired,
            message: "Administrator approval is required for guided service changes.".into(),
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
    let journal_for_execution = journal.clone();
    let services = state.adapters.services.clone();
    let (execution, record) = tauri::async_runtime::spawn_blocking(move || {
        execute_services_guidance_confirmed_with_journal(
            confirmed_plan,
            journal_for_execution,
            services,
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
    })?
    .map_err(|error: core::operation::OperationError| error.with_context(error_context.clone()))?;
    journal
        .append(&record)
        .map_err(|error| error.with_context(error_context))?;
    state.cache.invalidate("list_services");
    Ok(execution)
}

#[tauri::command]
pub(crate) async fn list_startup_programs(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<startup::programs::StartupProgram>> {
    let key = "list_startup_programs";
    if let Some(cached) = state
        .cache
        .get::<Vec<startup::programs::StartupProgram>>(key)
    {
        return Ok(cached);
    }
    let recovery_store = state
        .ensure_startup_recovery()
        .map_err(|error| core::error::WinOptimizerError::Other(error.to_string()))?
        .clone();
    let startup_programs = state.adapters.startup_programs.clone();
    let value = tauri::async_runtime::spawn_blocking(move || {
        startup_programs
            .list_with_recovery(&recovery_store)
            .map_err(|error| core::error::WinOptimizerError::Other(error.to_string()))
    })
    .await
    .map_err(|e| core::error::WinOptimizerError::Other(e.to_string()))??;
    state.cache.set(key.to_string(), &value);
    Ok(value)
}

#[tauri::command]
pub(crate) async fn list_services(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<startup::services::ServiceInfo>> {
    let key = "list_services";
    if let Some(cached) = state.cache.get::<Vec<startup::services::ServiceInfo>>(key) {
        return Ok(cached);
    }
    let adapter = state.adapters.services.clone();
    let value = tauri::async_runtime::spawn_blocking(move || {
        adapter
            .list()
            .map(|items| items.into_iter().map(Into::into).collect())
            .map_err(|error| core::error::WinOptimizerError::Other(error.to_string()))
    })
    .await
    .map_err(|e| core::error::WinOptimizerError::Other(e.to_string()))??;
    state.cache.set(key.to_string(), &value);
    Ok(value)
}

#[tauri::command]
pub(crate) async fn create_scheduled_task_plan(
    state: tauri::State<'_, Arc<AppState>>,
    task_name: String,
    enabled: bool,
) -> std::result::Result<core::operation::OperationPlan, core::operation::OperationError> {
    let tasks = state.adapters.tasks.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let task = tasks
            .find(&task_name)
            .map(Into::into)
            .map_err(|error| map_adapter_task_error(error, Some(task_name.clone())))?;
        core::scheduled_task_operation::create_plan(&task, enabled)
    })
    .await
    .map_err(internal_task_error)?
}

#[tauri::command]
pub(crate) async fn create_scheduled_tasks_guidance_plan(
    state: tauri::State<'_, Arc<AppState>>,
    targets: Vec<core::scheduled_tasks_guidance_operation::ScheduledTaskGuidanceTarget>,
) -> std::result::Result<core::operation::OperationPlan, core::operation::OperationError> {
    let adapter = state.adapters.tasks.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let tasks = adapter
            .list()
            .map_err(|error| core::operation::OperationError::validation(error.to_string()))?;
        let tasks = tasks.into_iter().map(Into::into).collect::<Vec<_>>();
        core::scheduled_tasks_guidance_operation::create_plan(&targets, &tasks)
    })
    .await
    .map_err(internal_task_error)?
}

pub(crate) fn execute_scheduled_task_confirmed_with_journal(
    confirmed_plan: core::operation::OperationPlan,
    journal: Arc<core::operation_journal::OperationJournal>,
    tasks: Arc<dyn core::adapters::TaskAdapter>,
) -> std::result::Result<
    (
        core::scheduled_task_operation::ScheduledTaskExecution,
        OperationJournalRecord,
    ),
    core::operation::OperationError,
> {
    let task_name = confirmed_plan
        .targets
        .first()
        .map(|target| target.target_id.clone())
        .ok_or_else(|| {
            core::operation::OperationError::validation("scheduled task plan has no target")
        })?;
    let before = match tasks.find(&task_name).map(Into::into) {
        Ok(before) => before,
        Err(error) => {
            let operation_error = map_adapter_task_error(error, Some(task_name.clone()));
            core::scheduled_task_operation::mark_target_revalidation_failed(
                &journal,
                &confirmed_plan,
                &task_name,
            )?;
            return Err(operation_error);
        }
    };
    let enabled = match core::scheduled_task_operation::confirmed_action(&confirmed_plan, &before) {
        Ok(enabled) => enabled,
        Err(error) => {
            core::scheduled_task_operation::mark_target_revalidation_failed(
                &journal,
                &confirmed_plan,
                &task_name,
            )?;
            return Err(error);
        }
    };
    core::scheduled_task_operation::mark_target_ready_and_mutation_started(
        &journal,
        &confirmed_plan,
        &task_name,
    )?;
    let after = match tasks
        .set_enabled(&task_name, enabled)
        .and_then(|_| tasks.find(&task_name))
        .map(Into::into)
    {
        Ok(after) => after,
        Err(error) => {
            let operation_error = map_adapter_task_error(error, Some(task_name.clone()));
            let outcome = core::scheduled_task_operation::failed_outcome(
                &before,
                enabled,
                operation_error.clone(),
            );
            core::scheduled_task_operation::record_target_outcome(
                &journal,
                &confirmed_plan,
                &outcome,
            )?;
            return Err(operation_error);
        }
    };
    let execution = match core::scheduled_task_operation::finish(&confirmed_plan, &before, &after) {
        Ok(execution) => execution,
        Err(error) => {
            let outcome =
                core::scheduled_task_operation::failed_outcome(&before, enabled, error.clone());
            core::scheduled_task_operation::record_target_outcome(
                &journal,
                &confirmed_plan,
                &outcome,
            )?;
            return Err(error);
        }
    };
    let outcome = execution.result.outcomes.first().ok_or_else(|| {
        core::operation::OperationError::validation(
            "scheduled-task execution produced no target outcome",
        )
    })?;
    core::scheduled_task_operation::record_target_outcome(&journal, &confirmed_plan, outcome)?;
    let record =
        core::scheduled_task_operation::journal_record(&confirmed_plan, &before, &execution);
    Ok((execution, record))
}

fn map_adapter_task_error(
    error: core::adapters::AdapterError,
    target_id: Option<String>,
) -> core::operation::OperationError {
    core::operation::OperationError {
        code: core::operation::OperationErrorCode::ExecutionFailed,
        message: error.to_string(),
        target_id,
        retryable: true,
    }
}

#[cfg(test)]
fn map_scheduled_task_error(
    error: startup::scheduler::SchedulerError,
    target_id: Option<String>,
) -> core::operation::OperationError {
    let (code, retryable) = match &error {
        startup::scheduler::SchedulerError::InvalidTaskName(_)
        | startup::scheduler::SchedulerError::InvalidDefinition(_) => {
            (core::operation::OperationErrorCode::ValidationFailed, false)
        }
        startup::scheduler::SchedulerError::ProtectedTarget(_) => {
            (core::operation::OperationErrorCode::TargetProtected, false)
        }
        startup::scheduler::SchedulerError::NotFound(_) => {
            (core::operation::OperationErrorCode::TargetNotFound, true)
        }
        startup::scheduler::SchedulerError::AccessDenied { .. } => {
            (core::operation::OperationErrorCode::PermissionDenied, false)
        }
        startup::scheduler::SchedulerError::StaleState(_) => (
            core::operation::OperationErrorCode::TargetChangedSincePreview,
            true,
        ),
        startup::scheduler::SchedulerError::VerificationFailed(_) => (
            core::operation::OperationErrorCode::VerificationFailed,
            true,
        ),
        startup::scheduler::SchedulerError::RestorationUnavailable(_)
        | startup::scheduler::SchedulerError::Native { .. } => {
            (core::operation::OperationErrorCode::ExecutionFailed, true)
        }
        startup::scheduler::SchedulerError::UnsupportedPlatform => (
            core::operation::OperationErrorCode::UnsupportedWindowsVersion,
            false,
        ),
    };
    core::operation::OperationError {
        code,
        message: error.to_string(),
        target_id,
        retryable,
    }
}

pub(crate) fn execute_scheduled_tasks_guidance_confirmed_with_journal(
    confirmed_plan: core::operation::OperationPlan,
    journal: Arc<core::operation_journal::OperationJournal>,
    tasks: Arc<dyn core::adapters::TaskAdapter>,
) -> std::result::Result<
    (
        core::scheduled_task_operation::ScheduledTaskExecution,
        OperationJournalRecord,
    ),
    core::operation::OperationError,
> {
    let current = tasks
        .list()
        .map_err(|error| core::operation::OperationError::validation(error.to_string()))?
        .into_iter()
        .map(Into::into)
        .collect::<Vec<_>>();
    core::scheduled_tasks_guidance_operation::execute_with_journal(
        &confirmed_plan,
        &current,
        &journal,
    )
}

#[tauri::command]
pub(crate) async fn execute_scheduled_task_plan(
    state: tauri::State<'_, Arc<AppState>>,
    confirmed_plan: core::operation::OperationPlan,
) -> std::result::Result<
    core::scheduled_task_operation::ScheduledTaskExecution,
    core::operation::ContextualOperationError,
> {
    let error_context = core::operation::OperationErrorContext::for_plan(&confirmed_plan);
    if !state.adapters.elevation.is_elevated() {
        return Err(core::operation::OperationError {
            code: core::operation::OperationErrorCode::ElevationRequired,
            message: "Administrator approval is required for scheduled-task changes.".into(),
            target_id: confirmed_plan
                .targets
                .first()
                .map(|target| target.target_id.clone()),
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
    let journal_for_execution = journal.clone();
    let tasks = state.adapters.tasks.clone();
    let (execution, record) = tauri::async_runtime::spawn_blocking(move || {
        execute_scheduled_task_confirmed_with_journal(confirmed_plan, journal_for_execution, tasks)
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
    })?
    .map_err(|error: core::operation::OperationError| error.with_context(error_context.clone()))?;
    journal
        .append(&record)
        .map_err(|error| error.with_context(error_context))?;
    state.cache.invalidate("list_startup_tasks");
    Ok(execution)
}

#[tauri::command]
pub(crate) async fn execute_scheduled_tasks_guidance_plan(
    state: tauri::State<'_, Arc<AppState>>,
    confirmed_plan: core::operation::OperationPlan,
) -> std::result::Result<
    core::scheduled_task_operation::ScheduledTaskExecution,
    core::operation::ContextualOperationError,
> {
    let error_context = core::operation::OperationErrorContext::for_plan(&confirmed_plan);
    if !state.adapters.elevation.is_elevated() {
        return Err(core::operation::OperationError {
            code: core::operation::OperationErrorCode::ElevationRequired,
            message: "Administrator approval is required for guided scheduled-task changes.".into(),
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
    let journal_for_execution = journal.clone();
    let tasks = state.adapters.tasks.clone();
    let (execution, record) = tauri::async_runtime::spawn_blocking(move || {
        execute_scheduled_tasks_guidance_confirmed_with_journal(
            confirmed_plan,
            journal_for_execution,
            tasks,
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
    })?
    .map_err(|error: core::operation::OperationError| error.with_context(error_context.clone()))?;
    journal
        .append(&record)
        .map_err(|error| error.with_context(error_context))?;
    state.cache.invalidate("list_startup_tasks");
    Ok(execution)
}

#[tauri::command]
pub(crate) async fn list_startup_tasks(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<startup::scheduler::ScheduledTask>> {
    let key = "list_startup_tasks";
    if let Some(cached) = state
        .cache
        .get::<Vec<startup::scheduler::ScheduledTask>>(key)
    {
        return Ok(cached);
    }
    let adapter = state.adapters.tasks.clone();
    let value = tauri::async_runtime::spawn_blocking(move || {
        adapter
            .list()
            .map(|items| items.into_iter().map(Into::into).collect())
            .map_err(|error| core::error::WinOptimizerError::Other(error.to_string()))
    })
    .await
    .map_err(|e| core::error::WinOptimizerError::Other(e.to_string()))??;
    state.cache.set(key.to_string(), &value);
    Ok(value)
}

#[tauri::command]
pub(crate) async fn scan_scheduled_tasks(
    _state: tauri::State<'_, Arc<AppState>>,
) -> std::result::Result<startup::scheduler::ScheduledTaskScan, core::operation::OperationError> {
    let adapter = _state.adapters.tasks.clone();
    let tasks = tauri::async_runtime::spawn_blocking(move || adapter.list())
        .await
        .map_err(internal_task_error)?
        .map_err(internal_task_error)?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(startup::scheduler::ScheduledTaskScan {
        tasks,
        collected_at: if _state.runtime.is_fixture() {
            "fixture".into()
        } else {
            chrono::Utc::now().to_rfc3339()
        },
        classification: "supporting".into(),
        mutation_available: _state.runtime.allows_host_operations(),
        error: None,
    })
}

#[cfg(test)]
mod scheduled_task_error_tests {
    use super::*;

    #[test]
    fn native_scheduler_errors_keep_operation_categories() {
        let protected = map_scheduled_task_error(
            startup::scheduler::SchedulerError::ProtectedTarget("protected".into()),
            Some(r"\Vendor\Task".into()),
        );
        assert_eq!(
            protected.code,
            core::operation::OperationErrorCode::TargetProtected
        );

        let stale = map_scheduled_task_error(
            startup::scheduler::SchedulerError::StaleState("stale".into()),
            Some(r"\Vendor\Task".into()),
        );
        assert_eq!(
            stale.code,
            core::operation::OperationErrorCode::TargetChangedSincePreview
        );
        assert!(stale.retryable);

        let denied = map_scheduled_task_error(
            startup::scheduler::SchedulerError::AccessDenied {
                operation: "set enabled state".into(),
                hresult: 0x8007_0005u32 as i32,
            },
            None,
        );
        assert_eq!(
            denied.code,
            core::operation::OperationErrorCode::PermissionDenied
        );
        assert!(!denied.retryable);
    }
}
