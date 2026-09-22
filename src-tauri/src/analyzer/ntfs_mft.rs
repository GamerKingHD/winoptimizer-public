//! Read-only NTFS MFT scanner for whole-volume disk analysis.
//!
//! A directory walk opens and stats every path.  NTFS already stores the
//! metadata needed by the analyzer in the MFT, so a volume-root scan can read
//! that metadata sequentially and avoid one filesystem call per file.

use super::disk_usage::{
    output_from_tree, DiskCacheSource, DiskDiagnostics, DiskNode, DiskScanSource, ScanEngine,
    ScanOutput, ScanStatus, TreeDiskNode, TreeEntryDetails, NODE_FLAG_COMPRESSED,
    NODE_FLAG_REPARSE, NODE_FLAG_SPARSE,
};
use super::snapshot::DiskStreamDetails;
use super::snapshot::{
    DiskEntryDetails, DiskSnapshot, SnapshotChange, SnapshotIdentity, SnapshotMatch,
    SnapshotMetadata, SnapshotStore,
};
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, SetFilePointerEx, FILE_BEGIN, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_GENERIC_READ, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Ioctl::{FSCTL_GET_NTFS_VOLUME_DATA, NTFS_VOLUME_DATA_BUFFER};
use windows::Win32::System::IO::DeviceIoControl;

const FILE_RECORD_IN_USE: u16 = 0x0001;
const FILE_RECORD_DIRECTORY: u16 = 0x0002;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
const USN_REASON_FILE_DELETE: u32 = 0x0000_0010;
const NTFS_RESERVED_RECORD_MAX: u64 = 15;
const ATTRIBUTE_STANDARD_INFORMATION: u32 = 0x10;
const ATTRIBUTE_LIST: u32 = 0x20;
const ATTRIBUTE_FILE_NAME: u32 = 0x30;
const ATTRIBUTE_DATA: u32 = 0x80;
const ATTRIBUTE_REPARSE_POINT: u32 = 0xc0;
const ATTRIBUTE_END: u32 = 0xffff_ffff;
const FILE_REFERENCE_MASK: u64 = 0x0000_ffff_ffff_ffff;
const MFT_READ_CHUNK: usize = 16 * 1024 * 1024;

fn missing_mft_record_is_safe_to_remove(reason_mask: u32) -> bool {
    reason_mask & USN_REASON_FILE_DELETE != 0
}

struct ReadOnlyVolume {
    handle: HANDLE,
}

impl ReadOnlyVolume {
    fn open(drive: char) -> Result<Self, String> {
        let volume_path = format!(r"\\.\{}:", drive.to_ascii_uppercase());
        let path = HSTRING::from(volume_path.as_str());
        let handle = unsafe {
            CreateFileW(
                &path,
                FILE_GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                None,
            )
        }
        .map_err(|error| format!("unable to open {} read-only: {}", volume_path, error))?;

        Ok(Self { handle })
    }

    fn query_ntfs_data(&self) -> Result<NTFS_VOLUME_DATA_BUFFER, String> {
        let mut data = NTFS_VOLUME_DATA_BUFFER::default();
        let mut returned = 0u32;
        let output_size = u32::try_from(std::mem::size_of::<NTFS_VOLUME_DATA_BUFFER>())
            .map_err(|_| "NTFS volume metadata structure is too large".to_string())?;

        unsafe {
            DeviceIoControl(
                self.handle,
                FSCTL_GET_NTFS_VOLUME_DATA,
                None,
                0,
                Some(&mut data as *mut _ as *mut c_void),
                output_size,
                Some(&mut returned),
                None,
            )
        }
        .map_err(|error| format!("unable to query NTFS volume metadata: {}", error))?;

        if returned < output_size {
            return Err("NTFS volume metadata response was incomplete".into());
        }
        Ok(data)
    }

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, String> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let offset = i64::try_from(offset)
            .map_err(|_| "NTFS metadata offset exceeds the Windows file API".to_string())?;
        let mut bytes_read = 0u32;

        unsafe {
            SetFilePointerEx(self.handle, offset, None, FILE_BEGIN)
                .map_err(|error| format!("unable to seek in NTFS metadata: {}", error))?;
            ReadFile(self.handle, Some(buffer), Some(&mut bytes_read), None)
                .map_err(|error| format!("unable to read NTFS metadata: {}", error))?;
        }

        Ok(bytes_read as usize)
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<(), String> {
        let read = self.read_at(offset, buffer)?;
        if read != buffer.len() {
            return Err(format!(
                "short NTFS metadata read at offset {}: {} of {} bytes",
                offset,
                read,
                buffer.len()
            ));
        }
        Ok(())
    }

    /// Read the point-in-time cluster bitmap without changing the volume.
    fn allocated_clusters(&self, bytes_per_cluster: u64) -> Result<u64, String> {
        const FSCTL_GET_VOLUME_BITMAP: u32 = 0x0009_006f;
        const ERROR_MORE_DATA: u32 = 234;
        #[repr(C)]
        struct StartLcn {
            starting_lcn: i64,
        }

        let mut starting_lcn = 0i64;
        let mut allocated_clusters = 0u64;
        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            let input = StartLcn { starting_lcn };
            let mut returned = 0u32;
            let result = unsafe {
                DeviceIoControl(
                    self.handle,
                    FSCTL_GET_VOLUME_BITMAP,
                    Some(&input as *const _ as *const c_void),
                    std::mem::size_of::<StartLcn>() as u32,
                    Some(buffer.as_mut_ptr() as *mut c_void),
                    buffer.len() as u32,
                    Some(&mut returned),
                    None,
                )
            };
            let more_data = result.as_ref().err().is_some_and(|error| {
                error.code() == windows::core::HRESULT::from_win32(ERROR_MORE_DATA)
            });
            if result.is_err() && !more_data {
                return Err(format!("FSCTL_GET_VOLUME_BITMAP failed: {:?}", result));
            }
            if returned < 16 {
                return Err("volume bitmap response was incomplete".into());
            }
            let bitmap_start = i64::from_le_bytes(
                buffer[0..8]
                    .try_into()
                    .map_err(|_| "volume bitmap start is truncated")?,
            );
            let bitmap_bits = u64::try_from(i64::from_le_bytes(
                buffer[8..16]
                    .try_into()
                    .map_err(|_| "volume bitmap length is truncated")?,
            ))
            .map_err(|_| "volume bitmap length is negative")?;
            if bitmap_bits == 0 {
                break;
            }
            let available = usize::try_from(returned)
                .unwrap_or(0)
                .saturating_sub(16)
                .min(buffer.len().saturating_sub(16));
            let available_bits = u64::try_from(available)
                .map_err(|_| "volume bitmap payload is too large")?
                .saturating_mul(8);
            let represented_bits = bitmap_bits.min(available_bits);
            if represented_bits == 0 {
                return Err("volume bitmap payload is truncated".into());
            }
            if !more_data && represented_bits < bitmap_bits {
                return Err("volume bitmap payload is truncated".into());
            }
            if bitmap_start < 0 || bitmap_start > starting_lcn {
                return Err("volume bitmap start moved past the requested LCN".into());
            }
            let bitmap_bytes = usize::try_from(represented_bits.saturating_add(7) / 8)
                .map_err(|_| "volume bitmap is too large")?;
            let skipped_bits = usize::try_from(
                u64::try_from(starting_lcn - bitmap_start)
                    .map_err(|_| "volume bitmap offset is negative")?,
            )
            .map_err(|_| "volume bitmap offset is too large")?;
            let represented_bits =
                usize::try_from(represented_bits).map_err(|_| "volume bitmap is too large")?;
            if skipped_bits >= represented_bits {
                return Err("volume bitmap did not advance between batches".into());
            }
            let set_bits = count_set_bits(
                &buffer[16..16 + bitmap_bytes],
                skipped_bits,
                represented_bits - skipped_bits,
            );
            allocated_clusters = allocated_clusters.saturating_add(set_bits);
            let next = bitmap_start
                .checked_add(
                    i64::try_from(represented_bits)
                        .map_err(|_| "volume bitmap offset is too large")?,
                )
                .ok_or_else(|| "volume bitmap offset overflowed".to_string())?;
            if more_data && next <= starting_lcn {
                return Err("volume bitmap did not advance between batches".into());
            }
            starting_lcn = next;
            if !more_data {
                break;
            }
        }
        allocated_clusters
            .checked_mul(bytes_per_cluster)
            .ok_or_else(|| "volume bitmap allocation size overflowed".to_string())
    }
}

fn count_set_bits(bytes: &[u8], start_bit: usize, bit_count: usize) -> u64 {
    if bit_count == 0 {
        return 0;
    }
    let end_bit = start_bit.saturating_add(bit_count);
    let first_byte = start_bit / 8;
    let last_byte = (end_bit - 1) / 8;
    (first_byte..=last_byte)
        .map(|index| {
            let mut byte = bytes[index];
            if index == first_byte {
                byte &= u8::MAX << (start_bit % 8);
            }
            if index == last_byte && !end_bit.is_multiple_of(8) {
                byte &= (1u16 << (end_bit % 8)) as u8 - 1;
            }
            u64::from(byte.count_ones())
        })
        .sum()
}

impl Drop for ReadOnlyVolume {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

#[derive(Debug, Clone)]
struct DataRun {
    lcn: Option<i64>,
    clusters: u64,
}

#[derive(Debug, Clone)]
struct FileLink {
    parent_record: u64,
    parent_sequence: u16,
    namespace: u8,
    size_bytes: u64,
    name: String,
}

#[derive(Debug, Clone)]
struct MftEntry {
    record_number: u64,
    base_reference: u64,
    sequence: u16,
    parent_record: u64,
    parent_sequence: u16,
    name: String,
    links: Vec<FileLink>,
    size_bytes: u64,
    allocated_size_bytes: u64,
    streams: Vec<DiskStreamDetails>,
    attribute_list_references: Vec<u64>,
    reparse_tag: Option<u32>,
    link_count: u32,
    is_directory: bool,
    is_reparse: bool,
}

#[derive(Debug, Default)]
struct ParseStats {
    files_scanned: u64,
    directories_scanned: u64,
    skipped_entries: u64,
    parse_errors: u64,
    parse_error_sample: Option<String>,
    unnamed_entries: u64,
    unnamed_sample: Vec<u64>,
    orphaned_entries: u64,
    reparse_points: u64,
    depth_limited_directories: u64,
}

struct StreamResult {
    entries: Vec<MftEntry>,
    extensions: Vec<MftEntry>,
    stats: ParseStats,
    complete: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct Aggregate {
    size_bytes: u64,
    allocated_size_bytes: u64,
    file_count: u64,
}

pub(crate) fn scan_volume(
    drive: char,
    max_depth: u32,
    cancel_flag: &Arc<AtomicBool>,
    progress: &Arc<AtomicU64>,
    allow_cache: bool,
) -> Result<ScanOutput, String> {
    let volume = ReadOnlyVolume::open(drive)?;
    let volume_data = volume.query_ntfs_data()?;
    let bytes_per_sector = usize::try_from(volume_data.BytesPerSector)
        .map_err(|_| "invalid NTFS sector size".to_string())?;
    let bytes_per_cluster = u64::from(volume_data.BytesPerCluster);
    let record_size = usize::try_from(volume_data.BytesPerFileRecordSegment)
        .map_err(|_| "invalid NTFS file record size".to_string())?;
    let mft_valid_length = u64::try_from(volume_data.MftValidDataLength)
        .map_err(|_| "invalid NTFS MFT length".to_string())?;
    let mft_start_lcn =
        u64::try_from(volume_data.MftStartLcn).map_err(|_| "invalid NTFS MFT start".to_string())?;
    let mut identity = SnapshotIdentity {
        volume_serial: u64::try_from(volume_data.VolumeSerialNumber).ok(),
        bytes_per_sector: Some(volume_data.BytesPerSector),
        bytes_per_cluster: Some(volume_data.BytesPerCluster),
        mft_record_size: Some(volume_data.BytesPerFileRecordSegment),
        mft_length: Some(mft_valid_length),
        journal_id: None,
        last_usn: None,
        parser_version: super::snapshot::PARSER_SCHEMA_VERSION,
    };
    let mut cached_delta = None;
    let mut delta_fallback_message = None;
    let journal = super::usn_journal::journal_state(drive).ok();
    if let Some((journal_id, next_usn)) = journal {
        identity.journal_id = Some(journal_id);
        identity.last_usn = Some(next_usn);
        if allow_cache {
            if let Some(candidate) =
                SnapshotStore::default_location()?.find_candidate(&identity, Some(next_usn))?
            {
                if candidate.match_kind == SnapshotMatch::Exact {
                    let mut snapshot = candidate.snapshot;
                    snapshot.diagnostics.source = DiskScanSource::Snapshot;
                    snapshot.diagnostics.cache_source = DiskCacheSource::Snapshot;
                    let result = snapshot.result();
                    return Ok(ScanOutput { result, snapshot });
                } else if let Some(saved_usn) = candidate.snapshot.metadata.identity.last_usn {
                    cached_delta = Some((candidate.snapshot, saved_usn));
                }
            }
        }
    }

    if !(512..=4096).contains(&bytes_per_sector)
        || bytes_per_cluster == 0
        || bytes_per_cluster < bytes_per_sector as u64
        || !(512..=1024 * 1024).contains(&record_size)
        || mft_valid_length < record_size as u64
    {
        return Err("NTFS volume geometry is outside the supported scanner range".into());
    }

    let mft_offset = mft_start_lcn
        .checked_mul(bytes_per_cluster)
        .ok_or_else(|| "NTFS MFT offset overflowed".to_string())?;
    let mut record_zero = vec![0u8; record_size];
    volume.read_exact_at(mft_offset, &mut record_zero)?;
    apply_update_sequence_fixups(&mut record_zero, bytes_per_sector)?;
    let runs = decode_mft_data_runs(&record_zero, bytes_per_sector)?;

    if let Some((mut snapshot, saved_usn)) = cached_delta {
        if let Some(current_usn) = identity.last_usn {
            match apply_usn_delta(
                drive,
                &volume,
                &runs,
                bytes_per_cluster,
                record_size,
                bytes_per_sector,
                mft_valid_length,
                &mut snapshot,
                saved_usn,
                current_usn,
            ) {
                Ok(()) => {
                    let result = snapshot.result();
                    return Ok(ScanOutput { result, snapshot });
                }
                Err(error) => {
                    log::debug!("USN delta unavailable; rebuilding MFT snapshot: {}", error);
                    delta_fallback_message = Some(format!(
                        "USN cache update was unavailable; performed a fresh MFT scan: {}",
                        error
                    ));
                }
            }
        }
    }

    let stream = stream_mft(
        &volume,
        &runs,
        mft_valid_length,
        bytes_per_cluster,
        record_size,
        bytes_per_sector,
        cancel_flag,
        progress,
    )?;

    if !stream.complete && stream.entries.iter().all(|entry| entry.record_number != 5) {
        return Ok(cancelled_empty_result(drive, stream.stats, identity));
    }

    let mut output = build_tree(
        drive,
        max_depth,
        stream.entries,
        stream.extensions,
        stream.stats,
        identity,
    )?;
    if let Some(message) = delta_fallback_message {
        output.result.diagnostics.messages.push(message);
        output.snapshot.diagnostics = output.result.diagnostics.clone();
    }
    let metadata_size_bytes =
        (mft_valid_length.saturating_add(bytes_per_cluster.saturating_sub(1)) / bytes_per_cluster)
            .saturating_mul(bytes_per_cluster);
    output.result.summary.metadata_size_bytes = metadata_size_bytes;
    output.snapshot.summary = output.result.summary.clone();
    if !cancel_flag.load(Ordering::Relaxed) {
        match volume.allocated_clusters(bytes_per_cluster) {
            Ok(volume_allocated_bytes) => {
                let accounted_bytes = output
                    .result
                    .summary
                    .unique_allocated_size_bytes
                    .saturating_add(metadata_size_bytes);
                output.result.summary.metadata_size_bytes = metadata_size_bytes;
                output.result.summary.unaccounted_size_bytes =
                    Some(volume_allocated_bytes.saturating_sub(accounted_bytes));
                output.result.diagnostics.reconciliation =
                    super::disk_usage::DiskReconciliation::VolumeBitmap;
                output
                    .result
                    .diagnostics
                    .messages
                    .push("Volume allocation is a point-in-time bitmap reconciliation".into());
                output.snapshot.summary = output.result.summary.clone();
                output.snapshot.diagnostics = output.result.diagnostics.clone();
            }
            Err(error) => {
                output.result.diagnostics.messages.push(format!(
                    "Volume allocation reconciliation unavailable: {}",
                    error
                ));
                output.snapshot.diagnostics = output.result.diagnostics.clone();
            }
        }
    }
    if cancel_flag.load(Ordering::Relaxed) {
        output.result.status = ScanStatus::Cancelled;
        output.snapshot.metadata.status = ScanStatus::Cancelled;
        output.snapshot.metadata.complete = false;
        output.snapshot.diagnostics.partial_scan = true;
        output.result.diagnostics.partial_scan = true;
    }
    Ok(output)
}

#[allow(clippy::too_many_arguments)]
fn apply_usn_delta(
    drive: char,
    volume: &ReadOnlyVolume,
    runs: &[DataRun],
    bytes_per_cluster: u64,
    record_size: usize,
    bytes_per_sector: usize,
    mft_valid_length: u64,
    snapshot: &mut DiskSnapshot,
    saved_usn: i64,
    current_usn: i64,
) -> Result<(), String> {
    if saved_usn >= current_usn {
        return Err("USN delta range is empty or reversed".into());
    }
    let journal_changes = super::usn_journal::read_changes_for_journal(
        drive,
        snapshot.metadata.identity.journal_id,
        saved_usn,
        current_usn,
    )?;
    let mut changed_records = HashMap::<u64, (Vec<u16>, u32)>::new();
    for change in journal_changes {
        let record_number = change.file_reference & FILE_REFERENCE_MASK;
        if record_number == 0 {
            continue;
        }
        let sequence = (change.file_reference >> 48) as u16;
        let (sequences, reason_mask) = changed_records.entry(record_number).or_default();
        if !sequences.contains(&sequence) {
            sequences.push(sequence);
        }
        *reason_mask |= change.reason;
    }

    let mut record_numbers: Vec<u64> = changed_records.keys().copied().collect();
    record_numbers.sort_unstable();
    let mut current_entries = Vec::with_capacity(record_numbers.len());
    for record_number in record_numbers {
        let entry = read_mft_entry_with_extensions(
            volume,
            runs,
            bytes_per_cluster,
            record_size,
            bytes_per_sector,
            mft_valid_length,
            record_number,
        )?;
        if entry
            .as_ref()
            .is_some_and(|entry| entry.base_reference != 0)
        {
            return Err("USN delta includes an MFT extension record".into());
        }
        let (old_sequences, reason_mask) =
            changed_records.remove(&record_number).unwrap_or_default();
        if entry.is_none() && !missing_mft_record_is_safe_to_remove(reason_mask) {
            return Err(format!(
                "changed MFT record {} was unavailable",
                record_number
            ));
        }
        current_entries.push((record_number, old_sequences, entry));
    }
    current_entries.sort_by(|left, right| {
        right
            .2
            .as_ref()
            .is_some_and(|entry| entry.is_directory)
            .cmp(&left.2.as_ref().is_some_and(|entry| entry.is_directory))
            .then_with(|| left.0.cmp(&right.0))
    });

    let mut next_id = snapshot
        .nodes
        .iter()
        .map(|node| node.id)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    let mut planned_directories = HashMap::<(u64, u16), u64>::new();
    let mut changes = Vec::new();

    for (record_number, old_sequences, entry) in current_entries {
        if record_number == 5 {
            continue;
        }
        let old_nodes = snapshot_nodes_for_record(snapshot, record_number, &old_sequences);
        let mut used_old_ids = HashSet::new();

        if let Some(entry) = entry {
            let mut variants = linked_entry_variants(entry);
            variants.sort_by(|left, right| {
                left.parent_record
                    .cmp(&right.parent_record)
                    .then_with(|| left.parent_sequence.cmp(&right.parent_sequence))
                    .then_with(|| left.name.cmp(&right.name))
            });
            for (variant_index, variant) in variants.into_iter().enumerate() {
                let parent_id = find_parent_node(
                    snapshot,
                    &planned_directories,
                    variant.parent_record,
                    variant.parent_sequence,
                )
                .ok_or_else(|| {
                    format!(
                        "USN delta parent {}:{} is not present in the snapshot",
                        variant.parent_record, variant.parent_sequence
                    )
                })?;
                let existing = old_nodes
                    .iter()
                    .find(|node| {
                        !used_old_ids.contains(&node.id)
                            && node.name == variant.name
                            && node.parent_id == Some(parent_id)
                    })
                    .or_else(|| {
                        old_nodes
                            .iter()
                            .find(|node| !used_old_ids.contains(&node.id))
                    });
                let node_id = if let Some(node) = existing {
                    used_old_ids.insert(node.id);
                    node.id
                } else {
                    let id = next_id;
                    next_id = next_id.saturating_add(1);
                    id
                };
                if variant.is_directory {
                    planned_directories.insert((record_number, variant.sequence), node_id);
                }
                let flags = entry_flags(&variant);
                changes.push(SnapshotChange {
                    node_id: Some(node_id),
                    parent_id: Some(parent_id),
                    name: variant.name.clone(),
                    is_directory: variant.is_directory,
                    logical_size_bytes: variant.size_bytes,
                    allocated_size_bytes: variant.allocated_size_bytes,
                    flags,
                    shared_alias: variant_index > 0,
                    detail: Some(delta_detail(snapshot, node_id, &variant, flags)),
                    deleted: false,
                });
            }
        }

        for node in old_nodes {
            if !used_old_ids.contains(&node.id) {
                changes.push(SnapshotChange {
                    node_id: Some(node.id),
                    parent_id: node.parent_id,
                    name: node.name,
                    is_directory: node.is_directory,
                    logical_size_bytes: node.logical_size_bytes,
                    allocated_size_bytes: node.allocated_size_bytes,
                    flags: node.flags,
                    shared_alias: node.shared_alias,
                    detail: None,
                    deleted: true,
                });
            }
        }
    }

    super::snapshot::apply_changes(snapshot, &changes, current_usn)
}

fn read_mft_entry_with_extensions(
    volume: &ReadOnlyVolume,
    runs: &[DataRun],
    bytes_per_cluster: u64,
    record_size: usize,
    bytes_per_sector: usize,
    mft_valid_length: u64,
    record_number: u64,
) -> Result<Option<MftEntry>, String> {
    let Some(mut entry) = read_mft_record(
        volume,
        runs,
        bytes_per_cluster,
        record_size,
        bytes_per_sector,
        mft_valid_length,
        record_number,
    )?
    else {
        return Ok(None);
    };
    if entry.base_reference != 0 || entry.attribute_list_references.is_empty() {
        return Ok(Some(entry));
    }
    let references = entry.attribute_list_references.clone();
    for extension_record in references {
        let Some(extension) = read_mft_record(
            volume,
            runs,
            bytes_per_cluster,
            record_size,
            bytes_per_sector,
            mft_valid_length,
            extension_record,
        )?
        else {
            return Err(format!(
                "NTFS attribute-list extension record {} was unavailable",
                extension_record
            ));
        };
        if extension.base_reference & FILE_REFERENCE_MASK != entry.record_number {
            return Err("NTFS attribute-list extension points at another base record".into());
        }
        merge_extension_into_base(&mut entry, extension);
    }
    Ok(Some(entry))
}

fn read_mft_record(
    volume: &ReadOnlyVolume,
    runs: &[DataRun],
    bytes_per_cluster: u64,
    record_size: usize,
    bytes_per_sector: usize,
    mft_valid_length: u64,
    record_number: u64,
) -> Result<Option<MftEntry>, String> {
    let logical_offset = record_number
        .checked_mul(record_size as u64)
        .ok_or_else(|| "MFT record offset overflowed".to_string())?;
    let record_end = logical_offset
        .checked_add(record_size as u64)
        .ok_or_else(|| "MFT record end overflowed".to_string())?;
    if record_end > mft_valid_length {
        return Ok(None);
    }
    let mut record = vec![0u8; record_size];
    read_logical_mft_bytes(volume, runs, bytes_per_cluster, logical_offset, &mut record)?;
    parse_record(
        &mut record,
        record_number,
        record_size,
        bytes_per_sector,
        bytes_per_cluster,
    )
}

fn read_logical_mft_bytes(
    volume: &ReadOnlyVolume,
    runs: &[DataRun],
    bytes_per_cluster: u64,
    logical_offset: u64,
    buffer: &mut [u8],
) -> Result<(), String> {
    if buffer.is_empty() {
        return Ok(());
    }
    let mut run_start = 0u64;
    let mut position = logical_offset;
    let mut written = 0usize;
    for run in runs {
        let run_bytes = run
            .clusters
            .checked_mul(bytes_per_cluster)
            .ok_or_else(|| "MFT data run length overflowed".to_string())?;
        let run_end = run_start
            .checked_add(run_bytes)
            .ok_or_else(|| "MFT data run end overflowed".to_string())?;
        if position < run_end {
            let within_run = position.saturating_sub(run_start);
            let available = run_end.saturating_sub(position);
            let count = available.min((buffer.len() - written) as u64) as usize;
            if let Some(lcn) = run.lcn {
                let physical_offset = u64::try_from(lcn)
                    .ok()
                    .and_then(|cluster| cluster.checked_mul(bytes_per_cluster))
                    .and_then(|offset| offset.checked_add(within_run))
                    .ok_or_else(|| "MFT data run physical offset is invalid".to_string())?;
                volume.read_exact_at(physical_offset, &mut buffer[written..written + count])?;
            } else {
                buffer[written..written + count].fill(0);
            }
            written += count;
            position = position
                .checked_add(count as u64)
                .ok_or_else(|| "MFT logical read offset overflowed".to_string())?;
            if written == buffer.len() {
                return Ok(());
            }
        }
        run_start = run_end;
    }
    Err("MFT data runs do not cover the requested record".into())
}

fn snapshot_nodes_for_record(
    snapshot: &DiskSnapshot,
    record_number: u64,
    sequences: &[u16],
) -> Vec<DiskNode> {
    snapshot
        .details
        .iter()
        .filter(|detail| {
            detail.mft_record_number == Some(record_number)
                && detail
                    .sequence
                    .is_some_and(|sequence| sequences.contains(&sequence))
        })
        .filter_map(|detail| {
            snapshot
                .nodes
                .iter()
                .find(|node| node.id == detail.node_id)
                .cloned()
        })
        .collect()
}

fn find_parent_node(
    snapshot: &DiskSnapshot,
    planned_directories: &HashMap<(u64, u16), u64>,
    record_number: u64,
    sequence: u16,
) -> Option<u64> {
    if record_number == 5 {
        return Some(0);
    }
    if let Some(node_id) = planned_directories.get(&(record_number, sequence)) {
        return Some(*node_id);
    }
    if sequence == 0 {
        if let Some(node_id) = planned_directories
            .iter()
            .find_map(|((record, _), node_id)| (*record == record_number).then_some(*node_id))
        {
            return Some(node_id);
        }
    }
    if let Some(node_id) = snapshot.details.iter().find_map(|detail| {
        if detail.mft_record_number == Some(record_number) && detail.sequence == Some(sequence) {
            snapshot
                .nodes
                .iter()
                .find(|node| node.id == detail.node_id && node.is_directory)
                .map(|node| node.id)
        } else {
            None
        }
    }) {
        return Some(node_id);
    }
    if sequence == 0 {
        return snapshot.details.iter().find_map(|detail| {
            (detail.mft_record_number == Some(record_number)
                && snapshot
                    .nodes
                    .iter()
                    .find(|node| node.id == detail.node_id)
                    .is_some_and(|node| node.is_directory))
            .then_some(detail.node_id)
        });
    }
    None
}

fn linked_entry_variants(mut entry: MftEntry) -> Vec<MftEntry> {
    let has_non_dos_link = entry.links.iter().any(|link| link.namespace != 2);
    if !has_non_dos_link {
        let mut kept_directory_link = false;
        entry.links.retain(|link| {
            if link.namespace != 2 {
                return true;
            }
            if kept_directory_link {
                false
            } else {
                kept_directory_link = true;
                true
            }
        });
    } else {
        entry.links.retain(|link| link.namespace != 2);
    }
    let link_count = entry.links.len() as u32;
    let streams = entry.streams.clone();
    let allocated_size_bytes = entry.allocated_size_bytes;
    let attribute_list_references = entry.attribute_list_references.clone();
    let reparse_tag = entry.reparse_tag;
    entry
        .links
        .into_iter()
        .map(|link| MftEntry {
            record_number: entry.record_number,
            base_reference: 0,
            sequence: entry.sequence,
            parent_record: link.parent_record,
            parent_sequence: link.parent_sequence,
            name: link.name.clone(),
            links: Vec::new(),
            size_bytes: entry.size_bytes.max(link.size_bytes),
            allocated_size_bytes,
            streams: streams.clone(),
            attribute_list_references: attribute_list_references.clone(),
            reparse_tag,
            link_count,
            is_directory: entry.is_directory,
            is_reparse: entry.is_reparse,
        })
        .collect()
}

fn delta_detail(
    snapshot: &DiskSnapshot,
    node_id: u64,
    entry: &MftEntry,
    flags: u32,
) -> DiskEntryDetails {
    let detail = entry_detail(entry);
    DiskEntryDetails {
        snapshot_id: snapshot.snapshot_id.clone(),
        node_id,
        path: String::new(),
        logical_size_bytes: entry.size_bytes,
        allocated_size_bytes: entry.allocated_size_bytes,
        unique_allocated_size_bytes: entry.allocated_size_bytes,
        mft_record_number: detail.mft_record_number,
        sequence: detail.sequence,
        file_reference: detail.file_reference,
        parent_reference: detail.parent_reference,
        flags,
        reparse_tag: detail.reparse_tag,
        link_count: detail.link_count,
        streams: detail.streams,
    }
}

fn merge_extension_into_base(base: &mut MftEntry, extension: MftEntry) {
    if extension.size_bytes > base.size_bytes {
        base.size_bytes = extension.size_bytes;
    }
    base.allocated_size_bytes = base
        .allocated_size_bytes
        .saturating_add(extension.allocated_size_bytes);
    base.streams.extend(extension.streams);
    base.is_directory |= extension.is_directory;
    for link in extension.links {
        if base.is_directory || link.namespace == 2 {
            continue;
        }
        let is_duplicate = base.links.iter().any(|existing| {
            existing.parent_record == link.parent_record && existing.name == link.name
        });
        if !is_duplicate {
            base.links.push(link);
        }
    }
    if base.name.is_empty() {
        if let Some(link) = base.links.first() {
            base.name = link.name.clone();
            base.parent_record = link.parent_record;
            base.parent_sequence = link.parent_sequence;
        }
    }
    base.is_reparse |= extension.is_reparse;
    base.reparse_tag = base.reparse_tag.or(extension.reparse_tag);
    base.link_count = base.links.len() as u32;
}

fn cancelled_empty_result(
    drive: char,
    stats: ParseStats,
    identity: SnapshotIdentity,
) -> ScanOutput {
    let root = TreeDiskNode {
        path: format!("{}:\\", drive.to_ascii_uppercase()),
        name: format!("{}:", drive.to_ascii_uppercase()),
        size_bytes: 0,
        allocated_size_bytes: 0,
        file_id: 5,
        file_count: 0,
        is_directory: true,
        is_truncated: true,
        flags: 0,
        detail: None,
        children: Vec::new(),
    };
    let diagnostics = DiskDiagnostics {
        source: DiskScanSource::Mft,
        cache_source: DiskCacheSource::Fresh,
        reconciliation: super::disk_usage::DiskReconciliation::Unavailable,
        elapsed_ms: 0,
        partial_scan: true,
        access_denied: false,
        skipped_records: stats.skipped_entries,
        reparse_points: stats.reparse_points,
        messages: vec!["The scan was cancelled before the MFT root was indexed".into()],
    };
    let metadata = SnapshotMetadata {
        root: root.path.clone(),
        engine: ScanEngine::NtfsMft,
        identity,
        status: ScanStatus::Cancelled,
        complete: false,
    };
    output_from_tree(
        root,
        ScanStatus::Cancelled,
        ScanEngine::NtfsMft,
        DiskScanSource::Mft,
        diagnostics,
        metadata,
    )
}

#[allow(clippy::too_many_arguments)]
fn stream_mft(
    volume: &ReadOnlyVolume,
    runs: &[DataRun],
    mft_valid_length: u64,
    bytes_per_cluster: u64,
    record_size: usize,
    bytes_per_sector: usize,
    cancel_flag: &Arc<AtomicBool>,
    progress: &Arc<AtomicU64>,
) -> Result<StreamResult, String> {
    let record_count = mft_valid_length / record_size as u64;
    if record_count == 0 {
        return Err("NTFS MFT contains no complete records".into());
    }

    let mut entries = Vec::new();
    let mut extensions = Vec::new();
    let mut stats = ParseStats::default();
    let mut carry = Vec::with_capacity(record_size);
    let mut record_number = 0u64;
    let mut logical_remaining = record_count * record_size as u64;
    let mut complete = true;
    let chunk_size = MFT_READ_CHUNK - (MFT_READ_CHUNK % record_size);
    let mut buffer = vec![0u8; chunk_size.max(record_size)];

    for data_run in runs {
        if logical_remaining == 0 || cancel_flag.load(Ordering::Relaxed) {
            complete = false;
            break;
        }

        let run_bytes = data_run
            .clusters
            .checked_mul(bytes_per_cluster)
            .ok_or_else(|| "NTFS MFT data run length overflowed".to_string())?
            .min(logical_remaining);
        let mut run_offset = 0u64;

        while run_offset < run_bytes {
            if cancel_flag.load(Ordering::Relaxed) {
                complete = false;
                break;
            }
            let read_len = usize::try_from((run_bytes - run_offset).min(buffer.len() as u64))
                .map_err(|_| "NTFS MFT chunk is too large".to_string())?;
            let target = &mut buffer[..read_len];

            if let Some(lcn) = data_run.lcn {
                let physical_offset = u64::try_from(lcn)
                    .ok()
                    .and_then(|cluster| cluster.checked_mul(bytes_per_cluster))
                    .and_then(|offset| offset.checked_add(run_offset))
                    .ok_or_else(|| "NTFS MFT physical offset is invalid".to_string())?;
                volume.read_exact_at(physical_offset, target)?;
            } else {
                target.fill(0);
            }

            process_mft_bytes(
                target,
                &mut carry,
                record_size,
                bytes_per_sector,
                bytes_per_cluster,
                &mut record_number,
                &mut entries,
                &mut extensions,
                &mut stats,
                progress,
            );
            run_offset += read_len as u64;
            logical_remaining -= read_len as u64;
        }

        if !complete {
            break;
        }
    }

    if logical_remaining != 0 && complete {
        return Err("NTFS MFT data runs do not cover the valid MFT length".into());
    }

    if complete && !carry.is_empty() {
        return Err("NTFS MFT ended with a partial file record".into());
    }

    Ok(StreamResult {
        entries,
        extensions,
        stats,
        complete,
    })
}

#[allow(clippy::too_many_arguments)]
fn process_mft_bytes(
    data: &mut [u8],
    carry: &mut Vec<u8>,
    record_size: usize,
    bytes_per_sector: usize,
    bytes_per_cluster: u64,
    record_number: &mut u64,
    entries: &mut Vec<MftEntry>,
    extensions: &mut Vec<MftEntry>,
    stats: &mut ParseStats,
    progress: &Arc<AtomicU64>,
) {
    let mut offset = 0usize;

    if !carry.is_empty() {
        let needed = record_size - carry.len();
        let copied = needed.min(data.len());
        carry.extend_from_slice(&data[..copied]);
        offset = copied;
        if carry.len() == record_size {
            process_record(
                carry.as_mut_slice(),
                record_size,
                bytes_per_sector,
                bytes_per_cluster,
                record_number,
                entries,
                extensions,
                stats,
                progress,
            );
            carry.clear();
        }
    }

    while offset + record_size <= data.len() {
        process_record(
            &mut data[offset..offset + record_size],
            record_size,
            bytes_per_sector,
            bytes_per_cluster,
            record_number,
            entries,
            extensions,
            stats,
            progress,
        );
        offset += record_size;
    }

    if offset < data.len() {
        carry.extend_from_slice(&data[offset..]);
    }
}

#[allow(clippy::too_many_arguments)]
fn process_record(
    record: &mut [u8],
    record_size: usize,
    bytes_per_sector: usize,
    bytes_per_cluster: u64,
    record_number: &mut u64,
    entries: &mut Vec<MftEntry>,
    extensions: &mut Vec<MftEntry>,
    stats: &mut ParseStats,
    progress: &Arc<AtomicU64>,
) {
    let current_record = *record_number;
    *record_number = record_number.saturating_add(1);

    match parse_record(
        record,
        current_record,
        record_size,
        bytes_per_sector,
        bytes_per_cluster,
    ) {
        Ok(Some(entry)) => {
            if entry.base_reference != 0 {
                extensions.push(entry);
            } else {
                if !entry.is_directory && !entry.links.is_empty() {
                    stats.files_scanned = stats.files_scanned.saturating_add(1);
                    progress.fetch_add(1, Ordering::Relaxed);
                }
                if entry.is_directory && !entry.links.is_empty() {
                    stats.directories_scanned = stats.directories_scanned.saturating_add(1);
                }
                if entry.is_reparse {
                    stats.reparse_points = stats.reparse_points.saturating_add(1);
                }
                entries.push(entry);
            }
        }
        Ok(None) => {}
        Err(error) => {
            stats.skipped_entries = stats.skipped_entries.saturating_add(1);
            stats.parse_errors = stats.parse_errors.saturating_add(1);
            if stats.parse_error_sample.is_none() {
                stats.parse_error_sample = Some(error);
            }
        }
    }
}

fn decode_mft_data_runs(record: &[u8], bytes_per_sector: usize) -> Result<Vec<DataRun>, String> {
    if record.get(0..4) != Some(&b"FILE"[..]) {
        return Err("NTFS MFT record 0 has an invalid signature".into());
    }

    let first_attribute = usize::from(read_u16(record, 20)?);
    let used_size = usize::try_from(read_u32(record, 24)?)
        .map_err(|_| "NTFS MFT record size is invalid".to_string())?;
    if first_attribute < 24 || used_size < first_attribute || used_size > record.len() {
        return Err("NTFS MFT record 0 has invalid attribute bounds".into());
    }

    let mut offset = first_attribute;
    while offset + 8 <= used_size {
        let attribute_type = read_u32(record, offset)?;
        if attribute_type == ATTRIBUTE_END {
            break;
        }
        let attribute_length = usize::try_from(read_u32(record, offset + 4)?)
            .map_err(|_| "NTFS MFT attribute length is invalid".to_string())?;
        if attribute_length < 24 || offset + attribute_length > used_size {
            return Err("NTFS MFT record 0 has invalid attribute bounds".into());
        }

        let nonresident = record[offset + 8] != 0;
        let name_length = record[offset + 9];
        if attribute_type == ATTRIBUTE_DATA && nonresident && name_length == 0 {
            let lowest_vcn = read_u64(record, offset + 16)?;
            if lowest_vcn != 0 {
                offset += attribute_length;
                continue;
            }
            let runlist_offset = usize::from(read_u16(record, offset + 32)?);
            let runlist_start = offset
                .checked_add(runlist_offset)
                .ok_or_else(|| "NTFS MFT runlist offset overflowed".to_string())?;
            if runlist_start >= offset + attribute_length {
                return Err("NTFS MFT data runlist is outside its attribute".into());
            }
            return decode_runlist(
                &record[runlist_start..offset + attribute_length],
                bytes_per_sector,
            );
        }
        offset += attribute_length;
    }

    Err("NTFS MFT record 0 has no unnamed data stream".into())
}

fn decode_runlist(data: &[u8], _bytes_per_sector: usize) -> Result<Vec<DataRun>, String> {
    let mut runs = Vec::new();
    let mut offset = 0usize;
    let mut current_lcn = 0i64;

    while offset < data.len() {
        let header = data[offset];
        offset += 1;
        if header == 0 {
            break;
        }

        let length_bytes = usize::from(header & 0x0f);
        let lcn_bytes = usize::from(header >> 4);
        if length_bytes == 0 || length_bytes > 8 || lcn_bytes > 8 {
            return Err("NTFS data run has an invalid field width".into());
        }
        let clusters = read_unsigned(data, offset, length_bytes)?;
        offset += length_bytes;
        let lcn = if lcn_bytes == 0 {
            None
        } else {
            let delta = read_signed(data, offset, lcn_bytes)?;
            offset += lcn_bytes;
            current_lcn = current_lcn
                .checked_add(delta)
                .ok_or_else(|| "NTFS data run LCN overflowed".to_string())?;
            if current_lcn < 0 {
                return Err("NTFS data run points before the volume".into());
            }
            Some(current_lcn)
        };

        if clusters == 0 {
            return Err("NTFS data run has zero length".into());
        }
        runs.push(DataRun { lcn, clusters });
    }

    if runs.is_empty() {
        return Err("NTFS data runlist is empty".into());
    }
    Ok(runs)
}

fn parse_record(
    record: &mut [u8],
    record_number: u64,
    record_size: usize,
    bytes_per_sector: usize,
    bytes_per_cluster: u64,
) -> Result<Option<MftEntry>, String> {
    if record.len() != record_size || record.get(0..4) != Some(&b"FILE"[..]) {
        return Ok(None);
    }
    let raw_flags = read_u16(record, 22)?;
    if raw_flags & FILE_RECORD_IN_USE == 0 {
        return Ok(None);
    }
    apply_update_sequence_fixups(record, bytes_per_sector)?;

    let flags = read_u16(record, 22)?;
    if flags & FILE_RECORD_IN_USE == 0 {
        return Ok(None);
    }
    let base_reference = read_u64(record, 32)?;

    let sequence = read_u16(record, 16)?;
    let first_attribute = usize::from(read_u16(record, 20)?);
    let used_size = usize::try_from(read_u32(record, 24)?)
        .map_err(|_| "NTFS file record size is invalid".to_string())?;
    if first_attribute < 24 || used_size < first_attribute || used_size > record.len() {
        return Err("NTFS file record has invalid attribute bounds".into());
    }

    let mut offset = first_attribute;
    let mut file_attributes = 0u32;
    let mut links = Vec::new();
    let mut streams: Vec<DiskStreamDetails> = Vec::new();
    let mut attribute_list_references = Vec::new();
    let mut reparse_tag = None;

    while offset + 8 <= used_size {
        let attribute_type = read_u32(record, offset)?;
        if attribute_type == ATTRIBUTE_END {
            break;
        }
        let attribute_length = usize::try_from(read_u32(record, offset + 4)?)
            .map_err(|_| "NTFS file attribute length is invalid".to_string())?;
        if attribute_length < 24 || offset + attribute_length > used_size {
            return Err("NTFS file attribute extends beyond its record".into());
        }

        let nonresident = record[offset + 8] != 0;
        let name_length = record[offset + 9];
        let attribute_name = if name_length == 0 {
            String::new()
        } else {
            let name_offset = usize::from(read_u16(record, offset + 10)?);
            read_utf16_name(record, offset + name_offset, usize::from(name_length))?
        };
        let value = if !nonresident {
            let value_length = usize::try_from(read_u32(record, offset + 16)?)
                .map_err(|_| "NTFS resident attribute length is invalid".to_string())?;
            let value_offset = usize::from(read_u16(record, offset + 20)?);
            let value_start = offset
                .checked_add(value_offset)
                .ok_or_else(|| "NTFS resident attribute offset overflowed".to_string())?;
            let value_end = value_start
                .checked_add(value_length)
                .ok_or_else(|| "NTFS resident attribute length overflowed".to_string())?;
            if value_start < offset || value_end > offset + attribute_length {
                return Err("NTFS resident attribute value is outside its attribute".into());
            }
            Some(&record[value_start..value_end])
        } else {
            None
        };

        match attribute_type {
            ATTRIBUTE_STANDARD_INFORMATION => {
                if let Some(value) = value {
                    if value.len() >= 36 {
                        file_attributes = read_u32(value, 32)?;
                    }
                }
            }
            ATTRIBUTE_LIST => {
                if let Some(value) = value {
                    parse_attribute_list(value, &mut attribute_list_references)?;
                }
            }
            ATTRIBUTE_FILE_NAME => {
                if let Some(value) = value {
                    if value.len() >= 66 {
                        let file_name_length = usize::from(value[64]);
                        let name_start = 66usize;
                        let name_end =
                            name_start
                                .checked_add(file_name_length.saturating_mul(2))
                                .ok_or_else(|| "NTFS file name length overflowed".to_string())?;
                        if name_end > value.len() {
                            return Err("NTFS file name extends beyond its attribute".into());
                        }
                        let name_units = (0..file_name_length)
                            .map(|index| read_u16(value, name_start + index * 2))
                            .collect::<Result<Vec<_>, _>>()?;
                        let name = String::from_utf16_lossy(&name_units);
                        let reference = read_u64(value, 0)?;
                        links.push(FileLink {
                            parent_record: reference & FILE_REFERENCE_MASK,
                            parent_sequence: (reference >> 48) as u16,
                            namespace: value[65],
                            size_bytes: if value.len() >= 56 {
                                read_u64(value, 48)?
                            } else {
                                0
                            },
                            name,
                        });
                        if value.len() >= 60 {
                            file_attributes |= read_u32(value, 56)?;
                        }
                    }
                }
            }
            ATTRIBUTE_DATA => {
                streams.push(parse_stream_attribute(
                    record,
                    offset,
                    attribute_length,
                    nonresident,
                    &attribute_name,
                    value,
                    bytes_per_cluster,
                )?);
            }
            ATTRIBUTE_REPARSE_POINT => {
                if let Some(value) = value {
                    if value.len() >= 4 {
                        reparse_tag = Some(read_u32(value, 0)?);
                    }
                }
            }
            _ => {}
        }

        offset += attribute_length;
    }

    let is_directory =
        flags & FILE_RECORD_DIRECTORY != 0 || file_attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
    let (parent_record, parent_sequence, name) = links
        .first()
        .map(|link| (link.parent_record, link.parent_sequence, link.name.clone()))
        .unwrap_or_default();
    // The top-level logical size is the ordinary file length: select the
    // unnamed DATA stream's full length across attribute-list segments, while
    // retaining named stream lengths in the per-entry stream details.
    let size_bytes = default_stream_logical_size_from_streams(&streams);
    let allocated_size_bytes = streams.iter().fold(0u64, |total, stream| {
        total.saturating_add(stream.allocated_size_bytes)
    });
    let link_count = links.len() as u32;
    Ok(Some(MftEntry {
        record_number,
        base_reference,
        sequence,
        parent_record,
        parent_sequence,
        name,
        links,
        size_bytes,
        allocated_size_bytes,
        streams,
        attribute_list_references,
        reparse_tag,
        link_count,
        is_directory,
        is_reparse: file_attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0,
    }))
}

fn default_stream_logical_size_from_streams(streams: &[DiskStreamDetails]) -> u64 {
    streams.iter().fold(0u64, |logical_size, stream| {
        if stream.name.is_empty() {
            logical_size.max(stream.logical_size_bytes)
        } else {
            logical_size
        }
    })
}

fn parse_stream_attribute(
    record: &[u8],
    offset: usize,
    attribute_length: usize,
    nonresident: bool,
    name: &str,
    value: Option<&[u8]>,
    bytes_per_cluster: u64,
) -> Result<DiskStreamDetails, String> {
    let attribute_flags = read_u16(record, offset + 12)?;
    if !nonresident {
        let logical_size = value.map(|value| value.len() as u64).unwrap_or(0);
        return Ok(DiskStreamDetails {
            name: name.to_string(),
            logical_size_bytes: logical_size,
            allocated_size_bytes: 0,
            valid_data_length_bytes: logical_size,
            resident: true,
            sparse: false,
            compressed: false,
            run_count: 0,
        });
    }
    if attribute_length < 64 || bytes_per_cluster == 0 {
        return Err("NTFS nonresident data attribute is too short".into());
    }
    let runlist_offset = usize::from(read_u16(record, offset + 32)?);
    let runlist_start = offset
        .checked_add(runlist_offset)
        .ok_or_else(|| "NTFS stream runlist offset overflowed".to_string())?;
    if runlist_start >= offset + attribute_length {
        return Err("NTFS stream runlist is outside its attribute".into());
    }
    let runlist = &record[runlist_start..offset + attribute_length];
    let runs = if runlist.first().copied() == Some(0) {
        Vec::new()
    } else {
        decode_runlist(runlist, 512)?
    };
    let allocated_size_bytes = runs.iter().fold(0u64, |total, run| {
        if run.lcn.is_some() {
            total.saturating_add(run.clusters.saturating_mul(bytes_per_cluster))
        } else {
            total
        }
    });
    Ok(DiskStreamDetails {
        name: name.to_string(),
        logical_size_bytes: read_u64(record, offset + 48)?,
        allocated_size_bytes,
        valid_data_length_bytes: read_u64(record, offset + 56)?,
        resident: false,
        sparse: attribute_flags & 0x8000 != 0,
        compressed: attribute_flags & 0x0001 != 0,
        run_count: runs.len() as u32,
    })
}

fn parse_attribute_list(value: &[u8], references: &mut Vec<u64>) -> Result<(), String> {
    let mut offset = 0usize;
    while offset + 26 <= value.len() {
        let attribute_type = read_u32(value, offset)?;
        if attribute_type == ATTRIBUTE_END {
            break;
        }
        let length = usize::from(read_u16(value, offset + 4)?);
        let end = offset
            .checked_add(length)
            .ok_or_else(|| "NTFS attribute-list entry length overflowed".to_string())?;
        if length < 26 || end > value.len() {
            return Err("NTFS attribute-list entry is outside its value".into());
        }
        let reference = read_u64(value, offset + 16)? & FILE_REFERENCE_MASK;
        if reference != 0 && !references.contains(&reference) {
            references.push(reference);
        }
        offset = end;
    }
    if offset != value.len() && offset + 26 > value.len() {
        return Err("NTFS attribute-list value has a truncated entry".into());
    }
    Ok(())
}

fn read_utf16_name(data: &[u8], offset: usize, length: usize) -> Result<String, String> {
    let byte_length = length
        .checked_mul(2)
        .ok_or_else(|| "NTFS name length overflowed".to_string())?;
    let bytes = data
        .get(
            offset
                ..offset
                    .checked_add(byte_length)
                    .ok_or_else(|| "NTFS name bounds overflowed".to_string())?,
        )
        .ok_or_else(|| "NTFS name extends beyond its attribute".to_string())?;
    let units = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    Ok(String::from_utf16_lossy(&units))
}

fn apply_update_sequence_fixups(record: &mut [u8], bytes_per_sector: usize) -> Result<(), String> {
    if bytes_per_sector < 2 || record.len() < 8 {
        return Err("NTFS file record is too small for update sequence fixups".into());
    }
    let update_offset = usize::from(read_u16(record, 4)?);
    let update_count = usize::from(read_u16(record, 6)?);
    let expected_count = record.len().div_ceil(bytes_per_sector) + 1;
    if update_count != expected_count
        || update_offset < 8
        || update_offset
            .checked_add(update_count * 2)
            .is_none_or(|end| end > record.len())
    {
        return Err("NTFS file record has invalid update sequence bounds".into());
    }

    let sequence = read_u16(record, update_offset)?;
    for index in 0..update_count.saturating_sub(1) {
        let sector_end = (index + 1)
            .checked_mul(bytes_per_sector)
            .ok_or_else(|| "NTFS sector boundary overflowed".to_string())?;
        if sector_end < 2 || sector_end > record.len() {
            return Err("NTFS file record sector boundary is invalid".into());
        }
        if read_u16(record, sector_end - 2)? != sequence {
            return Err("NTFS file record update sequence mismatch".into());
        }
        let replacement = read_u16(record, update_offset + 2 + index * 2)?;
        write_u16(record, sector_end - 2, replacement)?;
    }
    Ok(())
}

fn build_tree(
    drive: char,
    max_depth: u32,
    mut entries: Vec<MftEntry>,
    extensions: Vec<MftEntry>,
    mut stats: ParseStats,
    identity: SnapshotIdentity,
) -> Result<ScanOutput, String> {
    let mut by_record = HashMap::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        by_record.insert(entry.record_number, index);
    }
    for extension in extensions {
        let base_record = extension.base_reference & FILE_REFERENCE_MASK;
        if let Some(base_index) = by_record.get(&base_record).copied() {
            // The base record's ATTRIBUTE_LIST can itself be nonresident. In
            // that case the reference list is not available from the record
            // parser, but the extension's base reference is authoritative.
            merge_extension_into_base(&mut entries[base_index], extension);
        }
    }
    for entry in &mut entries {
        let has_non_dos_link = entry.links.iter().any(|link| link.namespace != 2);
        if !has_non_dos_link {
            let mut kept_directory_link = false;
            entry.links.retain(|link| {
                if link.namespace != 2 {
                    return true;
                }
                if kept_directory_link {
                    false
                } else {
                    kept_directory_link = true;
                    true
                }
            });
        } else {
            entry.links.retain(|link| link.namespace != 2);
        }
    }
    let mut linked_entries = Vec::new();
    let mut unnamed_entries = 0u64;
    for entry in entries {
        if entry.links.is_empty() {
            if entry.record_number == 5 {
                linked_entries.push(entry);
            } else if entry.record_number > NTFS_RESERVED_RECORD_MAX {
                unnamed_entries = unnamed_entries.saturating_add(1);
                if stats.unnamed_sample.len() < 8 {
                    stats.unnamed_sample.push(entry.record_number);
                }
            }
            continue;
        }

        let link_count = entry.links.len() as u32;
        let streams = entry.streams.clone();
        let allocated_size_bytes = entry.allocated_size_bytes;
        let attribute_list_references = entry.attribute_list_references.clone();
        let reparse_tag = entry.reparse_tag;
        for link in entry.links {
            linked_entries.push(MftEntry {
                record_number: entry.record_number,
                base_reference: 0,
                sequence: entry.sequence,
                parent_record: link.parent_record,
                parent_sequence: link.parent_sequence,
                name: link.name,
                links: Vec::new(),
                size_bytes: entry.size_bytes.max(link.size_bytes),
                allocated_size_bytes,
                streams: streams.clone(),
                attribute_list_references: attribute_list_references.clone(),
                reparse_tag,
                link_count,
                is_directory: entry.is_directory,
                is_reparse: entry.is_reparse,
            });
        }
    }
    stats.skipped_entries = stats.skipped_entries.saturating_add(unnamed_entries);
    stats.unnamed_entries = unnamed_entries;
    entries = linked_entries;
    stats.files_scanned = entries.iter().filter(|entry| !entry.is_directory).count() as u64;
    stats.directories_scanned = entries.iter().filter(|entry| entry.is_directory).count() as u64;
    stats.reparse_points = entries.iter().filter(|entry| entry.is_reparse).count() as u64;

    let root_index = entries
        .iter()
        .position(|entry| entry.record_number == 5 && entry.is_directory)
        .ok_or_else(|| "NTFS MFT root directory record was not found".to_string())?;
    let mut by_record = HashMap::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        by_record.insert(entry.record_number, index);
    }

    let mut parent_indices = vec![None; entries.len()];
    for index in 0..entries.len() {
        if index == root_index {
            continue;
        }
        let parent = by_record.get(&entries[index].parent_record).copied();
        let valid_parent = parent.filter(|parent_index| {
            *parent_index != index
                && entries[*parent_index].is_directory
                && (entries[index].parent_sequence == 0
                    || entries[*parent_index].sequence == entries[index].parent_sequence)
                && !would_create_cycle(index, *parent_index, &parent_indices)
        });

        if let Some(parent_index) = valid_parent {
            parent_indices[index] = Some(parent_index);
        } else {
            parent_indices[index] = Some(root_index);
            stats.skipped_entries = stats.skipped_entries.saturating_add(1);
            stats.orphaned_entries = stats.orphaned_entries.saturating_add(1);
        }
    }

    let mut children = vec![Vec::new(); entries.len()];
    for (index, parent) in parent_indices.iter().enumerate() {
        if let Some(parent_index) = parent {
            children[*parent_index].push(index);
        }
    }
    let aggregates = compute_aggregates(root_index, &children, &entries);
    let root_path = format!("{}:\\", drive.to_ascii_uppercase());
    let root_aggregate = aggregates[root_index];
    let mut root = TreeDiskNode {
        path: root_path.clone(),
        name: format!("{}:", drive.to_ascii_uppercase()),
        size_bytes: root_aggregate.size_bytes,
        allocated_size_bytes: root_aggregate.allocated_size_bytes,
        file_id: 5,
        file_count: root_aggregate.file_count,
        is_directory: true,
        is_truncated: false,
        flags: 0,
        detail: None,
        children: Vec::new(),
    };
    append_children(
        root_index,
        &root_path,
        1,
        max_depth,
        &children,
        &entries,
        &aggregates,
        &mut root,
        &mut stats,
    );

    let status =
        if stats.skipped_entries > 0 || root.is_truncated || stats.depth_limited_directories > 0 {
            ScanStatus::Partial
        } else {
            ScanStatus::Complete
        };
    let mut messages = Vec::new();
    if stats.parse_errors > 0 {
        messages.push(format!(
            "{} active MFT records could not be parsed (example: {})",
            stats.parse_errors,
            stats
                .parse_error_sample
                .as_deref()
                .unwrap_or("unknown error")
        ));
    }
    if stats.unnamed_entries > 0 {
        messages.push(format!(
            "{} active MFT records had no usable file name (records: {:?})",
            stats.unnamed_entries, stats.unnamed_sample
        ));
    }
    if stats.orphaned_entries > 0 {
        messages.push(format!(
            "{} MFT records had an invalid parent and were attached to the volume root",
            stats.orphaned_entries
        ));
    }
    let diagnostics = DiskDiagnostics {
        source: DiskScanSource::Mft,
        cache_source: DiskCacheSource::Fresh,
        reconciliation: super::disk_usage::DiskReconciliation::Unavailable,
        elapsed_ms: 0,
        partial_scan: status != ScanStatus::Complete,
        access_denied: false,
        skipped_records: stats.skipped_entries,
        reparse_points: stats.reparse_points,
        messages,
    };
    let metadata = SnapshotMetadata {
        root: root_path,
        engine: ScanEngine::NtfsMft,
        identity,
        status,
        complete: status == ScanStatus::Complete,
    };
    Ok(output_from_tree(
        root,
        status,
        ScanEngine::NtfsMft,
        DiskScanSource::Mft,
        diagnostics,
        metadata,
    ))
}

fn would_create_cycle(
    child_index: usize,
    parent_index: usize,
    parent_indices: &[Option<usize>],
) -> bool {
    let mut current = Some(parent_index);
    while let Some(index) = current {
        if index == child_index {
            return true;
        }
        current = parent_indices[index];
    }
    false
}

fn compute_aggregates(
    root_index: usize,
    children: &[Vec<usize>],
    entries: &[MftEntry],
) -> Vec<Aggregate> {
    let mut order = Vec::with_capacity(entries.len());
    let mut stack = vec![root_index];
    while let Some(index) = stack.pop() {
        order.push(index);
        stack.extend(children[index].iter().copied());
    }

    let mut aggregates = vec![Aggregate::default(); entries.len()];
    for index in order.into_iter().rev() {
        let mut aggregate = if entries[index].is_directory {
            Aggregate::default()
        } else {
            Aggregate {
                size_bytes: entries[index].size_bytes,
                allocated_size_bytes: entries[index].allocated_size_bytes,
                file_count: 1,
            }
        };
        for child in &children[index] {
            aggregate.size_bytes = aggregate
                .size_bytes
                .saturating_add(aggregates[*child].size_bytes);
            aggregate.allocated_size_bytes = aggregate
                .allocated_size_bytes
                .saturating_add(aggregates[*child].allocated_size_bytes);
            aggregate.file_count = aggregate
                .file_count
                .saturating_add(aggregates[*child].file_count);
        }
        aggregates[index] = aggregate;
    }
    aggregates
}

#[allow(clippy::too_many_arguments)]
fn append_children(
    index: usize,
    parent_path: &str,
    depth: u32,
    max_depth: u32,
    children: &[Vec<usize>],
    entries: &[MftEntry],
    aggregates: &[Aggregate],
    node: &mut TreeDiskNode,
    stats: &mut ParseStats,
) {
    let mut child_indices = children[index].clone();
    child_indices.sort_by(|left, right| {
        aggregates[*right]
            .allocated_size_bytes
            .cmp(&aggregates[*left].allocated_size_bytes)
            .then_with(|| {
                aggregates[*right]
                    .size_bytes
                    .cmp(&aggregates[*left].size_bytes)
            })
            .then_with(|| entries[*left].name.cmp(&entries[*right].name))
    });

    if depth >= max_depth {
        for child_index in child_indices {
            let child = &entries[child_index];
            if child.is_directory {
                stats.depth_limited_directories = stats.depth_limited_directories.saturating_add(1);
            }
            node.children.push(summary_node(
                child_index,
                parent_path,
                entries,
                aggregates,
                child.is_directory,
            ));
        }
        node.is_truncated = !node.children.is_empty();
        return;
    }

    for child_index in child_indices {
        let child = &entries[child_index];
        let child_path = join_path(parent_path, &child.name);
        let aggregate = aggregates[child_index];
        let mut child_node = TreeDiskNode {
            path: child_path.clone(),
            name: child.name.clone(),
            size_bytes: aggregate.size_bytes,
            allocated_size_bytes: aggregate.allocated_size_bytes,
            file_id: child.record_number,
            file_count: aggregate.file_count,
            is_directory: child.is_directory,
            is_truncated: false,
            flags: entry_flags(child),
            detail: Some(entry_detail(child)),
            children: Vec::new(),
        };
        if child.is_directory {
            append_children(
                child_index,
                &child_path,
                depth + 1,
                max_depth,
                children,
                entries,
                aggregates,
                &mut child_node,
                stats,
            );
        }
        node.children.push(child_node);
    }
}

fn summary_node(
    index: usize,
    parent_path: &str,
    entries: &[MftEntry],
    aggregates: &[Aggregate],
    truncated: bool,
) -> TreeDiskNode {
    let entry = &entries[index];
    let aggregate = aggregates[index];
    TreeDiskNode {
        path: join_path(parent_path, &entry.name),
        name: entry.name.clone(),
        size_bytes: aggregate.size_bytes,
        allocated_size_bytes: aggregate.allocated_size_bytes,
        file_id: entry.record_number,
        file_count: aggregate.file_count,
        is_directory: entry.is_directory,
        is_truncated: truncated,
        flags: entry_flags(entry),
        detail: Some(entry_detail(entry)),
        children: Vec::new(),
    }
}

fn join_path(parent: &str, name: &str) -> String {
    format!("{}\\{}", parent.trim_end_matches('\\'), name)
}

fn entry_flags(entry: &MftEntry) -> u32 {
    let mut flags = if entry.is_reparse {
        NODE_FLAG_REPARSE
    } else {
        0
    };
    if entry.streams.iter().any(|stream| stream.sparse) {
        flags |= NODE_FLAG_SPARSE;
    }
    if entry.streams.iter().any(|stream| stream.compressed) {
        flags |= NODE_FLAG_COMPRESSED;
    }
    flags
}

fn entry_detail(entry: &MftEntry) -> TreeEntryDetails {
    let file_reference = entry.record_number | (u64::from(entry.sequence) << 48);
    TreeEntryDetails {
        mft_record_number: Some(entry.record_number),
        sequence: Some(entry.sequence),
        file_reference: Some(file_reference),
        parent_reference: if entry.parent_record == 0 {
            None
        } else {
            Some(entry.parent_record | (u64::from(entry.parent_sequence) << 48))
        },
        reparse_tag: entry.reparse_tag,
        link_count: entry.link_count,
        streams: entry.streams.clone(),
    }
}

fn read_u16(data: &[u8], offset: usize) -> Result<u16, String> {
    let bytes = data
        .get(offset..offset + 2)
        .ok_or_else(|| "NTFS metadata ended before a u16 field".to_string())?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(data: &[u8], offset: usize) -> Result<u32, String> {
    let bytes = data
        .get(offset..offset + 4)
        .ok_or_else(|| "NTFS metadata ended before a u32 field".to_string())?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64, String> {
    let bytes = data
        .get(offset..offset + 8)
        .ok_or_else(|| "NTFS metadata ended before a u64 field".to_string())?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn read_unsigned(data: &[u8], offset: usize, length: usize) -> Result<u64, String> {
    if length == 0 || length > 8 {
        return Err("NTFS runlist integer width is invalid".into());
    }
    let bytes = data
        .get(offset..offset + length)
        .ok_or_else(|| "NTFS runlist ended before its integer".to_string())?;
    let mut value = 0u64;
    for (index, byte) in bytes.iter().enumerate() {
        value |= u64::from(*byte) << (index * 8);
    }
    Ok(value)
}

fn read_signed(data: &[u8], offset: usize, length: usize) -> Result<i64, String> {
    let raw = read_unsigned(data, offset, length)?;
    if length == 8 {
        return Ok(raw as i64);
    }
    let sign_bit = 1u64 << (length * 8 - 1);
    let extended = if raw & sign_bit != 0 {
        raw | (!0u64 << (length * 8))
    } else {
        raw
    };
    Ok(extended as i64)
}

fn write_u16(data: &mut [u8], offset: usize, value: u16) -> Result<(), String> {
    let target = data
        .get_mut(offset..offset + 2)
        .ok_or_else(|| "NTFS metadata ended before a replacement field".to_string())?;
    target.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn counts_only_requested_bitmap_bits() {
        assert_eq!(count_set_bits(&[0xff, 0xff], 3, 10), 10);
        assert_eq!(count_set_bits(&[0xff, 0x01], 4, 4), 4);
        assert_eq!(count_set_bits(&[0xff, 0x01], 8, 8), 1);
    }

    #[test]
    fn decodes_relative_and_sparse_data_runs() {
        let runs = decode_runlist(&[0x11, 0x03, 0x05, 0x01, 0x02, 0x00], 512).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].clusters, 3);
        assert_eq!(runs[0].lcn, Some(5));
        assert_eq!(runs[1].clusters, 2);
        assert_eq!(runs[1].lcn, None);
    }

    #[test]
    fn applies_update_sequence_fixups_without_writing_the_source() {
        let mut record = vec![0u8; 1024];
        record[0..4].copy_from_slice(b"FILE");
        record[4..6].copy_from_slice(&48u16.to_le_bytes());
        record[6..8].copy_from_slice(&3u16.to_le_bytes());
        record[48..50].copy_from_slice(&0x1234u16.to_le_bytes());
        record[50..52].copy_from_slice(&0xaaaa_u16.to_le_bytes());
        record[52..54].copy_from_slice(&0xbbbb_u16.to_le_bytes());
        record[510..512].copy_from_slice(&0x1234u16.to_le_bytes());
        record[1022..1024].copy_from_slice(&0x1234u16.to_le_bytes());

        apply_update_sequence_fixups(&mut record, 512).unwrap();

        assert_eq!(read_u16(&record, 510).unwrap(), 0xaaaa);
        assert_eq!(read_u16(&record, 1022).unwrap(), 0xbbbb);
    }

    #[test]
    fn ignores_inactive_records_before_fixup_validation() {
        let mut record = vec![0u8; 1024];
        record[0..4].copy_from_slice(b"FILE");

        assert!(parse_record(&mut record, 17, 1024, 512, 4096)
            .unwrap()
            .is_none());
    }

    #[test]
    fn missing_changed_mft_records_require_a_delete_reason() {
        assert!(missing_mft_record_is_safe_to_remove(USN_REASON_FILE_DELETE));
        assert!(missing_mft_record_is_safe_to_remove(
            USN_REASON_FILE_DELETE | 1
        ));
        assert!(!missing_mft_record_is_safe_to_remove(1));
    }

    #[test]
    fn parses_sparse_stream_allocation_and_run_characteristics() {
        let mut attribute = vec![0u8; 80];
        attribute[12..14].copy_from_slice(&0x8000u16.to_le_bytes());
        attribute[32..34].copy_from_slice(&64u16.to_le_bytes());
        attribute[48..56].copy_from_slice(&100u64.to_le_bytes());
        attribute[56..64].copy_from_slice(&80u64.to_le_bytes());
        attribute[64] = 0x11;
        attribute[65] = 0x02;
        attribute[66] = 0x05;
        attribute[67] = 0;
        let stream = parse_stream_attribute(
            &attribute,
            0,
            attribute.len(),
            true,
            "Zone.Identifier",
            None,
            4096,
        )
        .unwrap();
        assert_eq!(stream.logical_size_bytes, 100);
        assert_eq!(stream.allocated_size_bytes, 8192);
        assert!(stream.sparse);
        assert_eq!(stream.run_count, 1);
    }

    #[test]
    fn logical_size_uses_the_default_stream_and_retains_ads_details() {
        let streams = vec![
            DiskStreamDetails {
                name: String::new(),
                logical_size_bytes: 100,
                allocated_size_bytes: 4096,
                valid_data_length_bytes: 100,
                resident: false,
                sparse: false,
                compressed: false,
                run_count: 1,
            },
            DiskStreamDetails {
                name: String::new(),
                logical_size_bytes: 95,
                allocated_size_bytes: 4096,
                valid_data_length_bytes: 95,
                resident: false,
                sparse: false,
                compressed: false,
                run_count: 1,
            },
            DiskStreamDetails {
                name: "Zone.Identifier".into(),
                logical_size_bytes: 7,
                allocated_size_bytes: 4096,
                valid_data_length_bytes: 7,
                resident: true,
                sparse: false,
                compressed: false,
                run_count: 0,
            },
        ];

        assert_eq!(default_stream_logical_size_from_streams(&streams), 100);
        assert_eq!(streams[2].logical_size_bytes, 7);
    }

    #[test]
    fn resident_data_has_logical_bytes_but_no_separate_cluster_allocation() {
        let record = vec![0u8; 16];
        let stream = parse_stream_attribute(
            &record,
            0,
            record.len(),
            false,
            "Zone.Identifier",
            Some(&[1, 2, 3]),
            4096,
        )
        .unwrap();

        assert_eq!(stream.logical_size_bytes, 3);
        assert_eq!(stream.allocated_size_bytes, 0);
        assert!(stream.resident);
    }

    #[test]
    fn accepts_an_empty_nonresident_data_runlist() {
        let mut attribute = vec![0u8; 80];
        attribute[32..34].copy_from_slice(&64u16.to_le_bytes());
        attribute[64] = 0;

        let stream =
            parse_stream_attribute(&attribute, 0, attribute.len(), true, "", None, 4096).unwrap();
        assert_eq!(stream.allocated_size_bytes, 0);
        assert_eq!(stream.run_count, 0);
    }

    #[test]
    fn merges_extension_links_when_base_attribute_list_is_not_resident() {
        let root = MftEntry {
            record_number: 5,
            base_reference: 0,
            sequence: 1,
            parent_record: 5,
            parent_sequence: 1,
            name: "C:".into(),
            links: vec![FileLink {
                parent_record: 5,
                parent_sequence: 1,
                namespace: 1,
                size_bytes: 0,
                name: "C:".into(),
            }],
            size_bytes: 0,
            allocated_size_bytes: 0,
            streams: Vec::new(),
            attribute_list_references: Vec::new(),
            reparse_tag: None,
            link_count: 1,
            is_directory: true,
            is_reparse: false,
        };
        let base = MftEntry {
            record_number: 100,
            base_reference: 0,
            sequence: 3,
            parent_record: 0,
            parent_sequence: 0,
            name: String::new(),
            links: Vec::new(),
            size_bytes: 0,
            allocated_size_bytes: 0,
            streams: Vec::new(),
            attribute_list_references: Vec::new(),
            reparse_tag: None,
            link_count: 0,
            is_directory: false,
            is_reparse: false,
        };
        let extension = MftEntry {
            record_number: 101,
            base_reference: 100 | (3u64 << 48),
            sequence: 1,
            parent_record: 5,
            parent_sequence: 1,
            name: "large.bin".into(),
            links: vec![FileLink {
                parent_record: 5,
                parent_sequence: 1,
                namespace: 1,
                size_bytes: 9_000_000_000,
                name: "large.bin".into(),
            }],
            size_bytes: 9_000_000_000,
            allocated_size_bytes: 8_000_000_000,
            streams: Vec::new(),
            attribute_list_references: Vec::new(),
            reparse_tag: None,
            link_count: 1,
            is_directory: false,
            is_reparse: false,
        };

        let output = build_tree(
            'C',
            u32::MAX,
            vec![root, base],
            vec![extension],
            ParseStats::default(),
            SnapshotIdentity::default(),
        )
        .unwrap();

        assert_eq!(output.result.status, ScanStatus::Complete);
        assert_eq!(output.result.summary.file_count, 1);
        assert_eq!(output.result.summary.logical_size_bytes, 9_000_000_000);
        assert_eq!(output.snapshot.nodes[1].name, "large.bin");
    }

    #[test]
    fn parses_attribute_list_file_references_and_rejects_truncation() {
        let mut list = vec![0u8; 26];
        list[0..4].copy_from_slice(&ATTRIBUTE_DATA.to_le_bytes());
        list[4..6].copy_from_slice(&26u16.to_le_bytes());
        list[16..24].copy_from_slice(&((42u64) | (7u64 << 48)).to_le_bytes());
        let mut references = Vec::new();
        parse_attribute_list(&list, &mut references).unwrap();
        assert_eq!(references, vec![42]);
        assert!(parse_attribute_list(&list[..25], &mut Vec::new()).is_err());
    }

    #[test]
    #[ignore = "manual benchmark for a local NTFS volume"]
    fn benchmarks_c_volume_mft_scan() {
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicU64::new(0));
        let started = Instant::now();
        let result = scan_volume('C', 3, &cancel, &progress, true)
            .unwrap()
            .result;
        eprintln!(
            "NTFS MFT benchmark: {:?}, {} files (tree {}), {} directories, {} bytes, engine={:?}",
            started.elapsed(),
            result.summary.file_count,
            result.root.file_count,
            result.summary.directory_count,
            result.summary.logical_size_bytes,
            result.engine
        );
        eprintln!(
            "NTFS MFT diagnostics: skipped={}, reparse={}, depth_limited={}",
            result.diagnostics.skipped_records,
            result.diagnostics.reparse_points,
            result.diagnostics.partial_scan
        );
        eprintln!(
            "NTFS MFT source={:?}, cache={:?}, messages={:?}",
            result.diagnostics.source, result.diagnostics.cache_source, result.diagnostics.messages
        );
        assert_eq!(result.engine, ScanEngine::NtfsMft);
    }
}
