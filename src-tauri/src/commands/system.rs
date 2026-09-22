use crate::*;

// ============================================================
// Tauri Commands - System Info
// ============================================================

#[tauri::command]
pub(crate) async fn get_hardware_info(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<system_info::hardware::HardwareInfo> {
    let key = "get_hardware_info";
    if let Some(cached) = state.cache.get::<system_info::hardware::HardwareInfo>(key) {
        return Ok(cached);
    }
    let value = tauri::async_runtime::spawn_blocking(system_info::hardware::gather)
        .await
        .unwrap_or_else(|_| system_info::hardware::HardwareInfo {
            cpu: system_info::hardware::CpuInfo {
                name: "Unknown".into(),
                cores: 0,
                logical_processors: 0,
                architecture: "Unknown".into(),
                usage_percent: 0.0,
            },
            memory: system_info::hardware::MemoryInfo {
                total_bytes: 0,
                available_bytes: 0,
                in_use_percent: 0.0,
            },
            disks: vec![],
            gpu: None,
        });
    state.cache.set(key.to_string(), &value);
    Ok(value)
}

#[tauri::command]
pub(crate) async fn get_os_info(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<system_info::os::OsInfo> {
    let key = "get_os_info";
    if let Some(cached) = state.cache.get::<system_info::os::OsInfo>(key) {
        return Ok(cached);
    }
    let adapter = state.adapters.system_info.clone();
    let value = adapter.os();
    let value = system_info::os::OsInfo {
        version: value.version,
        edition: value.edition,
        build: value.build,
        arch: value.arch,
        release_id: value.release_id,
        install_date: value.install_date,
        last_boot: value.last_boot,
        user_name: value.user_name,
        domain: value.domain,
    };
    state.cache.set(key.to_string(), &value);
    Ok(value)
}

#[tauri::command]
pub(crate) async fn get_installed_updates(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<system_info::updates::UpdateInfo>> {
    let key = "get_installed_updates";
    if let Some(cached) = state
        .cache
        .get::<Vec<system_info::updates::UpdateInfo>>(key)
    {
        return Ok(cached);
    }
    let value = tauri::async_runtime::spawn_blocking(system_info::updates::list_installed_updates)
        .await
        .map_err(|e| core::error::WinOptimizerError::Other(e.to_string()))?;
    state.cache.set(key.to_string(), &value);
    Ok(value)
}

#[tauri::command]
pub(crate) async fn get_battery_info() -> Result<system_info::battery::BatteryInfo> {
    tauri::async_runtime::spawn_blocking(system_info::battery::gather)
        .await
        .map_err(|e| core::error::WinOptimizerError::Other(e.to_string()))
}
