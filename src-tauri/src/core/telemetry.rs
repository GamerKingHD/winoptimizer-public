//! Minimal, consent-gated product telemetry.
//!
//! Telemetry is deliberately local-only in this baseline.  Events use a fixed
//! schema, are validated before persistence, and are discarded when consent is
//! disabled.  There is no transport or upload path in this module.

use crate::core::settings::{SettingsError, SettingsStore};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const TELEMETRY_SCHEMA_VERSION: u32 = 1;
pub const PRIVACY_NOTICE_VERSION: &str = "1";

#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    #[error("telemetry settings could not be read: {0}")]
    Settings(#[from] SettingsError),
    #[error("telemetry storage failed: {0}")]
    Io(#[from] io::Error),
    #[error("telemetry serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("telemetry event is invalid: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryEventName {
    OperationCompleted,
    UiError,
    PerformanceSample,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryResultClass {
    Succeeded,
    PartiallySucceeded,
    Failed,
    Cancelled,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryPhase {
    Planning,
    Revalidation,
    Execution,
    Verification,
    Cancellation,
    Unknown,
}

/// The complete allow-listed event schema.
///
/// In particular, this has no field for paths, host data, process data,
/// hardware identifiers, license material, command output, or arbitrary user
/// text.  Token validation also prevents those values from being smuggled
/// through the bounded categorical fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelemetryEvent {
    pub schema_version: u32,
    pub privacy_notice_version: String,
    pub event_name: TelemetryEventName,
    pub app_version: String,
    pub channel: String,
    pub architecture: String,
    pub operation_id: Option<String>,
    pub result_class: Option<TelemetryResultClass>,
    pub phase: Option<TelemetryPhase>,
    pub error_code: Option<String>,
    pub duration_ms: Option<u64>,
}

impl TelemetryEvent {
    pub fn operation(
        operation_id: impl Into<String>,
        result_class: TelemetryResultClass,
        phase: TelemetryPhase,
        duration_ms: Option<u64>,
        error_code: Option<String>,
    ) -> Result<Self, TelemetryError> {
        let event = Self {
            schema_version: TELEMETRY_SCHEMA_VERSION,
            privacy_notice_version: PRIVACY_NOTICE_VERSION.into(),
            event_name: TelemetryEventName::OperationCompleted,
            app_version: env!("CARGO_PKG_VERSION").into(),
            channel: "stable".into(),
            architecture: std::env::consts::ARCH.into(),
            operation_id: Some(operation_id.into()),
            result_class: Some(result_class),
            phase: Some(phase),
            error_code,
            duration_ms,
        };
        event.validate()?;
        Ok(event)
    }

    pub fn validate(&self) -> Result<(), TelemetryError> {
        if self.schema_version != TELEMETRY_SCHEMA_VERSION {
            return Err(TelemetryError::Invalid(
                "unsupported telemetry schema version".into(),
            ));
        }
        if self.privacy_notice_version != PRIVACY_NOTICE_VERSION {
            return Err(TelemetryError::Invalid(
                "unsupported privacy notice version".into(),
            ));
        }
        validate_token("app_version", &self.app_version, 64)?;
        validate_token("channel", &self.channel, 32)?;
        if !matches!(self.architecture.as_str(), "x86_64" | "aarch64") {
            return Err(TelemetryError::Invalid(
                "architecture must be a coarse supported bucket".into(),
            ));
        }
        if let Some(value) = &self.operation_id {
            validate_token("operation_id", value, 128)?;
        }
        if let Some(value) = &self.error_code {
            validate_token("error_code", value, 128)?;
        }
        Ok(())
    }
}

fn validate_token(field: &str, value: &str, max_len: usize) -> Result<(), TelemetryError> {
    if value.is_empty()
        || value.len() > max_len
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(TelemetryError::Invalid(format!(
            "{field} must be a bounded categorical token"
        )));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryDisposition {
    RecordedLocally,
    Disabled,
}

#[derive(Debug, Clone)]
pub struct TelemetryStore {
    path: PathBuf,
}

impl TelemetryStore {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Re-check consent immediately before recording an event.
    pub fn record(
        &self,
        settings_path: &Path,
        event: &TelemetryEvent,
    ) -> Result<TelemetryDisposition, TelemetryError> {
        let settings = SettingsStore::at(settings_path).load()?;
        if !settings.telemetry_preferences.enabled {
            self.clear()?;
            return Ok(TelemetryDisposition::Disabled);
        }
        event.validate()?;

        let parent = self.path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "telemetry path has no parent")
        })?;
        fs::create_dir_all(parent)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        serde_json::to_writer(&mut file, event)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok(TelemetryDisposition::RecordedLocally)
    }

    pub fn clear(&self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::settings::{AppSettings, SettingsStore};
    use tempfile::tempdir;

    fn event() -> TelemetryEvent {
        TelemetryEvent::operation(
            "cleanup.reference_slice",
            TelemetryResultClass::Succeeded,
            TelemetryPhase::Verification,
            Some(42),
            None,
        )
        .unwrap()
    }

    #[test]
    fn telemetry_is_off_by_default_and_does_not_create_a_queue() {
        let directory = tempdir().unwrap();
        let settings_path = directory.path().join("settings.json");
        let store = TelemetryStore::at(directory.path().join("telemetry/events.jsonl"));

        assert_eq!(
            store.record(&settings_path, &event()).unwrap(),
            TelemetryDisposition::Disabled
        );
        assert!(!store.path().exists());
    }

    #[test]
    fn opted_in_events_use_only_the_fixed_schema() {
        let directory = tempdir().unwrap();
        let settings_path = directory.path().join("settings.json");
        let mut settings = AppSettings::default();
        settings.telemetry_preferences.enabled = true;
        SettingsStore::at(&settings_path).save(&settings).unwrap();
        let store = TelemetryStore::at(directory.path().join("telemetry/events.jsonl"));

        assert_eq!(
            store.record(&settings_path, &event()).unwrap(),
            TelemetryDisposition::RecordedLocally
        );
        let serialized = std::fs::read_to_string(store.path()).unwrap();
        assert!(serialized.contains("cleanup.reference_slice"));
        assert!(!serialized.contains("path"));
        assert!(!serialized.contains("host"));
    }

    #[test]
    fn prohibited_values_are_rejected_before_persistence() {
        let directory = tempdir().unwrap();
        let settings_path = directory.path().join("settings.json");
        let mut settings = AppSettings::default();
        settings.telemetry_preferences.enabled = true;
        SettingsStore::at(&settings_path).save(&settings).unwrap();
        let store = TelemetryStore::at(directory.path().join("telemetry/events.jsonl"));
        let mut invalid = event();
        invalid.operation_id = Some(r"C:\Users\private\secret.txt".into());

        assert!(matches!(
            store.record(&settings_path, &invalid),
            Err(TelemetryError::Invalid(_))
        ));
        assert!(!store.path().exists());
    }

    #[test]
    fn opt_out_immediately_discards_queued_events_and_blocks_future_events() {
        let directory = tempdir().unwrap();
        let settings_path = directory.path().join("settings.json");
        let mut settings = AppSettings::default();
        settings.telemetry_preferences.enabled = true;
        let settings_store = SettingsStore::at(&settings_path);
        settings_store.save(&settings).unwrap();
        let store = TelemetryStore::at(directory.path().join("telemetry/events.jsonl"));
        store.record(&settings_path, &event()).unwrap();
        assert!(store.path().exists());

        settings.telemetry_preferences.enabled = false;
        settings_store.save(&settings).unwrap();
        assert_eq!(
            store.record(&settings_path, &event()).unwrap(),
            TelemetryDisposition::Disabled
        );
        assert!(!store.path().exists());
    }
}
