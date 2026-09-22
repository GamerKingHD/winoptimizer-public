use crate::core::settings::{AppSettings, SettingsStore};
use crate::core::telemetry::{TelemetryDisposition, TelemetryEvent, TelemetryStore};
use crate::AppState;
use std::sync::Arc;

#[tauri::command]
pub(crate) async fn get_settings(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<AppSettings, String> {
    let settings_path = state.runtime.paths.settings.clone();
    tauri::async_runtime::spawn_blocking(move || {
        SettingsStore::at(settings_path)
            .load()
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn update_settings(
    state: tauri::State<'_, Arc<AppState>>,
    settings: AppSettings,
) -> Result<AppSettings, String> {
    let settings_path = state.runtime.paths.settings.clone();
    let telemetry_path = state.runtime.paths.telemetry_events();
    tauri::async_runtime::spawn_blocking(move || {
        let store = SettingsStore::at(settings_path);
        store.save(&settings).map_err(|error| error.to_string())?;
        if !settings.telemetry_preferences.enabled {
            TelemetryStore::at(telemetry_path)
                .clear()
                .map_err(|error| error.to_string())?;
        }
        Ok(settings)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn record_telemetry_event(
    state: tauri::State<'_, Arc<AppState>>,
    event: TelemetryEvent,
) -> Result<TelemetryDisposition, String> {
    let settings_path = state.runtime.paths.settings.clone();
    let telemetry_path = state.runtime.paths.telemetry_events();
    tauri::async_runtime::spawn_blocking(move || {
        TelemetryStore::at(telemetry_path)
            .record(&settings_path, &event)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}
