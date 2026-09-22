use crate::commands::hosts::hosts_internal_error;
use crate::*;

// ============================================================
// Tauri Commands - Environment Variables
// ============================================================

#[tauri::command]
pub(crate) async fn list_env_vars(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<system_info::env_vars::EnvVar>> {
    let registry = state.adapters.registry.clone();
    tauri::async_runtime::spawn_blocking(move || {
        core::env_operation::list_from_adapter(registry.as_ref())
            .map_err(|error| core::error::WinOptimizerError::Other(error.to_string()))
    })
    .await
    .map_err(|e| core::error::WinOptimizerError::Other(e.to_string()))?
}

#[tauri::command]
pub(crate) async fn create_env_plan(
    state: tauri::State<'_, Arc<AppState>>,
    action: core::env_operation::EnvAction,
) -> std::result::Result<core::operation::OperationPlan, core::operation::OperationError> {
    let registry = state.adapters.registry.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let current = core::env_operation::list_from_adapter(registry.as_ref())?;
        core::env_operation::create_plan(&current, action)
    })
    .await
    .map_err(|error| hosts_internal_error(error.to_string()))?
}

pub(crate) fn execute_env_confirmed_with_journal(
    plan: core::operation::OperationPlan,
    journal: &core::operation_journal::OperationJournal,
    registry: &dyn core::adapters::RegistryAdapter,
) -> std::result::Result<
    (core::env_operation::EnvExecution, OperationJournalRecord),
    core::operation::OperationError,
> {
    let current = core::env_operation::list_from_adapter(registry)?;
    let target = plan.targets.first().ok_or_else(|| {
        core::operation::OperationError::validation("environment plan has no target")
    })?;
    let action = match core::env_operation::confirmed_action(&plan, &current) {
        Ok(action) => action,
        Err(error) => {
            core::env_operation::mark_target_revalidation_failed(
                journal,
                &plan,
                &target.target_id,
            )?;
            return Err(error);
        }
    };
    let (target_name, target_scope) = match &action {
        core::env_operation::EnvAction::Set { variable } => {
            (variable.name.clone(), variable.scope.clone())
        }
        core::env_operation::EnvAction::Delete { name, scope } => (name.clone(), scope.clone()),
    };
    let execution_plan = plan.clone();
    let execution_name = target_name.clone();
    let execution_scope = target_scope.clone();
    let execution =
        core::env_operation::execute_target_with_journal(journal, &plan, target, move || {
            match action {
                core::env_operation::EnvAction::Set { variable } => {
                    core::env_operation::mutate_with_adapter(
                        registry,
                        &core::env_operation::EnvAction::Set { variable },
                    )
                }
                core::env_operation::EnvAction::Delete { name, scope } => {
                    core::env_operation::mutate_with_adapter(
                        registry,
                        &core::env_operation::EnvAction::Delete { name, scope },
                    )
                }
            }?;
            let after = core::env_operation::read_from_adapter(
                registry,
                &execution_name,
                &execution_scope,
            )?
            .into_iter()
            .collect::<Vec<_>>();
            core::env_operation::finish(&execution_plan, &after)
        })?;
    let record = core::env_operation::journal_record(&plan, &execution);
    Ok((execution, record))
}

#[tauri::command]
pub(crate) async fn execute_env_plan(
    state: tauri::State<'_, Arc<AppState>>,
    confirmed_plan: core::operation::OperationPlan,
) -> std::result::Result<core::env_operation::EnvExecution, core::operation::ContextualOperationError>
{
    let error_context = core::operation::OperationErrorContext::for_plan(&confirmed_plan);
    let current = core::env_operation::list_from_adapter(state.adapters.registry.as_ref())
        .map_err(|error| error.with_context(error_context.clone()))?;
    let action = core::env_operation::confirmed_action(&confirmed_plan, &current)
        .map_err(|error| error.with_context(error_context.clone()))?;
    if core::env_operation::action_requires_elevation(&action)
        && !state.adapters.elevation.is_elevated()
    {
        return Err(core::operation::OperationError {
            code: core::operation::OperationErrorCode::ElevationRequired,
            message: "Administrator approval is required for a System variable.".into(),
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
    let registry = state.adapters.registry.clone();
    let (execution, record) = tauri::async_runtime::spawn_blocking(move || {
        execute_env_confirmed_with_journal(confirmed_plan, &execution_journal, registry.as_ref())
    })
    .await
    .map_err(|error| hosts_internal_error(error.to_string()).with_context(error_context.clone()))?
    .map_err(|error: core::operation::OperationError| error.with_context(error_context.clone()))?;
    journal
        .append(&record)
        .map_err(|error| error.with_context(error_context))?;
    Ok(execution)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::adapters::{FixtureRegistryAdapter, RegistryAdapter, RegistryValue};
    use crate::core::operation_journal::OperationJournal;
    use crate::system_info::env_vars::EnvVar;

    #[test]
    fn confirmed_environment_command_routes_mutation_through_fixture_registry() {
        let registry = FixtureRegistryAdapter::new();
        let current = vec![EnvVar {
            name: "Path".into(),
            value: "before".into(),
            scope: "user".into(),
            var_type: "REG_SZ".into(),
        }];
        registry
            .write(
                crate::system_info::env_vars::USER_ENVIRONMENT_KEY,
                "Path",
                RegistryValue::String("before".into()),
            )
            .unwrap();
        let plan = core::env_operation::create_plan(
            &current,
            core::env_operation::EnvAction::Set {
                variable: EnvVar {
                    value: "after".into(),
                    ..current[0].clone()
                },
            },
        )
        .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let journal = OperationJournal::open_at(&directory.path().join("journal.db")).unwrap();
        journal.begin_confirmed_execution(&plan).unwrap();

        execute_env_confirmed_with_journal(plan, &journal, &registry).unwrap();

        assert_eq!(
            registry
                .read(crate::system_info::env_vars::USER_ENVIRONMENT_KEY, "pAtH")
                .unwrap(),
            Some(RegistryValue::Raw {
                value_type: 1,
                bytes: "after\0"
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect(),
            })
        );
    }
}
