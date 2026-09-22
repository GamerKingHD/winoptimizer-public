//! Safe process termination - validates a held process handle before terminating.

use crate::core::error::{Result, WinOptimizerError};

pub fn is_protected_process_name(name: &str) -> bool {
    const PROTECTED: &[&str] = &[
        "system",
        "idle",
        "smss.exe",
        "csrss.exe",
        "wininit.exe",
        "winlogon.exe",
        "services.exe",
        "lsass.exe",
        "svchost.exe",
        "dwm.exe",
        "conhost.exe",
        "fontdrvhost.exe",
        "sihost.exe",
        "taskhostw.exe",
        "runtimebroker.exe",
        "searchhost.exe",
        "startmenuexperiencehost.exe",
        "shellexperiencehost.exe",
        "winoptimizer.exe",
        "msmpeng.exe",
        "nissrv.exe",
        "mssense.exe",
        "mpdefendercoreservice.exe",
        "securityhealthservice.exe",
        "securityhealthsystray.exe",
        "memory compression",
    ];
    PROTECTED
        .iter()
        .any(|candidate| name.eq_ignore_ascii_case(candidate))
}

pub fn requires_elevation(pid: u32) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE};

    unsafe {
        match OpenProcess(PROCESS_TERMINATE, false, pid) {
            Ok(handle) => {
                let _ = CloseHandle(handle);
                false
            }
            Err(_) => true,
        }
    }
}

fn normalize_process_name(name: &str) -> &str {
    name.strip_suffix(".exe")
        .or_else(|| name.strip_suffix(".EXE"))
        .unwrap_or(name)
}

fn process_names_match(left: &str, right: &str) -> bool {
    normalize_process_name(left).eq_ignore_ascii_case(normalize_process_name(right))
}

fn filetime_to_unix_seconds(creation: windows::Win32::Foundation::FILETIME) -> Option<u64> {
    let ticks = ((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64;
    let unix_epoch_offset = 11_644_473_600_u64.checked_mul(10_000_000)?;
    let unix_100ns = ticks.checked_sub(unix_epoch_offset)?;
    Some(unix_100ns / 10_000_000)
}

/// Terminate a process only if a handle opened for `pid` still resolves to the
/// displayed identity.
///
/// The identity (PID, creation time, image path) is established from the same
/// handle used for termination, so there is no check-then-act window in which
/// the PID could be recycled between validation and `TerminateProcess`.
pub fn kill_process(
    pid: u32,
    expected_start_time: u64,
    expected_name: &str,
    expected_path: Option<&str>,
) -> Result<()> {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
    use windows::Win32::System::Threading::{
        GetProcessId, GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, TerminateProcess,
        PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    };

    if pid == std::process::id() {
        return Err(WinOptimizerError::Other(
            "Cannot terminate WinOptimizer itself".into(),
        ));
    }
    // Safety check: refuse known critical PIDs
    if pid <= 4 {
        return Err(WinOptimizerError::Other(
            "Cannot kill system idle or system process".into(),
        ));
    }

    struct HandleGuard(HANDLE);
    impl Drop for HandleGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    unsafe {
        let handle = OpenProcess(
            PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            pid,
        )
        .map_err(|error| {
            WinOptimizerError::Other(format!(
                "Cannot open a process handle for PID {pid}; refusing to terminate it: {error}"
            ))
        })?;
        let _guard = HandleGuard(handle);

        if GetProcessId(handle) != pid {
            return Err(WinOptimizerError::Other(
                "Process identity changed before termination; refresh before terminating it".into(),
            ));
        }

        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user).map_err(
            |error| {
                WinOptimizerError::Other(format!(
                    "Cannot establish process creation time for PID {pid}; refusing to terminate it: {error}"
                ))
            },
        )?;
        let actual_start_time = filetime_to_unix_seconds(creation).ok_or_else(|| {
            WinOptimizerError::Other(
                "Process creation time is unavailable; refusing to terminate it".into(),
            )
        })?;
        if actual_start_time != expected_start_time {
            return Err(WinOptimizerError::Other(
                "Process changed since the snapshot was loaded; refresh before terminating it"
                    .into(),
            ));
        }

        let mut buffer = [0u16; 32_768];
        let mut length = buffer.len() as u32;
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
        .map_err(|error| {
            WinOptimizerError::Other(format!(
                "Cannot establish the process image for PID {pid}; refusing to terminate it: {error}"
            ))
        })?;
        let actual_path = String::from_utf16_lossy(&buffer[..length as usize]);
        let path_matches = expected_path
            .map(|expected| actual_path.eq_ignore_ascii_case(expected))
            .unwrap_or(false);
        if !path_matches {
            return Err(WinOptimizerError::Other(
                "Process changed since the snapshot was loaded; refresh before terminating it"
                    .into(),
            ));
        }

        let actual_name = std::path::Path::new(&actual_path)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default();
        if !process_names_match(&actual_name, expected_name) {
            return Err(WinOptimizerError::Other(
                "Process changed since the snapshot was loaded; refresh before terminating it"
                    .into(),
            ));
        }
        if is_protected_process_name(&actual_name) {
            return Err(WinOptimizerError::Other(format!(
                "Cannot kill critical system process: {actual_name}"
            )));
        }

        TerminateProcess(handle, 1).map_err(|error| {
            WinOptimizerError::Other(format!("Failed to kill process {pid}: {error}"))
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process_snapshot(pid: u32) -> Option<(String, Option<String>, u64)> {
        let mut system =
            sysinfo::System::new_with_specifics(sysinfo::RefreshKind::nothing().with_processes(
                sysinfo::ProcessRefreshKind::nothing().with_exe(sysinfo::UpdateKind::Always),
            ));
        system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        system.process(sysinfo::Pid::from_u32(pid)).map(|process| {
            (
                process.name().to_string_lossy().to_string(),
                process.exe().map(|path| path.to_string_lossy().to_string()),
                process.start_time(),
            )
        })
    }

    #[test]
    fn handle_based_kill_validates_identity_before_terminating() {
        use std::process::Command;

        let mut child = Command::new("cmd")
            .args(["/C", "ping -n 60 127.0.0.1 > nul"])
            .spawn()
            .expect("spawn child process");
        let pid = child.id();

        let mut snapshot = None;
        for _ in 0..50 {
            snapshot = process_snapshot(pid);
            if snapshot.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let (name, path, start_time) = snapshot.expect("child process snapshot");
        let path = path.expect("child process image path");

        // A mismatched identity must fail closed and leave the child running.
        assert!(kill_process(pid, start_time + 5, &name, Some(&path)).is_err());
        assert!(child.try_wait().unwrap().is_none());

        // The exact identity terminates the process bound to the validated handle.
        kill_process(pid, start_time, &name, Some(&path)).unwrap();
        for _ in 0..50 {
            if child.try_wait().unwrap().is_some() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("child process was not terminated");
    }
}
