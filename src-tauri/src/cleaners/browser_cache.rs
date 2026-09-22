//! Browser cache cleaner - detects and cleans caches for Edge, Chrome, and Firefox.
//! Detection uses known paths under %LOCALAPPDATA%. Browser must NOT be running.

use crate::cleaners::CleanItem;
use crate::core::error::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserInfo {
    pub name: String,
    pub cache_path: Option<String>,
    pub is_running: bool,
}

/// Detect installed browsers and their cache locations.
pub fn detect_browsers() -> Result<Vec<BrowserInfo>> {
    let local_app_data = PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default());
    let mut browsers = Vec::new();

    browsers.extend(discover_profile_caches(
        &local_app_data
            .join("Microsoft")
            .join("Edge")
            .join("User Data"),
        "Microsoft Edge",
        "msedge.exe",
        "Cache",
    ));
    browsers.extend(discover_profile_caches(
        &local_app_data
            .join("Google")
            .join("Chrome")
            .join("User Data"),
        "Google Chrome",
        "chrome.exe",
        "Cache",
    ));
    browsers.extend(discover_profile_caches(
        &local_app_data
            .join("Mozilla")
            .join("Firefox")
            .join("Profiles"),
        "Mozilla Firefox",
        "firefox.exe",
        "cache2",
    ));

    Ok(browsers)
}

fn discover_profile_caches(
    profiles_root: &std::path::Path,
    browser_name: &str,
    process_name: &str,
    cache_dir_name: &str,
) -> Vec<BrowserInfo> {
    let is_running = is_process_running(process_name);
    let Ok(entries) = std::fs::read_dir(profiles_root) else {
        return Vec::new();
    };

    entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let profile_path = entry.path();
            if !profile_path.is_dir() {
                return None;
            }
            let cache_path = profile_path.join(cache_dir_name);
            if !cache_path.is_dir() {
                return None;
            }
            Some(BrowserInfo {
                name: format!("{} ({})", browser_name, entry.file_name().to_string_lossy()),
                cache_path: Some(cache_path.to_string_lossy().to_string()),
                is_running,
            })
        })
        .collect()
}

/// Scan browser cache directories for cleanable files.
pub fn scan(browsers: &[BrowserInfo]) -> Result<Vec<CleanItem>> {
    let mut items = Vec::new();

    for browser in browsers {
        if browser.is_running {
            continue; // Skip running browsers
        }
        if let Some(ref cache_path) = browser.cache_path {
            let path = std::path::Path::new(cache_path);
            if path.exists() {
                scan_cache_dir(path, &browser.name, &mut items)?;
            }
        }
    }

    items.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    Ok(items)
}

fn scan_cache_dir(
    dir: &std::path::Path,
    browser_name: &str,
    items: &mut Vec<CleanItem>,
) -> Result<()> {
    for entry in WalkDir::new(dir)
        .max_depth(5)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            let meta = entry.metadata().ok();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            if size > 0 {
                items.push(CleanItem {
                    path: entry.path().to_string_lossy().to_string(),
                    size_bytes: size,
                    last_accessed: meta.and_then(|m| m.accessed().ok()).map(|t| {
                        let st: std::time::SystemTime = t;
                        let dt: chrono::DateTime<chrono::Utc> = st.into();
                        dt.to_rfc3339()
                    }),
                    category: "browser_cache".to_string(),
                    description: format!("{} cache", browser_name),
                    group_key: browser_name.to_string(),
                    sub_group: "Cache".to_string(),
                });
            }
        }
    }

    Ok(())
}

fn is_process_running(name: &str) -> bool {
    let mut system = sysinfo::System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        sysinfo::ProcessRefreshKind::nothing(),
    );
    let found = system.processes_by_name(name.as_ref()).next().is_some();
    found
}

#[cfg(test)]
mod tests {
    use super::{discover_profile_caches, scan_cache_dir};
    use std::fs;

    #[test]
    fn discovers_each_profile_cache_without_scanning_the_profile_root() {
        let root = tempfile::tempdir().expect("temp directory");
        fs::create_dir_all(root.path().join("Default").join("Cache")).expect("edge cache");
        fs::create_dir_all(root.path().join("Profile 1").join("Cache")).expect("chrome cache");
        fs::create_dir_all(root.path().join("Profile 2")).expect("profile without cache");

        let browsers = discover_profile_caches(
            root.path(),
            "Test Browser",
            "process-that-does-not-exist.exe",
            "Cache",
        );

        assert_eq!(browsers.len(), 2);
        assert!(browsers
            .iter()
            .any(|browser| browser.name == "Test Browser (Default)"));
        assert!(browsers
            .iter()
            .any(|browser| browser.name == "Test Browser (Profile 1)"));
        assert!(browsers.iter().all(|browser| {
            browser
                .cache_path
                .as_deref()
                .is_some_and(|path| path.ends_with("Cache"))
        }));
    }

    #[test]
    fn scanned_browser_files_report_cache_as_the_cleanup_type() {
        let root = tempfile::tempdir().expect("temp directory");
        let cache = root.path().join("Cache");
        fs::create_dir_all(&cache).expect("cache directory");
        fs::write(cache.join("data.bin"), b"cache data").expect("cache file");
        let mut items = Vec::new();

        scan_cache_dir(&cache, "Google Chrome (Default)", &mut items).expect("scan cache");

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].group_key, "Google Chrome (Default)");
        assert_eq!(items[0].sub_group, "Cache");
    }
}
