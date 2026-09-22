//! Windows deletion primitives that keep the confirmed file identity attached
//! to the mutation instead of re-resolving the path after validation.
//!
//! Guarantee by mode:
//!
//! * Permanent deletion is handle-bound.  `SetFileInformationByHandle` mutates
//!   the exact object that passed the final path and identity checks, so a
//!   directory-entry replacement cannot redirect the deletion.  This path is
//!   race-safe with respect to the validated object identity.
//! * Recycle Bin deletion is handoff-bound, not handle-bound.  The shell
//!   `IFileOperation` API accepts only an `IShellItem`, which WinOptimizer must
//!   create from the confirmed path; there is no public handle-based recycle
//!   entry point.  A hostile process running as the same user could replace
//!   the directory entry in the narrow window between the final identity check
//!   and the shell operation resolving that path.  The race is narrowed by the
//!   immediately preceding handle path/identity validation and by the
//!   reparse-free canonical path check, but it is not eliminated.  Recycle Bin
//!   remains the default because recoverability is a core safety property.
//!
//! A handle-bound staging approach (handle rename to an application-controlled
//! quarantine directory, then recycle the quarantined path) was evaluated and
//! deliberately not adopted: the recycle still resolves a path, it changes the
//! restore destination recorded by the Recycle Bin, it requires same-volume
//! application directory and ACL management per volume, and it adds rollback
//! and journal complexity.  Those costs exceed the residual narrowing it
//! provides, so the narrow race is documented rather than papered over.

use std::path::Path;

use crate::core::file_identity::FileIdentity;

/// Delete the file represented by `path` after opening and validating one
/// stable handle. The permanent path uses `SetFileInformationByHandle`, so a
/// directory-entry replacement cannot redirect the mutation to another file.
/// Recycle Bin deletion uses a shell item captured while the confirmation
/// handle is open and is attempted only after the same handle identity check;
/// see the module-level note for the residual path race that mode retains.
pub fn delete_file(
    path: &Path,
    expected_path: &Path,
    expected_identity: Option<&FileIdentity>,
    recycle_bin: bool,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows_delete_file(path, expected_path, expected_identity, recycle_bin)
    }

    #[cfg(not(windows))]
    {
        let _ = (expected_path, expected_identity);
        if recycle_bin {
            return Err("Recycle Bin deletion is unavailable on this platform".into());
        }
        std::fs::remove_file(path).map_err(|error| error.to_string())
    }
}

#[cfg(windows)]
fn windows_delete_file(
    path: &Path,
    expected_path: &Path,
    expected_identity: Option<&FileIdentity>,
    recycle_bin: bool,
) -> Result<(), String> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FileDispositionInfoEx, SetFileInformationByHandle, DELETE,
        FILE_ATTRIBUTE_NORMAL, FILE_DISPOSITION_FLAG_DELETE,
        FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE, FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
        FILE_DISPOSITION_INFO_EX, FILE_DISPOSITION_INFO_EX_FLAGS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    if recycle_bin && expected_identity.is_none() {
        return Err("Recycle Bin deletion requires a confirmed file identity".into());
    }

    let _ = path;
    let expected = expected_path;
    let handle = unsafe {
        CreateFileW(
            &HSTRING::from(expected.as_os_str()),
            DELETE.0,
            FILE_SHARE_DELETE | FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    }
    .map_err(|error| format!("cannot open confirmed target for safe deletion: {error}"))?;

    let result = (|| {
        let _com = if recycle_bin {
            Some(ComApartment::initialize()?)
        } else {
            None
        };
        let actual = final_path_from_handle(handle)?;
        if normalize_path(&actual) != normalize_path(expected) {
            return Err("confirmed target was retargeted before deletion".into());
        }
        if let Some(expected_identity) = expected_identity {
            let actual_identity = file_identity_from_handle(handle)?;
            if &actual_identity != expected_identity {
                return Err("confirmed target was replaced since preview".into());
            }
        }

        if recycle_bin {
            // IFileOperation accepts only an IShellItem and there is no
            // handle-based recycle API, so the shell item is re-derived from
            // the confirmed path.  The handle below stays open, but it was
            // opened with FILE_SHARE_DELETE, so a same-user process that
            // replaces the directory entry after the identity check above and
            // before the shell operation resolves this path is not detected.
            // This is the documented residual for Recycle Bin mode; permanent
            // mode does not have it.
            let shell_item = create_shell_item(expected)?;
            return recycle_file(&shell_item);
        }

        let disposition = FILE_DISPOSITION_INFO_EX {
            Flags: FILE_DISPOSITION_INFO_EX_FLAGS(
                FILE_DISPOSITION_FLAG_DELETE.0
                    | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS.0
                    | FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE.0,
            ),
        };
        unsafe {
            SetFileInformationByHandle(
                handle,
                FileDispositionInfoEx,
                std::ptr::from_ref(&disposition).cast(),
                std::mem::size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
            )
        }
        .map_err(|error| format!("safe handle deletion failed: {error}"))
    })();

    let close = unsafe { CloseHandle(handle) };
    result.and(close.map_err(|error| format!("cannot close deletion handle: {error}")))
}

#[cfg(windows)]
fn file_identity_from_handle(
    handle: windows::Win32::Foundation::HANDLE,
) -> Result<FileIdentity, String> {
    use windows::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(handle, &mut information) }
        .map_err(|error| format!("cannot read confirmed file identity: {error}"))?;
    Ok(FileIdentity {
        volume_serial: Some(u64::from(information.dwVolumeSerialNumber)),
        file_reference: Some(
            ((u128::from(information.nFileIndexHigh) << 32)
                | u128::from(information.nFileIndexLow))
            .to_string(),
        ),
    })
}

#[cfg(windows)]
fn final_path_from_handle(
    handle: windows::Win32::Foundation::HANDLE,
) -> Result<std::path::PathBuf, String> {
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::Storage::FileSystem::{GetFinalPathNameByHandleW, VOLUME_NAME_DOS};

    let mut buffer = vec![0_u16; 32_768];
    let length = unsafe { GetFinalPathNameByHandleW(handle, &mut buffer, VOLUME_NAME_DOS) };
    if length == 0 || length as usize >= buffer.len() {
        return Err("cannot resolve the confirmed file handle to a final path".into());
    }
    Ok(std::ffi::OsString::from_wide(&buffer[..length as usize]).into())
}

#[cfg(windows)]
fn normalize_path(path: &Path) -> String {
    path.to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('/', "\\")
        .to_ascii_lowercase()
}

#[cfg(windows)]
struct ComApartment;

#[cfg(windows)]
impl ComApartment {
    fn initialize() -> Result<Self, String> {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};

        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if result.is_err() {
            Err(format!(
                "cannot initialize shell recycle operation: {result}"
            ))
        } else {
            Ok(Self)
        }
    }
}

#[cfg(windows)]
impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { windows::Win32::System::Com::CoUninitialize() };
    }
}

#[cfg(windows)]
fn create_shell_item(path: &Path) -> Result<windows::Win32::UI::Shell::IShellItem, String> {
    use windows::core::HSTRING;
    use windows::Win32::UI::Shell::SHCreateItemFromParsingName;

    // Shell parsing rejects the extended-length prefix returned by
    // Path::canonicalize on Windows. Convert it to the equivalent shell path
    // while retaining the canonical path for the handle and identity checks.
    let raw = path.to_string_lossy();
    let shell_path = if let Some(unc_path) = raw.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc_path}")
    } else {
        raw.strip_prefix(r"\\?\").unwrap_or(&raw).to_string()
    };
    unsafe { SHCreateItemFromParsingName(&HSTRING::from(shell_path), None) }
        .map_err(|error| format!("cannot bind confirmed target to shell item: {error}"))
}

#[cfg(windows)]
fn recycle_file(shell_item: &windows::Win32::UI::Shell::IShellItem) -> Result<(), String> {
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
    use windows::Win32::UI::Shell::{
        FileOperation, IFileOperation, IFileOperationProgressSink, FILEOPERATION_FLAGS,
        FOFX_RECYCLEONDELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT,
    };

    let operation: IFileOperation =
        unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER) }
            .map_err(|error| format!("cannot create shell file operation: {error}"))?;

    unsafe {
        operation
            .SetOperationFlags(FILEOPERATION_FLAGS(
                FOF_ALLOWUNDO.0
                    | FOF_NOCONFIRMATION.0
                    | FOF_NOERRORUI.0
                    | FOF_SILENT.0
                    | FOFX_RECYCLEONDELETE.0,
            ))
            .map_err(|error| format!("cannot configure Recycle Bin operation: {error}"))?;
        operation
            .DeleteItem(shell_item, Option::<&IFileOperationProgressSink>::None)
            .map_err(|error| format!("cannot queue Recycle Bin operation: {error}"))?;
        operation
            .PerformOperations()
            .map_err(|error| format!("Recycle Bin operation failed: {error}"))?;
        if operation
            .GetAnyOperationsAborted()
            .map_err(|error| format!("cannot verify Recycle Bin operation: {error}"))?
            .as_bool()
        {
            return Err("Recycle Bin operation was aborted".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::delete_file;
    use tempfile::tempdir;

    #[test]
    fn deletes_only_the_confirmed_file() {
        let directory = tempdir().unwrap();
        let target = directory.path().join("target.tmp");
        std::fs::write(&target, b"target").unwrap();

        let expected = target.canonicalize().unwrap();
        delete_file(&target, &expected, None, false).unwrap();

        assert!(!target.exists());
    }

    #[test]
    fn recycle_bin_fails_closed_without_atomic_identity_binding() {
        let directory = tempdir().unwrap();
        let target = directory.path().join("target.tmp");
        std::fs::write(&target, b"target").unwrap();

        let expected = target.canonicalize().unwrap();
        let error = delete_file(&target, &expected, None, true).unwrap_err();
        #[cfg(windows)]
        assert!(error.contains("requires a confirmed file identity"));
        #[cfg(not(windows))]
        assert!(error.contains("unavailable on this platform"));
        assert!(target.exists());
    }

    #[cfg(windows)]
    #[test]
    fn recycles_the_confirmed_file_when_identity_is_bound() {
        let directory = tempdir().unwrap();
        let target = directory.path().join("target.tmp");
        std::fs::write(&target, b"target").unwrap();

        let expected = target.canonicalize().unwrap();
        let snapshot = crate::core::file_identity::snapshot_for_path(&target).unwrap();
        delete_file(&target, &expected, Some(&snapshot.identity), true).unwrap();

        assert!(!target.exists());
    }
}
