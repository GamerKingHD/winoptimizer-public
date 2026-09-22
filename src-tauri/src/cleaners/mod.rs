pub mod browser_cache;
pub mod system_logs;
pub mod temp_files;
pub mod windows_update;

use serde::{Deserialize, Serialize};

/// Result of scanning for cleanable items.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanItem {
    pub path: String,
    pub size_bytes: u64,
    pub last_accessed: Option<String>,
    pub category: String, // "temp", "prefetch", "browser_cache", "update_cache", "logs"
    pub description: String,
    pub group_key: String, // Top-level grouping (e.g. "Microsoft Edge", "%TEMP%")
    pub sub_group: String, // Sub-group within group (e.g. "Cache", "Cookies")
}

/// Result of a clean operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanResult {
    pub category: String,
    pub items_cleaned: usize,
    pub bytes_freed: u64,
    pub failures: Vec<String>,
    /// Paths that were successfully cleaned (for frontend to remove from UI).
    #[serde(default)]
    pub cleaned_paths: Vec<String>,
}
