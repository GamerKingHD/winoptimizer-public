use crate::*;

// ============================================================
// Tauri Commands - Transactional Engine
// ============================================================

/// Build a read-only transaction preview for a set of registry targets.
/// Mutating execution requires an OperationPlan through the operation journal.
#[tauri::command]
pub(crate) async fn preview_transaction(
    description: String,
    registry_targets: Vec<core::transaction::RegistryTarget>,
) -> Result<Vec<core::transaction::PhaseResult>> {
    let mut guard = core::transaction::TransactionGuard::new(&description);
    for target in &registry_targets {
        guard.verify_registry(
            &target.hive,
            &target.key_path,
            &target.value_name,
            &target.description,
        );
    }
    Ok(guard.preview_results())
}

/// Export a binary registry hive backup of a specific key.
/// Used for manual backup or verification.
#[tauri::command]
pub(crate) async fn backup_registry_key_hive(
    state: tauri::State<'_, Arc<AppState>>,
    hive: String,
    subkey: String,
) -> Result<String> {
    let backup_mgr = state.backup_mgr.clone();
    let path = tauri::async_runtime::spawn_blocking(move || {
        let hive_hkey = match hive.as_str() {
            "HKLM" => windows::Win32::System::Registry::HKEY_LOCAL_MACHINE,
            "HKCU" => windows::Win32::System::Registry::HKEY_CURRENT_USER,
            _ => {
                return Err(core::error::WinOptimizerError::Other(format!(
                    "Unknown hive: {}",
                    hive
                )))
            }
        };
        backup_mgr
            .backup_registry_key(hive_hkey, &subkey)
            .map_err(|e| core::error::WinOptimizerError::Other(e.to_string()))
    })
    .await
    .map_err(|e| core::error::WinOptimizerError::Other(e.to_string()))??;

    Ok(path.to_string_lossy().to_string())
}
