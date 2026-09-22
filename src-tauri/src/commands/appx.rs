use crate::*;

// ============================================================
// Tauri Commands - AppX De-provisioning
// ============================================================

#[tauri::command]
pub(crate) async fn create_appx_plan(
    state: tauri::State<'_, Arc<AppState>>,
    action: core::appx_operation::AppxAction,
) -> std::result::Result<core::operation::OperationPlan, core::operation::OperationError> {
    if !state.runtime.allows_host_operations() {
        return Err(core::operation::OperationError::validation(
            "AppX operations are unavailable in fixture runtime",
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let policy =
            tweaks::appx::policy_snapshot().map_err(|message| core::operation::OperationError {
                code: core::operation::OperationErrorCode::InternalError,
                message,
                target_id: None,
                retryable: true,
            })?;
        core::appx_operation::create_plan(&policy, &tweaks::appx::list_candidates(), action)
    })
    .await
    .map_err(|error| core::operation::OperationError {
        code: core::operation::OperationErrorCode::InternalError,
        message: error.to_string(),
        target_id: None,
        retryable: true,
    })?
}

#[cfg(test)]
pub(crate) fn execute_appx_confirmed(
    confirmed_plan: core::operation::OperationPlan,
) -> std::result::Result<
    (core::appx_operation::AppxExecution, OperationJournalRecord),
    core::operation::OperationError,
> {
    execute_appx_confirmed_inner(confirmed_plan, None)
}

pub(crate) fn execute_appx_confirmed_with_journal(
    confirmed_plan: core::operation::OperationPlan,
    journal: Arc<core::operation_journal::OperationJournal>,
) -> std::result::Result<
    (core::appx_operation::AppxExecution, OperationJournalRecord),
    core::operation::OperationError,
> {
    execute_appx_confirmed_inner(confirmed_plan, Some(journal))
}

fn execute_appx_confirmed_inner(
    confirmed_plan: core::operation::OperationPlan,
    journal: Option<Arc<core::operation_journal::OperationJournal>>,
) -> std::result::Result<
    (core::appx_operation::AppxExecution, OperationJournalRecord),
    core::operation::OperationError,
> {
    if journal.is_none() {
        return Err(core::operation::OperationError::validation(
            "confirmed mutation requires an initialized operation journal",
        ));
    }
    let before =
        tweaks::appx::policy_snapshot().map_err(|message| core::operation::OperationError {
            code: core::operation::OperationErrorCode::InternalError,
            message,
            target_id: None,
            retryable: true,
        })?;
    let candidates = tweaks::appx::list_candidates();
    let action = match core::appx_operation::confirmed_action(&confirmed_plan, &before, &candidates)
    {
        Ok(action) => action,
        Err(error) => {
            if let Some(journal) = journal.as_deref() {
                for target in &confirmed_plan.targets {
                    core::appx_operation::mark_target_revalidation_failed(
                        journal,
                        &confirmed_plan,
                        &target.target_id,
                    )?;
                }
            }
            return Err(error);
        }
    };
    for target in &confirmed_plan.targets {
        if let Some(journal) = journal.as_deref() {
            core::appx_operation::mark_target_ready_and_mutation_started(
                journal,
                &confirmed_plan,
                &target.target_id,
            )?;
        }
        let mutation = match &action {
            core::appx_operation::AppxAction::Mark { .. } => {
                tweaks::appx::mark_packages_for_removal(std::slice::from_ref(&target.target_id))
                    .map(|_| ())
            }
            core::appx_operation::AppxAction::Unmark { family_name } => {
                tweaks::appx::unmark_package(family_name)
            }
            core::appx_operation::AppxAction::DisablePolicy => tweaks::appx::disable_policy(),
        };
        if let Err(message) = mutation {
            let error = core::operation::OperationError {
                code: core::operation::OperationErrorCode::ExecutionFailed,
                message,
                target_id: Some(target.target_id.clone()),
                retryable: true,
            };
            if let Some(journal) = journal.as_deref() {
                match tweaks::appx::policy_snapshot() {
                    Ok(after) => {
                        let outcome = core::appx_operation::target_outcome(
                            &confirmed_plan,
                            &target.target_id,
                            &before,
                            &after,
                        )?;
                        core::appx_operation::record_target_outcome(
                            journal,
                            &confirmed_plan,
                            &outcome,
                        )?;
                    }
                    Err(message) => journal.record_target_state(
                        &confirmed_plan.id,
                        &target.target_id,
                        core::operation::TargetExecutionState::OutcomeUnknown,
                        &message,
                    )?,
                }
            }
            return Err(error);
        }
        let after_target = match tweaks::appx::policy_snapshot() {
            Ok(after) => after,
            Err(message) => {
                if let Some(journal) = journal.as_deref() {
                    journal.record_target_state(
                        &confirmed_plan.id,
                        &target.target_id,
                        core::operation::TargetExecutionState::OutcomeUnknown,
                        &message,
                    )?;
                }
                return Err(core::operation::OperationError {
                    code: core::operation::OperationErrorCode::VerificationFailed,
                    message,
                    target_id: Some(target.target_id.clone()),
                    retryable: true,
                });
            }
        };
        if let Some(journal) = journal.as_deref() {
            let outcome = core::appx_operation::target_outcome(
                &confirmed_plan,
                &target.target_id,
                &before,
                &after_target,
            )?;
            core::appx_operation::record_target_outcome(journal, &confirmed_plan, &outcome)?;
        }
    }
    let after =
        tweaks::appx::policy_snapshot().map_err(|message| core::operation::OperationError {
            code: core::operation::OperationErrorCode::VerificationFailed,
            message,
            target_id: None,
            retryable: true,
        })?;
    let execution = core::appx_operation::finish(&confirmed_plan, &before, &after)?;
    let record = core::appx_operation::journal_record(&confirmed_plan, &before, &execution);
    Ok((execution, record))
}

#[tauri::command]
pub(crate) async fn execute_appx_plan(
    state: tauri::State<'_, Arc<AppState>>,
    confirmed_plan: core::operation::OperationPlan,
) -> std::result::Result<
    core::appx_operation::AppxExecution,
    core::operation::ContextualOperationError,
> {
    let error_context = core::operation::OperationErrorContext::for_plan(&confirmed_plan);
    if !state.runtime.allows_host_operations() {
        return Err(core::operation::OperationError::validation(
            "AppX operations are unavailable in fixture runtime",
        )
        .with_context(error_context));
    }
    if !state.adapters.elevation.is_elevated() {
        return Err(core::operation::OperationError {
            code: core::operation::OperationErrorCode::ElevationRequired,
            message: "Administrator approval is required for AppX policy changes.".into(),
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
        execute_appx_confirmed_with_journal(confirmed_plan, execution_journal)
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
    Ok(execution)
}

#[tauri::command]
pub(crate) async fn list_appx_candidates(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<tweaks::appx::AppxPackage>> {
    if !state.runtime.allows_host_operations() {
        return Ok(Vec::new());
    }
    tauri::async_runtime::spawn_blocking(tweaks::appx::list_candidates)
        .await
        .map_err(|e| core::error::WinOptimizerError::Other(e.to_string()))
}

#[tauri::command]
pub(crate) async fn get_appx_policy_status(state: tauri::State<'_, Arc<AppState>>) -> Result<bool> {
    if !state.runtime.allows_host_operations() {
        return Ok(false);
    }
    tauri::async_runtime::spawn_blocking(move || {
        Ok::<_, core::error::WinOptimizerError>(tweaks::appx::is_policy_enabled())
    })
    .await
    .map_err(|e| core::error::WinOptimizerError::Other(e.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_confirmed_wrapper_fails_closed_without_journal() {
        let plan = core::operation::synthetic_operation_contract()
            .unwrap()
            .plan;
        let result = execute_appx_confirmed(plan);

        assert!(matches!(
            result,
            Err(error)
                if error.code == core::operation::OperationErrorCode::ValidationFailed
        ));
    }
}
