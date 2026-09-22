use crate::core::operation::{OperationError, OperationErrorCode, OperationPlan};
use crate::core::performance_experiment::{linked_operation_id, PerformanceExperimentStore};
use serde_json::Value;

fn internal_error(error: impl std::fmt::Display) -> OperationError {
    OperationError {
        code: OperationErrorCode::InternalError,
        message: error.to_string(),
        target_id: None,
        retryable: true,
    }
}

#[tauri::command]
pub(crate) async fn list_performance_experiments() -> Result<Vec<Value>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        PerformanceExperimentStore::open_default()
            .and_then(|store| store.list())
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn save_performance_experiment(
    state: tauri::State<'_, std::sync::Arc<crate::AppState>>,
    experiment: Value,
) -> Result<Value, String> {
    let operation_id = linked_operation_id(&experiment).map_err(|error| error.to_string())?;
    let journal = match operation_id {
        Some(_) => Some(
            state
                .ensure_operation_journal()
                .map_err(|error| error.to_string())?,
        ),
        None => None,
    };
    tauri::async_runtime::spawn_blocking(move || {
        if let (Some(operation_id), Some(journal)) = (operation_id.as_deref(), journal.as_ref()) {
            let known = journal
                .contains_operation_id(operation_id)
                .map_err(|error| error.to_string())?;
            if !known {
                return Err(format!(
                    "performance experiment references unknown operation '{operation_id}'"
                ));
            }
        }
        PerformanceExperimentStore::open_default()
            .and_then(|store| store.save(experiment))
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn prepare_performance_experiment_revert(
    state: tauri::State<'_, std::sync::Arc<crate::AppState>>,
    operation_id: String,
) -> std::result::Result<OperationPlan, OperationError> {
    let journal = state.ensure_operation_journal()?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::core::performance_experiment::create_revert_plan(&journal, &operation_id)
    })
    .await
    .map_err(internal_error)?
}

#[tauri::command]
pub(crate) async fn record_performance_experiment_decision(
    state: tauri::State<'_, std::sync::Arc<crate::AppState>>,
    experiment: Value,
    decision: String,
    decision_operation_id: Option<String>,
) -> std::result::Result<Value, OperationError> {
    let journal = state.ensure_operation_journal()?;
    tauri::async_runtime::spawn_blocking(move || {
        let finalized = crate::core::performance_experiment::finalize_decision(
            experiment,
            &decision,
            decision_operation_id.as_deref(),
            &journal,
        )?;
        PerformanceExperimentStore::open_default()
            .and_then(|store| store.save(finalized))
            .map_err(internal_error)
    })
    .await
    .map_err(internal_error)?
}
