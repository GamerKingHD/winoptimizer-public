use crate::commands::hosts::hosts_internal_error;
use crate::*;

// ============================================================
// Tauri Commands - Process Manager
// ============================================================

#[tauri::command]
pub(crate) async fn process_snapshot(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<processes::snapshots::ProcessInfo>> {
    let key = "process_snapshot";
    if let Some(cached) = state
        .cache
        .get::<Vec<processes::snapshots::ProcessInfo>>(key)
    {
        return Ok(cached);
    }
    let adapter = state.adapters.processes.clone();
    let value = tauri::async_runtime::spawn_blocking(move || adapter.snapshot())
        .await
        .map_err(|e| core::error::WinOptimizerError::Other(e.to_string()))?;
    state.cache.set(key.to_string(), &value);
    Ok(value)
}

#[tauri::command]
pub(crate) async fn create_process_plan(
    process: processes::snapshots::ProcessInfo,
) -> std::result::Result<core::operation::OperationPlan, core::operation::OperationError> {
    core::process_operation::create_plan(&process)
}

#[cfg(test)]
pub(crate) fn execute_process_confirmed(
    plan: core::operation::OperationPlan,
) -> std::result::Result<
    (
        core::process_operation::ProcessExecution,
        OperationJournalRecord,
    ),
    core::operation::OperationError,
> {
    execute_process_confirmed_inner(plan, None, Arc::new(core::adapters::NativeProcessAdapter))
}

pub(crate) fn execute_process_confirmed_with_journal(
    plan: core::operation::OperationPlan,
    journal: Arc<core::operation_journal::OperationJournal>,
    adapter: Arc<dyn core::adapters::ProcessAdapter>,
) -> std::result::Result<
    (
        core::process_operation::ProcessExecution,
        OperationJournalRecord,
    ),
    core::operation::OperationError,
> {
    execute_process_confirmed_inner(plan, Some(journal), adapter)
}

fn execute_process_confirmed_inner(
    plan: core::operation::OperationPlan,
    journal: Option<Arc<core::operation_journal::OperationJournal>>,
    adapter: Arc<dyn core::adapters::ProcessAdapter>,
) -> std::result::Result<
    (
        core::process_operation::ProcessExecution,
        OperationJournalRecord,
    ),
    core::operation::OperationError,
> {
    if journal.is_none() {
        return Err(core::operation::OperationError::validation(
            "confirmed mutation requires an initialized operation journal",
        ));
    }
    let target = plan
        .targets
        .first()
        .ok_or_else(|| core::operation::OperationError::validation("process plan has no target"))?;
    let current = adapter.snapshot();
    let identity = match core::process_operation::confirmed_identity(&plan, &current) {
        Ok(identity) => identity,
        Err(error) => {
            if let Some(journal) = journal.as_deref() {
                core::process_operation::mark_target_revalidation_failed(
                    journal,
                    &plan,
                    &target.target_id,
                )?;
            }
            return Err(error);
        }
    };
    if let Some(journal) = journal.as_deref() {
        core::process_operation::mark_target_ready_and_mutation_started(
            journal,
            &plan,
            &target.target_id,
        )?;
    }
    if let Err(error) = adapter.kill(
        identity.pid,
        identity.start_time,
        &identity.name,
        identity.path.as_deref(),
    ) {
        if let Some(journal) = journal.as_deref() {
            journal.record_target_state(
                &plan.id,
                &target.target_id,
                core::operation::TargetExecutionState::Failed,
                &error.to_string(),
            )?;
        }
        return Err(core::operation::OperationError {
            code: core::operation::OperationErrorCode::ExecutionFailed,
            message: error.to_string(),
            target_id: Some(identity.pid.to_string()),
            retryable: true,
        });
    }
    let mut after = adapter.snapshot();
    for _ in 0..10 {
        if !after
            .iter()
            .any(|process| process.pid == identity.pid && process.start_time == identity.start_time)
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        after = adapter.snapshot();
    }
    let execution = core::process_operation::finish(&plan, &after)?;
    if let Some(journal) = journal.as_deref() {
        for outcome in &execution.result.outcomes {
            core::process_operation::record_target_outcome(journal, &plan, outcome)?;
        }
    }
    let record = core::process_operation::journal_record(&plan, &execution);
    Ok((execution, record))
}

#[tauri::command]
pub(crate) async fn execute_process_plan(
    state: tauri::State<'_, Arc<AppState>>,
    confirmed_plan: core::operation::OperationPlan,
) -> std::result::Result<
    core::process_operation::ProcessExecution,
    core::operation::ContextualOperationError,
> {
    let error_context = core::operation::OperationErrorContext::for_plan(&confirmed_plan);
    if confirmed_plan.requires_elevation && !state.adapters.elevation.is_elevated() {
        return Err(core::operation::OperationError {
            code: core::operation::OperationErrorCode::ElevationRequired,
            message: "Administrator approval is required to terminate this process.".into(),
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
    let execution_journal = journal.clone();
    let process_adapter = state.adapters.processes.clone();
    let (execution, record) = tauri::async_runtime::spawn_blocking(move || {
        execute_process_confirmed_with_journal(confirmed_plan, execution_journal, process_adapter)
    })
    .await
    .map_err(|error| hosts_internal_error(error.to_string()).with_context(error_context.clone()))?
    .map_err(|error: core::operation::OperationError| error.with_context(error_context.clone()))?;
    journal
        .append(&record)
        .map_err(|error| error.with_context(error_context))?;
    state.cache.invalidate("process_snapshot");
    Ok(execution)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_confirmed_wrapper_fails_closed_without_journal() {
        let plan = core::operation::synthetic_operation_contract()
            .unwrap()
            .plan;
        let result = execute_process_confirmed(plan);

        assert!(matches!(
            result,
            Err(error)
                if error.code == core::operation::OperationErrorCode::ValidationFailed
        ));
    }
}
