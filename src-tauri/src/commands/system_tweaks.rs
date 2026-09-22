use crate::*;

// ============================================================
// Tauri Commands - System Tweaks
// ============================================================

#[tauri::command]
pub(crate) async fn list_system_tweak_targets(
    state: tauri::State<'_, Arc<AppState>>,
) -> std::result::Result<
    Vec<core::system_tweak_operation::SystemTweakTarget>,
    core::operation::OperationError,
> {
    if !state.runtime.allows_host_operations() {
        return Ok(Vec::new());
    }
    let settings_path = state.runtime.paths.settings.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let labs_enabled = core::settings::SettingsStore::at(&settings_path)
            .load()
            .map(|settings| settings.labs_enabled)
            .unwrap_or(false);
        core::system_tweak_operation::list_targets_with_labs(labs_enabled)
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
pub(crate) async fn create_system_tweak_plan(
    state: tauri::State<'_, Arc<AppState>>,
    kind: core::system_tweak_operation::TweakKind,
    tweak_id: String,
) -> std::result::Result<core::operation::OperationPlan, core::operation::OperationError> {
    if !state.runtime.allows_host_operations() {
        return Err(core::operation::OperationError::validation(
            "system tweak operations are unavailable in fixture runtime",
        ));
    }
    let settings_path = state.runtime.paths.settings.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let labs_enabled = core::settings::SettingsStore::at(&settings_path)
            .load()
            .map(|settings| settings.labs_enabled)
            .unwrap_or(false);
        let target =
            core::system_tweak_operation::find_target_with_labs(&kind, &tweak_id, labs_enabled)?;
        core::system_tweak_operation::create_plan(&target)
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
pub(crate) fn execute_system_tweak_confirmed(
    confirmed_plan: core::operation::OperationPlan,
) -> std::result::Result<
    (
        core::system_tweak_operation::SystemTweakExecution,
        OperationJournalRecord,
    ),
    core::operation::OperationError,
> {
    execute_system_tweak_confirmed_inner(confirmed_plan, None)
}

pub(crate) fn execute_system_tweak_confirmed_with_journal(
    confirmed_plan: core::operation::OperationPlan,
    journal: Arc<core::operation_journal::OperationJournal>,
    journal_operation_id: core::operation::OperationId,
) -> std::result::Result<
    (
        core::system_tweak_operation::SystemTweakExecution,
        OperationJournalRecord,
    ),
    core::operation::OperationError,
> {
    execute_system_tweak_confirmed_inner(confirmed_plan, Some((journal, journal_operation_id)))
}

fn execute_system_tweak_confirmed_inner(
    confirmed_plan: core::operation::OperationPlan,
    journal: Option<(
        Arc<core::operation_journal::OperationJournal>,
        core::operation::OperationId,
    )>,
) -> std::result::Result<
    (
        core::system_tweak_operation::SystemTweakExecution,
        OperationJournalRecord,
    ),
    core::operation::OperationError,
> {
    if journal.is_none() {
        return Err(core::operation::OperationError::validation(
            "confirmed mutation requires an initialized operation journal",
        ));
    }
    let target = confirmed_plan
        .targets
        .first()
        .ok_or_else(|| core::operation::OperationError::validation("tweak plan has no target"))?;
    let (kind_text, tweak_id) = target.target_id.split_once(':').ok_or_else(|| {
        core::operation::OperationError::validation("tweak target identifier is invalid")
    })?;
    let kind = match kind_text {
        "privacy" => core::system_tweak_operation::TweakKind::Privacy,
        "performance" => core::system_tweak_operation::TweakKind::Performance,
        "customization" => core::system_tweak_operation::TweakKind::Customization,
        _ => {
            return Err(core::operation::OperationError::validation(
                "tweak target kind is invalid",
            ))
        }
    };
    let tweak_id = tweak_id.to_owned();
    let operation_id = journal
        .as_ref()
        .map(|(_, operation_id)| operation_id)
        .unwrap_or(&confirmed_plan.id);
    let before = match core::system_tweak_operation::find_target(&kind, &tweak_id) {
        Ok(before) => before,
        Err(error) => {
            if let Some((journal, _)) = journal.as_ref() {
                core::system_tweak_operation::mark_target_revalidation_failed_for_operation(
                    journal,
                    operation_id,
                    &confirmed_plan,
                    &target.target_id,
                )?;
            }
            return Err(error);
        }
    };
    let apply = match core::system_tweak_operation::confirmed_target(&confirmed_plan, &before) {
        Ok(apply) => apply,
        Err(error) => {
            if let Some((journal, _)) = journal.as_ref() {
                core::system_tweak_operation::mark_target_revalidation_failed_for_operation(
                    journal,
                    operation_id,
                    &confirmed_plan,
                    &target.target_id,
                )?;
            }
            return Err(error);
        }
    };
    if let Some((journal, operation_id)) = journal.as_ref() {
        core::system_tweak_operation::mark_target_ready_and_mutation_started_for_operation(
            journal,
            operation_id,
            &confirmed_plan,
            &target.target_id,
        )?;
    }
    let snapshot =
        serde_json::from_str::<serde_json::Value>(&confirmed_plan.targets[0].requested_action)
            .map_err(|error| core::operation::OperationError::validation(error.to_string()))?;
    let physical_state = snapshot.get("physical_state").cloned().ok_or_else(|| {
        core::operation::OperationError::validation("tweak plan has no physical pre-state")
    })?;
    let physical_state =
        serde_json::from_value::<core::system_tweak_operation::TweakPhysicalState>(physical_state)
            .map_err(|error| core::operation::OperationError::validation(error.to_string()))?;
    let mutation = match (&kind, apply) {
        (core::system_tweak_operation::TweakKind::Privacy, true) => {
            tweaks::privacy::apply(&tweak_id)
        }
        (core::system_tweak_operation::TweakKind::Privacy, false) => {
            tweaks::privacy::restore_state(&tweak_id, &physical_state)
        }
        (core::system_tweak_operation::TweakKind::Performance, true) => {
            tweaks::performance::apply(&tweak_id)
        }
        (core::system_tweak_operation::TweakKind::Performance, false) => {
            tweaks::performance::restore_state(&tweak_id, &physical_state)
        }
        (core::system_tweak_operation::TweakKind::Customization, true) => {
            tweaks::customization::apply(&tweak_id)
        }
        (core::system_tweak_operation::TweakKind::Customization, false) => {
            tweaks::customization::restore_state(&tweak_id, &physical_state)
        }
    };
    if let Err(message) = mutation {
        if let Some((journal, operation_id)) = journal.as_ref() {
            journal.record_target_state(
                operation_id,
                &target.target_id,
                core::operation::TargetExecutionState::Failed,
                &message,
            )?;
        }
        return Err(core::operation::OperationError {
            code: core::operation::OperationErrorCode::ExecutionFailed,
            message,
            target_id: Some(tweak_id.clone()),
            retryable: true,
        });
    }
    let after = match core::system_tweak_operation::find_target(&kind, &tweak_id) {
        Ok(after) => after,
        Err(error) => {
            if let Some((journal, operation_id)) = journal.as_ref() {
                journal.record_target_state(
                    operation_id,
                    &target.target_id,
                    core::operation::TargetExecutionState::OutcomeUnknown,
                    &error.message,
                )?;
            }
            return Err(error);
        }
    };
    let execution = match core::system_tweak_operation::finish(&confirmed_plan, &before, &after) {
        Ok(execution) => execution,
        Err(error) => {
            if let Some((journal, operation_id)) = journal.as_ref() {
                journal.record_target_state(
                    operation_id,
                    &target.target_id,
                    core::operation::TargetExecutionState::OutcomeUnknown,
                    &error.message,
                )?;
            }
            return Err(error);
        }
    };
    if let Some((journal, operation_id)) = journal.as_ref() {
        for outcome in &execution.result.outcomes {
            core::system_tweak_operation::record_target_outcome_for_operation(
                journal,
                operation_id,
                outcome,
            )?;
        }
    }
    let record = core::system_tweak_operation::journal_record(&confirmed_plan, &before, &execution);
    Ok((execution, record))
}

#[cfg(test)]
pub(crate) fn execute_tweak_preset_confirmed(
    confirmed_plan: core::operation::OperationPlan,
) -> std::result::Result<
    (
        core::preset_operation::PresetExecution,
        OperationJournalRecord,
    ),
    core::operation::OperationError,
> {
    execute_tweak_preset_confirmed_inner(confirmed_plan, None)
}

pub(crate) fn execute_tweak_preset_confirmed_with_journal(
    confirmed_plan: core::operation::OperationPlan,
    journal: Arc<core::operation_journal::OperationJournal>,
) -> std::result::Result<
    (
        core::preset_operation::PresetExecution,
        OperationJournalRecord,
    ),
    core::operation::OperationError,
> {
    execute_tweak_preset_confirmed_inner(confirmed_plan, Some(journal))
}

fn execute_tweak_preset_confirmed_inner(
    confirmed_plan: core::operation::OperationPlan,
    journal: Option<Arc<core::operation_journal::OperationJournal>>,
) -> std::result::Result<
    (
        core::preset_operation::PresetExecution,
        OperationJournalRecord,
    ),
    core::operation::OperationError,
> {
    let journal = journal.ok_or_else(|| {
        core::operation::OperationError::validation(
            "confirmed mutation requires an initialized operation journal",
        )
    })?;
    let child_plans = core::preset_operation::child_plans(&confirmed_plan)?;
    let mut outcomes = Vec::with_capacity(child_plans.len());
    let mut child_records = Vec::new();
    for child_plan in child_plans {
        let child_result = execute_system_tweak_confirmed_with_journal(
            child_plan.clone(),
            journal.clone(),
            confirmed_plan.id.clone(),
        );
        match child_result {
            Ok((execution, record)) => {
                outcomes.extend(execution.result.outcomes);
                child_records.push(record);
            }
            Err(error) => {
                let target_id = child_plan.targets[0].target_id.as_str();
                let pending = journal
                    .list_unresolved_targets(&confirmed_plan.id)?
                    .into_iter()
                    .find(|target| target.target_id == target_id)
                    .map(|target| target.state == core::operation::TargetExecutionState::Pending)
                    .unwrap_or(false);
                if pending {
                    core::system_tweak_operation::mark_target_revalidation_failed_for_operation(
                        &journal,
                        &confirmed_plan.id,
                        &child_plan,
                        target_id,
                    )?;
                }
                return Err(error);
            }
        }
    }
    let execution = core::preset_operation::finish(&confirmed_plan, outcomes)?;
    let record =
        core::preset_operation::journal_record(&confirmed_plan, &execution, &child_records);
    Ok((execution, record))
}

#[tauri::command]
pub(crate) async fn list_tweak_presets(
    state: tauri::State<'_, Arc<AppState>>,
) -> std::result::Result<
    Vec<core::preset_operation::PresetDefinition>,
    core::operation::OperationError,
> {
    if !state.runtime.allows_host_operations() {
        return Err(core::operation::OperationError::validation(
            "tweak presets are unavailable in fixture runtime",
        ));
    }
    tauri::async_runtime::spawn_blocking(core::preset_operation::machine_aware_definitions)
        .await
        .map_err(|error| core::operation::OperationError {
            code: core::operation::OperationErrorCode::InternalError,
            message: error.to_string(),
            target_id: None,
            retryable: true,
        })
}

#[tauri::command]
pub(crate) async fn create_tweak_preset_plan(
    state: tauri::State<'_, Arc<AppState>>,
    preset_id: String,
    selected_target_ids: Option<Vec<String>>,
) -> std::result::Result<core::operation::OperationPlan, core::operation::OperationError> {
    if !state.runtime.allows_host_operations() {
        return Err(core::operation::OperationError::validation(
            "tweak presets are unavailable in fixture runtime",
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        core::preset_operation::create_plan_with_targets(&preset_id, selected_target_ids.as_deref())
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
pub(crate) async fn execute_tweak_preset_plan(
    state: tauri::State<'_, Arc<AppState>>,
    confirmed_plan: core::operation::OperationPlan,
) -> std::result::Result<
    core::preset_operation::PresetExecution,
    core::operation::ContextualOperationError,
> {
    let error_context = core::operation::OperationErrorContext::for_plan(&confirmed_plan);
    if !state.runtime.allows_host_operations() {
        return Err(core::operation::OperationError::validation(
            "tweak presets are unavailable in fixture runtime",
        )
        .with_context(error_context));
    }
    if confirmed_plan.requires_elevation && !core::elevation::is_elevated() {
        return Err(core::operation::OperationError {
            code: core::operation::OperationErrorCode::ElevationRequired,
            message: "Administrator approval is required for this preset.".into(),
            target_id: Some(confirmed_plan.id.0),
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
        execute_tweak_preset_confirmed_with_journal(confirmed_plan, execution_journal)
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
pub(crate) async fn execute_system_tweak_plan(
    state: tauri::State<'_, Arc<AppState>>,
    confirmed_plan: core::operation::OperationPlan,
) -> std::result::Result<
    core::system_tweak_operation::SystemTweakExecution,
    core::operation::ContextualOperationError,
> {
    let error_context = core::operation::OperationErrorContext::for_plan(&confirmed_plan);
    if !state.runtime.allows_host_operations() {
        return Err(core::operation::OperationError::validation(
            "system tweak operations are unavailable in fixture runtime",
        )
        .with_context(error_context));
    }
    let settings_path = state.runtime.paths.settings.clone();
    core::system_tweak_operation::ensure_labs_enabled(&confirmed_plan, &settings_path)
        .map_err(|error| error.with_context(error_context.clone()))?;
    if confirmed_plan.requires_elevation && !core::elevation::is_elevated() {
        return Err(core::operation::OperationError {
            code: core::operation::OperationErrorCode::ElevationRequired,
            message: "Administrator approval is required for this system tweak.".into(),
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
    let plan_operation_id = confirmed_plan.id.clone();
    let execution = tauri::async_runtime::spawn_blocking(move || {
        execute_system_tweak_confirmed_with_journal(
            confirmed_plan,
            execution_journal,
            plan_operation_id,
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
    let (execution, record) = execution;
    journal
        .append(&record)
        .map_err(|error| error.with_context(error_context))?;
    Ok(execution)
}

#[tauri::command]
pub(crate) async fn list_context_menu_entries(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<tweaks::context_menu::ContextMenuEntry>> {
    if !state.runtime.allows_host_operations() {
        return Err(core::error::WinOptimizerError::Other(
            "context menu operations are unavailable in fixture runtime".into(),
        ));
    }
    let key = "list_context_menu_entries";
    if let Some(cached) = state
        .cache
        .get::<Vec<tweaks::context_menu::ContextMenuEntry>>(key)
    {
        return Ok(cached);
    }
    let value = tauri::async_runtime::spawn_blocking(tweaks::context_menu::list)
        .await
        .map_err(|e| core::error::WinOptimizerError::Other(e.to_string()))?;
    state.cache.set(key.to_string(), &value);
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_confirmed_wrappers_fail_closed_without_journal() {
        let plan = core::operation::synthetic_operation_contract()
            .unwrap()
            .plan;

        let tweak_result = execute_system_tweak_confirmed(plan.clone());
        let preset_result = execute_tweak_preset_confirmed(plan);

        assert!(matches!(
            tweak_result,
            Err(error)
                if error.code == core::operation::OperationErrorCode::ValidationFailed
        ));
        assert!(matches!(
            preset_result,
            Err(error)
                if error.code == core::operation::OperationErrorCode::ValidationFailed
        ));
    }
}
