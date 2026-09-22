//! Elevation / UAC token management.
//! Admin rights can be requested at startup or per operation.
//! Uses ShellExecuteExW with "runas" verb to re-launch the app elevated.

use crate::core::elevation_broker::BrokerSession;
use crate::core::error::{Result, WinOptimizerError};
use crate::core::resumable_elevation::ElevationTicket;

use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{ShellExecuteExW, SHELLEXECUTEINFOW};

/// Check whether the current process is running elevated.
pub fn is_elevated() -> bool {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_QUERY};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }

        let mut elevation: u32 = 0;
        let mut return_length: u32 = 0;
        let size = std::mem::size_of::<u32>() as u32;

        let elevated = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut u32 as *mut std::ffi::c_void),
            size,
            &mut return_length,
        )
        .is_ok()
            && elevation != 0;
        let _ = CloseHandle(token);
        elevated
    }
}

pub fn elevated_launch_argument_vector(operation_id: &str) -> Vec<String> {
    vec![
        "--elevated".into(),
        "--resume-operation".into(),
        operation_id.to_string(),
    ]
}

pub fn request_resumable_elevation(
    ticket: &ElevationTicket,
    broker_session: &BrokerSession,
) -> Result<()> {
    if is_elevated() {
        return Ok(());
    }
    uuid::Uuid::parse_str(&ticket.operation_id)
        .and_then(|_| uuid::Uuid::parse_str(&ticket.nonce))
        .map_err(|_| WinOptimizerError::Other("Invalid elevation ticket".into()))?;
    if broker_session.operation_id != ticket.plan_operation_id
        || broker_session.nonce != ticket.nonce
        || broker_session.plan_digest != ticket.plan_digest
    {
        return Err(WinOptimizerError::Other(
            "Elevation broker session does not match the confirmed plan".into(),
        ));
    }
    crate::core::resumable_elevation::store_elevation_handoff(ticket, broker_session)
        .map_err(|error| WinOptimizerError::Other(error.to_string()))?;
    let exe_path = std::env::current_exe()
        .map_err(|e| WinOptimizerError::Other(format!("Cannot get exe path: {e}")))?;
    let exe_str = exe_path.to_string_lossy().to_string();
    let params = elevated_launch_argument_vector(&ticket.operation_id).join(" ");
    let exe_wide: Vec<u16> = exe_str.encode_utf16().chain(std::iter::once(0)).collect();
    let params_wide: Vec<u16> = params.encode_utf16().chain(std::iter::once(0)).collect();
    let verb_wide: Vec<u16> = "runas\0".encode_utf16().collect();
    unsafe {
        let mut sei: SHELLEXECUTEINFOW = std::mem::zeroed();
        sei.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        sei.hwnd = HWND::default();
        sei.lpVerb = PCWSTR(verb_wide.as_ptr());
        sei.lpFile = PCWSTR(exe_wide.as_ptr());
        sei.lpParameters = PCWSTR(params_wide.as_ptr());
        sei.lpDirectory = PCWSTR(std::ptr::null());
        sei.nShow = 1;
        ShellExecuteExW(&mut sei).map_err(|_| {
            WinOptimizerError::Other(
                "Failed to launch elevated process. The request may have been cancelled.".into(),
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevated_launch_args_omit_broker_secrets() {
        let args = elevated_launch_argument_vector("11111111-1111-1111-1111-111111111111");
        let joined = args.join(" ");
        assert_eq!(
            args,
            vec![
                "--elevated".to_string(),
                "--resume-operation".to_string(),
                "11111111-1111-1111-1111-111111111111".to_string(),
            ]
        );
        for secret in [
            "nonce",
            "broker-pipe",
            "resume-plan-digest",
            "ticket-authenticator",
        ] {
            assert!(
                !joined.contains(secret),
                "launch argv must not include {secret}: {joined}"
            );
        }
    }
}
