//! Generic file-identity primitives shared by Free features.
//!
//! These describe a file's stable identity (volume serial + file reference)
//! and a point-in-time snapshot. They are deliberately independent of the
//! duplicate-analysis cache.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileIdentity {
    pub volume_serial: Option<u64>,
    pub file_reference: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileSnapshot {
    pub path: String,
    pub identity: FileIdentity,
    pub size_bytes: u64,
    pub modified_time: String,
}

pub fn snapshot_for_path(path: &Path) -> std::io::Result<FileSnapshot> {
    let metadata = std::fs::symlink_metadata(path)?;
    Ok(FileSnapshot {
        path: path.to_string_lossy().into_owned(),
        identity: file_identity_for_path(path),
        size_bytes: metadata.len(),
        modified_time: modified_time(&metadata),
    })
}

fn modified_time(metadata: &std::fs::Metadata) -> String {
    metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_nanos().to_string())
        .unwrap_or_default()
}

#[cfg(windows)]
fn file_identity_for_path(path: &Path) -> FileIdentity {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_GENERIC_READ, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    let Ok(handle) = (unsafe {
        CreateFileW(
            &HSTRING::from(path.to_string_lossy().as_ref()),
            FILE_GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
    }) else {
        return FileIdentity::default();
    };
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    let result = unsafe { GetFileInformationByHandle(handle, &mut information) };
    unsafe {
        let _ = CloseHandle(handle);
    }
    if result.is_err() {
        return FileIdentity::default();
    }
    FileIdentity {
        volume_serial: Some(u64::from(information.dwVolumeSerialNumber)),
        file_reference: Some(
            ((u128::from(information.nFileIndexHigh) << 32)
                | u128::from(information.nFileIndexLow))
            .to_string(),
        ),
    }
}

#[cfg(not(windows))]
fn file_identity_for_path(_path: &Path) -> FileIdentity {
    FileIdentity::default()
}
