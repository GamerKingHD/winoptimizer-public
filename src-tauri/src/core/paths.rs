use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub data_root: PathBuf,
    pub journal: PathBuf,
    pub backups: PathBuf,
    pub snapshots: PathBuf,
    pub preview_cache: PathBuf,
    pub settings: PathBuf,
    pub logs: PathBuf,
    pub licenses: PathBuf,
    pub performance_experiments: PathBuf,
}

impl AppPaths {
    pub fn installed() -> io::Result<Self> {
        let local_app_data = std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA is unavailable")
        })?;
        Ok(Self::from_data_root(
            PathBuf::from(local_app_data).join("WinOptimizer"),
        ))
    }

    pub fn from_data_root(data_root: impl AsRef<Path>) -> Self {
        let data_root = data_root.as_ref().to_path_buf();
        Self {
            journal: data_root.join("journal").join("operations.db"),
            backups: data_root.join("backups"),
            snapshots: data_root.join("disk-analyzer"),
            preview_cache: data_root.join("preview-cache"),
            settings: data_root.join("settings.json"),
            logs: data_root.join("logs"),
            licenses: data_root.join("licenses"),
            performance_experiments: data_root.join("performance-experiments.db"),
            data_root,
        }
    }

    /// Canonical path for retained legacy recovery history.
    ///
    /// The operation journal is the current history store, but legacy mutation
    /// and History/Startup compatibility paths still use this database. Keep
    /// the path stable until those callers are migrated; existing rows are not
    /// imported, rewritten, or deleted by this path abstraction.
    pub fn undo_log(&self) -> PathBuf {
        self.data_root.join("undo").join("undo.db")
    }

    /// Canonical path for current Startup Manager recovery records.
    pub fn startup_recovery(&self) -> PathBuf {
        self.data_root.join("startup").join("recovery.db")
    }

    /// Root for support-bundle artifacts, separate from both history stores.
    pub fn support_bundles(&self) -> PathBuf {
        self.data_root.join("support")
    }

    /// Local opt-in telemetry queue. It is never uploaded by the core.
    pub fn telemetry_events(&self) -> PathBuf {
        self.data_root.join("telemetry").join("events.jsonl")
    }

    /// Verified release packages awaiting an explicit install action.
    pub fn update_staging(&self) -> PathBuf {
        self.data_root.join("updates").join("staged")
    }

    pub fn confirmed_plans(&self) -> PathBuf {
        self.data_root.join("confirmed-plans")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_root_derives_all_application_paths() {
        let root = PathBuf::from("deterministic-root");
        let paths = AppPaths::from_data_root(&root);

        assert_eq!(paths.data_root, root);
        assert_eq!(paths.journal, root.join("journal").join("operations.db"));
        assert_eq!(paths.backups, root.join("backups"));
        assert_eq!(paths.snapshots, root.join("disk-analyzer"));
        assert_eq!(paths.preview_cache, root.join("preview-cache"));
        assert_eq!(paths.settings, root.join("settings.json"));
        assert_eq!(paths.logs, root.join("logs"));
        assert_eq!(paths.licenses, root.join("licenses"));
        assert_eq!(
            paths.performance_experiments,
            root.join("performance-experiments.db")
        );
        assert_eq!(paths.undo_log(), root.join("undo").join("undo.db"));
        assert_eq!(
            paths.startup_recovery(),
            root.join("startup").join("recovery.db")
        );
        assert_eq!(paths.support_bundles(), root.join("support"));
        assert_eq!(
            paths.telemetry_events(),
            root.join("telemetry").join("events.jsonl")
        );
        assert_eq!(paths.update_staging(), root.join("updates").join("staged"));
        assert_eq!(paths.confirmed_plans(), root.join("confirmed-plans"));
    }

    #[test]
    fn retained_legacy_history_path_is_separate_from_current_journal() {
        let root = PathBuf::from("deterministic-root");
        let paths = AppPaths::from_data_root(&root);

        assert_eq!(paths.undo_log(), root.join("undo").join("undo.db"));
        assert_ne!(paths.undo_log(), paths.journal);
        assert_ne!(paths.startup_recovery(), paths.undo_log());
    }
}
