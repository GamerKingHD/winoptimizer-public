//! Export analysis results to JSON for external use.
//!
//! Only JSON is exported. The former duplicate CSV exporter was unused and
//! removed rather than maintained: RFC 4180 quoting does not defend against
//! spreadsheet formula interpretation (`=`, `+`, `-`, `@`) in filesystem
//! controlled names, so a supported CSV surface would need its own
//! neutralize-on-export contract and tests.

use crate::analyzer::disk_usage::DiskNode;
use crate::analyzer::snapshot::DiskSnapshot;
use crate::core::error::{Result, WinOptimizerError};

/// Export disk usage tree to JSON.
pub fn disk_usage_to_json(node: &DiskNode) -> Result<String> {
    serde_json::to_string_pretty(node).map_err(WinOptimizerError::Serde)
}

/// Export the complete flat snapshot, including summary and diagnostics.
pub fn disk_snapshot_to_json(snapshot: &DiskSnapshot) -> Result<String> {
    serde_json::to_string_pretty(snapshot).map_err(WinOptimizerError::Serde)
}
