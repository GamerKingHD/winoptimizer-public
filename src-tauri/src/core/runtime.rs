use crate::core::paths::AppPaths;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMode {
    Production,
    Fixture,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeContext {
    pub mode: RuntimeMode,
    pub paths: AppPaths,
}

impl RuntimeContext {
    pub fn production() -> io::Result<Self> {
        Ok(Self {
            mode: RuntimeMode::Production,
            paths: AppPaths::installed()?,
        })
    }

    pub fn fixture(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref();
        let rooted = if root.is_absolute() {
            root.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|current| current.join(root))
                .unwrap_or_else(|_| root.to_path_buf())
        };
        let root = std::fs::canonicalize(&rooted).unwrap_or(rooted);
        #[cfg(windows)]
        let root = root
            .to_string_lossy()
            .strip_prefix(r"\\?\")
            .map(Path::new)
            .unwrap_or(root.as_path())
            .to_path_buf();
        Self {
            mode: RuntimeMode::Fixture,
            paths: AppPaths::from_data_root(root),
        }
    }

    pub fn data_root(&self) -> &Path {
        &self.paths.data_root
    }

    pub fn is_fixture(&self) -> bool {
        self.mode == RuntimeMode::Fixture
    }

    /// Host-bound operations must only run under the production runtime.
    pub fn allows_host_operations(&self) -> bool {
        !self.is_fixture()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixture_context_is_deterministic_and_never_reads_environment() {
        let context = RuntimeContext::fixture("fixture-root");

        assert_eq!(context.mode, RuntimeMode::Fixture);
        assert!(context.is_fixture());
        assert!(!context.allows_host_operations());
        assert!(!RuntimeContext::fixture("fixture-root").allows_host_operations());
        let expected_root = std::env::current_dir().unwrap().join("fixture-root");
        assert_eq!(context.data_root(), expected_root.as_path());
        assert_eq!(context.paths.settings, expected_root.join("settings.json"));
    }
}
