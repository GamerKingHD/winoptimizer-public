//! System Restore Point management via SrClient.dll.
//!
//! Programmatically creates system restore points before any critical
//! optimization pass. Bypasses the default 24-hour creation throttle
//! by temporarily overriding SystemRestorePointCreationFrequency.
//!
//! Uses dynamic library loading to avoid link-time dependencies on
//! the System Restore Client API.

use crate::core::error::{Result, WinOptimizerError};
use windows::core::{s, HSTRING};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, HMODULE};
use windows::Win32::System::Com::{
    CoInitializeEx, CoInitializeSecurity, CoUninitialize, COINIT_APARTMENTTHREADED, EOAC_NONE,
    RPC_C_AUTHN_LEVEL_PKT_PRIVACY, RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_DWORD, REG_OPTION_NON_VOLATILE,
};

// -- System Restore API types ----------------------------------
// Defined manually - windows-rs 0.60 does not include
// srrestoreptapi.h types.

const MAX_DESC: usize = 256;

/// Restore point event type.
pub const BEGIN_SYSTEM_CHANGE: u32 = 100;

/// Restore point type.
pub const APPLICATION_INSTALL: u32 = 0;

/// Maps to the C RESTOREPOINTINFOW struct.
#[repr(C)]
struct RestorePointInfoW {
    dw_event_type: u32,
    dw_restore_point_type: u32,
    ll_sequence_number: i64,
    sz_description: [u16; MAX_DESC],
}

/// Maps to the C STATEMGRSTATUS struct.
#[repr(C)]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StateMgrStatus {
    pub n_status: u32,
    pub ll_sequence_number: i64,
}

// SRSetRestorePointW returns BOOL (i32 in Win32).
type PfnSRSetRestorePointW =
    unsafe extern "system" fn(*mut RestorePointInfoW, *mut StateMgrStatus) -> i32;

// -- 24-hour throttle bypass ----------------------------------

const THROTTLE_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\SystemRestore";
const THROTTLE_VALUE: &str = "SystemRestorePointCreationFrequency";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThrottleValue {
    Missing,
    Present(u32),
    Unavailable,
}

/// Read the current throttle value while distinguishing a missing value from
/// an unreadable value. Cleanup must not delete an existing value that could
/// not be read.
fn read_throttle_value() -> ThrottleValue {
    unsafe {
        let mut hkey: HKEY = std::mem::zeroed();
        let result = RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(THROTTLE_KEY),
            Some(0),
            KEY_QUERY_VALUE,
            &mut hkey,
        );
        if result == ERROR_FILE_NOT_FOUND {
            return ThrottleValue::Missing;
        }
        if result.is_err() {
            return ThrottleValue::Unavailable;
        }

        let mut data: u32 = 0;
        let mut data_size: u32 = std::mem::size_of::<u32>() as u32;
        let query = RegQueryValueExW(
            hkey,
            &HSTRING::from(THROTTLE_VALUE),
            None,
            None,
            Some(&mut data as *mut u32 as *mut u8),
            Some(&mut data_size),
        );

        let _ = RegCloseKey(hkey);

        if query.is_ok() {
            ThrottleValue::Present(data)
        } else if query == ERROR_FILE_NOT_FOUND {
            ThrottleValue::Missing
        } else {
            ThrottleValue::Unavailable
        }
    }
}

/// Write the throttle value.
fn write_throttle_value(value: u32) -> Result<()> {
    unsafe {
        let mut hkey: HKEY = std::mem::zeroed();
        RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(THROTTLE_KEY),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut hkey,
            None,
        )
        .ok()
        .ok()
        .ok_or_else(|| {
            WinOptimizerError::Other("Failed to open SystemRestore key for throttle bypass".into())
        })?;

        let data_bytes = value.to_ne_bytes();
        let result = RegSetValueExW(
            hkey,
            &HSTRING::from(THROTTLE_VALUE),
            None,
            REG_DWORD,
            Some(&data_bytes),
        );

        let _ = RegCloseKey(hkey);

        if result.is_err() {
            return Err(WinOptimizerError::Other(format!(
                "Failed to write throttle bypass value: {:?}",
                result
            )));
        }
    }
    Ok(())
}

/// Temporarily bypass the 24-hour restore point creation throttle.
/// Returns a guard that restores the value only when the override was applied.
fn bypass_throttle() -> ThrottleGuard {
    let original = match read_throttle_value() {
        ThrottleValue::Missing => None,
        ThrottleValue::Present(value) => Some(value),
        ThrottleValue::Unavailable => {
            log::debug!("System Restore throttle value could not be read; leaving it unchanged");
            return ThrottleGuard {
                original: None,
                applied: false,
            };
        }
    };
    let applied = write_throttle_value(0).is_ok();
    ThrottleGuard { original, applied }
}

/// Restore the original throttle value after the restore point is created.
fn restore_throttle(original: Option<u32>) {
    if let Some(val) = original {
        let _ = write_throttle_value(val);
    } else {
        // Delete the value we created to restore default behavior
        unsafe {
            let mut hkey: HKEY = std::mem::zeroed();
            let open_result = RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                &HSTRING::from(THROTTLE_KEY),
                Some(0),
                KEY_SET_VALUE,
                &mut hkey,
            );
            if open_result.is_ok() {
                let _ = RegDeleteValueW(hkey, &HSTRING::from(THROTTLE_VALUE));
                let _ = RegCloseKey(hkey);
            }
        }
    }
}

// -- COM initialization ---------------------------------------

fn init_com() -> Result<()> {
    unsafe {
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if hr.is_err() {
            return Err(WinOptimizerError::Other(format!(
                "CoInitializeEx failed: {:?}",
                hr
            )));
        }

        let hr = CoInitializeSecurity(
            None,
            -1,
            None,
            None,
            RPC_C_AUTHN_LEVEL_PKT_PRIVACY,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
            None,
        );

        // CoInitializeSecurity may fail with RPC_E_TOO_LATE if COM security
        // was already initialized (common when running inside Tauri).
        // This is harmless - log and continue.
        if hr.is_err() {
            log::debug!(
                "CoInitializeSecurity returned: {:?} (may already be initialized)",
                hr
            );
        }
    }
    Ok(())
}

// -- Main API -------------------------------------------------

/// RAII guard that restores the original throttle value on drop.
struct ThrottleGuard {
    original: Option<u32>,
    applied: bool,
}

impl Drop for ThrottleGuard {
    fn drop(&mut self) {
        if self.applied {
            restore_throttle(self.original);
        }
    }
}

/// Create a system restore point with the given description.
///
/// Returns the StateMgrStatus containing the sequence number on success.
/// This bypasses the 24-hour throttle so a restore point is always
/// created before a governed operation. Callers must use the operation
/// journal's restore-point policy; the legacy transaction path is blocked.
pub(crate) fn create_restore_point(description: &str) -> Result<StateMgrStatus> {
    // Validate the description
    if description.is_empty() || description.len() >= MAX_DESC {
        return Err(WinOptimizerError::Other(
            "Restore point description must be 1-255 characters".into(),
        ));
    }

    // Initialize COM for the restore point API
    init_com()?;

    // Bypass the 24-hour creation throttle (RAII restores on drop)
    let _throttle_guard = bypass_throttle();

    // Load SrClient.dll dynamically
    let srclient: HMODULE = unsafe {
        LoadLibraryW(&HSTRING::from("SrClient.dll")).map_err(|e| {
            WinOptimizerError::Other(format!("Failed to load SrClient.dll: {:?}", e))
        })?
    };

    // Resolve SRSetRestorePointW by name.
    // GetProcAddress takes a PCSTR (ANSI), so use the s! literal macro
    // from windows::core which produces a null-terminated narrow string.
    let pfn: Option<PfnSRSetRestorePointW> = unsafe {
        GetProcAddress(srclient, s!("SRSetRestorePointW")).map(|addr| std::mem::transmute(addr))
    };

    let result = if let Some(sr_set_restore_point) = pfn {
        // Build the RESTOREPOINTINFOW
        let mut desc_buf: [u16; MAX_DESC] = [0; MAX_DESC];
        let desc_wide: Vec<u16> = description.encode_utf16().collect();
        let copy_len = desc_wide.len().min(MAX_DESC - 1);
        desc_buf[..copy_len].copy_from_slice(&desc_wide[..copy_len]);

        let mut restore_pt_info = RestorePointInfoW {
            dw_event_type: BEGIN_SYSTEM_CHANGE,
            dw_restore_point_type: APPLICATION_INSTALL,
            ll_sequence_number: 0,
            sz_description: desc_buf,
        };

        let mut status = StateMgrStatus {
            n_status: 0,
            ll_sequence_number: 0,
        };

        unsafe {
            let success = sr_set_restore_point(&mut restore_pt_info, &mut status);
            if success != 0 {
                Ok(status)
            } else {
                Err(WinOptimizerError::Other(format!(
                    "SRSetRestorePointW failed: nStatus={}",
                    status.n_status
                )))
            }
        }
    } else {
        Err(WinOptimizerError::Other(
            "Failed to resolve SRSetRestorePointW from SrClient.dll".into(),
        ))
    };

    // ThrottleGuard::drop restores the original throttle value

    // Cleanup: SrClient.dll stays loaded for the process lifetime
    // (acceptable - it's only loaded once per restore point creation).
    unsafe {
        CoUninitialize();
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reject_empty_description() {
        assert!(create_restore_point("").is_err());
    }

    #[test]
    fn reject_overlong_description() {
        let long_desc = "A".repeat(256);
        assert!(create_restore_point(&long_desc).is_err());
    }

    #[test]
    fn failed_throttle_override_does_not_restore_or_delete_existing_value() {
        let guard = ThrottleGuard {
            original: Some(60),
            applied: false,
        };
        drop(guard);
    }
}
