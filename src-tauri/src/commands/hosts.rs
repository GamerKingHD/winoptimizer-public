use crate::*;

// ============================================================
// Tauri Commands - Hosts File Editor
// ============================================================

#[tauri::command]
pub(crate) async fn list_hosts_entries(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<tweaks::hosts::HostEntry>> {
    if !state.runtime.allows_host_operations() {
        return Ok(Vec::new());
    }
    tauri::async_runtime::spawn_blocking(|| {
        tweaks::hosts::list().map_err(core::error::WinOptimizerError::Other)
    })
    .await
    .map_err(|e| core::error::WinOptimizerError::Other(e.to_string()))?
}

#[tauri::command]
pub(crate) async fn create_hosts_plan(
    state: tauri::State<'_, Arc<AppState>>,
    entries: Vec<tweaks::hosts::HostEntry>,
) -> std::result::Result<core::operation::OperationPlan, core::operation::OperationError> {
    if !state.runtime.allows_host_operations() {
        return Err(hosts_internal_error(
            "hosts operations are unavailable in fixture runtime",
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let before = tweaks::hosts::snapshot().map_err(hosts_internal_error)?;
        core::hosts_operation::create_plan(&before, entries)
    })
    .await
    .map_err(|error| hosts_internal_error(error.to_string()))?
}

pub(crate) fn execute_hosts_confirmed_with_journal(
    confirmed_plan: core::operation::OperationPlan,
    journal: &core::operation_journal::OperationJournal,
) -> std::result::Result<
    (
        core::hosts_operation::HostsExecution,
        OperationJournalRecord,
    ),
    core::operation::OperationError,
> {
    let before = tweaks::hosts::snapshot().map_err(hosts_internal_error)?;
    let target = confirmed_plan
        .targets
        .first()
        .ok_or_else(|| core::operation::OperationError::validation("hosts plan has no target"))?;
    let entries = match core::hosts_operation::confirmed_entries(&confirmed_plan, &before) {
        Ok(entries) => entries,
        Err(error) => {
            core::hosts_operation::mark_target_revalidation_failed(
                journal,
                &confirmed_plan,
                &target.target_id,
            )?;
            return Err(error);
        }
    };
    let execution_plan = confirmed_plan.clone();
    let execution_before = before.clone();
    let execution_entries = entries.clone();
    let execution = core::hosts_operation::execute_target_with_journal(
        journal,
        &confirmed_plan,
        target,
        move || {
            tweaks::hosts::save(&execution_before, &execution_entries).map_err(|message| {
                core::operation::OperationError {
                    code: core::operation::OperationErrorCode::ExecutionFailed,
                    message,
                    target_id: execution_plan
                        .targets
                        .first()
                        .map(|target| target.target_id.clone()),
                    retryable: true,
                }
            })?;
            let after =
                tweaks::hosts::snapshot().map_err(|message| core::operation::OperationError {
                    code: core::operation::OperationErrorCode::VerificationFailed,
                    message,
                    target_id: execution_plan
                        .targets
                        .first()
                        .map(|target| target.target_id.clone()),
                    retryable: true,
                })?;
            core::hosts_operation::finish(&execution_plan, &execution_before, &after)
        },
    )?;
    let record = core::hosts_operation::journal_record(&confirmed_plan, &before, &execution);
    Ok((execution, record))
}

#[tauri::command]
pub(crate) async fn execute_hosts_plan(
    state: tauri::State<'_, Arc<AppState>>,
    confirmed_plan: core::operation::OperationPlan,
) -> std::result::Result<
    core::hosts_operation::HostsExecution,
    core::operation::ContextualOperationError,
> {
    let error_context = core::operation::OperationErrorContext::for_plan(&confirmed_plan);
    if !state.runtime.allows_host_operations() {
        return Err(
            hosts_internal_error("hosts operations are unavailable in fixture runtime")
                .with_context(error_context),
        );
    }
    if !core::elevation::is_elevated() {
        return Err(core::operation::OperationError {
            code: core::operation::OperationErrorCode::ElevationRequired,
            message: "Administrator approval is required to save the hosts file.".into(),
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
    let execution_journal = journal.clone();
    let (execution, record) = tauri::async_runtime::spawn_blocking(move || {
        execute_hosts_confirmed_with_journal(confirmed_plan, &execution_journal)
    })
    .await
    .map_err(|error| hosts_internal_error(error.to_string()).with_context(error_context.clone()))?
    .map_err(|error: core::operation::OperationError| error.with_context(error_context.clone()))?;
    journal
        .append(&record)
        .map_err(|error| error.with_context(error_context))?;
    Ok(execution)
}

pub(crate) fn hosts_internal_error(message: impl Into<String>) -> core::operation::OperationError {
    core::operation::OperationError {
        code: core::operation::OperationErrorCode::InternalError,
        message: message.into(),
        target_id: None,
        retryable: true,
    }
}
