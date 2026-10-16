//! Programmatic DISM and SFC via native API DLLs.
//!
//! Uses documented command execution for completion-tracked SFC and direct
//! DismAPI calls where the Windows API contract is available. The legacy
//! asynchronous SFC DLL entry point remains isolated as experimental
//! compatibility code.
//!
//! Both DLLs are loaded dynamically at call time - no link-time dependency.

use windows::core::{s, HRESULT, HSTRING};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
};

use crate::core::command_helpers::create_command;

// -- DISM API types -------------------------------------------

/// DismLogLevel enum values.
const _DISM_LOG_ERRORS: u32 = 0;
#[allow(dead_code)]
const _DISM_LOG_ERRORS_WARNINGS: u32 = 1;
const _DISM_LOG_ERRORS_WARNINGS_INFO: u32 = 2;

/// DismImageHealthState enum.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DismImageHealth {
    Healthy = 0,
    Repairable = 1,
    NonRepairable = 2,
}

/// Opaque session handle.
type DismSession = usize;

// Function pointer types
type PfnDismInitialize =
    unsafe extern "system" fn(u32, *const std::ffi::c_void, *const std::ffi::c_void) -> HRESULT;
type PfnDismOpenSession = unsafe extern "system" fn(
    *const u16,
    *const std::ffi::c_void,
    *const std::ffi::c_void,
    *mut DismSession,
) -> HRESULT;
type PfnDismCheckImageHealth = unsafe extern "system" fn(
    DismSession,
    i32,
    *const std::ffi::c_void,
    *mut std::ffi::c_void,
    *mut std::ffi::c_void,
    *mut u32,
) -> HRESULT;
type DismProgressCallback = unsafe extern "system" fn(u32, u32, *mut std::ffi::c_void);
type PfnDismRestoreImageHealth = unsafe extern "system" fn(
    DismSession,
    *const *const u16,
    u32,
    i32,
    windows::Win32::Foundation::HANDLE,
    Option<DismProgressCallback>,
    *mut std::ffi::c_void,
) -> HRESULT;
type PfnDismCloseSession = unsafe extern "system" fn(DismSession) -> HRESULT;
type PfnDismShutdown = unsafe extern "system" fn() -> HRESULT;

// -- DISM API implementation ----------------------------------

/// Check the Windows component store health using DismAPI.dll.
///
/// Returns the image health state (Healthy, Repairable, or NonRepairable).
pub fn dism_check_health() -> Result<DismImageHealth, String> {
    let dll = load_dism_dll()?;

    // Resolve functions
    let (initialize, open_session, check_health, close_session, shutdown) =
        resolve_dism_functions(dll)?;

    unsafe {
        // Initialize
        let hr = initialize(
            _DISM_LOG_ERRORS_WARNINGS_INFO,
            std::ptr::null(),
            std::ptr::null(),
        );
        if hr.is_err() {
            return Err(format!("DismInitialize failed: {:?}", hr));
        }

        // Open session for online image (pass NULL image path)
        let mut session: DismSession = 0;
        let hr = open_session(
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            &mut session,
        );
        if hr.is_err() {
            let _ = shutdown();
            return Err(format!("DismOpenSession failed: {:?}", hr));
        }

        // Check health
        let mut health_state: u32 = 0;
        let hr = check_health(
            session,
            1, // TRUE = scan all files
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut health_state,
        );

        // Always clean up
        let _ = close_session(session);
        let _ = shutdown();

        if hr.is_err() {
            return Err(format!("DismCheckImageHealth failed: {:?}", hr));
        }

        Ok(match health_state {
            0 => DismImageHealth::Healthy,
            1 => DismImageHealth::Repairable,
            _ => DismImageHealth::NonRepairable,
        })
    }
}

/// Repair the Windows component store using DismAPI.dll.
///
/// Equivalent to `DISM /Online /Cleanup-Image /RestoreHealth`.
/// Restores corrupted system files from Windows Update or local repair sources.
pub fn dism_restore_health() -> Result<(), String> {
    let dll = load_dism_dll()?;

    let (initialize, open_session, _, close_session, shutdown) = resolve_dism_functions(dll)?;
    let restore_health: PfnDismRestoreImageHealth = unsafe {
        GetProcAddress(dll, s!("DismRestoreImageHealth"))
            .map(|addr| std::mem::transmute(addr))
            .ok_or_else(|| "DismRestoreImageHealth not found in DismAPI.dll".to_string())?
    };

    unsafe {
        let hr = initialize(
            _DISM_LOG_ERRORS_WARNINGS_INFO,
            std::ptr::null(),
            std::ptr::null(),
        );
        if hr.is_err() {
            return Err(format!("DismInitialize failed: {:?}", hr));
        }

        let mut session: DismSession = 0;
        let hr = open_session(
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            &mut session,
        );
        if hr.is_err() {
            let _ = shutdown();
            return Err(format!("DismOpenSession failed: {:?}", hr));
        }

        let hr = restore_health(
            session,
            std::ptr::null(), // NULL = use default repair sources (Windows Update)
            0,                // No source paths: use the default repair sources
            0,                // FALSE = allow Windows Update as a repair source
            windows::Win32::Foundation::HANDLE::default(),
            None,
            std::ptr::null_mut(),
        );

        let _ = close_session(session);
        let _ = shutdown();

        if hr.is_err() {
            return Err(format!("DismRestoreImageHealth failed: {:?}", hr));
        }
    }

    Ok(())
}

fn load_dism_dll() -> Result<HMODULE, String> {
    // Restrict the search to the system directory so a DismAPI.dll planted in
    // the application directory or on PATH cannot be loaded instead.
    unsafe {
        LoadLibraryExW(
            &HSTRING::from("DismAPI.dll"),
            None,
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
        .map_err(|e| format!("Failed to load DismAPI.dll: {:?}", e))
    }
}

fn resolve_dism_functions(
    dll: HMODULE,
) -> Result<
    (
        PfnDismInitialize,
        PfnDismOpenSession,
        PfnDismCheckImageHealth,
        PfnDismCloseSession,
        PfnDismShutdown,
    ),
    String,
> {
    unsafe {
        let initialize: PfnDismInitialize = GetProcAddress(dll, s!("DismInitialize"))
            .map(|addr| std::mem::transmute(addr))
            .ok_or_else(|| "DismInitialize not found".to_string())?;
        let open_session: PfnDismOpenSession = GetProcAddress(dll, s!("DismOpenSession"))
            .map(|addr| std::mem::transmute(addr))
            .ok_or_else(|| "DismOpenSession not found".to_string())?;
        let check_health: PfnDismCheckImageHealth = GetProcAddress(dll, s!("DismCheckImageHealth"))
            .map(|addr| std::mem::transmute(addr))
            .ok_or_else(|| "DismCheckImageHealth not found".to_string())?;
        let close_session: PfnDismCloseSession = GetProcAddress(dll, s!("DismCloseSession"))
            .map(|addr| std::mem::transmute(addr))
            .ok_or_else(|| "DismCloseSession not found".to_string())?;
        let shutdown: PfnDismShutdown = GetProcAddress(dll, s!("DismShutdown"))
            .map(|addr| std::mem::transmute(addr))
            .ok_or_else(|| "DismShutdown not found".to_string())?;

        Ok((
            initialize,
            open_session,
            check_health,
            close_session,
            shutdown,
        ))
    }
}

// -- SFC implementation ---------------------------------------

// -- Combined repair flow -------------------------------------

/// Run DISM check -> DISM repair (if needed) -> SFC scan.
///
/// This is the recommended repair sequence: DISM restores the
/// component store first, ensuring SFC has valid source files.
pub fn full_system_repair() -> Result<Vec<String>, String> {
    let mut log = Vec::new();

    // Step 1: Check component store health
    log.push("Checking component store health via DismAPI...".to_string());
    match dism_check_health() {
        Ok(DismImageHealth::Healthy) => {
            log.push("Component store is healthy.".to_string());
        }
        Ok(DismImageHealth::Repairable) => {
            log.push("Component store is repairable - starting repair...".to_string());
            dism_restore_health()?;
            log.push("DISM RestoreHealth completed.".to_string());
        }
        Ok(DismImageHealth::NonRepairable) => {
            log.push("Component store is non-repairable. SFC may still help.".to_string());
        }
        Err(e) => {
            log.push(format!("DISM check failed: {}", e));
        }
    }

    // Step 2: Run the documented executable and wait for its completion.
    log.push("Running the documented sfc.exe scan...".to_string());
    let output = create_command("sfc")
        .args(["/scannow"])
        .output()
        .map_err(|error| format!("SFC scan failed to start: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "SFC scan failed with exit code {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    log.push("SFC scan completed; follow-up verification is required.".to_string());

    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_state_values() {
        assert_eq!(DismImageHealth::Healthy as u32, 0);
        assert_eq!(DismImageHealth::Repairable as u32, 1);
        assert_eq!(DismImageHealth::NonRepairable as u32, 2);
    }

    #[test]
    fn dism_api_loads_from_the_system_directory_and_exports_every_entry_point() {
        let dll =
            load_dism_dll().expect("DismAPI.dll must load with a system-directory-only search");
        assert!(resolve_dism_functions(dll).is_ok());
    }
}
