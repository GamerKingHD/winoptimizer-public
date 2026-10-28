//! Batched Windows directory fallback for non-volume and non-elevated scans.
//!
//! `FileIdBothDirectoryInfo` supplies names, file IDs, logical sizes,
//! allocation sizes, and attributes in one call.  This avoids the old
//! per-file metadata round trip while preserving the no-follow-reparse rule.

use super::disk_usage::{
    should_skip_directory, DiskCacheSource, DiskDiagnostics, DiskReconciliation, DiskScanSource,
    ScanStatus, TreeDiskNode, NODE_FLAG_COMPRESSED, NODE_FLAG_REPARSE, NODE_FLAG_SPARSE,
};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use windows::core::HSTRING;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo,
    GetFileInformationByHandleEx, FILE_ATTRIBUTE_COMPRESSED, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_SPARSE_FILE, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_GENERIC_READ, FILE_ID_BOTH_DIR_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
const DIRECTORY_BUFFER_SIZE: usize = 128 * 1024;

#[derive(Debug, Default)]
struct Counters {
    files: AtomicU64,
    directories: AtomicU64,
    skipped: AtomicU64,
    inaccessible: AtomicU64,
    reparse: AtomicU64,
    depth_limited: AtomicU64,
}

#[derive(Debug)]
pub(crate) struct DirectoryEntry {
    pub(crate) name: String,
    pub(crate) file_id: u64,
    pub(crate) logical_size: u64,
    pub(crate) allocated_size: u64,
    pub(crate) attributes: u32,
}
pub(crate) fn scan(
    root: &Path,
    max_depth: u32,
    cancel_flag: &Arc<AtomicBool>,
    progress: &Arc<AtomicU64>,
) -> (TreeDiskNode, ScanStatus, DiskDiagnostics) {
    let counters = Arc::new(Counters::default());
    let root_name = root
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| root.to_string_lossy().to_string());
    let tree = scan_directory(
        root.to_path_buf(),
        root_name,
        1,
        max_depth,
        cancel_flag,
        progress,
        &counters,
    );
    let cancelled = cancel_flag.load(Ordering::Relaxed);
    let partial = cancelled
        || counters.skipped.load(Ordering::Relaxed) > 0
        || counters.inaccessible.load(Ordering::Relaxed) > 0
        || counters.reparse.load(Ordering::Relaxed) > 0
        || counters.depth_limited.load(Ordering::Relaxed) > 0;
    let status = if cancelled {
        ScanStatus::Cancelled
    } else if partial {
        ScanStatus::Partial
    } else {
        ScanStatus::Complete
    };
    let inaccessible = counters.inaccessible.load(Ordering::Relaxed);
    let mut messages = Vec::new();
    if inaccessible > 0 {
        messages.push(format!(
            "{} directory batches could not be read",
            inaccessible
        ));
    }
    if counters.depth_limited.load(Ordering::Relaxed) > 0 {
        messages.push("The selected depth left some directories unexplored".into());
    }
    let diagnostics = DiskDiagnostics {
        source: DiskScanSource::Win32,
        cache_source: DiskCacheSource::Fresh,
        reconciliation: DiskReconciliation::Unavailable,
        elapsed_ms: 0,
        partial_scan: partial,
        access_denied: inaccessible > 0,
        skipped_records: counters
            .skipped
            .load(Ordering::Relaxed)
            .saturating_add(counters.depth_limited.load(Ordering::Relaxed)),
        reparse_points: counters.reparse.load(Ordering::Relaxed),
        messages,
    };
    (tree, status, diagnostics)
}

fn scan_directory(
    path: PathBuf,
    name: String,
    depth: u32,
    max_depth: u32,
    cancel_flag: &Arc<AtomicBool>,
    progress: &Arc<AtomicU64>,
    counters: &Arc<Counters>,
) -> TreeDiskNode {
    counters.directories.fetch_add(1, Ordering::Relaxed);
    let mut node = TreeDiskNode {
        path: path.to_string_lossy().to_string(),
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
    if cancel_flag.load(Ordering::Relaxed) {
        return node;
    }

    let entries = match enumerate_directory(&path) {
        Ok(entries) => entries,
        Err(_) => {
            counters.inaccessible.fetch_add(1, Ordering::Relaxed);
            return node;
        }
    };
    for entry in entries {
        if cancel_flag.load(Ordering::Relaxed) {
            node.is_truncated = true;
            break;
        }
        if entry.name == "." || entry.name == ".." {
            continue;
        }
        let is_directory = entry.attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0;
        let is_reparse = entry.attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0;
        if is_directory && should_skip_directory(&entry.name) {
            counters.skipped.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        if is_reparse {
            counters.reparse.fetch_add(1, Ordering::Relaxed);
        }

        let flags = if is_reparse { NODE_FLAG_REPARSE } else { 0 }
            | if entry.attributes & FILE_ATTRIBUTE_SPARSE_FILE.0 != 0 {
                NODE_FLAG_SPARSE
            } else {
                0
            }
            | if entry.attributes & FILE_ATTRIBUTE_COMPRESSED.0 != 0 {
                NODE_FLAG_COMPRESSED
            } else {
                0
            };
        let child_path = path.join(&entry.name);
        let mut child = TreeDiskNode {
            path: child_path.to_string_lossy().to_string(),
            name: entry.name,
            size_bytes: entry.logical_size,
            allocated_size_bytes: entry.allocated_size,
            file_id: entry.file_id,
            file_count: if is_directory { 0 } else { 1 },
            is_directory,
            is_truncated: false,
            flags,
            detail: None,
            children: Vec::new(),
        };

        if is_directory {
            if is_reparse || depth >= max_depth {
                if depth >= max_depth && !is_reparse {
                    counters.depth_limited.fetch_add(1, Ordering::Relaxed);
                    child.is_truncated = true;
                }
            } else {
                child = scan_directory(
                    child_path,
                    child.name.clone(),
                    depth + 1,
                    max_depth,
                    cancel_flag,
                    progress,
                    counters,
                );
            }
        } else {
            counters.files.fetch_add(1, Ordering::Relaxed);
            progress.fetch_add(1, Ordering::Relaxed);
        }
        node.children.push(child);
    }

    node.children.sort_by(|left, right| {
        right
            .allocated_size_bytes
            .cmp(&left.allocated_size_bytes)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    for child in &node.children {
        node.size_bytes = node.size_bytes.saturating_add(child.size_bytes);
        node.allocated_size_bytes = node
            .allocated_size_bytes
            .saturating_add(child.allocated_size_bytes);
        node.file_count = node.file_count.saturating_add(child.file_count);
    }
    node
}

fn enumerate_directory(path: &Path) -> Result<Vec<DirectoryEntry>, String> {
    let handle = open_directory(path)?;
    let result = enumerate_with_handle(handle);
    unsafe {
        let _ = CloseHandle(handle);
    }
    result
}
fn open_directory(path: &Path) -> Result<windows::Win32::Foundation::HANDLE, String> {
    use std::os::windows::ffi::OsStrExt;

    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    unsafe {
        CreateFileW(
            &HSTRING::from_wide(&wide[..wide.len() - 1]),
            FILE_GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }
    .map_err(|error| format!("unable to open directory {}: {}", path.display(), error))
}

fn enumerate_with_handle(
    handle: windows::Win32::Foundation::HANDLE,
) -> Result<Vec<DirectoryEntry>, String> {
    let mut buffer = vec![0u8; DIRECTORY_BUFFER_SIZE];
    let mut entries = Vec::new();
    let mut restart = true;
    loop {
        let information_class = if restart {
            FileIdBothDirectoryRestartInfo
        } else {
            FileIdBothDirectoryInfo
        };
        let result = unsafe {
            GetFileInformationByHandleEx(
                handle,
                information_class,
                buffer.as_mut_ptr() as *mut c_void,
                u32::try_from(buffer.len()).map_err(|_| "directory buffer is too large")?,
            )
        };
        if let Err(error) = result {
            if !entries.is_empty() {
                break;
            }
            return Err(format!("directory batch enumeration failed: {}", error));
        }
        let mut offset = 0usize;
        let mut batch_count = 0usize;
        loop {
            if offset + std::mem::size_of::<FILE_ID_BOTH_DIR_INFO>() > buffer.len() {
                return Err("directory batch record header is truncated".into());
            }
            let record = unsafe {
                std::ptr::read_unaligned(buffer.as_ptr().add(offset) as *const FILE_ID_BOTH_DIR_INFO)
            };
            let name_length = usize::try_from(record.FileNameLength)
                .map_err(|_| "directory entry name length is invalid")?;
            if name_length % 2 != 0 {
                return Err("directory entry name is not UTF-16 aligned".into());
            }
            let name_offset = std::mem::offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
            let name_end = offset
                .checked_add(name_offset)
                .and_then(|start| start.checked_add(name_length))
                .ok_or_else(|| "directory entry name length overflowed".to_string())?;
            if name_end > buffer.len() {
                return Err("directory entry name is outside its batch".into());
            }
            let name = unsafe {
                let name_ptr = buffer.as_ptr().add(offset + name_offset) as *const u16;
                let name_units = std::slice::from_raw_parts(name_ptr, name_length / 2);
                String::from_utf16_lossy(name_units)
            };
            entries.push(DirectoryEntry {
                name,
                file_id: record.FileId as u64,
                logical_size: u64::try_from(record.EndOfFile.max(0)).unwrap_or(0),
                allocated_size: u64::try_from(record.AllocationSize.max(0)).unwrap_or(0),
                attributes: record.FileAttributes,
            });
            batch_count += 1;
            if record.NextEntryOffset == 0 {
                break;
            }
            let next = usize::try_from(record.NextEntryOffset)
                .map_err(|_| "directory batch offset is invalid")?;
            offset = offset
                .checked_add(next)
                .ok_or_else(|| "directory batch offset overflowed".to_string())?;
            if offset >= buffer.len() {
                return Err("directory batch next offset is outside its buffer".into());
            }
        }
        if batch_count == 0 {
            break;
        }
        restart = false;
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicU64};
    use std::sync::Arc;
    #[test]
    fn directory_batch_buffer_is_bounded() {
        assert_eq!(super::DIRECTORY_BUFFER_SIZE, 128 * 1024);
    }

    #[test]
    fn batches_report_file_ids_and_allocation_sizes() {
        let directory = tempfile::tempdir().unwrap();
        let file_path = directory.path().join("fixture.bin");
        std::fs::write(&file_path, b"fixture").unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicU64::new(0));
        let (tree, status, diagnostics) = super::scan(directory.path(), 2, &cancel, &progress);
        assert_eq!(status, super::ScanStatus::Complete);
        assert!(!diagnostics.access_denied);
        let file = tree
            .children
            .iter()
            .find(|child| child.name == "fixture.bin")
            .unwrap();
        assert!(file.file_id != 0);
        assert!(file.size_bytes >= 7);
        assert!(file.allocated_size_bytes >= file.size_bytes);
    }
}
