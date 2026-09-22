//! Persistent, versioned storage for completed disk-analysis snapshots.
//!
//! The on-volume scanner remains read-only.  This module only writes the
//! analyzer's own compact index under the user's local application data
//! directory, and only after a scan has completed.

use super::disk_usage::{
    DiskCacheSource, DiskDiagnostics, DiskNode, DiskScanResult, DiskScanSource, DiskSummary,
    ScanEngine, ScanStatus,
};
use memmap2::MmapOptions;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use uuid::Uuid;

const SNAPSHOT_MAGIC: &[u8; 8] = b"WODSNP01";
pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;
pub const PARSER_SCHEMA_VERSION: u32 = 1;
const HEADER_SIZE: usize = SNAPSHOT_MAGIC.len() + 4 + 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotCompressionAlgorithm {
    Xpress4k,
    Xpress8k,
    Xpress16k,
    Lzx,
}

impl SnapshotCompressionAlgorithm {
    fn for_logical_processors(logical_processors: usize) -> Self {
        match logical_processors {
            16.. => Self::Lzx,
            12..=15 => Self::Xpress16k,
            6..=11 => Self::Xpress8k,
            _ => Self::Xpress4k,
        }
    }

    #[cfg(windows)]
    fn provider_algorithm(self) -> u32 {
        use windows::Win32::Storage::FileSystem::{
            FILE_PROVIDER_COMPRESSION_LZX, FILE_PROVIDER_COMPRESSION_XPRESS16K,
            FILE_PROVIDER_COMPRESSION_XPRESS4K, FILE_PROVIDER_COMPRESSION_XPRESS8K,
        };

        match self {
            Self::Xpress4k => FILE_PROVIDER_COMPRESSION_XPRESS4K,
            Self::Xpress8k => FILE_PROVIDER_COMPRESSION_XPRESS8K,
            Self::Xpress16k => FILE_PROVIDER_COMPRESSION_XPRESS16K,
            Self::Lzx => FILE_PROVIDER_COMPRESSION_LZX,
        }
    }
}

fn selected_snapshot_compression_algorithm() -> SnapshotCompressionAlgorithm {
    static SELECTED: OnceLock<SnapshotCompressionAlgorithm> = OnceLock::new();
    *SELECTED.get_or_init(|| {
        let logical_processors = std::thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(1);
        SnapshotCompressionAlgorithm::for_logical_processors(logical_processors)
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SnapshotIdentity {
    /// Stable identity of the NTFS volume, when the volume API supplied it.
    pub volume_serial: Option<u64>,
    /// NTFS geometry used to interpret the MFT records.
    pub bytes_per_sector: Option<u32>,
    pub bytes_per_cluster: Option<u32>,
    pub mft_record_size: Option<u32>,
    pub mft_length: Option<u64>,
    /// USN journal identity. A delta is invalid if this changes.
    pub journal_id: Option<u64>,
    /// The exclusive USN boundary observed for this snapshot. It is the
    /// `NextUsn` value, not the last record number returned by the journal.
    pub last_usn: Option<i64>,
    /// Parser contract used to decode the snapshot's MFT records.
    pub parser_version: u32,
}

impl Default for SnapshotIdentity {
    fn default() -> Self {
        Self {
            volume_serial: None,
            bytes_per_sector: None,
            bytes_per_cluster: None,
            mft_record_size: None,
            mft_length: None,
            journal_id: None,
            last_usn: None,
            parser_version: PARSER_SCHEMA_VERSION,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMetadata {
    pub root: String,
    pub engine: ScanEngine,
    pub identity: SnapshotIdentity,
    pub status: ScanStatus,
    pub complete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskEntryDetails {
    pub snapshot_id: String,
    pub node_id: u64,
    pub path: String,
    #[serde(default)]
    pub logical_size_bytes: u64,
    #[serde(default)]
    pub allocated_size_bytes: u64,
    #[serde(default)]
    pub unique_allocated_size_bytes: u64,
    pub mft_record_number: Option<u64>,
    pub sequence: Option<u16>,
    pub file_reference: Option<u64>,
    pub parent_reference: Option<u64>,
    pub flags: u32,
    pub reparse_tag: Option<u32>,
    pub link_count: u32,
    /// The unnamed DATA stream and any NTFS alternate data streams.
    pub streams: Vec<DiskStreamDetails>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskStreamDetails {
    /// Empty for the unnamed/default DATA stream; otherwise the ADS name.
    pub name: String,
    /// Logical bytes in this DATA stream.
    pub logical_size_bytes: u64,
    /// Mapped physical cluster bytes. Resident and sparse holes are zero.
    pub allocated_size_bytes: u64,
    /// NTFS valid-data length for a non-resident stream.
    pub valid_data_length_bytes: u64,
    /// Resident DATA is stored inside the MFT record, not a separate cluster.
    pub resident: bool,
    /// The stream contains sparse holes that are excluded from allocation.
    pub sparse: bool,
    /// NTFS compression is represented by the run-list allocation as exposed.
    pub compressed: bool,
    /// Number of data runs represented by the parsed stream.
    pub run_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskSnapshot {
    pub snapshot_id: String,
    pub metadata: SnapshotMetadata,
    pub root: DiskNode,
    pub nodes: Vec<DiskNode>,
    pub details: Vec<DiskEntryDetails>,
    pub summary: DiskSummary,
    pub diagnostics: DiskDiagnostics,
}

/// In-memory navigation index for the current UI session. The serialized
/// snapshot remains the durable fallback, while directory navigation uses
/// child indexes instead of scanning every node on every click.
#[derive(Debug)]
pub struct DiskSnapshotIndex {
    pub snapshot: Arc<DiskSnapshot>,
    pub children: HashMap<Option<u64>, Vec<usize>>,
}

impl DiskSnapshotIndex {
    pub fn new(snapshot: DiskSnapshot) -> Self {
        Self::from_arc(Arc::new(snapshot))
    }

    pub fn from_arc(snapshot: Arc<DiskSnapshot>) -> Self {
        let mut children = HashMap::new();
        for (index, node) in snapshot.nodes.iter().enumerate() {
            children
                .entry(node.parent_id)
                .or_insert_with(Vec::new)
                .push(index);
        }
        Self { snapshot, children }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotChange {
    pub node_id: Option<u64>,
    pub parent_id: Option<u64>,
    pub name: String,
    pub is_directory: bool,
    pub logical_size_bytes: u64,
    pub allocated_size_bytes: u64,
    pub flags: u32,
    #[serde(default)]
    pub shared_alias: bool,
    #[serde(default)]
    pub detail: Option<DiskEntryDetails>,
    pub deleted: bool,
}

/// Apply a validated set of USN-derived node changes and recompute affected
/// ancestors.  The Windows adapter supplies the current sizes by re-reading
/// the corresponding MFT records; USN records themselves do not contain them.
pub fn apply_changes(
    snapshot: &mut DiskSnapshot,
    changes: &[SnapshotChange],
    last_usn: i64,
) -> Result<(), String> {
    if last_usn < 0 {
        return Err("USN snapshot boundary cannot be negative".into());
    }
    if snapshot
        .metadata
        .identity
        .last_usn
        .is_some_and(|previous| last_usn < previous)
    {
        return Err("USN snapshot boundary moved backwards".into());
    }
    let mut next_id = snapshot
        .nodes
        .iter()
        .map(|node| node.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    for change in changes {
        if change.deleted {
            if let Some(node_id) = change.node_id {
                remove_node_and_descendants(&mut snapshot.nodes, node_id);
                snapshot.details.retain(|detail| detail.node_id != node_id);
            }
            continue;
        }
        let node_id = change.node_id.unwrap_or_else(|| {
            let id = next_id;
            next_id = next_id.saturating_add(1);
            id
        });
        if node_id == 0 && !change.is_directory {
            return Err("a delta cannot replace the snapshot root with a file".into());
        }
        if let Some(node) = snapshot.nodes.iter_mut().find(|node| node.id == node_id) {
            node.parent_id = change.parent_id;
            node.name = change.name.clone();
            node.is_directory = change.is_directory;
            node.logical_size_bytes = change.logical_size_bytes;
            node.allocated_size_bytes = change.allocated_size_bytes;
            node.unique_allocated_size_bytes = if change.shared_alias {
                0
            } else {
                change.allocated_size_bytes
            };
            node.flags = change.flags;
            node.shared_alias = change.shared_alias;
        } else {
            snapshot.nodes.push(DiskNode {
                id: node_id,
                parent_id: change.parent_id,
                name: change.name.clone(),
                is_directory: change.is_directory,
                has_children: false,
                logical_size_bytes: change.logical_size_bytes,
                allocated_size_bytes: change.allocated_size_bytes,
                unique_allocated_size_bytes: if change.shared_alias {
                    0
                } else {
                    change.allocated_size_bytes
                },
                file_count: if change.is_directory { 0 } else { 1 },
                directory_count: if change.is_directory { 1 } else { 0 },
                flags: change.flags,
                shared_alias: change.shared_alias,
            });
        }
        if let Some(mut detail) = change.detail.clone() {
            detail.node_id = node_id;
            snapshot.details.retain(|detail| detail.node_id != node_id);
            snapshot.details.push(detail);
        }
    }
    recompute_aggregates(snapshot)?;
    snapshot.metadata.identity.last_usn = Some(last_usn);
    snapshot.diagnostics.source = DiskScanSource::UsnDelta;
    snapshot.diagnostics.cache_source = DiskCacheSource::UsnDelta;
    snapshot.root = snapshot
        .nodes
        .iter()
        .find(|node| node.id == 0)
        .cloned()
        .ok_or_else(|| "delta removed the snapshot root".to_string())?;
    for detail in &mut snapshot.details {
        if let Some(node) = snapshot.nodes.iter().find(|node| node.id == detail.node_id) {
            detail.logical_size_bytes = node.logical_size_bytes;
            detail.allocated_size_bytes = node.allocated_size_bytes;
            detail.unique_allocated_size_bytes = node.unique_allocated_size_bytes;
        }
    }
    refresh_detail_paths(snapshot);
    snapshot.diagnostics.reconciliation = super::disk_usage::DiskReconciliation::Unavailable;
    snapshot
        .diagnostics
        .messages
        .push("Volume bitmap reconciliation was not refreshed for this USN delta".into());
    // A delta without a fresh allocation reconciliation is useful for an
    // in-memory view, but it is not a complete allocation result and must not
    // replace the last complete snapshot.
    snapshot.diagnostics.partial_scan = true;
    snapshot.metadata.status = ScanStatus::Partial;
    snapshot.metadata.complete = false;
    snapshot.summary = summary_from_root(snapshot);
    Ok(())
}

fn remove_node_and_descendants(nodes: &mut Vec<DiskNode>, node_id: u64) {
    let mut removed = HashSet::from([node_id]);
    let mut changed = true;
    while changed {
        changed = false;
        for node in nodes.iter() {
            if node
                .parent_id
                .is_some_and(|parent| removed.contains(&parent))
                && removed.insert(node.id)
            {
                changed = true;
            }
        }
    }
    nodes.retain(|node| !removed.contains(&node.id));
}

fn recompute_aggregates(snapshot: &mut DiskSnapshot) -> Result<(), String> {
    let ids: HashSet<u64> = snapshot.nodes.iter().map(|node| node.id).collect();
    if !ids.contains(&0) {
        return Err("snapshot has no root node".into());
    }
    if snapshot
        .nodes
        .iter()
        .any(|node| node.parent_id.is_some_and(|parent| !ids.contains(&parent)))
    {
        return Err("snapshot delta references a missing parent".into());
    }
    let mut children = HashMap::<u64, Vec<u64>>::new();
    for node in &snapshot.nodes {
        if let Some(parent) = node.parent_id {
            children.entry(parent).or_default().push(node.id);
        }
    }
    let mut visiting = HashSet::new();
    let mut metrics = HashMap::new();
    aggregate_node(0, &children, snapshot, &mut visiting, &mut metrics)?;
    if metrics.len() != snapshot.nodes.len() {
        return Err("snapshot delta introduced an unreachable node or cycle".into());
    }
    for node in &mut snapshot.nodes {
        if let Some((logical, allocated, unique, files, directories)) = metrics.get(&node.id) {
            node.logical_size_bytes = *logical;
            node.allocated_size_bytes = *allocated;
            node.unique_allocated_size_bytes = *unique;
            node.file_count = *files;
            node.directory_count = *directories;
            node.has_children = children.contains_key(&node.id);
        }
    }
    Ok(())
}

fn aggregate_node(
    node_id: u64,
    children: &HashMap<u64, Vec<u64>>,
    snapshot: &DiskSnapshot,
    visiting: &mut HashSet<u64>,
    metrics: &mut HashMap<u64, (u64, u64, u64, u64, u64)>,
) -> Result<(u64, u64, u64, u64, u64), String> {
    if !visiting.insert(node_id) {
        return Err("snapshot delta introduced a cycle".into());
    }
    let node = snapshot
        .nodes
        .iter()
        .find(|node| node.id == node_id)
        .ok_or_else(|| "snapshot delta references a missing node".to_string())?;
    let result = if let Some(child_ids) = children.get(&node_id) {
        let mut total: (u64, u64, u64, u64, u64) = (0, 0, 0, 0, 1);
        for child_id in child_ids {
            let child = aggregate_node(*child_id, children, snapshot, visiting, metrics)?;
            total.0 = total.0.saturating_add(child.0);
            total.1 = total.1.saturating_add(child.1);
            total.2 = total.2.saturating_add(child.2);
            total.3 = total.3.saturating_add(child.3);
            total.4 = total.4.saturating_add(child.4);
        }
        total
    } else {
        if node.is_directory {
            (0, 0, 0, 0, 1)
        } else {
            (
                node.logical_size_bytes,
                node.allocated_size_bytes,
                node.unique_allocated_size_bytes,
                1,
                0,
            )
        }
    };
    visiting.remove(&node_id);
    metrics.insert(node_id, result);
    Ok(result)
}

fn refresh_detail_paths(snapshot: &mut DiskSnapshot) {
    let paths: HashMap<u64, String> = snapshot
        .nodes
        .iter()
        .map(|node| {
            let mut names = vec![node.name.clone()];
            let mut current = node.parent_id;
            while let Some(parent_id) = current {
                if let Some(parent) = snapshot.nodes.iter().find(|parent| parent.id == parent_id) {
                    names.push(parent.name.clone());
                    current = parent.parent_id;
                } else {
                    break;
                }
            }
            names.reverse();
            let path = if names.len() <= 1 {
                snapshot.metadata.root.clone()
            } else {
                format!(
                    "{}\\{}",
                    snapshot.metadata.root.trim_end_matches('\\'),
                    names[1..].join("\\")
                )
            };
            (node.id, path)
        })
        .collect();
    for detail in &mut snapshot.details {
        if let Some(path) = paths.get(&detail.node_id) {
            detail.path = path.clone();
        }
    }
}

fn summary_from_root(snapshot: &DiskSnapshot) -> DiskSummary {
    let root = &snapshot.root;
    let mut seen_records = HashSet::new();
    let (stream_count, hard_link_count) =
        snapshot
            .details
            .iter()
            .fold((0u64, 0u64), |(streams, hard_links), detail| {
                let record_key = detail.mft_record_number.unwrap_or(detail.node_id);
                if seen_records.insert(record_key) {
                    (
                        streams.saturating_add(detail.streams.len() as u64),
                        hard_links.saturating_add(u64::from(detail.link_count.saturating_sub(1))),
                    )
                } else {
                    (streams, hard_links)
                }
            });
    DiskSummary {
        logical_size_bytes: root.logical_size_bytes,
        allocated_size_bytes: root.allocated_size_bytes,
        unique_allocated_size_bytes: root.unique_allocated_size_bytes,
        metadata_size_bytes: snapshot.summary.metadata_size_bytes,
        unaccounted_size_bytes: None,
        file_count: root.file_count,
        directory_count: root.directory_count,
        stream_count,
        hard_link_count,
    }
}

impl DiskSnapshot {
    pub fn result(&self) -> DiskScanResult {
        DiskScanResult {
            snapshot_id: self.snapshot_id.clone(),
            status: self.metadata.status,
            engine: self.metadata.engine,
            source: self.diagnostics.source,
            root: self.root.clone(),
            summary: self.summary.clone(),
            diagnostics: self.diagnostics.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotMatch {
    Exact,
    NeedsDelta,
}

/// Classify a cached snapshot only when its USN continuity can be proven.
/// Missing or reversed boundaries are deliberately not treated as an exact
/// match: the caller must rebuild a fresh MFT snapshot in that case.
fn classify_snapshot_match(
    saved: &SnapshotIdentity,
    current: &SnapshotIdentity,
    current_usn: Option<i64>,
) -> Option<SnapshotMatch> {
    if saved.journal_id != current.journal_id {
        return None;
    }

    match (saved.last_usn, current_usn) {
        (Some(saved_usn), Some(current_usn)) if saved_usn >= 0 && current_usn >= 0 => {
            if saved_usn == current_usn {
                Some(SnapshotMatch::Exact)
            } else if saved_usn < current_usn {
                Some(SnapshotMatch::NeedsDelta)
            } else {
                None
            }
        }
        (None, None) if saved.journal_id.is_none() => Some(SnapshotMatch::Exact),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct SnapshotCandidate {
    pub snapshot: DiskSnapshot,
    pub match_kind: SnapshotMatch,
}

#[derive(Debug, Clone)]
pub struct SnapshotStore {
    directory: PathBuf,
}

impl SnapshotStore {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    pub fn default_location() -> Result<Self, String> {
        crate::core::paths::AppPaths::installed()
            .map(|paths| Self::new(paths.snapshots))
            .map_err(|error| error.to_string())
    }

    pub fn write_complete(&self, snapshot: &DiskSnapshot) -> Result<(), String> {
        if !snapshot.metadata.complete {
            return Err("partial disk snapshots cannot replace a valid cache".into());
        }
        fs::create_dir_all(&self.directory)
            .map_err(|error| format!("unable to create disk snapshot directory: {}", error))?;

        let payload = serde_json::to_vec(snapshot)
            .map_err(|error| format!("unable to encode disk snapshot: {}", error))?;
        let payload_len = u64::try_from(payload.len())
            .map_err(|_| "disk snapshot is too large to persist".to_string())?;
        let mut bytes = Vec::with_capacity(HEADER_SIZE + payload.len());
        bytes.extend_from_slice(SNAPSHOT_MAGIC);
        bytes.extend_from_slice(&SNAPSHOT_SCHEMA_VERSION.to_le_bytes());
        bytes.extend_from_slice(&payload_len.to_le_bytes());
        bytes.extend_from_slice(&payload);

        let target = self.path_for_id(&snapshot.snapshot_id)?;
        let temp = self
            .directory
            .join(format!(".{}.{}.tmp", snapshot.snapshot_id, Uuid::new_v4()));
        let write_result = (|| -> Result<(), String> {
            let mut file = File::create(&temp).map_err(|error| {
                format!("unable to create disk snapshot temporary file: {}", error)
            })?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|error| format!("unable to write disk snapshot: {}", error))?;
            atomic_replace(&temp, &target)
        })();

        if write_result.is_err() {
            let _ = fs::remove_file(&temp);
        } else {
            #[cfg(windows)]
            if crate::core::paths::AppPaths::installed()
                .map(|paths| paths.snapshots)
                .is_ok_and(|path| self.directory == path)
            {
                Self::schedule_snapshot_compression(target.clone());
            }
            #[cfg(not(windows))]
            let _ = compress_snapshot_file(&target);
            self.prune_obsolete_snapshots(snapshot, &target);
        }
        write_result
    }

    pub fn read(&self, snapshot_id: &str) -> Result<DiskSnapshot, String> {
        let path = self.path_for_id(snapshot_id)?;
        let file = File::open(&path)
            .map_err(|error| format!("unable to open disk snapshot {}: {}", snapshot_id, error))?;
        let map = unsafe {
            MmapOptions::new().map(&file).map_err(|error| {
                format!("unable to map disk snapshot {}: {}", snapshot_id, error)
            })?
        };
        decode_snapshot(&map)
    }

    pub fn find_candidate(
        &self,
        identity: &SnapshotIdentity,
        current_usn: Option<i64>,
    ) -> Result<Option<SnapshotCandidate>, String> {
        let entries = match fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("unable to list disk snapshots: {}", error)),
        };

        let mut best: Option<SnapshotCandidate> = None;
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            if entry.path().extension().and_then(|value| value.to_str()) != Some("wasnap") {
                continue;
            }
            let entry_path = entry.path();
            let snapshot_id = match entry_path.file_stem().and_then(|value| value.to_str()) {
                Some(value) => value,
                None => continue,
            };
            let snapshot = match self.read(snapshot_id) {
                Ok(snapshot) => snapshot,
                Err(_) => continue,
            };
            if !snapshot.metadata.complete
                || snapshot.metadata.identity.volume_serial != identity.volume_serial
            {
                continue;
            }
            if !same_geometry(&snapshot.metadata.identity, identity) {
                continue;
            }
            let Some(match_kind) =
                classify_snapshot_match(&snapshot.metadata.identity, identity, current_usn)
            else {
                continue;
            };
            let candidate = SnapshotCandidate {
                snapshot,
                match_kind,
            };
            let should_replace = best.as_ref().is_none_or(|current| {
                candidate
                    .snapshot
                    .metadata
                    .identity
                    .last_usn
                    .unwrap_or(i64::MIN)
                    > current
                        .snapshot
                        .metadata
                        .identity
                        .last_usn
                        .unwrap_or(i64::MIN)
            });
            if should_replace {
                best = Some(candidate);
            }
            if match_kind == SnapshotMatch::Exact {
                break;
            }
        }
        #[cfg(windows)]
        if crate::core::paths::AppPaths::installed()
            .map(|paths| paths.snapshots)
            .is_ok_and(|path| self.directory == path)
        {
            self.schedule_existing_snapshot_compression();
        }
        Ok(best)
    }

    fn prune_obsolete_snapshots(&self, snapshot: &DiskSnapshot, target: &Path) {
        if snapshot.metadata.identity.volume_serial.is_none() {
            return;
        }

        let entries = match fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path == target || path.extension().and_then(|value| value.to_str()) != Some("wasnap")
            {
                continue;
            }
            let Some(snapshot_id) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            let Ok(candidate_path) = self.path_for_id(snapshot_id) else {
                continue;
            };
            let Ok(candidate) = self.read(snapshot_id) else {
                continue;
            };
            if same_snapshot_volume(snapshot, &candidate) && is_older_snapshot(snapshot, &candidate)
            {
                if let Err(error) = fs::remove_file(&candidate_path) {
                    log::debug!(
                        "Unable to prune old disk snapshot {}: {}",
                        candidate_path.display(),
                        error
                    );
                }
            }
        }
    }

    #[cfg(windows)]
    fn configure_background_thread_priority() {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
        };

        let _ = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) };
    }

    #[cfg(windows)]
    fn schedule_snapshot_compression(path: PathBuf) {
        let display_path = path.display().to_string();
        if let Err(error) = std::thread::Builder::new()
            .name("disk-cache-compression".into())
            .spawn(move || {
                Self::configure_background_thread_priority();
                if let Err(error) = compress_snapshot_file(&path) {
                    log::debug!(
                        "Unable to apply transparent compression to disk snapshot {}: {}",
                        path.display(),
                        error
                    );
                }
            })
        {
            log::debug!(
                "Unable to schedule disk snapshot compression for {}: {}",
                display_path,
                error
            );
        }
    }

    #[cfg(windows)]
    fn schedule_existing_snapshot_compression(&self) {
        use std::sync::{Mutex, OnceLock};

        static SCHEDULED: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
        let scheduled = SCHEDULED.get_or_init(|| Mutex::new(HashSet::new()));
        let should_schedule = scheduled
            .lock()
            .map(|mut directories| directories.insert(self.directory.clone()))
            .unwrap_or(false);
        if !should_schedule {
            return;
        }

        let directory = self.directory.clone();
        if std::thread::Builder::new()
            .name("disk-cache-compression".into())
            .spawn(move || {
                Self::configure_background_thread_priority();
                if let Err(error) = compress_existing_snapshots(&directory) {
                    log::debug!(
                        "Unable to migrate existing disk snapshot compression: {}",
                        error
                    );
                }
            })
            .is_err()
        {
            if let Ok(mut directories) = scheduled.lock() {
                directories.remove(&self.directory);
            }
        }
    }

    fn path_for_id(&self, snapshot_id: &str) -> Result<PathBuf, String> {
        if snapshot_id.is_empty()
            || !snapshot_id
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '-')
        {
            return Err("invalid disk snapshot identifier".into());
        }
        Ok(self.directory.join(format!("{}.wasnap", snapshot_id)))
    }
}

fn same_geometry(left: &SnapshotIdentity, right: &SnapshotIdentity) -> bool {
    left.bytes_per_sector == right.bytes_per_sector
        && left.bytes_per_cluster == right.bytes_per_cluster
        && left.mft_record_size == right.mft_record_size
        && left.mft_length == right.mft_length
        && left.parser_version == right.parser_version
}

fn same_snapshot_volume(left: &DiskSnapshot, right: &DiskSnapshot) -> bool {
    left.metadata
        .root
        .eq_ignore_ascii_case(&right.metadata.root)
        && left.metadata.identity.volume_serial == right.metadata.identity.volume_serial
        && left.metadata.identity.bytes_per_sector == right.metadata.identity.bytes_per_sector
        && left.metadata.identity.bytes_per_cluster == right.metadata.identity.bytes_per_cluster
        && left.metadata.identity.mft_record_size == right.metadata.identity.mft_record_size
        && left.metadata.identity.parser_version == right.metadata.identity.parser_version
}

fn is_older_snapshot(current: &DiskSnapshot, candidate: &DiskSnapshot) -> bool {
    match (
        current.metadata.identity.last_usn,
        candidate.metadata.identity.last_usn,
    ) {
        (Some(current), Some(candidate)) => candidate <= current,
        (Some(_), None) => true,
        (None, _) => true,
    }
}

#[cfg(windows)]
#[repr(C)]
struct WofFileCompressionInfo {
    algorithm: u32,
    flags: u32,
}

fn compress_snapshot_file(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows::core::{BOOL, HSTRING};
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, WofIsExternalFile, WofSetFileDataLocation, FILE_GENERIC_READ,
            FILE_GENERIC_WRITE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
            OPEN_EXISTING, WOF_PROVIDER_FILE,
        };

        let path_string = HSTRING::from(path.to_string_lossy().as_ref());
        let mut is_external = BOOL(0);
        let already_compressed = unsafe {
            WofIsExternalFile(&path_string, Some(&mut is_external), None, None, None).is_ok()
                && is_external.as_bool()
        };
        if already_compressed {
            return Ok(());
        }

        let handle = unsafe {
            CreateFileW(
                &path_string,
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                Default::default(),
                None,
            )
        }
        .map_err(|error| format!("unable to open snapshot for compression: {}", error))?;

        let info = WofFileCompressionInfo {
            algorithm: selected_snapshot_compression_algorithm().provider_algorithm(),
            flags: 0,
        };
        let result = unsafe {
            WofSetFileDataLocation(
                handle,
                WOF_PROVIDER_FILE,
                std::ptr::addr_of!(info).cast(),
                std::mem::size_of::<WofFileCompressionInfo>() as u32,
            )
        };
        let _ = unsafe { CloseHandle(handle) };
        result.map_err(|error| format!("WOF compression failed: {}", error))
    }

    #[cfg(not(windows))]
    {
        let _ = path;
        Ok(())
    }
}

#[cfg(windows)]
fn compress_existing_snapshots(directory: &Path) -> Result<(), String> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("unable to enumerate disk snapshots: {}", error))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("wasnap") {
            continue;
        }
        if let Err(error) = compress_snapshot_file(&path) {
            log::debug!(
                "Unable to compress existing disk snapshot {}: {}",
                path.display(),
                error
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Ok(())
}

fn decode_snapshot(bytes: &[u8]) -> Result<DiskSnapshot, String> {
    if bytes.len() < HEADER_SIZE || &bytes[..SNAPSHOT_MAGIC.len()] != SNAPSHOT_MAGIC {
        return Err("disk snapshot has an invalid header".into());
    }
    let schema_start = SNAPSHOT_MAGIC.len();
    let schema_end = schema_start + 4;
    let schema = u32::from_le_bytes(
        bytes[schema_start..schema_end]
            .try_into()
            .map_err(|_| "disk snapshot schema header is truncated")?,
    );
    if schema != SNAPSHOT_SCHEMA_VERSION {
        return Err(format!(
            "unsupported disk snapshot schema {} (expected {})",
            schema, SNAPSHOT_SCHEMA_VERSION
        ));
    }
    let length_start = schema_end;
    let length_end = length_start + 8;
    let payload_len = usize::try_from(u64::from_le_bytes(
        bytes[length_start..length_end]
            .try_into()
            .map_err(|_| "disk snapshot length header is truncated")?,
    ))
    .map_err(|_| "disk snapshot payload is too large")?;
    let payload_end = HEADER_SIZE
        .checked_add(payload_len)
        .ok_or_else(|| "disk snapshot payload length overflowed".to_string())?;
    if payload_end != bytes.len() {
        return Err("disk snapshot payload length does not match the file".into());
    }
    let snapshot: DiskSnapshot = serde_json::from_slice(&bytes[HEADER_SIZE..payload_end])
        .map_err(|error| format!("disk snapshot payload is invalid: {}", error))?;
    if snapshot.snapshot_id.is_empty() || !snapshot.metadata.complete {
        return Err("disk snapshot is not a complete usable snapshot".into());
    }
    Ok(snapshot)
}

fn atomic_replace(source: &Path, target: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING};

        let mut source_wide: Vec<u16> = source.as_os_str().encode_wide().collect();
        let mut target_wide: Vec<u16> = target.as_os_str().encode_wide().collect();
        source_wide.push(0);
        target_wide.push(0);
        unsafe {
            MoveFileExW(
                PCWSTR(source_wide.as_ptr()),
                PCWSTR(target_wide.as_ptr()),
                MOVEFILE_REPLACE_EXISTING,
            )
        }
        .map_err(|error| format!("unable to replace disk snapshot atomically: {}", error))
    }

    #[cfg(not(windows))]
    {
        fs::rename(source, target)
            .map_err(|error| format!("unable to replace disk snapshot atomically: {}", error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::disk_usage::{
        DiskCacheSource, DiskDiagnostics, DiskNode, DiskReconciliation, DiskScanSource,
    };
    use tempfile::tempdir;

    fn sample_snapshot() -> DiskSnapshot {
        let root = DiskNode {
            id: 0,
            parent_id: None,
            name: "C:".into(),
            is_directory: true,
            has_children: true,
            logical_size_bytes: 12,
            allocated_size_bytes: 16,
            unique_allocated_size_bytes: 16,
            file_count: 1,
            directory_count: 1,
            flags: 0,
            shared_alias: false,
        };
        DiskSnapshot {
            snapshot_id: "sample".into(),
            metadata: SnapshotMetadata {
                root: "C:\\".into(),
                engine: ScanEngine::NtfsMft,
                identity: SnapshotIdentity {
                    volume_serial: Some(1),
                    bytes_per_sector: Some(512),
                    bytes_per_cluster: Some(4096),
                    mft_record_size: Some(1024),
                    mft_length: Some(2048),
                    journal_id: Some(2),
                    last_usn: Some(3),
                    parser_version: PARSER_SCHEMA_VERSION,
                },
                status: ScanStatus::Complete,
                complete: true,
            },
            root: root.clone(),
            nodes: vec![root],
            details: Vec::new(),
            summary: DiskSummary::default(),
            diagnostics: DiskDiagnostics {
                source: DiskScanSource::Mft,
                cache_source: DiskCacheSource::Fresh,
                reconciliation: DiskReconciliation::Unavailable,
                elapsed_ms: 0,
                partial_scan: false,
                access_denied: false,
                skipped_records: 0,
                reparse_points: 0,
                messages: Vec::new(),
            },
        }
    }

    #[test]
    fn selects_compression_algorithm_by_logical_processor_count() {
        assert_eq!(
            SnapshotCompressionAlgorithm::for_logical_processors(4),
            SnapshotCompressionAlgorithm::Xpress4k
        );
        assert_eq!(
            SnapshotCompressionAlgorithm::for_logical_processors(5),
            SnapshotCompressionAlgorithm::Xpress4k
        );
        assert_eq!(
            SnapshotCompressionAlgorithm::for_logical_processors(6),
            SnapshotCompressionAlgorithm::Xpress8k
        );
        assert_eq!(
            SnapshotCompressionAlgorithm::for_logical_processors(12),
            SnapshotCompressionAlgorithm::Xpress16k
        );
        assert_eq!(
            SnapshotCompressionAlgorithm::for_logical_processors(16),
            SnapshotCompressionAlgorithm::Lzx
        );
    }

    #[test]
    fn indexes_snapshot_children_for_navigation() {
        let mut snapshot = sample_snapshot();
        let mut first = snapshot.root.clone();
        first.id = 1;
        first.parent_id = Some(0);
        first.name = "first.bin".into();
        let mut second = first.clone();
        second.id = 2;
        second.name = "second.bin".into();
        snapshot.nodes.extend([first, second]);

        let index = DiskSnapshotIndex::new(snapshot);

        assert_eq!(index.children.get(&None), Some(&vec![0]));
        assert_eq!(index.children.get(&Some(0)), Some(&vec![1, 2]));
    }

    #[test]
    fn round_trips_through_the_versioned_mmap_format() {
        let directory = tempdir().unwrap();
        let store = SnapshotStore::new(directory.path().to_path_buf());
        let snapshot = sample_snapshot();
        store.write_complete(&snapshot).unwrap();
        let loaded = store.read("sample").unwrap();
        assert_eq!(loaded.snapshot_id, snapshot.snapshot_id);
        assert_eq!(loaded.nodes[0].allocated_size_bytes, 16);
    }

    #[test]
    fn rejects_corrupt_and_schema_mismatched_snapshots() {
        let directory = tempdir().unwrap();
        let store = SnapshotStore::new(directory.path().to_path_buf());
        let snapshot = sample_snapshot();
        store.write_complete(&snapshot).unwrap();
        let path = directory.path().join("sample.wasnap");
        let mut bytes = fs::read(&path).unwrap();
        bytes[0] = b'X';
        fs::write(&path, &bytes).unwrap();
        assert!(store.read("sample").is_err());

        store.write_complete(&snapshot).unwrap();
        let mut bytes = fs::read(&path).unwrap();
        bytes[8..12].copy_from_slice(&(SNAPSHOT_SCHEMA_VERSION + 1).to_le_bytes());
        fs::write(&path, &bytes).unwrap();
        assert!(store.read("sample").is_err());
    }

    #[test]
    fn candidate_matching_distinguishes_exact_and_delta_ranges() {
        let directory = tempdir().unwrap();
        let store = SnapshotStore::new(directory.path().to_path_buf());
        let snapshot = sample_snapshot();
        store.write_complete(&snapshot).unwrap();
        let identity = snapshot.metadata.identity.clone();
        assert_eq!(
            store
                .find_candidate(&identity, Some(3))
                .unwrap()
                .unwrap()
                .match_kind,
            SnapshotMatch::Exact
        );
        assert_eq!(
            store
                .find_candidate(&identity, Some(4))
                .unwrap()
                .unwrap()
                .match_kind,
            SnapshotMatch::NeedsDelta
        );
    }

    #[test]
    fn rejects_missing_or_reversed_usn_continuity() {
        let directory = tempdir().unwrap();
        let store = SnapshotStore::new(directory.path().to_path_buf());
        let mut snapshot = sample_snapshot();
        snapshot.snapshot_id = "unverifiable".into();
        snapshot.metadata.identity.last_usn = None;
        store.write_complete(&snapshot).unwrap();

        let identity = sample_snapshot().metadata.identity;
        assert!(store.find_candidate(&identity, Some(3)).unwrap().is_none());
        assert!(store.find_candidate(&identity, Some(2)).unwrap().is_none());
    }

    #[test]
    fn prunes_older_snapshot_for_the_same_volume() {
        let directory = tempdir().unwrap();
        let store = SnapshotStore::new(directory.path().to_path_buf());
        let old = sample_snapshot();
        store.write_complete(&old).unwrap();

        let mut current = old.clone();
        current.snapshot_id = "current".into();
        current.metadata.identity.last_usn = Some(4);
        store.write_complete(&current).unwrap();

        assert!(!directory.path().join("sample.wasnap").exists());
        assert!(directory.path().join("current.wasnap").exists());
        assert_eq!(store.read("current").unwrap().snapshot_id, "current");
    }

    #[test]
    fn applies_create_rename_size_and_delete_changes_to_ancestors() {
        let mut snapshot = sample_snapshot();
        apply_changes(
            &mut snapshot,
            &[
                SnapshotChange {
                    node_id: Some(1),
                    parent_id: Some(0),
                    name: "folder".into(),
                    is_directory: true,
                    logical_size_bytes: 0,
                    allocated_size_bytes: 0,
                    flags: 0,
                    shared_alias: false,
                    detail: None,
                    deleted: false,
                },
                SnapshotChange {
                    node_id: Some(2),
                    parent_id: Some(1),
                    name: "file.bin".into(),
                    is_directory: false,
                    logical_size_bytes: 64,
                    allocated_size_bytes: 4096,
                    flags: 0,
                    shared_alias: false,
                    detail: None,
                    deleted: false,
                },
            ],
            10,
        )
        .unwrap();
        assert_eq!(snapshot.root.logical_size_bytes, 64);
        assert_eq!(snapshot.root.allocated_size_bytes, 4096);
        assert_eq!(snapshot.metadata.identity.last_usn, Some(10));

        apply_changes(
            &mut snapshot,
            &[SnapshotChange {
                node_id: Some(2),
                parent_id: Some(0),
                name: "renamed.bin".into(),
                is_directory: false,
                logical_size_bytes: 128,
                allocated_size_bytes: 8192,
                flags: 0,
                shared_alias: false,
                detail: None,
                deleted: false,
            }],
            11,
        )
        .unwrap();
        assert_eq!(snapshot.root.logical_size_bytes, 128);
        assert_eq!(
            snapshot
                .nodes
                .iter()
                .find(|node| node.id == 2)
                .unwrap()
                .name,
            "renamed.bin"
        );

        apply_changes(
            &mut snapshot,
            &[SnapshotChange {
                node_id: Some(2),
                parent_id: None,
                name: String::new(),
                is_directory: false,
                logical_size_bytes: 0,
                allocated_size_bytes: 0,
                flags: 0,
                shared_alias: false,
                detail: None,
                deleted: true,
            }],
            12,
        )
        .unwrap();
        assert!(!snapshot.nodes.iter().any(|node| node.id == 2));
    }

    #[test]
    fn rejects_usn_boundaries_that_move_backwards() {
        let mut snapshot = sample_snapshot();
        snapshot.metadata.identity.last_usn = Some(10);
        assert!(apply_changes(&mut snapshot, &[], 9).is_err());
    }

    #[test]
    fn un_reconciled_usn_delta_is_partial_and_cannot_replace_the_cache() {
        let directory = tempdir().unwrap();
        let mut snapshot = sample_snapshot();
        snapshot.summary.unaccounted_size_bytes = Some(1234);
        snapshot.diagnostics.reconciliation = DiskReconciliation::VolumeBitmap;

        apply_changes(&mut snapshot, &[], 4).unwrap();

        assert_eq!(snapshot.metadata.status, ScanStatus::Partial);
        assert!(!snapshot.metadata.complete);
        assert!(snapshot.diagnostics.partial_scan);
        assert_eq!(
            snapshot.diagnostics.reconciliation,
            DiskReconciliation::Unavailable
        );
        assert_eq!(snapshot.summary.unaccounted_size_bytes, None);
        assert!(SnapshotStore::new(directory.path().to_path_buf())
            .write_complete(&snapshot)
            .is_err());
    }

    #[test]
    fn shared_aliases_are_counted_once_in_unique_allocation() {
        let mut snapshot = sample_snapshot();
        apply_changes(
            &mut snapshot,
            &[
                SnapshotChange {
                    node_id: Some(1),
                    parent_id: Some(0),
                    name: "primary.bin".into(),
                    is_directory: false,
                    logical_size_bytes: 64,
                    allocated_size_bytes: 4096,
                    flags: 0,
                    shared_alias: false,
                    detail: None,
                    deleted: false,
                },
                SnapshotChange {
                    node_id: Some(2),
                    parent_id: Some(0),
                    name: "alias.bin".into(),
                    is_directory: false,
                    logical_size_bytes: 64,
                    allocated_size_bytes: 4096,
                    flags: 0,
                    shared_alias: true,
                    detail: None,
                    deleted: false,
                },
            ],
            13,
        )
        .unwrap();
        assert_eq!(snapshot.root.allocated_size_bytes, 8192);
        assert_eq!(snapshot.root.unique_allocated_size_bytes, 4096);
        assert_eq!(
            snapshot
                .nodes
                .iter()
                .find(|node| node.id == 2)
                .unwrap()
                .unique_allocated_size_bytes,
            0
        );
    }
}
