//! High-speed NTFS volume indexing via USN (Update Sequence Number) journal.
//!
//! Instead of recursive directory crawling (FindFirstFile/FindNextFile -
//! ~500-1500 files/sec), this module reads the NTFS USN journal sequentially
//! from the raw volume, yielding 20,000+ files/sec.
//!
//! The trade-off: USN records contain file name + parent FRN, but not full
//! paths. We resolve paths in-memory using a FRN->name hash table, which
//! adds a small post-processing cost that is still orders of magnitude
//! faster than recursive crawling for large volumes.
//!
//! Requires: Administrator + SeManageVolumePrivilege.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_GENERIC_READ, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows::Win32::System::IO::DeviceIoControl;

// -- NTFS control codes ---------------------------------------

/// FSCTL_QUERY_USN_JOURNAL - retrieve the active USN journal bounds.
const FSCTL_QUERY_USN_JOURNAL: u32 = 0x000900F4;

/// FSCTL_ENUM_USN_DATA - bulk-enumerate USN records between two USN values.
const FSCTL_ENUM_USN_DATA: u32 = 0x000900B3;
const FSCTL_READ_USN_JOURNAL: u32 = 0x000900BB;

// -- NTFS structs (not in windows-rs 0.60) --------------------

/// Maps to winioctl.h USN_JOURNAL_DATA.
#[repr(C)]
struct UsnJournalData {
    usn_journal_id: u64,
    first_usn: i64,
    next_usn: i64,
    lowest_valid_usn: i64,
    max_usn: i64,
    maximum_size: u64,
    allocation_delta: u64,
}

/// Maps to winioctl.h MFT_ENUM_DATA.
#[repr(C)]
struct MftEnumData {
    start_file_reference_number: u64,
    low_usn: i64,
    high_usn: i64,
}

/// Maps to winioctl.h USN_RECORD_V2 (header portion - variable-length).
/// The file name follows at offset FileNameOffset.
#[repr(C)]
struct UsnRecordV2Header {
    record_length: u32,
    major_version: u16,
    minor_version: u16,
    file_reference_number: u64,
    parent_file_reference_number: u64,
    usn: i64,
    timestamp: i64,
    reason: u32,
    source_info: u32,
    security_id: u32,
    file_attributes: u32,
    file_name_length: u16,
    file_name_offset: u16,
    // WCHAR FileName[1] follows here
}

// -- Public types ---------------------------------------------

/// A single file record discovered via USN enumeration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsnFileEntry {
    /// Full resolved path (e.g. "C:\\Users\\...\\file.txt").
    pub full_path: String,
    /// File name only.
    pub name: String,
    /// File size in bytes. Always 0 for USN-only enumeration
    /// (size must be queried separately).
    pub size_bytes: u64,
    /// Parent directory FRN.
    pub parent_frn: u64,
    /// File attributes mask.
    pub attributes: u32,
    /// True if this is a directory.
    pub is_directory: bool,
    /// True if the path could not be fully resolved.
    pub path_unresolved: bool,
}

/// Statistics about a USN enumeration run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsnScanStats {
    pub drive: String,
    pub total_records: u64,
    pub resolved_paths: u64,
    pub unresolved_paths: u64,
    pub directories: u64,
    pub files: u64,
    pub elapsed_ms: u64,
}

/// A journal change.  It deliberately contains no size field: Microsoft’s
/// USN record format does not provide one, so callers must re-read the MFT or
/// directory entry before changing accounting totals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsnChange {
    pub file_reference: u64,
    pub parent_file_reference: u64,
    pub usn: i64,
    pub reason: u32,
    pub name: String,
    pub attributes: u32,
    pub is_directory: bool,
}

// -- Volume handle --------------------------------------------

/// Open a volume for raw USN journal access.
fn open_volume(drive_letter: char) -> Result<HANDLE, String> {
    let volume_path = format!(r"\\.\{}:", drive_letter);
    let handle = unsafe {
        CreateFileW(
            &HSTRING::from(&volume_path),
            FILE_GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
        .map_err(|e| format!("CreateFileW failed for {}: {:?}", volume_path, e))?
    };

    if handle == INVALID_HANDLE_VALUE {
        return Err(format!("Invalid handle for volume {}", volume_path));
    }

    Ok(handle)
}

/// Query the USN journal bounds for a volume.
fn query_usn_journal(handle: HANDLE) -> Result<UsnJournalData, String> {
    let mut ujd: UsnJournalData = unsafe { std::mem::zeroed() };
    let mut bytes_returned: u32 = 0;

    let result = unsafe {
        DeviceIoControl(
            handle,
            FSCTL_QUERY_USN_JOURNAL,
            None,
            0,
            Some(&mut ujd as *mut UsnJournalData as *mut std::ffi::c_void),
            std::mem::size_of::<UsnJournalData>() as u32,
            Some(&mut bytes_returned),
            None,
        )
    };

    if result.is_err() {
        return Err(format!("FSCTL_QUERY_USN_JOURNAL failed: {:?}", result));
    }

    Ok(ujd)
}

/// Return the journal identity and current upper bound without enumerating or
/// modifying the journal.  A caller can use this to validate a disk snapshot.
pub fn journal_state(drive_letter: char) -> Result<(u64, i64), String> {
    let handle = open_volume(drive_letter)?;
    let result =
        query_usn_journal(handle).map(|journal| (journal.usn_journal_id, journal.next_usn));
    unsafe {
        let _ = CloseHandle(handle);
    }
    result
}

#[repr(C)]
struct ReadUsnJournalData {
    start_usn: i64,
    reason_mask: u32,
    return_only_on_close: u32,
    timeout: u64,
    bytes_to_wait_for: u64,
    journal_id: u64,
    min_major_version: u16,
    max_major_version: u16,
}

fn validate_journal_continuity(
    journal: &UsnJournalData,
    expected_journal_id: Option<u64>,
    first_usn: i64,
    last_usn: i64,
) -> Result<(), String> {
    if first_usn < 0 || last_usn < 0 {
        return Err("USN journal boundaries cannot be negative".into());
    }
    if journal.first_usn < 0
        || journal.lowest_valid_usn < 0
        || journal.next_usn < 0
        || journal.first_usn > journal.next_usn
        || journal.lowest_valid_usn > journal.next_usn
    {
        return Err("USN journal bounds are reversed".into());
    }
    if expected_journal_id.is_some_and(|expected| expected != journal.usn_journal_id) {
        return Err("USN journal identity changed since the snapshot".into());
    }
    let earliest_covered_usn = journal.first_usn.max(journal.lowest_valid_usn);
    if earliest_covered_usn > first_usn || journal.next_usn < last_usn {
        return Err("USN journal no longer covers the snapshot range".into());
    }
    Ok(())
}

fn validate_continuation(previous_usn: i64, next_usn: i64, last_usn: i64) -> Result<bool, String> {
    if next_usn < previous_usn {
        return Err("USN journal continuation moved backwards".into());
    }
    if next_usn == previous_usn && next_usn < last_usn {
        return Err("USN journal continuation did not advance".into());
    }
    Ok(next_usn >= last_usn)
}

/// Read journal records after a persisted USN without creating, deleting, or
/// changing the journal.  The caller remains responsible for re-reading the
/// changed MFT records before applying size updates.
pub fn read_changes(
    drive_letter: char,
    first_usn: i64,
    last_usn: i64,
) -> Result<Vec<UsnChange>, String> {
    read_changes_for_journal(drive_letter, None, first_usn, last_usn)
}

/// Read a delta only when the active journal still has the identity captured
/// by the snapshot.  This is kept separate from the legacy public wrapper so
/// existing callers retain their API while the snapshot reopen path can fail
/// closed on a journal reset or replacement.
pub(crate) fn read_changes_for_journal(
    drive_letter: char,
    expected_journal_id: Option<u64>,
    first_usn: i64,
    last_usn: i64,
) -> Result<Vec<UsnChange>, String> {
    if first_usn > last_usn {
        return Err("USN journal range is reversed".into());
    }
    let handle = open_volume(drive_letter)?;
    let journal = query_usn_journal(handle)?;
    if let Err(error) =
        validate_journal_continuity(&journal, expected_journal_id, first_usn, last_usn)
    {
        unsafe {
            let _ = CloseHandle(handle);
        }
        return Err(error);
    }
    let mut request = ReadUsnJournalData {
        start_usn: first_usn,
        reason_mask: u32::MAX,
        return_only_on_close: 0,
        timeout: 0,
        bytes_to_wait_for: 0,
        journal_id: journal.usn_journal_id,
        min_major_version: 2,
        max_major_version: 2,
    };
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut changes = Vec::new();
    let mut last_record_usn = None;
    loop {
        let mut returned = 0u32;
        let result = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_READ_USN_JOURNAL,
                Some(&mut request as *mut _ as *mut std::ffi::c_void),
                std::mem::size_of::<ReadUsnJournalData>() as u32,
                Some(buffer.as_mut_ptr() as *mut std::ffi::c_void),
                buffer.len() as u32,
                Some(&mut returned),
                None,
            )
        };
        if let Err(error) = result {
            let code = error.code().0;
            if code == 38 || code == 0x8007_0026u32 as i32 {
                if request.start_usn < last_usn {
                    unsafe {
                        let _ = CloseHandle(handle);
                    }
                    return Err("USN journal ended before the requested boundary".into());
                }
                break;
            }
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err(format!("FSCTL_READ_USN_JOURNAL failed: {:?}", error));
        }
        if returned < 8 {
            if request.start_usn < last_usn {
                unsafe {
                    let _ = CloseHandle(handle);
                }
                return Err("USN journal continuation is truncated".into());
            }
            break;
        }
        let next_start_usn = i64::from_le_bytes(
            buffer[0..8]
                .try_into()
                .map_err(|_| "USN journal continuation is truncated")?,
        );
        let reached_end = validate_continuation(request.start_usn, next_start_usn, last_usn)?;
        request.start_usn = next_start_usn;
        let mut offset = 8usize;
        while offset + std::mem::size_of::<UsnRecordV2Header>() <= returned as usize {
            let header = unsafe {
                std::ptr::read_unaligned(buffer.as_ptr().add(offset) as *const UsnRecordV2Header)
            };
            let record_length = usize::try_from(header.record_length)
                .map_err(|_| "USN record length is invalid")?;
            if record_length < std::mem::size_of::<UsnRecordV2Header>()
                || offset
                    .checked_add(record_length)
                    .is_none_or(|end| end > returned as usize)
            {
                return Err("USN record extends beyond its batch".into());
            }
            if header.major_version != 2 {
                return Err("USN journal returned an unsupported record version".into());
            }
            let name_start = offset
                .checked_add(usize::from(header.file_name_offset))
                .ok_or_else(|| "USN file name offset overflowed".to_string())?;
            let name_bytes = usize::from(header.file_name_length);
            let name_end = name_start
                .checked_add(name_bytes)
                .ok_or_else(|| "USN file name length overflowed".to_string())?;
            if name_bytes % 2 != 0
                || name_end
                    > offset
                        .checked_add(record_length)
                        .ok_or_else(|| "USN record bounds overflowed".to_string())?
            {
                return Err("USN file name is outside its record".into());
            }
            let name = unsafe {
                let name_ptr = buffer.as_ptr().add(name_start) as *const u16;
                String::from_utf16_lossy(std::slice::from_raw_parts(name_ptr, name_bytes / 2))
            };
            if header.usn > last_usn {
                break;
            }
            if header.usn < first_usn
                || last_record_usn.is_some_and(|previous| header.usn < previous)
            {
                return Err("USN journal records are outside a continuous ordered range".into());
            }
            last_record_usn = Some(header.usn);
            changes.push(UsnChange {
                file_reference: header.file_reference_number,
                parent_file_reference: header.parent_file_reference_number,
                usn: header.usn,
                reason: header.reason,
                name,
                attributes: header.file_attributes,
                is_directory: header.file_attributes & 0x10 != 0,
            });
            offset += record_length;
        }
        if offset == 8 && !reached_end {
            return Err("USN journal returned no records before the requested boundary".into());
        }
        if reached_end || offset == 8 {
            break;
        }
    }
    unsafe {
        let _ = CloseHandle(handle);
    }
    Ok(changes)
}

// -- USN enumeration -----------------------------------------

/// Enumerate all USN records on a volume and return them as flat entries
/// with resolved paths.
pub fn enumerate_volume(drive_letter: char) -> Result<(Vec<UsnFileEntry>, UsnScanStats), String> {
    let start = std::time::Instant::now();

    let handle = open_volume(drive_letter)?;
    let ujd = query_usn_journal(handle)?;

    // Build the FRN -> (parent_frn, name, is_dir) index from USN records
    let mut frn_index: HashMap<u64, (u64, String, bool, u32)> = HashMap::new();

    let mut med = MftEnumData {
        start_file_reference_number: 0,
        low_usn: 0,
        high_usn: ujd.next_usn,
    };

    const BUFFER_SIZE: usize = 1024 * 1024; // 1 MB
    let mut buffer: Vec<u8> = vec![0u8; BUFFER_SIZE];
    let mut total_records: u64 = 0;

    loop {
        let mut bytes_returned: u32 = 0;

        let result = unsafe {
            DeviceIoControl(
                handle,
                FSCTL_ENUM_USN_DATA,
                Some(&med as *const MftEnumData as *const std::ffi::c_void),
                std::mem::size_of::<MftEnumData>() as u32,
                Some(buffer.as_mut_ptr() as *mut std::ffi::c_void),
                BUFFER_SIZE as u32,
                Some(&mut bytes_returned),
                None,
            )
        };

        if let Err(err) = result {
            // ERROR_HANDLE_EOF (0x26) means we've read all records
            if err.code() == windows::core::HRESULT::from_win32(0x26) {
                break;
            }
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err(format!("FSCTL_ENUM_USN_DATA failed: {:?}", err));
        }

        if bytes_returned <= std::mem::size_of::<i64>() as u32 {
            break; // Only the next FRN present - no more records
        }

        // The first 8 bytes of the returned buffer are the next StartFileReferenceNumber
        let next_frn = unsafe { *(buffer.as_ptr() as *const u64) };
        med.start_file_reference_number = next_frn;

        // Walk the USN records
        let mut offset = std::mem::size_of::<u64>(); // skip the next FRN
        while offset + std::mem::size_of::<UsnRecordV2Header>() <= bytes_returned as usize {
            let record = unsafe { &*(buffer.as_ptr().add(offset) as *const UsnRecordV2Header) };

            let record_len = record.record_length as usize;
            if record_len == 0 || offset + record_len > bytes_returned as usize {
                break;
            }

            // Extract the file name (WCHAR at offset + FileNameOffset)
            let name_offset = record.file_name_offset as usize;
            let name_len = (record.file_name_length as usize) / 2; // bytes -> WCHAR count
            if name_len > 0 && offset + name_offset + name_len * 2 <= bytes_returned as usize {
                let name_ptr = unsafe { buffer.as_ptr().add(offset + name_offset) as *const u16 };
                let name_slice = unsafe { std::slice::from_raw_parts(name_ptr, name_len) };
                let name = String::from_utf16_lossy(name_slice);
                let is_dir = (record.file_attributes & 0x10) != 0; // FILE_ATTRIBUTE_DIRECTORY

                frn_index.insert(
                    record.file_reference_number,
                    (
                        record.parent_file_reference_number,
                        name,
                        is_dir,
                        record.file_attributes,
                    ),
                );
                total_records += 1;
            }

            offset += record_len;
        }
    }

    unsafe {
        let _ = CloseHandle(handle);
    }

    // Resolve full paths by walking parent FRN chains
    let (entries, resolved, unresolved) = resolve_paths(&frn_index);

    let stats = UsnScanStats {
        drive: format!("{}:", drive_letter),
        total_records,
        resolved_paths: resolved,
        unresolved_paths: unresolved,
        directories: entries.iter().filter(|e| e.is_directory).count() as u64,
        files: entries.iter().filter(|e| !e.is_directory).count() as u64,
        elapsed_ms: start.elapsed().as_millis() as u64,
    };

    Ok((entries, stats))
}

// -- Path resolution -----------------------------------------

const PATH_SEPARATOR: &str = "\\";

/// Resolve full paths from FRN -> (parent, name) index.
///
/// Walks the parent chain from each entry to the volume root
/// (parent_frn == file_reference_number, or parent not found).
fn resolve_paths(
    frn_index: &HashMap<u64, (u64, String, bool, u32)>,
) -> (Vec<UsnFileEntry>, u64, u64) {
    let mut entries = Vec::with_capacity(frn_index.len());
    let mut resolved: u64 = 0;
    let mut unresolved: u64 = 0;

    // Path cache: FRN -> resolved path string
    let mut path_cache: HashMap<u64, Option<String>> = HashMap::new();

    for (&frn, (parent_frn, name, is_dir, attrs)) in frn_index {
        let (full_path, path_ok) =
            resolve_single_path(frn, parent_frn, name, frn_index, &mut path_cache);

        if path_ok {
            resolved += 1;
        } else {
            unresolved += 1;
        }

        entries.push(UsnFileEntry {
            full_path,
            name: name.clone(),
            size_bytes: 0, // USN records don't include size
            parent_frn: *parent_frn,
            attributes: *attrs,
            is_directory: *is_dir,
            path_unresolved: !path_ok,
        });
    }

    (entries, resolved, unresolved)
}

/// Resolve a single FRN to a full path by walking up to root.
fn resolve_single_path(
    frn: u64,
    parent_frn: &u64,
    name: &str,
    frn_index: &HashMap<u64, (u64, String, bool, u32)>,
    path_cache: &mut HashMap<u64, Option<String>>,
) -> (String, bool) {
    // Check cache
    if let Some(cached) = path_cache.get(&frn) {
        return (
            cached.clone().unwrap_or_else(|| name.to_string()),
            cached.is_some(),
        );
    }

    // Root sentinel: parent FRN equals own FRN, or parent is 0, or root directory names
    if *parent_frn == frn || *parent_frn == 0 {
        let path = name.to_string();
        // Trim trailing separator
        let path = path.trim_end_matches('\\').to_string();
        path_cache.insert(frn, Some(path.clone()));
        return (path, true);
    }

    // Walk up to parent
    if let Some((grandparent_frn, parent_name, _, _)) = frn_index.get(parent_frn) {
        let (parent_path, ok) = resolve_single_path(
            *parent_frn,
            grandparent_frn,
            parent_name,
            frn_index,
            path_cache,
        );

        if ok {
            let full = if parent_path.is_empty() {
                name.to_string()
            } else {
                format!("{}{}{}", parent_path, PATH_SEPARATOR, name)
            };
            path_cache.insert(frn, Some(full.clone()));
            return (full, true);
        }
    }

    // Unresolved - store partial path
    path_cache.insert(frn, None);
    (name.to_string(), false)
}

// -- Fast file finder -----------------------------------------

/// Quickly find files matching a glob-like pattern on a volume.
/// Uses USN journal enumeration for speed, then filters by pattern.
pub fn find_files_fast(
    drive_letter: char,
    pattern_contains: &[&str],
    max_results: usize,
) -> Result<Vec<UsnFileEntry>, String> {
    let (entries, _stats) = enumerate_volume(drive_letter)?;

    let filtered: Vec<UsnFileEntry> = entries
        .into_iter()
        .filter(|e| {
            !e.is_directory
                && !e.path_unresolved
                && pattern_contains
                    .iter()
                    .any(|pat| e.full_path.to_lowercase().contains(&pat.to_lowercase()))
        })
        .take(max_results)
        .collect();

    Ok(filtered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_usn_journal_request_matches_windows_layout() {
        assert_eq!(std::mem::size_of::<ReadUsnJournalData>(), 48);
    }

    #[test]
    fn continuity_requires_the_same_journal_and_covered_boundaries() {
        let journal = UsnJournalData {
            usn_journal_id: 7,
            first_usn: 10,
            next_usn: 20,
            lowest_valid_usn: 10,
            max_usn: i64::MAX,
            maximum_size: 1024,
            allocation_delta: 512,
        };

        assert!(validate_journal_continuity(&journal, Some(7), 12, 20).is_ok());
        assert!(validate_journal_continuity(&journal, Some(8), 12, 20).is_err());
        assert!(validate_journal_continuity(&journal, Some(7), 9, 20).is_err());
        assert!(validate_journal_continuity(&journal, Some(7), 12, 21).is_err());

        let mut journal_with_unavailable_prefix = journal;
        journal_with_unavailable_prefix.first_usn = 8;
        journal_with_unavailable_prefix.lowest_valid_usn = 12;
        assert!(
            validate_journal_continuity(&journal_with_unavailable_prefix, Some(7), 11, 20).is_err()
        );
        assert!(
            validate_journal_continuity(&journal_with_unavailable_prefix, Some(7), 12, 20).is_ok()
        );
    }

    #[test]
    fn continuity_rejects_a_stalled_or_reversed_continuation() {
        assert!(validate_continuation(10, 11, 20).is_ok());
        assert!(validate_continuation(10, 10, 20).is_err());
        assert!(validate_continuation(10, 9, 20).is_err());
        assert!(!validate_continuation(10, 15, 20).unwrap());
        assert!(validate_continuation(10, 20, 20).unwrap());
    }

    #[test]
    fn test_resolve_root_sentinel() {
        let mut index = HashMap::new();
        // Root: frn == parent_frn
        index.insert(5, (5, "C:".to_string(), true, 0x10));

        let (entries, resolved, unresolved) = resolve_paths(&index);
        assert_eq!(resolved, 1);
        assert_eq!(unresolved, 0);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].full_path.contains("C:"));
    }

    #[test]
    fn test_resolve_two_levels() {
        let mut index = HashMap::new();
        // C:\
        index.insert(5, (5, "C:".to_string(), true, 0x10));
        // C:\Users
        index.insert(10, (5, "Users".to_string(), true, 0x10));
        // C:\Users\test.txt
        index.insert(15, (10, "test.txt".to_string(), false, 0x20));

        let (entries, resolved, unresolved) = resolve_paths(&index);
        assert_eq!(resolved, 3);
        assert_eq!(unresolved, 0);
        assert_eq!(entries.len(), 3);

        let test_file = entries.iter().find(|e| e.name == "test.txt").unwrap();
        assert!(test_file.full_path.to_lowercase().contains("users"));
        assert!(test_file.full_path.to_lowercase().contains("test.txt"));
    }

    #[test]
    fn test_unresolved_orphan() {
        let mut index = HashMap::new();
        // Orphan: parent FRN not present in index
        index.insert(15, (99, "orphan.txt".to_string(), false, 0x20));

        let (entries, resolved, unresolved) = resolve_paths(&index);
        assert_eq!(resolved, 0);
        assert_eq!(unresolved, 1);
        assert!(entries[0].path_unresolved);
    }
}
