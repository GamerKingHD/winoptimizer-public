//! System log cleaner - Windows event logs, memory dumps, old chkdsk files.

use crate::cleaners::CleanItem;
use crate::core::error::Result;
use std::path::PathBuf;
use walkdir::WalkDir;

/// Known log and dump directories.
fn log_dirs() -> Vec<(PathBuf, &'static str)> {
    let system_root = std::env::var("SYSTEMROOT").unwrap_or_else(|_| "C:\\Windows".into());
    let system = PathBuf::from(&system_root);

    vec![
        (system.join("Logs"), "Windows log files"),
        (system.join("LiveKernelReports"), "Live kernel error dumps"),
        (system.join("Minidump"), "Minidump files (crash dumps)"),
        (system.join("memory.dmp"), "System memory dump"),
    ]
}

/// Scan for cleanable log/dump files.
pub fn scan() -> Result<Vec<CleanItem>> {
    scan_dirs(log_dirs())
}

/// Scan an explicit set of log/dump locations.
///
/// Kept separate from environment discovery so characterization tests can
/// exercise real scanner output against the production authorization model.
pub(crate) fn scan_dirs(dirs: Vec<(PathBuf, &'static str)>) -> Result<Vec<CleanItem>> {
    let mut items = Vec::new();

    for (dir_or_file, description) in dirs {
        if !dir_or_file.exists() {
            continue;
        }

        // Derive a short group_key from the file/directory name
        let group_key = dir_or_file
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| description.to_string());

        if dir_or_file.is_file() {
            let meta = dir_or_file.metadata().ok();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            if size > 0 {
                items.push(CleanItem {
                    path: dir_or_file.to_string_lossy().to_string(),
                    size_bytes: size,
                    last_accessed: meta.and_then(|m| m.accessed().ok()).map(|t| {
                        let st: std::time::SystemTime = t;
                        let dt: chrono::DateTime<chrono::Utc> = st.into();
                        dt.to_rfc3339()
                    }),
                    category: "logs".to_string(),
                    description: description.to_string(),
                    group_key,
                    sub_group: String::new(),
                });
            }
        } else {
            for entry in WalkDir::new(&dir_or_file)
                .max_depth(2)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if entry.file_type().is_file() {
                    let meta = entry.metadata().ok();
                    let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                    if size > 0 {
                        let sub_group = entry
                            .path()
                            .strip_prefix(&dir_or_file)
                            .ok()
                            .and_then(|rel| {
                                rel.components()
                                    .next()
                                    .map(|c| c.as_os_str().to_string_lossy().to_string())
                            })
                            .unwrap_or_else(|| "Root".to_string());

                        items.push(CleanItem {
                            path: entry.path().to_string_lossy().to_string(),
                            size_bytes: size,
                            last_accessed: meta.and_then(|m| m.accessed().ok()).map(|t| {
                                let st: std::time::SystemTime = t;
                                let dt: chrono::DateTime<chrono::Utc> = st.into();
                                dt.to_rfc3339()
                            }),
                            category: "logs".to_string(),
                            description: description.to_string(),
                            group_key: group_key.clone(),
                            sub_group,
                        });
                    }
                }
            }
        }
    }

    items.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    Ok(items)
}
