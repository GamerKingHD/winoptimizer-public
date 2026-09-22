use crate::core::paths::AppPaths;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const SETTINGS_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("settings I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("settings data is invalid: {0}")]
    InvalidData(String),
    #[error("settings serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperienceLevel {
    #[default]
    Standard,
    Advanced,
    Expert,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiskAnalyzerPreferences {
    pub persist_snapshots: bool,
    pub max_cached_snapshots: u8,
}

impl Default for DiskAnalyzerPreferences {
    fn default() -> Self {
        Self {
            persist_snapshots: true,
            max_cached_snapshots: 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct CleanupPreferences {
    pub preselect_safe_candidates: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryPreferences {
    pub retention_days: u16,
    pub show_internal_identifiers: bool,
}

impl Default for HistoryPreferences {
    fn default() -> Self {
        Self {
            retention_days: 365,
            show_internal_identifiers: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct OnlinePreferences {
    pub allow_metered_connections: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct TelemetryPreferences {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct CrashReportingPreferences {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct UpdatePreferences {
    pub check_automatically: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct RecommendationPreferences {
    pub update_guidance: bool,
    pub preselect_safe_recommendations: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub schema_version: u32,
    pub experience_level: ExperienceLevel,
    pub labs_enabled: bool,
    pub disk_analyzer_preferences: DiskAnalyzerPreferences,
    pub cleanup_preferences: CleanupPreferences,
    pub history_preferences: HistoryPreferences,
    pub online_preferences: OnlinePreferences,
    pub telemetry_preferences: TelemetryPreferences,
    pub crash_reporting_preferences: CrashReportingPreferences,
    pub update_preferences: UpdatePreferences,
    pub recommendation_preferences: RecommendationPreferences,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            experience_level: ExperienceLevel::Standard,
            labs_enabled: false,
            disk_analyzer_preferences: DiskAnalyzerPreferences::default(),
            cleanup_preferences: CleanupPreferences::default(),
            history_preferences: HistoryPreferences::default(),
            online_preferences: OnlinePreferences::default(),
            telemetry_preferences: TelemetryPreferences::default(),
            crash_reporting_preferences: CrashReportingPreferences::default(),
            update_preferences: UpdatePreferences::default(),
            recommendation_preferences: RecommendationPreferences::default(),
        }
    }
}

impl AppSettings {
    pub fn validate(&self) -> Result<(), SettingsError> {
        if self.schema_version != SETTINGS_SCHEMA_VERSION {
            return Err(SettingsError::InvalidData(format!(
                "expected schema version {SETTINGS_SCHEMA_VERSION}, got {}",
                self.schema_version
            )));
        }
        if !(1..=20).contains(&self.disk_analyzer_preferences.max_cached_snapshots) {
            return Err(SettingsError::InvalidData(
                "cached snapshot count must be between 1 and 20".into(),
            ));
        }
        if !(1..=3650).contains(&self.history_preferences.retention_days) {
            return Err(SettingsError::InvalidData(
                "history retention must be between 1 and 3650 days".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct SettingsV1 {
    experience_level: ExperienceLevel,
    labs_enabled: bool,
}

impl Default for SettingsV1 {
    fn default() -> Self {
        let current = AppSettings::default();
        Self {
            experience_level: current.experience_level,
            labs_enabled: current.labs_enabled,
        }
    }
}

impl From<SettingsV1> for AppSettings {
    fn from(old: SettingsV1) -> Self {
        Self {
            experience_level: old.experience_level,
            labs_enabled: old.labs_enabled,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn installed() -> Result<Self, SettingsError> {
        Ok(Self::at(AppPaths::installed()?.settings))
    }

    pub fn at(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    pub fn load(&self) -> Result<AppSettings, SettingsError> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(AppSettings::default())
            }
            Err(error) => return Err(error.into()),
        };
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        let schema_version = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(1);
        let (settings, migrated) = match schema_version {
            1 => (
                AppSettings::from(serde_json::from_value::<SettingsV1>(value)?),
                true,
            ),
            version if version == u64::from(SETTINGS_SCHEMA_VERSION) => {
                (serde_json::from_value::<AppSettings>(value)?, false)
            }
            version => {
                return Err(SettingsError::InvalidData(format!(
                    "unsupported settings schema version {version}"
                )))
            }
        };
        settings.validate()?;
        if migrated {
            self.save(&settings)?;
        }
        Ok(settings)
    }

    pub fn save(&self, settings: &AppSettings) -> Result<(), SettingsError> {
        settings.validate()?;
        let mut serialized = serde_json::to_vec_pretty(settings)?;
        serialized.push(b'\n');
        write_atomic(&self.path, &serialized)
    }
}

fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), SettingsError> {
    let parent = path.parent().ok_or_else(|| {
        SettingsError::InvalidData("settings path has no parent directory".into())
    })?;
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("settings.json");
    let temporary = parent.join(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));
    let write_result = (|| -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        replace_atomically(&temporary, path)
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result.map_err(SettingsError::Io)
}

fn replace_atomically(source: &Path, target: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING};

        let mut source_wide: Vec<u16> = source.as_os_str().encode_wide().collect();
        let mut target_wide: Vec<u16> = target.as_os_str().encode_wide().collect();
        source_wide.push(0);
        target_wide.push(0);

        unsafe {
            MoveFileExW(
                PCWSTR(source_wide.as_ptr()),
                PCWSTR(target_wide.as_ptr()),
                MOVEFILE_REPLACE_EXISTING,
            )
        }
        .map_err(|error| io::Error::other(format!("atomic settings replacement failed: {error}")))
    }

    #[cfg(not(windows))]
    {
        fs::rename(source, target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_standard_local_and_opt_in_off() {
        let settings = AppSettings::default();

        assert_eq!(settings.schema_version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(settings.experience_level, ExperienceLevel::Standard);
        assert!(!settings.labs_enabled);
        assert!(!settings.cleanup_preferences.preselect_safe_candidates);
        assert!(!settings.online_preferences.allow_metered_connections);
        assert!(!settings.telemetry_preferences.enabled);
        assert!(!settings.crash_reporting_preferences.enabled);
        assert!(!settings.update_preferences.check_automatically);
        assert!(!settings.recommendation_preferences.update_guidance);
        assert!(
            !settings
                .recommendation_preferences
                .preselect_safe_recommendations
        );
        settings.validate().unwrap();
    }
    #[test]
    fn atomic_save_round_trips_without_leaving_temporary_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let store = SettingsStore::at(&path);
        let mut settings = AppSettings {
            experience_level: ExperienceLevel::Advanced,
            labs_enabled: true,
            ..Default::default()
        };

        store.save(&settings).unwrap();
        assert_eq!(store.load().unwrap(), settings);
        settings.experience_level = ExperienceLevel::Expert;
        store.save(&settings).unwrap();
        assert_eq!(store.load().unwrap(), settings);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn future_schema_is_rejected_without_overwriting_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let original = br#"{"schema_version":99,"experience_level":"expert"}"#;
        fs::write(&path, original).unwrap();

        let error = SettingsStore::at(&path).load().unwrap_err();

        assert!(error
            .to_string()
            .contains("unsupported settings schema version 99"));
        assert_eq!(fs::read(path).unwrap(), original);
    }
}
