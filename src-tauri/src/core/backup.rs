//! Backup primitives - file copies and binary registry hive exports.
//! Backups go to %LOCALAPPDATA%\WinOptimizer\backups\.
//! Domain-specific recovery stores may retain a backup path for a possible
//! best-effort reversal; creating a backup alone never establishes undo.
//!
//! Terminology follows the operation contract:
//! - a backup is a captured original artifact or state;
//! - undo is an automated operation that restores prior state;
//! - exact recovery requires a tested restoration and verification;
//! - best-effort recovery attempts restoration without guaranteeing exact
//!   equivalence;
//! - a restore point is Windows System Restore and is independent of
//!   WinOptimizer exact undo.
//!
//! This module creates and copies backup artifacts. The presence of a backup
//! must not be treated as UI undo eligibility; that signal belongs to the
//! operation journal's explicit `UndoEligibility` contract.
//!
//! Registry backups use RegSaveKeyExW which produces a compressed binary
//! hive file that preserves security descriptors - unlike text-based
//! .reg exports which discard binary structures.

use crate::core::safe_paths::{check_path_under_root, SafePathDecision};
use std::path::{Path, PathBuf};
#[cfg(any(test, debug_assertions))]
use std::sync::Mutex;
use windows::core::HSTRING;
use windows::Win32::Security::{
    AdjustTokenPrivileges, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES,
    TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegRestoreKeyW, RegSaveKeyExW, HKEY, KEY_READ, KEY_WRITE,
    REG_LATEST_FORMAT, REG_RESTORE_KEY_FLAGS,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

pub struct BackupManager {
    backup_dir: PathBuf,
    #[cfg(any(test, debug_assertions))]
    fail_next_backup: Mutex<bool>,
}

impl BackupManager {
    pub fn new() -> std::io::Result<Self> {
        Self::new_at(crate::core::paths::AppPaths::installed()?.backups)
    }

    pub fn new_at(backup_dir: impl Into<PathBuf>) -> std::io::Result<Self> {
        let backup_dir = backup_dir.into();
        std::fs::create_dir_all(&backup_dir)?;
        Ok(Self {
            backup_dir,
            #[cfg(any(test, debug_assertions))]
            fail_next_backup: Mutex::new(false),
        })
    }
    #[cfg(any(test, debug_assertions))]
    #[allow(dead_code)]
    pub(crate) fn fail_next_backup(&self) {
        *self
            .fail_next_backup
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = true;
    }

    #[cfg(any(test, debug_assertions))]
    fn take_fail_next_backup(&self) -> bool {
        let mut failure = self
            .fail_next_backup
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let should_fail = *failure;
        *failure = false;
        should_fail
    }

    /// Capture a file artifact before deletion.
    ///
    /// Returns the backup path for a domain-specific recovery record. Creating
    /// this artifact does not establish exact recovery or UI undo eligibility.
    pub fn backup_file<P: AsRef<Path>>(&self, source: P) -> std::io::Result<PathBuf> {
        #[cfg(any(test, debug_assertions))]
        if self.take_fail_next_backup() {
            return Err(std::io::Error::other("failure injection: fail_backup"));
        }

        let source = source.as_ref();
        let backup_id = uuid::Uuid::new_v4();
        let safe_name = source
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_else(|| "unknown".into());
        let dest = self.backup_dir.join(format!("{}_{}", backup_id, safe_name));
        std::fs::copy(source, &dest)?;
        Ok(dest)
    }

    fn canonical_backup_path(&self, backup_path: &Path) -> std::io::Result<PathBuf> {
        match check_path_under_root(&self.backup_dir, backup_path) {
            SafePathDecision::Protected(reason) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    reason.to_string(),
                ));
            }
            SafePathDecision::Allowed => {}
        }

        let canonical_backup = std::fs::canonicalize(backup_path)?;
        if !canonical_backup.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Backup artifact is not a file",
            ));
        }
        Ok(canonical_backup)
    }

    /// Attempt best-effort restoration of a file from a backup artifact.
    ///
    /// The copy must be followed by domain-specific read-back verification
    /// before it can be described as exact recovery.
    pub fn restore_file_best_effort(
        &self,
        backup_path: &Path,
        original_path: &Path,
    ) -> std::io::Result<()> {
        let backup_path = self.canonical_backup_path(backup_path)?;
        if let Some(parent) = original_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(backup_path, original_path)?;
        Ok(())
    }

    /// Binary registry hive backup using RegSaveKeyExW.
    ///
    /// Opens the registry key, enables SeBackupPrivilege, and saves the key
    /// with all subkeys in standard compressed binary hive format.
    /// The resulting .hiv file preserves binary structures and security
    /// descriptors that text-based .reg exports discard.
    /// This is a captured backup artifact, not an exact-recovery claim.
    ///
    /// `hive` - e.g. HKEY_LOCAL_MACHINE or HKEY_CURRENT_USER
    /// `subkey` - path relative to hive, e.g. r"SOFTWARE\Microsoft\..."
    /// Returns the path to the .hiv backup file.
    pub fn backup_registry_key(&self, hive: HKEY, subkey: &str) -> std::io::Result<PathBuf> {
        #[cfg(any(test, debug_assertions))]
        if self.take_fail_next_backup() {
            return Err(std::io::Error::other("failure injection: fail_backup"));
        }

        // Enable SeBackupPrivilege on the current thread
        enable_privilege("SeBackupPrivilege")?;

        // Open the target registry key
        let hkey = unsafe {
            let mut key: HKEY = std::mem::zeroed();
            if RegOpenKeyExW(hive, &HSTRING::from(subkey), Some(0), KEY_READ, &mut key).is_err() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("Registry key not found: {}", subkey),
                ));
            }
            key
        };

        // Build a safe filename from the subkey path
        let backup_id = uuid::Uuid::new_v4();
        let safe_name = sanitize_filename(subkey);
        let dest = self
            .backup_dir
            .join(format!("{}_{}.hiv", backup_id, safe_name));

        // Save the key and all subkeys in binary hive format
        let result = unsafe {
            RegSaveKeyExW(
                hkey,
                &HSTRING::from(dest.to_string_lossy().as_ref()),
                None,
                REG_LATEST_FORMAT,
            )
        };

        unsafe {
            let _ = RegCloseKey(hkey);
        }

        if result.is_err() {
            return Err(std::io::Error::other(format!(
                "RegSaveKeyExW failed for: {}",
                subkey
            )));
        }

        Ok(dest)
    }

    /// Attempt best-effort restoration of a registry key from a binary .hiv
    /// backup via RegRestoreKeyW.
    ///
    /// This replaces the target key and all its subkeys with the saved state.
    /// Requires SeRestorePrivilege and domain-specific read-back verification
    /// before it can be described as exact recovery.
    pub fn restore_registry_key_best_effort(
        &self,
        hive: HKEY,
        subkey: &str,
        backup_path: &Path,
    ) -> std::io::Result<()> {
        let backup_path = self.canonical_backup_path(backup_path)?;
        enable_privilege("SeRestorePrivilege")?;

        // Open the key with write access (required by RegRestoreKeyW)
        let hkey = unsafe {
            let mut key: HKEY = std::mem::zeroed();
            if RegOpenKeyExW(
                hive,
                &HSTRING::from(subkey),
                Some(0),
                KEY_READ | KEY_WRITE,
                &mut key,
            )
            .is_err()
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("Registry key not found for restore: {}", subkey),
                ));
            }
            key
        };

        let result = unsafe {
            RegRestoreKeyW(
                hkey,
                &HSTRING::from(backup_path.to_string_lossy().as_ref()),
                REG_RESTORE_KEY_FLAGS(0),
            )
        };

        unsafe {
            let _ = RegCloseKey(hkey);
        }

        if result.is_err() {
            return Err(std::io::Error::other(format!(
                "RegRestoreKeyW failed for: {}",
                subkey
            )));
        }

        Ok(())
    }

    /// Get the backup directory path.
    pub fn backup_dir(&self) -> &PathBuf {
        &self.backup_dir
    }
}

// -- Privilege helpers ----------------------------------------

/// RAII guard for Windows HANDLE — closes on drop.
struct TokenGuard(windows::Win32::Foundation::HANDLE);

impl Drop for TokenGuard {
    fn drop(&mut self) {
        if self.0 != windows::Win32::Foundation::HANDLE::default() {
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }
}

/// Enable a named privilege (e.g. "SeBackupPrivilege") on the current thread token.
fn enable_privilege(name: &str) -> std::io::Result<()> {
    unsafe {
        let mut raw_token = windows::Win32::Foundation::HANDLE::default();
        if OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut raw_token,
        )
        .is_err()
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Cannot open process token",
            ));
        }
        let token = TokenGuard(raw_token);

        // LUID is in Win32::Foundation in windows-rs 0.60
        let mut luid = windows::Win32::Foundation::LUID::default();

        if LookupPrivilegeValueW(None, &HSTRING::from(name), &mut luid).is_err() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Privilege not found: {}", name),
            ));
        }

        let tp = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [windows::Win32::Security::LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: SE_PRIVILEGE_ENABLED,
            }],
        };

        if AdjustTokenPrivileges(token.0, false, Some(&tp), 0, None, None).is_err() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("Cannot enable privilege: {}", name),
            ));
        }

        // Check if the privilege was actually assigned
        let last_err = windows::Win32::Foundation::GetLastError();
        if last_err == windows::Win32::Foundation::ERROR_NOT_ALL_ASSIGNED {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("Privilege not held by token: {}", name),
            ));
        }

        // TokenGuard::drop closes the handle
    }
    Ok(())
}

/// Sanitize a registry key path into a safe filename component.
fn sanitize_filename(path: &str) -> String {
    path.chars()
        .map(|c| if c == '\\' { '_' } else { c })
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_file_creates_a_captured_artifact_without_undo_claims() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.txt");
        let backup_dir = directory.path().join("backups");
        std::fs::create_dir_all(&backup_dir).unwrap();
        std::fs::write(&source, b"original bytes").unwrap();

        let manager = BackupManager {
            backup_dir,
            #[cfg(any(test, debug_assertions))]
            fail_next_backup: Mutex::new(false),
        };
        let backup_path = manager.backup_file(&source).unwrap();

        assert!(backup_path.starts_with(manager.backup_dir()));
        assert_eq!(std::fs::read(&backup_path).unwrap(), b"original bytes");
    }

    #[test]
    fn restore_file_best_effort_requires_external_verification() {
        let directory = tempfile::tempdir().unwrap();
        let backup_dir = directory.path().join("backups");
        std::fs::create_dir_all(&backup_dir).unwrap();
        let backup_path = backup_dir.join("captured.bin");
        let original_path = directory.path().join("restored").join("target.bin");
        std::fs::write(&backup_path, b"captured bytes").unwrap();
        let manager = BackupManager {
            backup_dir,
            #[cfg(any(test, debug_assertions))]
            fail_next_backup: Mutex::new(false),
        };

        manager
            .restore_file_best_effort(&backup_path, &original_path)
            .unwrap();

        assert_eq!(std::fs::read(original_path).unwrap(), b"captured bytes");
    }

    #[test]
    fn restore_rejects_backup_artifacts_outside_the_manager_root() {
        let directory = tempfile::tempdir().unwrap();
        let backup_dir = directory.path().join("backups");
        std::fs::create_dir_all(&backup_dir).unwrap();
        let outside_backup = directory.path().join("captured.bin");
        let original_path = directory.path().join("restored").join("target.bin");
        std::fs::write(&outside_backup, b"captured bytes").unwrap();
        let manager = BackupManager {
            backup_dir,
            #[cfg(any(test, debug_assertions))]
            fail_next_backup: Mutex::new(false),
        };

        let error = manager
            .restore_file_best_effort(&outside_backup, &original_path)
            .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(error.to_string(), "Protected because outside approved root");
        assert!(!original_path.exists());
    }

    #[test]
    fn injected_backup_failure_is_one_shot_and_aborts_before_copy() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.txt");
        let backup_dir = directory.path().join("backups");
        std::fs::create_dir_all(&backup_dir).unwrap();
        std::fs::write(&source, b"original bytes").unwrap();

        let manager = BackupManager {
            backup_dir,
            #[cfg(any(test, debug_assertions))]
            fail_next_backup: Mutex::new(false),
        };
        manager.fail_next_backup();

        let error = manager.backup_file(&source).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::Other);
        assert_eq!(error.to_string(), "failure injection: fail_backup");
        assert!(source.exists());
        assert_eq!(std::fs::read_dir(manager.backup_dir()).unwrap().count(), 0);

        let backup_path = manager.backup_file(&source).unwrap();
        assert_eq!(std::fs::read(backup_path).unwrap(), b"original bytes");
    }
}
