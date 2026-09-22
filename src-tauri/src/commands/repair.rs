use crate::*;
use std::sync::atomic::Ordering;

use crate::core::bounded_process::{RunControl, DEFAULT_REPAIR_TIMEOUT};

const REPAIR_CANCEL_KEY: &str = "repair";

// ============================================================
// Tauri Commands - Repair Center
// ============================================================

/// Request cancellation of the currently running repair.  The bounded process
/// runner polls this flag and terminates the owned process tree.
#[tauri::command]
pub(crate) fn cancel_repair(state: tauri::State<'_, Arc<AppState>>) -> Result<()> {
    let flags = state.cancel_flags.lock().map_err(|_| {
        core::error::WinOptimizerError::Other("repair cancellation state is unavailable".into())
    })?;
    let flag = flags.get(REPAIR_CANCEL_KEY).ok_or_else(|| {
        core::error::WinOptimizerError::Other("no repair operation is currently running".into())
    })?;
    flag.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub(crate) fn list_repair_targets() -> Vec<core::repair_operation::RepairTarget> {
    core::repair_operation::targets()
}

#[tauri::command]
pub(crate) fn list_repair_guides() -> Vec<core::repair_operation::RepairGuide> {
    core::repair_operation::guides()
}

#[tauri::command]
pub(crate) fn create_repair_plan(
    action_id: String,
) -> std::result::Result<core::operation::OperationPlan, core::operation::OperationError> {
    let target = core::repair_operation::find_target(&action_id)?;
    core::repair_operation::create_plan(&target)
}

pub(crate) fn execute_repair_confirmed(
    confirmed_plan: core::operation::OperationPlan,
    journal: std::sync::Arc<core::operation_journal::OperationJournal>,
    control: RunControl,
) -> std::result::Result<
    (
        core::repair_operation::RepairExecution,
        OperationJournalRecord,
    ),
    core::operation::OperationError,
> {
    core::repair_operation::validate_plan(&confirmed_plan)?;
    let action_id = confirmed_plan
        .targets
        .first()
        .map(|target| target.target_id.clone())
        .ok_or_else(|| core::operation::OperationError::validation("repair plan has no target"))?;
    let current = core::repair_operation::find_target(&action_id)?;
    let action = core::repair_operation::confirmed_action(&confirmed_plan, &current)?;
    if action.mutates_system {
        let payload = serde_json::json!({
            "target_id": action.id.clone(),
            "phase": "ready",
            "verification_scope": current.verification_scope,
        })
        .to_string();
        journal.record_target_state(
            &confirmed_plan.id,
            &action.id,
            core::operation::TargetExecutionState::Ready,
            &payload,
        )?;
        let payload = serde_json::json!({
            "target_id": action.id.clone(),
            "phase": "mutation_started",
        })
        .to_string();
        journal.record_target_state(
            &confirmed_plan.id,
            &action.id,
            core::operation::TargetExecutionState::MutationStarted,
            &payload,
        )?;
    }

    let log = if action.mutates_system {
        repair::run_action_through_kernel_with_control(&action.id, &control)
    } else {
        repair::run_action_with_control(&action.id, &control)
    };
    let log = match log {
        Ok(log) => log,
        Err(error) => {
            if action.mutates_system {
                let payload = serde_json::json!({
                    "target_id": action.id.clone(),
                    "phase": "outcome_unknown",
                    "error": error.to_string(),
                })
                .to_string();
                journal.record_target_state(
                    &confirmed_plan.id,
                    &action.id,
                    core::operation::TargetExecutionState::OutcomeUnknown,
                    &payload,
                )?;
            }
            return Err(core::operation::OperationError {
                code: core::operation::OperationErrorCode::ExecutionFailed,
                message: error.to_string(),
                target_id: Some(action.id),
                retryable: true,
            });
        }
    };
    let execution = match core::repair_operation::finish(&confirmed_plan, log) {
        Ok(execution) => execution,
        Err(error) => {
            if action.mutates_system {
                let payload = serde_json::json!({
                    "target_id": action.id.clone(),
                    "phase": "outcome_unknown",
                    "error": error.message.clone(),
                })
                .to_string();
                journal.record_target_state(
                    &confirmed_plan.id,
                    &action.id,
                    core::operation::TargetExecutionState::OutcomeUnknown,
                    &payload,
                )?;
            }
            return Err(error);
        }
    };
    if action.mutates_system {
        journal.record_target_outcome(&confirmed_plan.id, &execution.result.outcomes[0])?;
    }
    let record = core::repair_operation::journal_record(&confirmed_plan, &execution);
    Ok((execution, record))
}

#[tauri::command]
pub(crate) async fn execute_repair_plan(
    state: tauri::State<'_, Arc<AppState>>,
    app_handle: tauri::AppHandle,
    confirmed_plan: core::operation::OperationPlan,
) -> std::result::Result<
    core::repair_operation::RepairExecution,
    core::operation::ContextualOperationError,
> {
    let error_context = core::operation::OperationErrorContext::for_plan(&confirmed_plan);
    if !state.runtime.allows_host_operations() {
        return Err(core::operation::OperationError::validation(
            "repair operations are unavailable in fixture runtime",
        )
        .with_context(error_context));
    }
    core::repair_operation::validate_plan(&confirmed_plan)
        .map_err(|error| error.with_context(error_context.clone()))?;
    if confirmed_plan.requires_elevation && !core::elevation::is_elevated() {
        return Err(core::operation::OperationError {
            code: core::operation::OperationErrorCode::ElevationRequired,
            message: if confirmed_plan.descriptor.mutates_system {
                "Administrator approval is required for this repair operation.".into()
            } else {
                "Administrator approval is required for this repair diagnostic.".into()
            },
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
    app_handle
        .emit(
            "progress-event",
            ProgressEvent {
                operation: "repair".into(),
                phase: "started".into(),
                percent: None,
                message: Some(if confirmed_plan.descriptor.mutates_system {
                    "Running confirmed Windows repair...".into()
                } else {
                    "Running verified repair diagnostic...".into()
                }),
                bytes_processed: None,
                bytes_total: None,
            },
        )
        .ok();
    // Register a cancellation flag for this repair.  It is removed once the
    // execution returns so a cancel request can never hit a stale operation.
    let cancel_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    if let Ok(mut flags) = state.cancel_flags.lock() {
        flags.insert(
            REPAIR_CANCEL_KEY.to_string(),
            std::sync::Arc::clone(&cancel_flag),
        );
    }
    let control = RunControl::new(cancel_flag, DEFAULT_REPAIR_TIMEOUT);
    let execution_journal = std::sync::Arc::clone(&journal);
    let result = tauri::async_runtime::spawn_blocking(move || {
        execute_repair_confirmed(confirmed_plan, execution_journal, control)
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
    if let Ok(mut flags) = state.cancel_flags.lock() {
        flags.remove(REPAIR_CANCEL_KEY);
    }
    let (execution, record) = result.map_err(|error: core::operation::OperationError| {
        error.with_context(error_context.clone())
    })?;
    journal
        .append(&record)
        .map_err(|error| error.with_context(error_context))?;
    app_handle
        .emit(
            "progress-event",
            ProgressEvent {
                operation: "repair".into(),
                phase: "completed".into(),
                percent: Some(100.0),
                message: Some(execution.verification_message.clone()),
                bytes_processed: None,
                bytes_total: None,
            },
        )
        .ok();
    Ok(execution)
}

#[tauri::command]
pub(crate) async fn list_repair_actions(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<repair::RepairAction>> {
    if !state.runtime.allows_host_operations() {
        return Ok(Vec::new());
    }
    tauri::async_runtime::spawn_blocking(repair::supported_repair_actions)
        .await
        .map_err(|e| core::error::WinOptimizerError::Other(e.to_string()))
}
