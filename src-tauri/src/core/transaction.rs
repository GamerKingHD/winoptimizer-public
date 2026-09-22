//! Legacy transaction preview model.
//!
//! Read-only manifests and previews remain available here. Mutating execution
//! is governed by `OperationPlan` and `OperationJournal::begin_execution`.

use crate::core::backup::BackupManager;
use crate::core::restore_point::StateMgrStatus;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// -- Phase status ---------------------------------------------

/// Outcome of a single pipeline phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PhaseStatus {
    /// Phase succeeded.
    Ok,
    /// Phase was skipped (no targets registered for this phase).
    Skipped,
    /// Phase failed with a message - pipeline halts.
    Failed(String),
}

/// Result of executing a pipeline phase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseResult {
    pub phase: String,
    pub status: PhaseStatus,
    /// Human-readable detail (e.g. restore point sequence number).
    pub detail: Option<String>,
}

impl PhaseResult {
    fn ok(phase: &str, detail: Option<String>) -> Self {
        Self {
            phase: phase.to_string(),
            status: PhaseStatus::Ok,
            detail,
        }
    }

    fn skipped(phase: &str) -> Self {
        Self {
            phase: phase.to_string(),
            status: PhaseStatus::Skipped,
            detail: None,
        }
    }

    fn failed(phase: &str, msg: &str) -> Self {
        Self {
            phase: phase.to_string(),
            status: PhaseStatus::Failed(msg.to_string()),
            detail: None,
        }
    }
}

// -- Dry-run manifest ----------------------------------------

/// A single registry key that will be modified.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryTarget {
    pub hive: String, // "HKLM" | "HKCU"
    pub key_path: String,
    pub value_name: String,
    pub description: String,
}

/// A single file that will be deleted or modified.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTarget {
    pub path: String,
    pub description: String,
}

/// A single service whose state will change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceTarget {
    pub service_name: String,
    pub description: String,
}

/// Pre-execution manifest - shows everything that will change.
/// Sent to the frontend for user review before commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DryRunManifest {
    pub description: String,
    pub registry_targets: Vec<RegistryTarget>,
    pub file_targets: Vec<FileTarget>,
    pub service_targets: Vec<ServiceTarget>,
}

impl DryRunManifest {
    pub fn is_empty(&self) -> bool {
        self.registry_targets.is_empty()
            && self.file_targets.is_empty()
            && self.service_targets.is_empty()
    }

    pub fn total_changes(&self) -> usize {
        self.registry_targets.len() + self.file_targets.len() + self.service_targets.len()
    }
}

// -- TransactionGuard -----------------------------------------

/// Builds a read-only manifest before a governed operation is executed.
///
/// Usage:
/// ```ignore
/// let guard = TransactionGuard::new("Apply privacy tweaks");
/// guard.verify_registry("HKLM", r"SOFTWARE\...", "AllowTelemetry", "Disable telemetry");
/// guard.verify_service("DiagTrack", "Stop telemetry service");
///
/// // Frontend shows guard.manifest() to user
/// let preview = guard.preview_results();
/// // Mutations require an OperationPlan through OperationJournal.
/// ```
pub struct TransactionGuard {
    description: String,
    registry_targets: Vec<RegistryTarget>,
    file_targets: Vec<FileTarget>,
    service_targets: Vec<ServiceTarget>,
    /// Legacy compatibility storage; fail-closed commits never populate it.
    registry_backups: Vec<(String, PathBuf)>,
}

impl TransactionGuard {
    /// Create a new transaction guard with a human-readable description.
    pub fn new(description: &str) -> Self {
        Self {
            description: description.to_string(),
            registry_targets: Vec::new(),
            file_targets: Vec::new(),
            service_targets: Vec::new(),
            registry_backups: Vec::new(),
        }
    }

    // -- Phase 1: Verification (dry-run) ----------------------

    /// Register a registry key that will be modified.
    pub fn verify_registry(
        &mut self,
        hive: &str,
        key_path: &str,
        value_name: &str,
        description: &str,
    ) {
        self.registry_targets.push(RegistryTarget {
            hive: hive.to_string(),
            key_path: key_path.to_string(),
            value_name: value_name.to_string(),
            description: description.to_string(),
        });
    }

    /// Register a file that will be deleted or modified.
    pub fn verify_file(&mut self, path: &str, description: &str) {
        self.file_targets.push(FileTarget {
            path: path.to_string(),
            description: description.to_string(),
        });
    }

    /// Register a service whose state will change.
    pub fn verify_service(&mut self, service_name: &str, description: &str) {
        self.service_targets.push(ServiceTarget {
            service_name: service_name.to_string(),
            description: description.to_string(),
        });
    }

    /// Produce the dry-run manifest for frontend preview.
    pub fn manifest(&self) -> DryRunManifest {
        DryRunManifest {
            description: self.description.clone(),
            registry_targets: self.registry_targets.clone(),
            file_targets: self.file_targets.clone(),
            service_targets: self.service_targets.clone(),
        }
    }

    /// Return the validation phase without creating restore points or backups.
    pub fn preview_results(&self) -> Vec<PhaseResult> {
        if self.manifest().is_empty() {
            vec![PhaseResult::skipped("verify")]
        } else {
            vec![PhaseResult::ok(
                "verify",
                Some(format!(
                    "{} total changes previewed",
                    self.manifest().total_changes()
                )),
            )]
        }
    }

    // -- Legacy commit compatibility --------------------------

    /// Reject the legacy ungoverned commit path.
    ///
    /// Restore-point creation is authorized only by the operation journal,
    /// after it has validated and persisted an `OperationPlan`. Keeping this
    /// compatibility method fail-closed prevents callers from mutating after
    /// an unjournaled safety pipeline.
    pub fn commit(
        &mut self,
        _backup_mgr: &BackupManager,
    ) -> (Vec<PhaseResult>, Option<StateMgrStatus>) {
        let mut results = Vec::new();

        // -- Phase 1: Verify ---------------------------------
        if self.manifest().is_empty() {
            results.push(PhaseResult::skipped("verify"));
        } else {
            results.push(PhaseResult::ok(
                "verify",
                Some(format!(
                    "{} total changes previewed",
                    self.manifest().total_changes()
                )),
            ));
        }

        results.push(PhaseResult::failed(
            "restore_point",
            "legacy TransactionGuard::commit is disabled; mutating execution requires an OperationPlan through OperationJournal::begin_execution",
        ));

        (results, None)
    }

    /// Get the legacy backup list; fail-closed commits never populate it.
    pub fn backup_paths(&self) -> &[(String, PathBuf)] {
        &self.registry_backups
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_reflects_registered_targets() {
        let mut guard = TransactionGuard::new("Test transaction");
        guard.verify_registry("HKLM", r"SOFTWARE\Test", "Example", "Test registry change");
        guard.verify_file(r"C:\test.txt", "Test file deletion");
        guard.verify_service("TestSvc", "Test service change");

        let manifest = guard.manifest();
        assert_eq!(manifest.description, "Test transaction");
        assert_eq!(manifest.total_changes(), 3);
        assert_eq!(manifest.registry_targets.len(), 1);
        assert_eq!(manifest.file_targets.len(), 1);
        assert_eq!(manifest.service_targets.len(), 1);
    }

    #[test]
    fn empty_manifest_is_empty() {
        let guard = TransactionGuard::new("Empty");
        assert!(guard.manifest().is_empty());
    }

    #[test]
    fn preview_remains_read_only() {
        let mut guard = TransactionGuard::new("Preview only");
        guard.verify_file(r"C:\test.txt", "Test file deletion");

        let results = guard.preview_results();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].phase, "verify");
        assert!(matches!(&results[0].status, PhaseStatus::Ok));
    }

    #[test]
    fn legacy_commit_fails_closed_without_journal_authorization() {
        let mut guard = TransactionGuard::new("Legacy commit");
        guard.verify_registry("HKCU", r"SOFTWARE\Test", "Example", "Test registry change");

        let backup_mgr = BackupManager::new().expect("LOCALAPPDATA should be available in tests");
        let (results, restore_point) = guard.commit(&backup_mgr);

        assert!(matches!(&results[0].status, PhaseStatus::Ok));
        assert!(matches!(
            &results[1].status,
            PhaseStatus::Failed(message)
                if message.contains("OperationPlan")
                    && message.contains("OperationJournal::begin_execution")
        ));
        assert!(restore_point.is_none());
        assert!(guard.backup_paths().is_empty());
    }
}
