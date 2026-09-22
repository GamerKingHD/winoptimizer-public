use crate::core::settings::SettingsStore;
use crate::core::update::{
    UpdateDistribution, UpdateInstallResult, UpdateKind, UpdateRequest, UpdateVerifier,
};
use crate::AppState;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateAvailability {
    UpToDate,
    Available,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateCheckResult {
    pub kind: UpdateKind,
    pub channel: String,
    pub current_version: String,
    pub version: String,
    pub availability: UpdateAvailability,
    pub distribution: UpdateDistribution,
    pub package_downloaded: bool,
    pub staged_artifact: Option<PathBuf>,
}

/// Check an authenticated release or guidance manifest.
///
/// `manual=true` is the explicit user action and downloads/stages a release
/// package for a later install command. Automatic checks authenticate only the
/// bounded manifest and never install or download the package.
#[tauri::command]
pub(crate) async fn check_for_updates(
    state: tauri::State<'_, Arc<AppState>>,
    manual: bool,
    kind: UpdateKind,
    distribution: UpdateDistribution,
) -> Result<UpdateCheckResult, String> {
    let settings_path = state.runtime.paths.settings.clone();
    let staging_dir = state.runtime.paths.update_staging();
    tauri::async_runtime::spawn_blocking(move || {
        let settings = SettingsStore::at(&settings_path)
            .load()
            .map_err(|error| error.to_string())?;
        let request = UpdateRequest::configured(kind).map_err(|error| error.to_string())?;
        let verifier = UpdateVerifier::production().map_err(|error| error.to_string())?;
        let current_version = env!("CARGO_PKG_VERSION").to_owned();

        let (manifest, package_downloaded, staged_artifact) = if manual {
            let update = verifier
                .fetch_signed_update_explicit(&request)
                .map_err(|error| error.to_string())?;
            verifier
                .verify_not_downgrade(update.manifest(), &current_version)
                .map_err(|error| error.to_string())?;
            let manifest = update.manifest().clone();
            let staged_artifact = if kind == UpdateKind::Release
                && verifier.is_newer_version(&manifest, &current_version)
            {
                Some(
                    verifier
                        .stage_release(&update, &staging_dir, distribution)
                        .map_err(|error| error.to_string())?
                        .artifact_path,
                )
            } else {
                None
            };
            (manifest, true, staged_artifact)
        } else {
            let manifest = verifier
                .fetch_signed_manifest(&request, &settings)
                .map_err(|error| error.to_string())?;
            verifier
                .verify_not_downgrade(&manifest, &current_version)
                .map_err(|error| error.to_string())?;
            (manifest, false, None)
        };

        let availability = if verifier.is_newer_version(&manifest, &current_version) {
            UpdateAvailability::Available
        } else {
            UpdateAvailability::UpToDate
        };
        Ok(UpdateCheckResult {
            kind,
            channel: manifest.channel,
            current_version: current_version.clone(),
            availability,
            version: manifest.version,
            distribution,
            package_downloaded,
            staged_artifact,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Install a release that was downloaded and re-verified by an explicit
/// manual check. No URL or arbitrary artifact path is accepted here.
#[tauri::command]
pub(crate) async fn install_update(
    state: tauri::State<'_, Arc<AppState>>,
    distribution: UpdateDistribution,
) -> Result<UpdateInstallResult, String> {
    let staging_dir = state.runtime.paths.update_staging();
    tauri::async_runtime::spawn_blocking(move || {
        let verifier = UpdateVerifier::production().map_err(|error| error.to_string())?;
        let staged = verifier
            .load_staged_release(&staging_dir, distribution)
            .map_err(|error| error.to_string())?;
        verifier
            .install_staged_release(&staged, env!("CARGO_PKG_VERSION"))
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}
