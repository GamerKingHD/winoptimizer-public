//! Scans temporary files and optional Prefetch space: Windows temp, user temp,
//! and Prefetch are separate cleanup categories.

use crate::cleaners::CleanItem;
use crate::core::error::Result;
use crate::core::safe_paths::SafePaths;
use std::path::PathBuf;
use walkdir::WalkDir;

/// Scan all known temp directories for cleanable files.
/// Returns a list of items sorted by size descending.
pub fn scan(safe_paths: &SafePaths) -> Result<Vec<CleanItem>> {
    let temp = std::env::var("TEMP").unwrap_or_else(|_| "".into());
    let system_root = std::env::var("SYSTEMROOT").unwrap_or_else(|_| "C:\\Windows".into());
    let win_temp = PathBuf::from(&system_root).join("Temp");
    let prefetch = PathBuf::from(&system_root).join("Prefetch");

    let dirs = vec![
        (temp, "temp", "User temporary files"),
        (
            win_temp.to_string_lossy().to_string(),
            "temp",
            "Windows temporary files",
        ),
        (
            prefetch.to_string_lossy().to_string(),
            "prefetch",
            "Optional Prefetch space cleanup",
        ),
    ];

    scan_dirs(dirs, safe_paths)
}

/// Scan an explicit set of temp/prefetch directories.
///
/// Kept separate from environment discovery so characterization tests can
/// exercise real scanner output against the production authorization model.
pub(crate) fn scan_dirs(
    dirs: Vec<(String, &'static str, &'static str)>,
    safe_paths: &SafePaths,
) -> Result<Vec<CleanItem>> {
    let mut items = Vec::new();

    for (dir, category, description) in dirs {
        if dir.is_empty() || !std::path::Path::new(&dir).exists() {
            continue;
        }
        scan_directory(&dir, category, description, safe_paths, &mut items)?;
    }

    // Sort by size descending
    items.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    Ok(items)
}

fn scan_directory(
    dir: &str,
    category: &str,
    description: &str,
    safe_paths: &SafePaths,
    items: &mut Vec<CleanItem>,
) -> Result<()> {
    let dir_path = std::path::Path::new(dir);
    if !safe_paths.is_safe(dir_path) {
        return Ok(()); // Skip directories not in whitelist
    }

    for entry in WalkDir::new(dir)
        .max_depth(3)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            let path = entry.path().to_string_lossy().to_string();
            let meta = entry.metadata().ok();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let last_accessed = meta.and_then(|m| m.accessed().ok()).map(|t| {
                let st: std::time::SystemTime = t;
                let dt: chrono::DateTime<chrono::Utc> = st.into();
                dt.to_rfc3339()
            });

            // Derive sub_group from first subdirectory under the source root
            let sub_group = entry
                .path()
                .strip_prefix(dir_path)
                .ok()
                .and_then(|rel| {
                    rel.components()
                        .next()
                        .map(|c| c.as_os_str().to_string_lossy().to_string())
                })
                .unwrap_or_else(|| "Root".to_string());

            if size > 0 {
                items.push(CleanItem {
                    path,
                    size_bytes: size,
                    last_accessed,
                    category: category.to_string(),
                    description: description.to_string(),
                    group_key: description.to_string(),
                    sub_group,
                });
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::safe_paths::SafePaths;
    use std::process::Command;

    #[test]
    fn scan_directory_does_not_list_junction_children_but_metadata_follows() {
        let sandbox = tempfile::tempdir().unwrap();
        let scan_root = sandbox.path().join("temp-root");
        let outside = sandbox.path().join("outside");
        std::fs::create_dir_all(&scan_root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let secret = outside.join("secret.tmp");
        std::fs::write(&secret, b"canary").unwrap();
        let junction = scan_root.join("junction");
        let status = Command::new("cmd")
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

        let safe_paths =
            SafePaths::from_canonical_roots(vec![std::fs::canonicalize(&scan_root).unwrap()]);
        assert!(
            !safe_paths.is_safe(junction.join("secret.tmp")),
            "SafePaths must reject the canonical target outside the scan root"
        );

        let mut items = Vec::new();
        scan_directory(
            scan_root.to_str().unwrap(),
            "temp",
            "junction probe",
            &safe_paths,
            &mut items,
        )
        .unwrap();
        assert!(
            items
                .iter()
                .all(|item| !item.path.contains("secret.tmp")),
            "WalkDir (follow_links=false) must not list files reached only through a directory junction: {items:?}"
        );
        let followed = std::fs::metadata(junction.join("secret.tmp")).unwrap();
        assert_eq!(
            followed.len(),
            6,
            "execute_target metadata() follows the junction"
        );
    }
}
