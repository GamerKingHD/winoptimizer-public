use crate::*;
use std::collections::HashMap;
use std::time::Instant;
// ============================================================
// Tauri Commands - Disk Analyzer
// ============================================================

#[derive(Debug, serde::Serialize)]
struct DiskDiagnosticsResponse {
    #[serde(flatten)]
    diagnostics: analyzer::disk_usage::DiskDiagnostics,
    #[serde(default)]
    timings: analyzer::disk_usage::DiskTimingDiagnostics,
}

fn phase_event(operation: &str, phase: &str, message: impl Into<String>) -> ProgressEvent {
    ProgressEvent {
        operation: operation.into(),
        phase: phase.into(),
        percent: None,
        message: Some(message.into()),
        bytes_processed: None,
        bytes_total: None,
    }
}

fn emit_phase(
    app_handle: &tauri::AppHandle,
    operation: &str,
    phase: &str,
    message: impl Into<String>,
) {
    app_handle
        .emit("progress-event", phase_event(operation, phase, message))
        .ok();
}
#[derive(Debug, serde::Serialize)]
pub(crate) struct DiskScanResponse {
    snapshot_id: String,
    source: analyzer::disk_usage::DiskScanSource,
    root: analyzer::disk_usage::DiskNode,
    status: analyzer::disk_usage::ScanStatus,
    engine: analyzer::disk_usage::ScanEngine,
    summary: analyzer::disk_usage::DiskSummary,
    diagnostics: DiskDiagnosticsResponse,
}

impl DiskScanResponse {
    fn from_result(
        result: analyzer::disk_usage::DiskScanResult,
        timings: analyzer::disk_usage::DiskTimingDiagnostics,
    ) -> Self {
        Self {
            snapshot_id: result.snapshot_id,
            source: result.source,
            root: result.root,
            status: result.status,
            engine: result.engine,
            summary: result.summary,
            diagnostics: DiskDiagnosticsResponse {
                diagnostics: result.diagnostics,
                timings,
            },
        }
    }
}

#[tauri::command]
pub(crate) async fn scan_disk_usage(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    path: String,
    max_depth: u32,
) -> Result<DiskScanResponse> {
    let command_started = Instant::now();
    let validation_started = Instant::now();
    let path = path.trim().to_string();
    if path.is_empty() {
        return Err(core::error::WinOptimizerError::Other(
            "A directory path is required".into(),
        ));
    }
    if !(1..=64).contains(&max_depth) {
        return Err(core::error::WinOptimizerError::Other(
            "Scan depth must be between 1 and 64".into(),
        ));
    }
    analyzer::disk_usage::validate_directory(&path)
        .map_err(core::error::WinOptimizerError::Other)?;
    let validation_ms = validation_started.elapsed().as_millis() as u64;
    let scan_started = Instant::now();
    let path_clone = path.clone();
    app_handle
        .emit(
            "progress-event",
            ProgressEvent {
                operation: "disk_usage_scan".into(),
                phase: "started".into(),
                percent: None,
                message: Some(format!("Preparing disk scan for {}...", path_clone)),
                bytes_processed: None,
                bytes_total: None,
            },
        )
        .ok();
    emit_phase(
        &app_handle,
        "disk_usage_scan",
        "validation",
        format!("Validated {}; starting read-only scan...", path),
    );

    let cancel = {
        let mut flags = state.cancel_flags.lock().unwrap_or_else(|e| e.into_inner());
        let flag = Arc::new(AtomicBool::new(false));
        flags.insert("disk_usage".to_string(), flag.clone());
        flag
    };

    // Auto-detect storage tier from the mount point path
    let tier = analyzer::disk_usage::StorageTier::from_mount_point(&path);
    emit_phase(
        &app_handle,
        "disk_usage_scan",
        "scanning",
        format!("Scanning {}...", path),
    );

    // Shared progress counter — walk_tree increments it as files are processed
    let progress = Arc::new(AtomicU64::new(0));
    // Spawn a progress-poller that emits events every ~500ms while scan runs
    let poll_handle = {
        let ah = app_handle.clone();
        let pc = progress.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                let count = pc.load(Ordering::Relaxed);
                if count > 0 {
                    let _ = ah.emit(
                        "progress-event",
                        ProgressEvent {
                            operation: "disk_usage_scan".into(),
                            phase: "progress".into(),
                            percent: None,
                            message: Some(format!("Processed {} files...", count)),
                            bytes_processed: Some(count),
                            bytes_total: None,
                        },
                    );
                }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                // This runs until the scan command returns; we abandon it after.
            }
        })
    };

    let output = match tauri::async_runtime::spawn_blocking(move || {
        analyzer::disk_usage::scan(&path, max_depth, cancel, progress, tier)
    })
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            if let Ok(mut flags) = state.cancel_flags.lock() {
                flags.remove("disk_usage");
            }
            poll_handle.abort();
            return Err(core::error::WinOptimizerError::Other(error));
        }
        Err(error) => {
            if let Ok(mut flags) = state.cancel_flags.lock() {
                flags.remove("disk_usage");
            }
            poll_handle.abort();
            return Err(core::error::WinOptimizerError::Other(error.to_string()));
        }
    };
    let scan_ms = scan_started.elapsed().as_millis() as u64;

    // Clean up the cancel flag and stop poller
    if let Ok(mut flags) = state.cancel_flags.lock() {
        flags.remove("disk_usage");
    }
    poll_handle.abort();

    emit_phase(
        &app_handle,
        "disk_usage_scan",
        "finalization",
        "Finalizing scan results...",
    );
    let result_finalization_started = Instant::now();
    let mut result = output.result;
    let snapshot_id = result.snapshot_id.clone();
    let elapsed_ms = command_started.elapsed().as_millis() as u64;
    result.diagnostics.elapsed_ms = elapsed_ms;
    let mut snapshot = output.snapshot;
    snapshot.diagnostics.elapsed_ms = elapsed_ms;
    let persist_snapshot = result.status == analyzer::disk_usage::ScanStatus::Complete
        && result.diagnostics.cache_source != analyzer::disk_usage::DiskCacheSource::Snapshot;
    let snapshot = Arc::new(snapshot);
    if persist_snapshot {
        let store = analyzer::snapshot::SnapshotStore::default_location()
            .map_err(core::error::WinOptimizerError::Other)?;
        let snapshot_for_persistence = snapshot.clone();
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(error) = store.write_complete(snapshot_for_persistence.as_ref()) {
                log::warn!("Completed disk scan was not cached: {}", error);
            }
        });
    }
    let result_finalization_ms = result_finalization_started.elapsed().as_millis() as u64;

    emit_phase(
        &app_handle,
        "disk_usage_scan",
        "indexing",
        "Updating the in-memory navigation index...",
    );
    let index_finalization_started = Instant::now();
    if let Ok(mut snapshots) = state.disk_snapshots.lock() {
        snapshots.insert(
            snapshot_id.clone(),
            Arc::new(analyzer::snapshot::DiskSnapshotIndex::from_arc(snapshot)),
        );
        while snapshots.len() > 2 {
            let old_id = snapshots
                .keys()
                .find(|candidate| candidate.as_str() != snapshot_id)
                .cloned();
            if let Some(old_id) = old_id {
                snapshots.remove(&old_id);
            } else {
                break;
            }
        }
    }
    let index_finalization_ms = index_finalization_started.elapsed().as_millis() as u64;
    let timings = analyzer::disk_usage::DiskTimingDiagnostics {
        validation_ms,
        scan_ms,
        result_finalization_ms,
        index_finalization_ms,
    };

    let size = result.summary.allocated_size_bytes;
    let (phase, percent, status_message) = match result.status {
        analyzer::disk_usage::ScanStatus::Complete => (
            "completed",
            Some(100.0),
            format!("Scan complete — {:.2} GB", size as f64 / 1_073_741_824.0),
        ),
        analyzer::disk_usage::ScanStatus::Partial => (
            "partial",
            Some(100.0),
            format!(
                "Scan finished; access denied: {}, {} reparse points",
                result.diagnostics.access_denied, result.diagnostics.reparse_points
            ),
        ),
        analyzer::disk_usage::ScanStatus::Cancelled => (
            "cancelled",
            None,
            format!("Scan cancelled after {} files", result.files_scanned()),
        ),
        analyzer::disk_usage::ScanStatus::Failed => ("error", None, "Scan failed".into()),
    };
    app_handle
        .emit(
            "progress-event",
            ProgressEvent {
                operation: "disk_usage_scan".into(),
                phase: phase.into(),
                percent,
                message: Some(status_message),
                bytes_processed: None,
                bytes_total: None,
            },
        )
        .ok();

    Ok(DiskScanResponse::from_result(result, timings))
}
/// Cancel a long-running scan operation (disk usage or duplicate finder).
#[tauri::command]
pub(crate) fn cancel_scan(state: tauri::State<'_, Arc<AppState>>, operation: String) -> Result<()> {
    let flags = state.cancel_flags.lock().map_err(|_| {
        core::error::WinOptimizerError::Other("scan cancellation state is unavailable".into())
    })?;
    let flag = flags.get(&operation).ok_or_else(|| {
        core::error::WinOptimizerError::Other(format!(
            "scan operation '{operation}' is not running"
        ))
    })?;
    flag.store(true, Ordering::SeqCst);
    log::info!("Cancel requested for operation: {}", operation);
    Ok(())
}
/// Raw user-controlled export of the caller's own disk-usage tree.
///
/// Intentionally unsanitized: this returns exactly the paths the user scanned
/// so they can archive their own data. It is not a "share support data"
/// artifact and must not be described as redacted. Anything shareable with
/// third parties goes through `core::support_bundle` instead.
#[tauri::command]
pub(crate) async fn export_disk_usage_json(node: analyzer::disk_usage::DiskNode) -> Result<String> {
    tauri::async_runtime::spawn_blocking(move || analyzer::export::disk_usage_to_json(&node))
        .await
        .map_err(|e| core::error::WinOptimizerError::Other(e.to_string()))?
}

#[tauri::command]
pub(crate) async fn list_disk_children(
    state: tauri::State<'_, Arc<AppState>>,
    snapshot_id: String,
    parent_id: Option<u64>,
    sort: String,
    offset: u64,
    limit: u64,
) -> Result<Vec<analyzer::disk_usage::DiskNode>> {
    let snapshots = state.disk_snapshots.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let index = load_snapshot(&snapshots, &snapshot_id)?;
        let child_indices = index.children.get(&parent_id).cloned().unwrap_or_default();
        let child_indices = sorted_child_indices(&index.snapshot.nodes, child_indices, &sort);
        let offset = usize::try_from(offset).map_err(|_| {
            core::error::WinOptimizerError::Other("child offset is too large".into())
        })?;
        let limit = usize::try_from(limit.clamp(1, 1000)).map_err(|_| {
            core::error::WinOptimizerError::Other("child limit is too large".into())
        })?;
        Ok(child_indices
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|node_index| index.snapshot.nodes[node_index].clone())
            .collect())
    })
    .await
    .map_err(|error| core::error::WinOptimizerError::Other(error.to_string()))?
}

fn sorted_child_indices(
    nodes: &[analyzer::disk_usage::DiskNode],
    mut indices: Vec<usize>,
    sort: &str,
) -> Vec<usize> {
    indices.sort_unstable_by(|left, right| {
        let left_node = &nodes[*left];
        let right_node = &nodes[*right];
        if sort == "name" {
            left_node
                .name
                .to_lowercase()
                .cmp(&right_node.name.to_lowercase())
        } else {
            let normalized_sort = match sort {
                "logical_size" => "logical",
                "unique_allocated" => "unique",
                other => other,
            };
            compare_file_nodes(left_node, right_node, normalized_sort)
        }
    });
    indices
}

#[tauri::command]
pub(crate) async fn get_disk_entry_details(
    state: tauri::State<'_, Arc<AppState>>,
    snapshot_id: String,
    node_id: u64,
) -> Result<analyzer::snapshot::DiskEntryDetails> {
    let snapshots = state.disk_snapshots.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let index = load_snapshot(&snapshots, &snapshot_id)?;
        let snapshot = index.snapshot.as_ref();
        if let Some(details) = snapshot
            .details
            .iter()
            .find(|details| details.node_id == node_id)
        {
            let mut details = details.clone();
            if let Some(node) = snapshot.nodes.iter().find(|node| node.id == node_id) {
                details.logical_size_bytes = node.logical_size_bytes;
                details.allocated_size_bytes = node.allocated_size_bytes;
                details.unique_allocated_size_bytes = node.unique_allocated_size_bytes;
            }
            return Ok(details);
        }
        let node = snapshot
            .nodes
            .iter()
            .find(|node| node.id == node_id)
            .ok_or_else(|| {
                core::error::WinOptimizerError::Other("disk entry was not found".into())
            })?;
        Ok(analyzer::snapshot::DiskEntryDetails {
            snapshot_id,
            node_id,
            path: snapshot_node_path(snapshot, node_id),
            logical_size_bytes: node.logical_size_bytes,
            allocated_size_bytes: node.allocated_size_bytes,
            unique_allocated_size_bytes: node.unique_allocated_size_bytes,
            mft_record_number: None,
            sequence: None,
            file_reference: None,
            parent_reference: None,
            flags: node.flags,
            reparse_tag: None,
            link_count: if node.shared_alias { 2 } else { 1 },
            streams: Vec::new(),
        })
    })
    .await
    .map_err(|error| core::error::WinOptimizerError::Other(error.to_string()))?
}

/// Raw user-controlled export of an entire disk snapshot.
///
/// Intentionally unsanitized like [`export_disk_usage_json`]; it is not a
/// shareable support artifact and must not be marketed as redacted.
#[tauri::command]
pub(crate) async fn export_disk_snapshot_json(
    state: tauri::State<'_, Arc<AppState>>,
    snapshot_id: String,
) -> Result<String> {
    let snapshots = state.disk_snapshots.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let index = load_snapshot(&snapshots, &snapshot_id)?;
        analyzer::export::disk_snapshot_to_json(index.snapshot.as_ref())
    })
    .await
    .map_err(|error| core::error::WinOptimizerError::Other(error.to_string()))?
}

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct LargestDiskFile {
    pub node_id: u64,
    pub name: String,
    pub path: String,
    pub logical_size_bytes: u64,
    pub allocated_size_bytes: u64,
    pub unique_allocated_size_bytes: u64,
    pub shared_alias: bool,
}

#[tauri::command]
pub(crate) async fn list_largest_disk_files(
    state: tauri::State<'_, Arc<AppState>>,
    snapshot_id: String,
    sort: String,
    limit: u64,
) -> Result<Vec<LargestDiskFile>> {
    let snapshots = state.disk_snapshots.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let index = load_snapshot(&snapshots, &snapshot_id)?;
        let snapshot = index.snapshot.as_ref();
        let limit = usize::try_from(limit.clamp(1, 200))
            .map_err(|_| core::error::WinOptimizerError::Other("file limit is too large".into()))?;
        let indices = largest_file_indices(&snapshot.nodes, &sort, limit);
        Ok(indices
            .into_iter()
            .map(|index| {
                let node = &snapshot.nodes[index];
                LargestDiskFile {
                    node_id: node.id,
                    name: node.name.clone(),
                    path: snapshot_node_path(snapshot, node.id),
                    logical_size_bytes: node.logical_size_bytes,
                    allocated_size_bytes: node.allocated_size_bytes,
                    unique_allocated_size_bytes: node.unique_allocated_size_bytes,
                    shared_alias: node.shared_alias,
                }
            })
            .collect())
    })
    .await
    .map_err(|error| core::error::WinOptimizerError::Other(error.to_string()))?
}

fn compare_file_nodes(
    left: &analyzer::disk_usage::DiskNode,
    right: &analyzer::disk_usage::DiskNode,
    sort: &str,
) -> std::cmp::Ordering {
    let left_size = match sort {
        "logical" => left.logical_size_bytes,
        "unique" => left.unique_allocated_size_bytes,
        _ => left.allocated_size_bytes,
    };
    let right_size = match sort {
        "logical" => right.logical_size_bytes,
        "unique" => right.unique_allocated_size_bytes,
        _ => right.allocated_size_bytes,
    };
    right_size
        .cmp(&left_size)
        .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
}

fn largest_file_indices(
    nodes: &[analyzer::disk_usage::DiskNode],
    sort: &str,
    limit: usize,
) -> Vec<usize> {
    let mut indices: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| (!node.is_directory).then_some(index))
        .collect();
    let compare =
        |left: &usize, right: &usize| compare_file_nodes(&nodes[*left], &nodes[*right], sort);
    if indices.len() > limit {
        indices.select_nth_unstable_by(limit, compare);
        indices.truncate(limit);
    }
    indices.sort_unstable_by(compare);
    indices
}

fn load_snapshot(
    snapshots: &Arc<Mutex<HashMap<String, Arc<analyzer::snapshot::DiskSnapshotIndex>>>>,
    snapshot_id: &str,
) -> Result<Arc<analyzer::snapshot::DiskSnapshotIndex>> {
    if let Ok(cache) = snapshots.lock() {
        if let Some(snapshot) = cache.get(snapshot_id) {
            return Ok(snapshot.clone());
        }
    }
    let snapshot = Arc::new(analyzer::snapshot::DiskSnapshotIndex::new(
        analyzer::snapshot::SnapshotStore::default_location()
            .map_err(core::error::WinOptimizerError::Other)?
            .read(snapshot_id)
            .map_err(core::error::WinOptimizerError::Other)?,
    ));
    if let Ok(mut cache) = snapshots.lock() {
        cache.insert(snapshot_id.to_string(), snapshot.clone());
        while cache.len() > 2 {
            let old_id = cache
                .keys()
                .find(|candidate| candidate.as_str() != snapshot_id)
                .cloned();
            if let Some(old_id) = old_id {
                cache.remove(&old_id);
            } else {
                break;
            }
        }
    }
    Ok(snapshot)
}

fn snapshot_node_path(snapshot: &analyzer::snapshot::DiskSnapshot, node_id: u64) -> String {
    let mut names = Vec::new();
    let mut current = snapshot_node(snapshot, node_id);
    while let Some(node) = current {
        names.push(node.name.clone());
        current = node
            .parent_id
            .and_then(|parent_id| snapshot_node(snapshot, parent_id));
    }
    names.reverse();
    if names.len() <= 1 {
        snapshot.metadata.root.clone()
    } else {
        format!(
            "{}\\{}",
            snapshot.metadata.root.trim_end_matches('\\'),
            names[1..].join("\\")
        )
    }
}

fn snapshot_node(
    snapshot: &analyzer::snapshot::DiskSnapshot,
    node_id: u64,
) -> Option<&analyzer::disk_usage::DiskNode> {
    let indexed = usize::try_from(node_id)
        .ok()
        .and_then(|index| snapshot.nodes.get(index))
        .filter(|node| node.id == node_id);
    indexed.or_else(|| snapshot.nodes.iter().find(|node| node.id == node_id))
}

#[tauri::command]
pub(crate) async fn usn_scan_volume(
    app_handle: tauri::AppHandle,
    drive_letter: String,
) -> Result<(
    Vec<analyzer::usn_journal::UsnFileEntry>,
    analyzer::usn_journal::UsnScanStats,
)> {
    let dl = drive_letter.clone();
    app_handle
        .emit(
            "progress-event",
            ProgressEvent {
                operation: "usn_scan".into(),
                phase: "started".into(),
                percent: None,
                message: Some(format!("Enumerating NTFS journal on {}:...", dl)),
                bytes_processed: None,
                bytes_total: None,
            },
        )
        .ok();

    let result = tauri::async_runtime::spawn_blocking(move || {
        let drive = drive_letter
            .chars()
            .next()
            .ok_or_else(|| core::error::WinOptimizerError::Other("Empty drive letter".into()))?;
        let (entries, stats) = analyzer::usn_journal::enumerate_volume(drive)
            .map_err(core::error::WinOptimizerError::Other)?;
        Ok::<_, core::error::WinOptimizerError>((entries, stats))
    })
    .await
    .map_err(|e| core::error::WinOptimizerError::Other(e.to_string()))?;

    if let Ok((_, ref stats)) = &result {
        app_handle
            .emit(
                "progress-event",
                ProgressEvent {
                    operation: "usn_scan".into(),
                    phase: "completed".into(),
                    percent: Some(100.0),
                    message: Some(format!(
                        "{} records in {}ms",
                        stats.total_records, stats.elapsed_ms
                    )),
                    bytes_processed: Some(stats.total_records),
                    bytes_total: Some(stats.total_records),
                },
            )
            .ok();
    }

    result
}

#[tauri::command]
pub(crate) async fn usn_find_files(
    app_handle: tauri::AppHandle,
    drive_letter: String,
    patterns: Vec<String>,
    max_results: usize,
) -> Result<Vec<analyzer::usn_journal::UsnFileEntry>> {
    app_handle
        .emit(
            "progress-event",
            ProgressEvent {
                operation: "usn_find".into(),
                phase: "started".into(),
                percent: None,
                message: Some(format!("Searching USN journal on {}:...", drive_letter)),
                bytes_processed: None,
                bytes_total: None,
            },
        )
        .ok();

    let result = tauri::async_runtime::spawn_blocking(move || {
        let drive = drive_letter
            .chars()
            .next()
            .ok_or_else(|| core::error::WinOptimizerError::Other("Empty drive letter".into()))?;
        let pattern_refs: Vec<&str> = patterns.iter().map(|s| s.as_str()).collect();
        let entries = analyzer::usn_journal::find_files_fast(drive, &pattern_refs, max_results)
            .map_err(core::error::WinOptimizerError::Other)?;
        Ok::<_, core::error::WinOptimizerError>(entries)
    })
    .await
    .map_err(|e| core::error::WinOptimizerError::Other(e.to_string()))?;

    if let Ok(ref entries) = &result {
        app_handle
            .emit(
                "progress-event",
                ProgressEvent {
                    operation: "usn_find".into(),
                    phase: "completed".into(),
                    percent: Some(100.0),
                    message: Some(format!("Found {} files", entries.len())),
                    bytes_processed: Some(entries.len() as u64),
                    bytes_total: Some(entries.len() as u64),
                },
            )
            .ok();
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_events_keep_developer_visible_stage_names() {
        let event = phase_event(
            "disk_usage_scan",
            "finalization",
            "Finalizing scan results...",
        );
        assert_eq!(event.operation, "disk_usage_scan");
        assert_eq!(event.phase, "finalization");
        assert_eq!(event.message.as_deref(), Some("Finalizing scan results..."));
    }

    #[test]
    fn diagnostics_response_flattens_timing_buckets() {
        let payload = serde_json::to_value(DiskDiagnosticsResponse {
            diagnostics: analyzer::disk_usage::DiskDiagnostics::default(),
            timings: analyzer::disk_usage::DiskTimingDiagnostics {
                validation_ms: 1,
                scan_ms: 2,
                result_finalization_ms: 3,
                index_finalization_ms: 4,
            },
        })
        .unwrap();

        assert_eq!(payload["cache_source"], "unavailable");
        assert_eq!(payload["timings"]["validation_ms"], 1);
        assert_eq!(payload["timings"]["scan_ms"], 2);
        assert_eq!(payload["timings"]["result_finalization_ms"], 3);
        assert_eq!(payload["timings"]["index_finalization_ms"], 4);
    }

    fn file(name: &str, allocated_size_bytes: u64) -> analyzer::disk_usage::DiskNode {
        analyzer::disk_usage::DiskNode {
            id: 0,
            parent_id: Some(0),
            name: name.into(),
            is_directory: false,
            has_children: false,
            logical_size_bytes: allocated_size_bytes,
            allocated_size_bytes,
            unique_allocated_size_bytes: allocated_size_bytes,
            file_count: 1,
            directory_count: 0,
            flags: 0,
            shared_alias: false,
        }
    }

    #[test]
    fn largest_file_indices_are_descending() {
        let nodes = vec![file("small.bin", 1), file("large.bin", 8)];
        assert_eq!(largest_file_indices(&nodes, "allocated", 2), vec![1, 0]);
        assert_eq!(largest_file_indices(&nodes, "logical", 1), vec![1]);
    }

    #[test]
    fn child_indices_are_sorted_before_page_slicing() {
        let nodes = vec![
            file("small.bin", 1),
            file("large.bin", 8),
            file("middle.bin", 4),
        ];
        let sorted = sorted_child_indices(&nodes, vec![0, 1, 2], "allocated");
        assert_eq!(sorted, vec![1, 2, 0]);
        assert_eq!(
            sorted.into_iter().skip(1).take(1).collect::<Vec<_>>(),
            vec![2]
        );

        let mut logical_first = file("logical-first.bin", 1);
        logical_first.logical_size_bytes = 10;
        let mut logical_second = file("logical-second.bin", 8);
        logical_second.logical_size_bytes = 2;
        assert_eq!(
            sorted_child_indices(&[logical_first, logical_second], vec![0, 1], "logical_size"),
            vec![0, 1]
        );
    }
}
