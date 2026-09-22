//! Authoritative cleanup-target authorization.
//!
//! A single model is shared by the scanners and the executor so the two never
//! disagree about what may be cleaned. A target is authorized only when it
//! belongs to a narrow, named cleanup domain: Temporary files, Windows Update
//! cache, Windows logs/dumps, browser profile caches, Delivery Optimization,
//! or Crash Dumps. No broad root such as `C:\Windows` is ever approved.
//!
//! This module owns the shared filesystem protection rule: a target must stay
//! inside an approved, canonicalized domain. Service, task, process, and AppX
//! protection remains domain-specific because those targets are not paths.
//!
//! Uses canonicalized path matching to prevent `..` traversal, symlink/junction
//! escapes, and case-sensitivity issues on Windows. Browser caches are matched
//! structurally (`<trusted-root>\<profile>\<cache-dir>\...`) so no fixed
//! profile-name list is required, while a profile root or an unrelated file
//! beneath the browser data root stays protected.

use std::fmt;
use std::path::{Component, Path, PathBuf};

/// Environment-derived base directories that anchor the cleanup domains.
///
/// Kept separate from [`SafePaths`] so tests can exercise the production
/// authorization model against a sandbox tree without touching real
/// operating-system directories.
#[derive(Debug, Clone)]
pub(crate) struct CleanupEnvironment {
    pub(crate) user_profile: PathBuf,
    pub(crate) system_root: PathBuf,
    pub(crate) local_app_data: PathBuf,
    pub(crate) temp: PathBuf,
}

impl CleanupEnvironment {
    pub(crate) fn detect() -> Self {
        let user = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\Users\\Default".into());
        let system = std::env::var("SYSTEMROOT").unwrap_or_else(|_| "C:\\Windows".into());
        let local_app_data =
            std::env::var("LOCALAPPDATA").unwrap_or_else(|_| format!("{}\\AppData\\Local", user));
        let temp = std::env::var("TEMP").unwrap_or_else(|_| format!("{}\\Temp", user));
        Self {
            user_profile: PathBuf::from(user),
            system_root: PathBuf::from(system),
            local_app_data: PathBuf::from(local_app_data),
            temp: PathBuf::from(temp),
        }
    }
}

/// A trusted browser data root. Only cache directories inside its profiles are
/// authorized; the profile root and non-cache data stay protected.
#[derive(Debug, Clone)]
struct BrowserCacheRoot {
    /// Canonical root when it existed at construction; `None` otherwise.
    canonical_root: Option<PathBuf>,
    /// Raw root, retried at check time so a directory created after startup is
    /// still authorized.
    raw_root: PathBuf,
    cache_dir_names: Vec<String>,
}

/// A compiled set of authorized cleanup domains.
pub struct SafePaths {
    /// Canonicalized approved cleanup root directories.
    canonical_roots: Vec<PathBuf>,
    /// Raw approved roots; canonicalized lazily for roots that appeared after
    /// construction.
    raw_roots: Vec<PathBuf>,
    /// Canonicalized individually approved files (for example `memory.dmp`).
    exact_files: Vec<PathBuf>,
    /// Raw approved files; canonicalized lazily like `raw_roots`.
    raw_exact_files: Vec<PathBuf>,
    /// Trusted browser roots with their approved profile cache directory names.
    browser_cache_roots: Vec<BrowserCacheRoot>,
}

/// Explainable reason for rejecting a filesystem target at the shared path
/// boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtectionReason {
    /// The canonical target is not below an approved cleanup root.
    OutsideApprovedRoot,
    /// The target could not be canonicalized for a safe identity check.
    UnresolvablePath,
}

impl ProtectionReason {
    pub const fn message(self) -> &'static str {
        match self {
            Self::OutsideApprovedRoot => "Protected because outside approved root",
            Self::UnresolvablePath => "Protected because path could not be resolved",
        }
    }
}

impl fmt::Display for ProtectionReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

/// Result of the shared filesystem protection check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafePathDecision {
    Allowed,
    Protected(ProtectionReason),
}

impl SafePathDecision {
    pub const fn is_allowed(self) -> bool {
        matches!(self, Self::Allowed)
    }

    pub const fn protection_reason(self) -> Option<ProtectionReason> {
        match self {
            Self::Allowed => None,
            Self::Protected(reason) => Some(reason),
        }
    }

    pub const fn message(self) -> Option<&'static str> {
        match self {
            Self::Allowed => None,
            Self::Protected(reason) => Some(reason.message()),
        }
    }
}

fn canonicalize_existing(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    paths
        .into_iter()
        .filter_map(|path| std::fs::canonicalize(path).ok())
        .collect()
}

/// True when the canonical target is a cache directory (or a descendant of one)
/// of some profile under the trusted browser root.
fn browser_authorizes(root: &Path, cache_dir_names: &[String], canonical: &Path) -> bool {
    let Ok(relative) = canonical.strip_prefix(root) else {
        return false;
    };
    let mut components = relative.components();
    let Some(Component::Normal(_profile)) = components.next() else {
        return false;
    };
    let Some(Component::Normal(cache_dir)) = components.next() else {
        return false;
    };
    let cache_dir = cache_dir.to_string_lossy();
    cache_dir_names
        .iter()
        .any(|name| name.eq_ignore_ascii_case(&cache_dir))
}

/// Apply the same canonicalized-root check to a single caller-owned root.
/// This keeps backup-artifact validation on the shared protection boundary
/// without expanding the cleanup whitelist for other domains.
pub(crate) fn check_path_under_root(root: &Path, path: &Path) -> SafePathDecision {
    let canonical_root = match std::fs::canonicalize(root) {
        Ok(root) => root,
        Err(_) => return SafePathDecision::Protected(ProtectionReason::UnresolvablePath),
    };
    match std::fs::canonicalize(path) {
        Ok(canonical) if canonical.starts_with(&canonical_root) => SafePathDecision::Allowed,
        Ok(_) => SafePathDecision::Protected(ProtectionReason::OutsideApprovedRoot),
        Err(_) => SafePathDecision::Protected(ProtectionReason::UnresolvablePath),
    }
}

impl SafePaths {
    /// Build the authorization model from the current process environment.
    pub fn new() -> Self {
        Self::from_environment(&CleanupEnvironment::detect())
    }

    /// Build the production authorization model for an explicit environment.
    pub(crate) fn from_environment(environment: &CleanupEnvironment) -> Self {
        let system = &environment.system_root;
        let local = &environment.local_app_data;
        let user = &environment.user_profile;

        let raw_roots = vec![
            environment.temp.clone(),
            system.join("Temp"),
            system.join("Prefetch"),
            system.join("SoftwareDistribution").join("Download"),
            system.join("SoftwareDistribution").join("DataStore"),
            system.join("Logs"),
            system.join("LiveKernelReports"),
            system.join("Minidump"),
            local.join("Microsoft").join("Windows").join("INetCache"),
            local.join("DeliveryOptimization"),
            local.join("CrashDumps"),
            user.join("AppData").join("Local").join("Temp"),
        ];
        let raw_exact_files = vec![system.join("memory.dmp")];
        let raw_browser_roots = vec![
            (
                local.join("Microsoft").join("Edge").join("User Data"),
                vec!["Cache".to_string(), "Code Cache".to_string()],
            ),
            (
                local.join("Google").join("Chrome").join("User Data"),
                vec!["Cache".to_string(), "Code Cache".to_string()],
            ),
            (
                local.join("Mozilla").join("Firefox").join("Profiles"),
                vec!["cache2".to_string()],
            ),
        ];

        let browser_cache_roots = raw_browser_roots
            .into_iter()
            .map(|(raw_root, cache_dir_names)| BrowserCacheRoot {
                canonical_root: std::fs::canonicalize(&raw_root).ok(),
                raw_root,
                cache_dir_names,
            })
            .collect();

        let canonical_roots = canonicalize_existing(raw_roots.clone());
        let exact_files = canonicalize_existing(raw_exact_files.clone());

        Self {
            canonical_roots,
            raw_roots,
            exact_files,
            raw_exact_files,
            browser_cache_roots,
        }
    }

    /// Apply the shared cleanup-domain authorization rule and explain rejection.
    ///
    /// Canonicalizes the target path to resolve `..`, symlinks, and junctions,
    /// then checks if the resolved path is inside an approved cleanup domain.
    /// Non-existent paths are rejected (canonicalize fails).
    pub fn check<P: AsRef<Path>>(&self, path: P) -> SafePathDecision {
        let canonical = match std::fs::canonicalize(path.as_ref()) {
            Ok(path) => path,
            Err(_) => return SafePathDecision::Protected(ProtectionReason::UnresolvablePath),
        };
        if self.matches_canonical(&canonical) || self.matches_appeared_later(&canonical) {
            SafePathDecision::Allowed
        } else {
            SafePathDecision::Protected(ProtectionReason::OutsideApprovedRoot)
        }
    }

    fn matches_canonical(&self, canonical: &Path) -> bool {
        self.canonical_roots
            .iter()
            .any(|root| canonical.starts_with(root))
            || self
                .exact_files
                .iter()
                .any(|file| canonical == file.as_path())
            || self.browser_cache_roots.iter().any(|browser| {
                browser.canonical_root.as_deref().is_some_and(|root| {
                    browser_authorizes(root, &browser.cache_dir_names, canonical)
                })
            })
    }

    /// Domains that did not exist at construction (for example a Windows Update
    /// cache created after startup) are canonicalized on demand so scan-time and
    /// execution-time authorization stay consistent.
    fn matches_appeared_later(&self, canonical: &Path) -> bool {
        for raw in &self.raw_roots {
            if let Ok(root) = std::fs::canonicalize(raw) {
                if canonical.starts_with(&root) {
                    return true;
                }
            }
        }
        for raw in &self.raw_exact_files {
            if let Ok(file) = std::fs::canonicalize(raw) {
                if canonical == file {
                    return true;
                }
            }
        }
        for browser in &self.browser_cache_roots {
            if browser.canonical_root.is_some() {
                continue;
            }
            if let Ok(root) = std::fs::canonicalize(&browser.raw_root) {
                if browser_authorizes(&root, &browser.cache_dir_names, canonical) {
                    return true;
                }
            }
        }
        false
    }

    /// Returns `true` if the path is in an approved cleanup domain.
    ///
    /// Kept as the compatibility boolean API; callers that need an
    /// explainable rejection should use [`Self::check`].
    pub fn is_safe<P: AsRef<Path>>(&self, path: P) -> bool {
        self.check(path).is_allowed()
    }

    /// Get the list of authorized cleanup domains (for display purposes).
    pub fn safe_roots(&self) -> Vec<&str> {
        vec![
            "User Temp (%TEMP%)",
            "Windows Temp",
            "Prefetch",
            "Windows Update cache (SoftwareDistribution)",
            "Windows logs and dumps",
            "Browser caches (Edge/Chrome/Firefox profiles)",
            "Delivery Optimization",
            "Crash Dumps",
        ]
    }
}

impl Default for SafePaths {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl SafePaths {
    /// Test-only constructor so characterization tests can pin a single root.
    pub(crate) fn from_canonical_roots(canonical_roots: Vec<PathBuf>) -> Self {
        Self {
            canonical_roots,
            raw_roots: Vec::new(),
            exact_files: Vec::new(),
            raw_exact_files: Vec::new(),
            browser_cache_roots: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    /// Build the production authorization model over a sandbox tree so every
    /// cleanup domain can be exercised without touching real system paths.
    fn sandbox_safe_paths(sandbox: &Path) -> (SafePaths, CleanupEnvironment) {
        let environment = CleanupEnvironment {
            user_profile: sandbox.join("Users/Alice"),
            system_root: sandbox.join("Windows"),
            local_app_data: sandbox.join("Users/Alice/AppData/Local"),
            temp: sandbox.join("Users/Alice/AppData/Local/Temp"),
        };
        std::fs::create_dir_all(&environment.user_profile).unwrap();
        std::fs::create_dir_all(&environment.temp).unwrap();
        (SafePaths::from_environment(&environment), environment)
    }

    fn write_file(path: &Path) -> PathBuf {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"data").unwrap();
        path.to_path_buf()
    }

    #[test]
    fn production_model_authorizes_every_advertised_cleanup_domain() {
        let sandbox = tempfile::tempdir().unwrap();
        let (safe_paths, environment) = sandbox_safe_paths(sandbox.path());

        let update_cache = write_file(
            &environment
                .system_root
                .join("SoftwareDistribution/Download/pkg.tmp"),
        );
        let update_store = write_file(
            &environment
                .system_root
                .join("SoftwareDistribution/DataStore/logs/edb.log"),
        );
        let windows_log = write_file(&environment.system_root.join("Logs/CBS/cbs.log"));
        let live_kernel = write_file(
            &environment
                .system_root
                .join("LiveKernelReports/WATCHDOG/report.dmp"),
        );
        let minidump = write_file(&environment.system_root.join("Minidump/0112.dmp"));
        let memory_dump = write_file(&environment.system_root.join("memory.dmp"));
        let prefetch = write_file(&environment.system_root.join("Prefetch/APP.pf"));
        let user_temp = write_file(&environment.temp.join("junk.tmp"));
        let chrome_default = write_file(
            &environment
                .local_app_data
                .join("Google/Chrome/User Data/Default/Cache/data_0"),
        );
        let chrome_profile_one = write_file(
            &environment
                .local_app_data
                .join("Google/Chrome/User Data/Profile 1/Cache/data_0"),
        );
        let firefox = write_file(
            &environment
                .local_app_data
                .join("Mozilla/Firefox/Profiles/abc.default-release/cache2/entries/x"),
        );

        for (label, path) in [
            ("update download", &update_cache),
            ("update datastore", &update_store),
            ("windows logs", &windows_log),
            ("live kernel", &live_kernel),
            ("minidump", &minidump),
            ("memory.dmp", &memory_dump),
            ("prefetch", &prefetch),
            ("user temp", &user_temp),
            ("chrome default cache", &chrome_default),
            ("chrome profile 1 cache", &chrome_profile_one),
            ("firefox cache2", &firefox),
        ] {
            assert!(
                safe_paths.is_safe(path),
                "{label} should be authorized: {}",
                path.display()
            );
        }
    }

    #[test]
    fn production_model_rejects_protected_neighbors() {
        let sandbox = tempfile::tempdir().unwrap();
        let (safe_paths, environment) = sandbox_safe_paths(sandbox.path());

        let system32 = write_file(&environment.system_root.join("System32/kernel32.dll"));
        let windows_root_file = write_file(&environment.system_root.join("notepad.exe"));
        let software_distribution_root = write_file(
            &environment
                .system_root
                .join("SoftwareDistribution/Other/keep.txt"),
        );
        let chrome_profile_root = write_file(
            &environment
                .local_app_data
                .join("Google/Chrome/User Data/Default/Preferences"),
        );
        let chrome_root = write_file(
            &environment
                .local_app_data
                .join("Google/Chrome/User Data/Local State"),
        );
        let firefox_profile = write_file(
            &environment
                .local_app_data
                .join("Mozilla/Firefox/Profiles/abc.default-release/prefs.js"),
        );

        for (label, path) in [
            ("system32", &system32),
            ("windows root", &windows_root_file),
            ("SoftwareDistribution sibling", &software_distribution_root),
            ("chrome profile root file", &chrome_profile_root),
            ("chrome data root file", &chrome_root),
            ("firefox profile root", &firefox_profile),
        ] {
            assert_eq!(
                safe_paths.check(path),
                SafePathDecision::Protected(ProtectionReason::OutsideApprovedRoot),
                "{label} must be protected: {}",
                path.display()
            );
        }

        assert_eq!(
            safe_paths.check(environment.system_root.join("Logs/missing.log")),
            SafePathDecision::Protected(ProtectionReason::UnresolvablePath)
        );
    }

    #[test]
    fn production_model_rejects_a_browser_profile_cache_that_escapes_via_junction() {
        let sandbox = tempfile::tempdir().unwrap();
        let (safe_paths, environment) = sandbox_safe_paths(sandbox.path());

        let outside = sandbox.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let secret = write_file(&outside.join("secret.tmp"));
        let profile = environment
            .local_app_data
            .join("Google/Chrome/User Data/Default");
        std::fs::create_dir_all(&profile).unwrap();
        let junction = profile.join("Cache");
        let status = std::process::Command::new("cmd")
            .args([
                "/C",
                "mklink",
                "/J",
                &junction.to_string_lossy(),
                &outside.to_string_lossy(),
            ])
            .status()
            .expect("mklink");
        assert!(status.success(), "mklink /J should succeed without admin");

        assert_eq!(
            safe_paths.check(junction.join("secret.tmp")),
            SafePathDecision::Protected(ProtectionReason::OutsideApprovedRoot)
        );
        assert!(secret.exists());
    }

    #[test]
    fn test_safe_temp_paths() {
        let sp = SafePaths::new();
        let temp = env::var("TEMP").unwrap_or_else(|_| "C:\\Windows\\Temp".into());

        // TEMP directory should be safe
        let temp_dir = format!("{}\\safepaths_test_dir", temp);
        let _ = std::fs::create_dir_all(&temp_dir);
        let temp_file = format!("{}\\test_file.tmp", temp_dir);
        let _ = std::fs::write(&temp_file, "test");
        assert!(sp.is_safe(&temp_file), "TEMP path should be safe");

        let temp_subdir = format!("{}\\subdir", temp_dir);
        let _ = std::fs::create_dir_all(&temp_subdir);
        let sub_file = format!("{}\\file.txt", temp_subdir);
        let _ = std::fs::write(&sub_file, "test");
        assert!(sp.is_safe(&sub_file), "TEMP subdir path should be safe");

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_unsafe_paths() {
        let sp = SafePaths::new();

        // System32 should NOT be safe
        assert!(
            !sp.is_safe("C:\\Windows\\System32\\kernel32.dll"),
            "System32 should be unsafe"
        );
        assert!(
            !sp.is_safe("C:\\Program Files\\app.exe"),
            "Program Files should be unsafe"
        );
        assert!(
            !sp.is_safe("C:\\Users\\Public\\file.txt"),
            "Public user should be unsafe"
        );
    }

    #[test]
    fn test_empty_path() {
        let sp = SafePaths::new();
        assert!(!sp.is_safe(""), "Empty path should be unsafe");
    }

    #[test]
    fn test_nonexistent_path() {
        let sp = SafePaths::new();
        assert!(
            !sp.is_safe("C:\\nonexistent\\path\\file.tmp"),
            "Nonexistent path should be unsafe"
        );
    }

    #[test]
    fn test_safe_roots_non_empty() {
        let sp = SafePaths::new();
        let roots = sp.safe_roots();
        assert!(!roots.is_empty(), "Safe roots list should not be empty");
    }

    #[test]
    fn test_canonical_roots_exist() {
        let sp = SafePaths::new();
        assert!(
            !sp.canonical_roots.is_empty(),
            "Should have at least one canonical root"
        );
    }

    #[test]
    fn path_decision_explains_the_shared_protection_boundary() {
        let directory = tempfile::tempdir().unwrap();
        let approved_root = directory.path().join("approved");
        let outside_root = directory.path().join("outside");
        std::fs::create_dir_all(&approved_root).unwrap();
        std::fs::create_dir_all(&outside_root).unwrap();
        let approved_file = approved_root.join("file.tmp");
        let outside_file = outside_root.join("file.tmp");
        std::fs::write(&approved_file, b"approved").unwrap();
        std::fs::write(&outside_file, b"protected").unwrap();

        let safe_paths =
            SafePaths::from_canonical_roots(vec![std::fs::canonicalize(&approved_root).unwrap()]);

        assert_eq!(safe_paths.check(&approved_file), SafePathDecision::Allowed);
        assert_eq!(
            safe_paths.check(&outside_file),
            SafePathDecision::Protected(ProtectionReason::OutsideApprovedRoot)
        );
        assert_eq!(
            ProtectionReason::OutsideApprovedRoot.message(),
            "Protected because outside approved root"
        );
        assert_eq!(
            ProtectionReason::OutsideApprovedRoot.to_string(),
            "Protected because outside approved root"
        );
        assert_eq!(
            safe_paths
                .check(&outside_file)
                .message()
                .expect("protected decision should explain itself"),
            "Protected because outside approved root"
        );
        assert_eq!(
            safe_paths.check(directory.path().join("missing.tmp")),
            SafePathDecision::Protected(ProtectionReason::UnresolvablePath)
        );
    }

    #[test]
    fn path_prefix_siblings_are_not_inside_the_approved_root() {
        let directory = tempfile::tempdir().unwrap();
        let approved_root = directory.path().join("approved");
        let sibling_root = directory.path().join("approved-sibling");
        std::fs::create_dir_all(&approved_root).unwrap();
        std::fs::create_dir_all(&sibling_root).unwrap();
        let sibling_file = sibling_root.join("file.tmp");
        std::fs::write(&sibling_file, b"protected").unwrap();

        let safe_paths =
            SafePaths::from_canonical_roots(vec![std::fs::canonicalize(&approved_root).unwrap()]);

        assert_eq!(
            safe_paths.check(&sibling_file),
            SafePathDecision::Protected(ProtectionReason::OutsideApprovedRoot)
        );
    }

    #[test]
    fn junction_child_is_rejected_after_canonicalize() {
        let directory = tempfile::tempdir().unwrap();
        let approved_root = directory.path().join("approved");
        let outside_root = directory.path().join("outside");
        std::fs::create_dir_all(&approved_root).unwrap();
        std::fs::create_dir_all(&outside_root).unwrap();
        let outside_file = outside_root.join("secret.tmp");
        std::fs::write(&outside_file, b"protected").unwrap();
        let junction = approved_root.join("junction");
        let status = std::process::Command::new("cmd")
            .args([
                "/C",
                "mklink",
                "/J",
                &junction.to_string_lossy(),
                &outside_root.to_string_lossy(),
            ])
            .status()
            .expect("mklink");
        assert!(status.success(), "mklink /J should succeed without admin");

        let safe_paths =
            SafePaths::from_canonical_roots(vec![std::fs::canonicalize(&approved_root).unwrap()]);
        assert_eq!(
            safe_paths.check(junction.join("secret.tmp")),
            SafePathDecision::Protected(ProtectionReason::OutsideApprovedRoot)
        );
    }
}
