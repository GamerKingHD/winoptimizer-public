use crate::*;

// ============================================================
// Tauri Commands - Network Monitor
// ============================================================

#[tauri::command]
pub(crate) async fn get_network_info() -> system_info::network::NetworkInfo {
    tauri::async_runtime::spawn_blocking(system_info::network::gather)
        .await
        .unwrap_or_else(|_| system_info::network::NetworkInfo {
            adapters: vec![],
            connections: vec![],
        })
}
