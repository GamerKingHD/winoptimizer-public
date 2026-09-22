//! Disk usage analyzer - walks a directory tree and returns
//! a sorted list of directories by size. Used for treemap visualization.
//!
//! Uses jwalk (parallel directory iterator) for fast directory iteration.
//! Skips known unreadable system dirs.
//! Follows no symlinks/junctions to avoid loops and cross-volume duplication.
//!
//! ## Disk size semantics
//!
//! WinOptimizer reports three different size concepts.  They are deliberately
//! not interchangeable:
//!
//! * **Logical size** is the ordinary file length represented by the unnamed
//!   DATA stream.  The NTFS MFT engine still parses alternate data streams and
//!   retains their per-stream logical lengths in entry details; the Win32
//!   fallback can only report the ordinary file length.
//! * **Allocated size** is the physical cluster allocation reported by each
//!   non-resident DATA stream in the MFT engine.  Sparse holes do not count.
//!   Resident DATA is reported as zero here because its storage is inside the
//!   MFT record and is represented by `metadata_size_bytes`.  Compression and
//!   WOF-backed files use the allocation exposed by their NTFS run lists;
//!   WinOptimizer does not expand their logical contents.  The Win32 fallback
//!   reports the `FileIdBothDirectoryInfo` allocation size and is labelled
//!   "Allocated (estimated)" in the UI; it does not claim NTFS run-list parity.
//! * **Unique allocated size** counts each active NTFS file record's DATA
//!   allocation once.  Hard-link aliases still contribute to logical and
//!   allocated totals, but contribute zero to this unique total after the first
//!   alias.  Directories contribute hierarchy/count information, not file DATA
//!   bytes.  Reparse targets are not followed.
//!
//! In an MFT scan, `metadata_size_bytes` is the rounded allocation of `$MFT`.
//! Other NTFS metadata, inaccessible records, and records omitted by a partial
//! scan are not invented as file sizes.  Inaccessible/invalid records are
//! reflected in diagnostics and make the scan partial.  When available,
//! `unaccounted_size_bytes` is the saturated remainder of the point-in-time
//! volume allocation bitmap after unique DATA and `$MFT` allocation.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[cfg(not(windows))]
use std::sync::atomic::Ordering;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::Arc;

/// Directories always skipped (system-protected or always-denied).
const SKIP_DIRS: &[&str] = &[
    "System Volume Information",
    "$Recycle.Bin",
    "$WinREAgent",
    "$SysReset",
    "$GetCurrent",
    "Recovery",
    "Windows.old",
    "$Windows.~BT",
    "$Windows.~WS",
];

pub(crate) const NODE_FLAG_REPARSE: u32 = 1 << 0;
pub(crate) const NODE_FLAG_SPARSE: u32 = 1 << 1;
pub(crate) const NODE_FLAG_COMPRESSED: u32 = 1 << 2;

/// Describes the storage tier of the volume being scanned.
/// Used to select the optimal parallelism strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageTier {
    /// NVMe — very high IOPS, full parallelism.
    NVMe,
    /// SATA / USB SSD — moderate IOPS, bounded parallelism (4–8 threads).
    SSD,
    /// Spinning disk — sequential reads only, avoid seek thrashing (1 thread).
    HDD,
    /// Unknown or removable — use moderate defaults.
    Unknown,
}

impl StorageTier {
    /// Determine the storage tier from a mount point (e.g. "C:\").
    pub fn from_mount_point(mount: &str) -> Self {
        let drive = mount
            .chars()
            .next()
            .and_then(|c| {
                if c.is_ascii_alphabetic() {
                    Some(c.to_ascii_uppercase())
                } else {
                    None
                }
            })
            .unwrap_or('C');
        let vol = format!(r"\\.\{}:", drive);
        // Try to detect via the same IOCTL path used in system_info::hardware
        match detect_seek_penalty_and_bus(&vol) {
            Some((false, _)) => StorageTier::HDD,
            Some((true, info)) => {
                if info.contains("NVMe") || info.contains("NVM") {
                    StorageTier::NVMe
                } else {
                    StorageTier::SSD
                }
            }
            None => StorageTier::Unknown,
        }
    }

    /// Return the maximum number of concurrent directory scans to use.
    pub fn max_concurrency(self) -> usize {
        match self {
            StorageTier::NVMe => 0, // 0 = use rayon default pool (all cores)
            StorageTier::SSD => 6,
            StorageTier::HDD => 1,
            StorageTier::Unknown => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanStatus {
    Complete,
    Partial,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanEngine {
    NtfsMft,
    Win32Fallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiskScanSource {
    Mft,
    Win32,
    Snapshot,
    UsnDelta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiskCacheSource {
    Fresh,
    Snapshot,
    UsnDelta,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiskReconciliation {
    /// A point-in-time volume allocation bitmap was compared with the scan.
    VolumeBitmap,
    /// The engine did not obtain a trustworthy volume allocation comparison.
    Unavailable,
}

#[derive(Debug, Default)]
#[cfg(not(windows))]
struct ScanCounters {
    files_scanned: AtomicU64,
    directories_scanned: AtomicU64,
    skipped_entries: AtomicU64,
    inaccessible_entries: AtomicU64,
    reparse_points: AtomicU64,
    depth_limited_directories: AtomicU64,
}

/// Query seek penalty and bus-type info for a volume device path.
fn detect_seek_penalty_and_bus(vol_path: &str) -> Option<(bool, String)> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::*;
    use windows::Win32::Storage::FileSystem::*;
    use windows::Win32::System::IO::DeviceIoControl;

    const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D1400;
    const STORAGE_PROPERTY_ID_SEEK_PENALTY: i32 = 7;
    const STORAGE_PROPERTY_ID_ADAPTER: i32 = 1;
    const BUS_TYPE_SATA: i32 = 0x0B;
    const BUS_TYPE_NVME: i32 = 0x11;
    const BUS_TYPE_USB: i32 = 0x07;
    const BUS_TYPE_SAS: i32 = 0x0A;

    #[repr(C)]
    struct STORAGE_PROPERTY_QUERY {
        property_id: i32,
        query_type: i32,
        additional_params: [u8; 4],
    }

    #[repr(C)]
    struct STORAGE_DEVICE_SEEK_PENALTY_DESCRIPTOR {
        version: u32,
        size: u32,
        incurs_seek_penalty: u8,
    }

    #[repr(C)]
    struct STORAGE_ADAPTER_DESCRIPTOR {
        version: u32,
        size: u32,
        _max_transfer_length: u32,
        _physical_bytes: u32,
        _alignment: u32,
        _adapter_flags: u32,
        _adapter_uid: u32,
        bus_type: i32,
    }

    unsafe {
        let handle = match CreateFileW(
            &HSTRING::from(vol_path),
            FILE_GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        ) {
            Ok(h) => h,
            Err(_) => return None,
        };

        // Seek penalty
        let query = STORAGE_PROPERTY_QUERY {
            property_id: STORAGE_PROPERTY_ID_SEEK_PENALTY,
            query_type: 0,
            additional_params: [0u8; 4],
        };
        let mut seek_desc: STORAGE_DEVICE_SEEK_PENALTY_DESCRIPTOR = std::mem::zeroed();
        let mut bytes_returned: u32 = 0;
        let seek_ok = DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(&query as *const _ as *const std::ffi::c_void),
            std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            Some(&mut seek_desc as *mut _ as *mut std::ffi::c_void),
            std::mem::size_of::<STORAGE_DEVICE_SEEK_PENALTY_DESCRIPTOR>() as u32,
            Some(&mut bytes_returned),
            None,
        )
        .is_ok();
        let no_seek_penalty = seek_ok && seek_desc.incurs_seek_penalty == 0;

        // Bus type
        let query2 = STORAGE_PROPERTY_QUERY {
            property_id: STORAGE_PROPERTY_ID_ADAPTER,
            query_type: 0,
            additional_params: [0u8; 4],
        };
        let mut adapter_desc: STORAGE_ADAPTER_DESCRIPTOR = std::mem::zeroed();
        let mut bytes_returned2: u32 = 0;
        let bus_ok = DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(&query2 as *const _ as *const std::ffi::c_void),
            std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            Some(&mut adapter_desc as *mut _ as *mut std::ffi::c_void),
            std::mem::size_of::<STORAGE_ADAPTER_DESCRIPTOR>() as u32,
            Some(&mut bytes_returned2),
            None,
        )
        .is_ok();

        let _ = CloseHandle(handle);

        let bt = if bus_ok { adapter_desc.bus_type } else { 0 };
        let bus_label = match bt {
            BUS_TYPE_NVME => "NVMe".to_string(),
            BUS_TYPE_SATA => "SATA".to_string(),
            BUS_TYPE_SAS => "SAS".to_string(),
            BUS_TYPE_USB => "USB".to_string(),
            _ => format!("BusType_{}", bt),
        };

        Some((no_seek_penalty, bus_label))
    }
}

/// Internal tree representation. It is converted to the flat snapshot model
/// before a result crosses the Tauri boundary.
#[derive(Debug, Clone)]
pub(crate) struct TreeDiskNode {
    pub path: String,
    pub name: String,
    pub size_bytes: u64,
    pub allocated_size_bytes: u64,
    pub file_id: u64,
    pub file_count: u64,
    pub is_directory: bool,
    pub is_truncated: bool,
    pub flags: u32,
    pub detail: Option<TreeEntryDetails>,
    pub children: Vec<TreeDiskNode>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TreeEntryDetails {
    pub mft_record_number: Option<u64>,
    pub sequence: Option<u16>,
    pub file_reference: Option<u64>,
    pub parent_reference: Option<u64>,
    pub reparse_tag: Option<u32>,
    pub link_count: u32,
    pub streams: Vec<crate::analyzer::snapshot::DiskStreamDetails>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskNode {
    pub id: u64,
    pub parent_id: Option<u64>,
    pub name: String,
    pub is_directory: bool,
    pub has_children: bool,
    /// Ordinary file logical bytes below this node; named stream details remain
    /// available on the corresponding entries.
    pub logical_size_bytes: u64,
    /// Physical bytes below this node, counting hard-link aliases.
    pub allocated_size_bytes: u64,
    /// Physical DATA bytes below this node after hard-link de-duplication.
    pub unique_allocated_size_bytes: u64,
    pub file_count: u64,
    pub directory_count: u64,
    pub flags: u32,
    pub shared_alias: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiskSummary {
    /// Ordinary unnamed DATA-stream bytes for accessible records.
    pub logical_size_bytes: u64,
    /// Physical DATA allocation, including repeated hard-link aliases.
    pub allocated_size_bytes: u64,
    /// Physical DATA allocation counted once per active NTFS file record.
    pub unique_allocated_size_bytes: u64,
    /// Rounded `$MFT` allocation for an NTFS MFT scan; zero when unavailable.
    pub metadata_size_bytes: u64,
    /// Volume bitmap allocation minus unique DATA and metadata allocation.
    pub unaccounted_size_bytes: Option<u64>,
    pub file_count: u64,
    pub directory_count: u64,
    pub stream_count: u64,
    pub hard_link_count: u64,
}

/// Backend timing buckets exposed with a live disk scan response.
///
/// The default keeps older diagnostic payloads readable when a bucket is
/// absent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiskTimingDiagnostics {
    pub validation_ms: u64,
    pub scan_ms: u64,
    pub result_finalization_ms: u64,
    pub index_finalization_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskDiagnostics {
    pub source: DiskScanSource,
    pub cache_source: DiskCacheSource,
    pub reconciliation: DiskReconciliation,
    #[serde(default)]
    pub elapsed_ms: u64,
    pub partial_scan: bool,
    pub access_denied: bool,
    pub skipped_records: u64,
    pub reparse_points: u64,
    pub messages: Vec<String>,
}

impl Default for DiskDiagnostics {
    fn default() -> Self {
        Self {
            source: DiskScanSource::Win32,
            cache_source: DiskCacheSource::Unavailable,
            reconciliation: DiskReconciliation::Unavailable,
            elapsed_ms: 0,
            partial_scan: false,
            access_denied: false,
            skipped_records: 0,
            reparse_points: 0,
            messages: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskScanResult {
    pub snapshot_id: String,
    pub source: DiskScanSource,
    pub root: DiskNode,
    pub status: ScanStatus,
    pub engine: ScanEngine,
    pub summary: DiskSummary,
    pub diagnostics: DiskDiagnostics,
}

#[derive(Debug)]
pub struct ScanOutput {
    pub result: DiskScanResult,
    pub snapshot: crate::analyzer::snapshot::DiskSnapshot,
}

impl DiskScanResult {
    pub fn files_scanned(&self) -> u64 {
        self.summary.file_count
    }

    pub fn directories_scanned(&self) -> u64 {
        self.summary.directory_count
    }

    pub fn skipped_entries(&self) -> u64 {
        self.diagnostics.skipped_records
    }

    pub fn reparse_points(&self) -> u64 {
        self.diagnostics.reparse_points
    }
}

/// Convert an engine's temporary tree into the flat, paginated snapshot
/// representation.  The tree is dropped before the result is serialized.
pub(crate) fn output_from_tree(
    tree: TreeDiskNode,
    status: ScanStatus,
    engine: ScanEngine,
    source: DiskScanSource,
    mut diagnostics: DiskDiagnostics,
    mut metadata: crate::analyzer::snapshot::SnapshotMetadata,
) -> ScanOutput {
    let snapshot_id = uuid::Uuid::new_v4().to_string();
    let mut nodes = Vec::new();
    let mut details = Vec::new();
    let mut seen_file_ids = HashMap::<u64, u64>::new();
    let aggregate = append_flat_node(
        &tree,
        None,
        &tree.path,
        &snapshot_id,
        &mut nodes,
        &mut details,
        &mut seen_file_ids,
    );

    diagnostics.source = source;
    diagnostics.partial_scan |= status != ScanStatus::Complete;
    metadata.engine = engine;
    metadata.status = status;
    metadata.complete = status == ScanStatus::Complete;

    let summary = DiskSummary {
        logical_size_bytes: aggregate.logical_size_bytes,
        allocated_size_bytes: aggregate.allocated_size_bytes,
        unique_allocated_size_bytes: aggregate.unique_allocated_size_bytes,
        metadata_size_bytes: 0,
        unaccounted_size_bytes: None,
        file_count: aggregate.file_count,
        directory_count: aggregate.directory_count,
        stream_count: aggregate.stream_count,
        hard_link_count: aggregate.hard_link_count,
    };
    let root = nodes.first().cloned().unwrap_or(DiskNode {
        id: 0,
        parent_id: None,
        name: tree.name.clone(),
        is_directory: true,
        has_children: false,
        logical_size_bytes: 0,
        allocated_size_bytes: 0,
        unique_allocated_size_bytes: 0,
        file_count: 0,
        directory_count: 1,
        flags: 0,
        shared_alias: false,
    });
    let snapshot = crate::analyzer::snapshot::DiskSnapshot {
        snapshot_id: snapshot_id.clone(),
        metadata,
        root: root.clone(),
        nodes,
        details,
        summary: summary.clone(),
        diagnostics: diagnostics.clone(),
    };
    let result = DiskScanResult {
        snapshot_id,
        source,
        root,
        status,
        engine,
        summary,
        diagnostics,
    };
    ScanOutput { result, snapshot }
}

#[derive(Debug, Default)]
struct FlatAggregate {
    logical_size_bytes: u64,
    allocated_size_bytes: u64,
    unique_allocated_size_bytes: u64,
    file_count: u64,
    directory_count: u64,
    stream_count: u64,
    hard_link_count: u64,
}

fn append_flat_node(
    tree: &TreeDiskNode,
    parent_id: Option<u64>,
    parent_path: &str,
    snapshot_id: &str,
    nodes: &mut Vec<DiskNode>,
    details: &mut Vec<crate::analyzer::snapshot::DiskEntryDetails>,
    seen_file_ids: &mut HashMap<u64, u64>,
) -> FlatAggregate {
    let id = nodes.len() as u64;
    let path = if parent_id.is_none() {
        parent_path.to_string()
    } else {
        format!("{}\\{}", parent_path.trim_end_matches('\\'), tree.name)
    };
    let node_index = nodes.len();
    nodes.push(DiskNode {
        id,
        parent_id,
        name: tree.name.clone(),
        is_directory: tree.is_directory,
        has_children: !tree.children.is_empty() || tree.is_truncated,
        logical_size_bytes: 0,
        allocated_size_bytes: 0,
        unique_allocated_size_bytes: 0,
        file_count: 0,
        directory_count: 0,
        flags: tree.flags,
        shared_alias: false,
    });

    let mut aggregate = FlatAggregate::default();
    if tree.is_directory {
        aggregate.directory_count = 1;
        for child in &tree.children {
            let child_aggregate = append_flat_node(
                child,
                Some(id),
                &path,
                snapshot_id,
                nodes,
                details,
                seen_file_ids,
            );
            aggregate.logical_size_bytes = aggregate
                .logical_size_bytes
                .saturating_add(child_aggregate.logical_size_bytes);
            aggregate.allocated_size_bytes = aggregate
                .allocated_size_bytes
                .saturating_add(child_aggregate.allocated_size_bytes);
            aggregate.unique_allocated_size_bytes = aggregate
                .unique_allocated_size_bytes
                .saturating_add(child_aggregate.unique_allocated_size_bytes);
            aggregate.file_count = aggregate
                .file_count
                .saturating_add(child_aggregate.file_count);
            aggregate.directory_count = aggregate
                .directory_count
                .saturating_add(child_aggregate.directory_count);
            aggregate.stream_count = aggregate
                .stream_count
                .saturating_add(child_aggregate.stream_count);
            aggregate.hard_link_count = aggregate
                .hard_link_count
                .saturating_add(child_aggregate.hard_link_count);
        }
    } else {
        let allocated_size = tree.allocated_size_bytes;
        let is_primary = if tree.file_id != 0 {
            seen_file_ids.insert(tree.file_id, id).is_none()
        } else {
            true
        };
        let unique_allocated_size = if is_primary {
            allocated_size
        } else {
            nodes[node_index].shared_alias = true;
            0
        };
        aggregate.logical_size_bytes = tree.size_bytes;
        aggregate.allocated_size_bytes = allocated_size;
        aggregate.unique_allocated_size_bytes = unique_allocated_size;
        aggregate.file_count = 1;
        aggregate.stream_count = if is_primary {
            tree.detail
                .as_ref()
                .map(|detail| detail.streams.len() as u64)
                .unwrap_or(1)
        } else {
            0
        };
        aggregate.hard_link_count = if is_primary {
            tree.detail
                .as_ref()
                .map(|detail| u64::from(detail.link_count.saturating_sub(1)))
                .unwrap_or(0)
        } else {
            0
        };
    }

    if let Some(node) = nodes.get_mut(node_index) {
        node.logical_size_bytes = aggregate.logical_size_bytes;
        node.allocated_size_bytes = aggregate.allocated_size_bytes;
        node.unique_allocated_size_bytes = aggregate.unique_allocated_size_bytes;
        node.file_count = aggregate.file_count;
        node.directory_count = aggregate.directory_count;
    }

    let detail = tree
        .detail
        .as_ref()
        .map(|detail| crate::analyzer::snapshot::DiskEntryDetails {
            snapshot_id: snapshot_id.to_string(),
            node_id: id,
            path: path.clone(),
            logical_size_bytes: aggregate.logical_size_bytes,
            allocated_size_bytes: aggregate.allocated_size_bytes,
            unique_allocated_size_bytes: aggregate.unique_allocated_size_bytes,
            mft_record_number: detail.mft_record_number,
            sequence: detail.sequence,
            file_reference: detail.file_reference,
            parent_reference: detail.parent_reference,
            flags: tree.flags,
            reparse_tag: detail.reparse_tag,
            link_count: detail.link_count,
            streams: detail.streams.clone(),
        });
    if let Some(detail) = detail {
        details.push(detail);
    }

    aggregate
}

/// Scan a directory and return a tree of DiskNodes with sizes.
/// Uses jwalk for parallel directory iteration.
/// The `cancel_flag` allows the frontend to cancel long scans.
/// `progress` is incremented occasionally so callers can monitor activity.
///
/// The `tier` parameter controls parallelism — use
/// [`StorageTier::from_mount_point`] to auto-detect, or pass
/// [`StorageTier::NVMe`] / [`StorageTier::SSD`] / [`StorageTier::HDD`]
/// explicitly.
pub fn validate_directory<P: AsRef<Path>>(root: P) -> std::result::Result<PathBuf, String> {
    let root = root.as_ref();
    let metadata = std::fs::symlink_metadata(root)
        .map_err(|error| format!("Unable to inspect {}: {}", root.display(), error))?;
    if !metadata.is_dir() {
        return Err(format!(
            "The scan path is not a directory: {}",
            root.display()
        ));
    }
    if is_reparse_point(root, &metadata.file_type()) {
        return Err(format!(
            "The scan path is a reparse point: {}",
            root.display()
        ));
    }
    Ok(root.to_path_buf())
}

pub fn scan<P: AsRef<Path>>(
    root: P,
    max_depth: u32,
    cancel_flag: Arc<AtomicBool>,
    progress: Arc<AtomicU64>,
    tier: StorageTier,
) -> std::result::Result<ScanOutput, String> {
    scan_with_cache(root, max_depth, cancel_flag, progress, tier, true)
}

/// Run a volume scan without consulting the persisted snapshot or USN delta.
/// This is reserved for measurements that must time a fresh MFT read.
pub fn scan_fresh<P: AsRef<Path>>(
    root: P,
    max_depth: u32,
    cancel_flag: Arc<AtomicBool>,
    progress: Arc<AtomicU64>,
    tier: StorageTier,
) -> std::result::Result<ScanOutput, String> {
    scan_with_cache(root, max_depth, cancel_flag, progress, tier, false)
}

fn scan_with_cache<P: AsRef<Path>>(
    root: P,
    max_depth: u32,
    cancel_flag: Arc<AtomicBool>,
    progress: Arc<AtomicU64>,
    tier: StorageTier,
    allow_cache: bool,
) -> std::result::Result<ScanOutput, String> {
    let root = validate_directory(root)?;
    if max_depth == 0 {
        return Err("Scan depth must be at least 1".into());
    }

    #[cfg(windows)]
    let _ = tier;

    #[cfg(windows)]
    let mut ntfs_fallback_error = None;

    #[cfg(windows)]
    if let Some(drive) = volume_root_drive(&root) {
        match super::ntfs_mft::scan_volume(drive, max_depth, &cancel_flag, &progress, allow_cache) {
            Ok(output) => return Ok(output),
            Err(error) => {
                log::debug!(
                    "NTFS MFT scan unavailable for {}:\\; using Win32 fallback: {}",
                    drive,
                    error
                );
                ntfs_fallback_error = Some(error);
            }
        }
    }

    #[cfg(windows)]
    {
        let (root_node, status, mut diagnostics) =
            super::directory_scan::scan(&root, max_depth, &cancel_flag, &progress);
        if let Some(error) = ntfs_fallback_error {
            diagnostics.access_denied |= is_access_denied_error(&error);
            diagnostics.messages.insert(
                0,
                format!(
                    "Direct NTFS scan unavailable; using Win32 fallback: {}",
                    error
                ),
            );
        }
        let metadata = crate::analyzer::snapshot::SnapshotMetadata {
            root: root.to_string_lossy().to_string(),
            engine: ScanEngine::Win32Fallback,
            identity: crate::analyzer::snapshot::SnapshotIdentity::default(),
            status,
            complete: status == ScanStatus::Complete,
        };
        Ok(output_from_tree(
            root_node,
            status,
            ScanEngine::Win32Fallback,
            DiskScanSource::Win32,
            diagnostics,
            metadata,
        ))
    }

    #[cfg(not(windows))]
    {
        let root_path = root.to_string_lossy().to_string();
        let counters = Arc::new(ScanCounters::default());
        let root_node = walk_tree(root, 1, max_depth, &cancel_flag, &progress, &counters, tier);
        let status = if cancel_flag.load(Ordering::Relaxed) {
            ScanStatus::Cancelled
        } else if counters.skipped_entries.load(Ordering::Relaxed) > 0
            || counters.inaccessible_entries.load(Ordering::Relaxed) > 0
            || counters.reparse_points.load(Ordering::Relaxed) > 0
            || counters.depth_limited_directories.load(Ordering::Relaxed) > 0
        {
            ScanStatus::Partial
        } else {
            ScanStatus::Complete
        };

        let skipped_records = counters
            .skipped_entries
            .load(Ordering::Relaxed)
            .saturating_add(counters.depth_limited_directories.load(Ordering::Relaxed));
        let inaccessible_entries = counters.inaccessible_entries.load(Ordering::Relaxed);
        let mut diagnostics = DiskDiagnostics {
            source: DiskScanSource::Win32,
            cache_source: DiskCacheSource::Fresh,
            reconciliation: DiskReconciliation::Unavailable,
            elapsed_ms: 0,
            partial_scan: status != ScanStatus::Complete,
            access_denied: inaccessible_entries > 0,
            skipped_records,
            reparse_points: counters.reparse_points.load(Ordering::Relaxed),
            messages: Vec::new(),
        };
        if inaccessible_entries > 0 {
            diagnostics.messages.push(format!(
                "{} directory entries could not be read",
                inaccessible_entries
            ));
        }
        let metadata = crate::analyzer::snapshot::SnapshotMetadata {
            root: root_path,
            engine: ScanEngine::Win32Fallback,
            identity: crate::analyzer::snapshot::SnapshotIdentity::default(),
            status,
            complete: status == ScanStatus::Complete,
        };
        Ok(output_from_tree(
            root_node,
            status,
            ScanEngine::Win32Fallback,
            DiskScanSource::Win32,
            diagnostics,
            metadata,
        ))
    }
}

#[cfg(windows)]
fn volume_root_drive(path: &Path) -> Option<char> {
    let text = path.to_string_lossy();
    let bytes = text.as_bytes();
    (bytes.len() == 3
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
        && bytes[0].is_ascii_alphabetic())
    .then(|| bytes[0] as char)
}

#[cfg(windows)]
fn is_access_denied_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("access") || error.contains("denied") || error.contains("privilege")
}

#[cfg(not(windows))]
fn walk_tree(
    dir: PathBuf,
    current_depth: u32,
    max_depth: u32,
    cancel_flag: &Arc<AtomicBool>,
    progress: &Arc<AtomicU64>,
    counters: &Arc<ScanCounters>,
    tier: StorageTier,
) -> TreeDiskNode {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| dir.to_string_lossy().to_string());

    let mut node = TreeDiskNode {
        path: dir.to_string_lossy().to_string(),
        name,
        size_bytes: 0,
        allocated_size_bytes: 0,
        file_id: 0,
        file_count: 0,
        is_directory: true,
        is_truncated: false,
        flags: 0,
        detail: None,
        children: Vec::new(),
    };

    counters.directories_scanned.fetch_add(1, Ordering::Relaxed);

    if cancel_flag.load(Ordering::Relaxed) {
        return node;
    }

    // Use jwalk to list immediate children
    let (total_size, total_allocated, total_files, child_dirs, file_nodes): (
        u64,
        u64,
        u64,
        Vec<PathBuf>,
        Vec<TreeDiskNode>,
    ) = {
        let mut size = 0u64;
        let mut allocated = 0u64;
        let mut files = 0u64;
        let mut dirs: Vec<PathBuf> = Vec::new();
        let mut file_nodes = Vec::new();

        let walk_builder = jwalk::WalkDir::new(&dir)
            .min_depth(1)
            .max_depth(1)
            .skip_hidden(false);

        // For HDD, force serial directory reads to avoid seek thrashing
        if tier == StorageTier::HDD {
            // jwalk doesn't support directly setting parallelism on builder,
            // but we use a small worker pool below via the rayon limit.
        }

        for result in walk_builder {
            if cancel_flag.load(Ordering::Relaxed) {
                break;
            }

            let entry = match result {
                Ok(e) => e,
                Err(_) => {
                    counters
                        .inaccessible_entries
                        .fetch_add(1, Ordering::Relaxed);
                    continue;
                }
            };

            let entry_path = entry.path();
            if is_reparse_point(&entry_path, &entry.file_type) {
                counters.reparse_points.fetch_add(1, Ordering::Relaxed);
                continue;
            }

            let name_os = &entry.file_name;
            let name_str = name_os.to_string_lossy();
            if entry.file_type.is_dir() && should_skip_directory(&name_str) {
                counters.skipped_entries.fetch_add(1, Ordering::Relaxed);
                continue;
            }

            if entry.file_type.is_dir() {
                dirs.push(entry_path);
            } else if entry.file_type.is_file() {
                match std::fs::symlink_metadata(&entry_path) {
                    Ok(meta) if meta.is_file() => {
                        size = size.saturating_add(meta.len());
                        allocated = allocated.saturating_add(meta.len());
                        files += 1;
                        progress.fetch_add(1, Ordering::Relaxed);
                        counters.files_scanned.fetch_add(1, Ordering::Relaxed);
                        file_nodes.push(TreeDiskNode {
                            path: entry_path.to_string_lossy().to_string(),
                            name: name_str.to_string(),
                            size_bytes: meta.len(),
                            allocated_size_bytes: meta.len(),
                            file_id: 0,
                            file_count: 1,
                            is_directory: false,
                            is_truncated: false,
                            flags: 0,
                            detail: None,
                            children: Vec::new(),
                        });
                    }
                    Ok(_) => {
                        counters.skipped_entries.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(_) => {
                        counters
                            .inaccessible_entries
                            .fetch_add(1, Ordering::Relaxed);
                    }
                }
            } else {
                counters.skipped_entries.fetch_add(1, Ordering::Relaxed);
            }
        }

        (size, allocated, files, dirs, file_nodes)
    };

    node.size_bytes += total_size;
    node.allocated_size_bytes += total_allocated;
    node.file_count += total_files;
    node.children = file_nodes;

    // Process child directories with concurrency appropriate for the storage tier
    if !child_dirs.is_empty() && current_depth < max_depth && !cancel_flag.load(Ordering::Relaxed) {
        let max_conc = tier.max_concurrency();

        let child_nodes = if max_conc == 0 {
            // Unlimited concurrency (NVMe) — use rayon default pool
            use rayon::prelude::*;
            child_dirs
                .into_par_iter()
                .map(|subdir| {
                    walk_tree(
                        subdir,
                        current_depth + 1,
                        max_depth,
                        cancel_flag,
                        progress,
                        counters,
                        tier,
                    )
                })
                .collect()
        } else if max_conc == 1 {
            // Sequential (HDD) — avoid any parallelism to prevent seek thrashing
            child_dirs
                .into_iter()
                .map(|subdir| {
                    walk_tree(
                        subdir,
                        current_depth + 1,
                        max_depth,
                        cancel_flag,
                        progress,
                        counters,
                        tier,
                    )
                })
                .collect()
        } else {
            // Bounded parallelism (SSD) — use a custom rayon pool with limited threads
            // We use a simple iter-based approach with a bounded parallel executor
            bounded_parallel(child_dirs, max_conc, |subdir| {
                walk_tree(
                    subdir,
                    current_depth + 1,
                    max_depth,
                    cancel_flag,
                    progress,
                    counters,
                    tier,
                )
            })
        };

        let mut child_nodes: Vec<TreeDiskNode> = child_nodes;

        for child in &child_nodes {
            node.size_bytes += child.size_bytes;
            node.allocated_size_bytes += child.allocated_size_bytes;
            node.file_count += child.file_count;
        }

        child_nodes.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
        node.children.extend(child_nodes);
    } else if !child_dirs.is_empty() && current_depth >= max_depth {
        counters
            .depth_limited_directories
            .fetch_add(child_dirs.len() as u64, Ordering::Relaxed);
        node.is_truncated = true;
        node.children.extend(
            child_dirs
                .into_iter()
                .map(|path| TreeDiskNode {
                    name: path
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .unwrap_or_else(|| path.to_string_lossy().to_string()),
                    path: path.to_string_lossy().to_string(),
                    size_bytes: 0,
                    allocated_size_bytes: 0,
                    file_id: 0,
                    file_count: 0,
                    is_directory: true,
                    is_truncated: true,
                    flags: 0,
                    detail: None,
                    children: Vec::new(),
                })
                .collect::<Vec<_>>(),
        );
    }

    node
}

pub(crate) fn should_skip_directory(name: &str) -> bool {
    SKIP_DIRS
        .iter()
        .any(|skip_name| name.eq_ignore_ascii_case(skip_name))
}

pub(crate) fn is_reparse_point(path: &Path, file_type: &std::fs::FileType) -> bool {
    if file_type.is_symlink() {
        return true;
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{
            GetFileAttributesW, FILE_ATTRIBUTE_REPARSE_POINT, INVALID_FILE_ATTRIBUTES,
        };

        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        wide.push(0);
        let attributes = unsafe { GetFileAttributesW(PCWSTR(wide.as_ptr())) };
        attributes != INVALID_FILE_ATTRIBUTES && attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
    }

    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

/// Process items in parallel with a bounded number of threads.
/// Uses a custom rayon thread pool with the given limit.
#[cfg(not(windows))]
fn bounded_parallel<T, U, F>(items: Vec<T>, max_threads: usize, f: F) -> Vec<U>
where
    T: Send,
    U: Send,
    F: Fn(T) -> U + Send + Sync,
{
    use rayon::prelude::*;
    // Create a thread pool with limited threads
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(max_threads)
        .build()
        .expect("Failed to build bounded rayon thread pool");

    pool.install(|| items.into_par_iter().map(f).collect())
}

/// Get a flat list of the largest directories (for engine-side analysis).
#[cfg(not(windows))]
pub(crate) fn largest_directories(node: &TreeDiskNode, n: usize) -> Vec<TreeDiskNode> {
    let mut flat: Vec<TreeDiskNode> = Vec::new();
    flatten_dirs(node, &mut flat);
    flat.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    flat.truncate(n);
    flat
}

#[cfg(not(windows))]
fn flatten_dirs(node: &TreeDiskNode, acc: &mut Vec<TreeDiskNode>) {
    if node.is_directory && !node.children.is_empty() {
        acc.push(node.clone());
        for child in &node.children {
            flatten_dirs(child, acc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_protected_directory_names_case_insensitively() {
        assert!(should_skip_directory("system volume information"));
        assert!(should_skip_directory("$Recycle.Bin"));
        assert!(!should_skip_directory("Documents"));
    }

    #[test]
    fn storage_tier_concurrency_is_bounded() {
        assert_eq!(StorageTier::HDD.max_concurrency(), 1);
        assert_eq!(StorageTier::SSD.max_concurrency(), 6);
    }

    #[test]
    fn disk_timing_diagnostics_defaults_for_legacy_or_partial_payloads() {
        let legacy: DiskTimingDiagnostics = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy, DiskTimingDiagnostics::default());

        let partial: DiskTimingDiagnostics = serde_json::from_str(r#"{"scan_ms":7}"#).unwrap();
        assert_eq!(partial.scan_ms, 7);
        assert_eq!(partial.validation_ms, 0);
        assert_eq!(partial.result_finalization_ms, 0);
        assert_eq!(partial.index_finalization_ms, 0);
    }

    #[test]
    fn scan_propagates_requested_max_depth() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first");
        let second = first.join("second");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        std::fs::write(second.join("fixture.bin"), b"fixture").unwrap();

        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicU64::new(0));
        let output = scan(directory.path(), 2, cancel, progress, StorageTier::HDD).unwrap();

        let first_node = output
            .snapshot
            .nodes
            .iter()
            .find(|node| node.name == "first")
            .unwrap();
        assert!(output.snapshot.root.has_children);
        assert!(first_node.has_children);
        assert!(output
            .snapshot
            .nodes
            .iter()
            .any(|node| node.name == "second"));
        assert!(!output
            .snapshot
            .nodes
            .iter()
            .any(|node| node.name == "fixture.bin"));
    }
}
