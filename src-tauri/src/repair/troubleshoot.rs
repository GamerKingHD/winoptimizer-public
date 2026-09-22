//! Scriptless core troubleshooting via Win32 APIs.
//!
//! Implements Section 4 of the research document: programmatic system repair
//! without spawning shell commands or batch files.
//!
//! Three subsystems covered:
//! 1. DNS resolver cache flush (dnsapi.dll)
//! 2. Print spooler recovery (stop -> purge -> restart)
//! 3. Windows Update component rebuild (service suspension -> directory rename -> restart)

use crate::startup::services;
use std::path::Path;
use windows::core::{s, HSTRING};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

/// Canonical postcondition evidence tokens emitted by scriptless repairs.
/// Verifiers match these case-insensitively; keep snake_case so lowercased
/// output comparison is exact.
pub const SPOOLER_POSTCONDITION_TOKEN: &str = "postcondition=spooler_state_restored";
pub const WINDOWS_UPDATE_POSTCONDITION_TOKEN: &str =
    "postcondition=windows_update_service_states_restored";

// -- DNS Flush via dnsapi.dll ---------------------------------

/// Programmatic DNS resolver cache flush.
///
/// Calls `DnsFlushResolverCache` from dnsapi.dll directly - no
/// `ipconfig /flushdns` shell spawn.
pub fn dns_flush() -> Result<(), String> {
    unsafe {
        let dnsapi = LoadLibraryW(&HSTRING::from("dnsapi.dll"))
            .map_err(|e| format!("Failed to load dnsapi.dll: {:?}", e))?;

        // DnsFlushResolverCache: BOOL WINAPI DnsFlushResolverCache(void)
        type PfnDnsFlush = unsafe extern "system" fn() -> i32;
        let pfn: PfnDnsFlush = GetProcAddress(dnsapi, s!("DnsFlushResolverCache"))
            .map(|addr| std::mem::transmute(addr))
            .ok_or_else(|| "DnsFlushResolverCache not found in dnsapi.dll".to_string())?;

        let result = pfn();
        if result == 0 {
            return Err("DnsFlushResolverCache returned FALSE".to_string());
        }
    }
    Ok(())
}

// -- Print Spooler Recovery ----------------------------------

/// Recovery the print spooler: stop the service, purge queued spool files,
/// then restart the service cleanly.
///
/// Steps:
/// 1. Stop the Spooler service
/// 2. Delete all files in %systemroot%\System32\spool\PRINTERS
/// 3. Start the Spooler service
pub fn recover_print_spooler() -> Result<String, String> {
    let mut log = Vec::new();
    let was_running = service_is_running("Spooler")?;
    let restore_guard = SpoolerRestoreGuard { was_running };

    // Step 1: Stop Spooler
    log.push("Stopping Print Spooler service...".to_string());
    if was_running {
        stop_service("Spooler")?;
        log.push("Spooler stopped.".to_string());
    } else {
        log.push("Spooler was already stopped.".to_string());
    }

    // Step 2: Purge spool files
    log.push("Purging spool files...".to_string());
    let spool_dir = std::env::var("SystemRoot")
        .map(|r| format!(r"{}\System32\spool\PRINTERS", r))
        .unwrap_or_else(|_| r"C:\Windows\System32\spool\PRINTERS".to_string());

    let spool_path = Path::new(&spool_dir);
    if spool_path.is_dir() {
        match std::fs::read_dir(spool_path) {
            Ok(entries) => {
                let mut deleted = 0u32;
                let mut failed = 0u32;
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        match std::fs::remove_file(&path) {
                            Ok(()) => deleted += 1,
                            Err(e) => {
                                failed += 1;
                                log::debug!("Failed to delete spool file {:?}: {}", path, e);
                            }
                        }
                    }
                }
                log.push(format!("Purged {} files ({} skipped).", deleted, failed));
                if failed > 0 {
                    return Err(format!("Failed to purge {} spool files", failed));
                }
            }
            Err(e) => {
                return Err(format!("Cannot read spool directory: {}", e));
            }
        }
    } else {
        log.push("Spool directory not found - skipping purge.".to_string());
    }

    // Step 3: Start Spooler
    if was_running {
        log.push("Starting Print Spooler service...".to_string());
        start_service("Spooler")?;
        log.push("Spooler started successfully.".to_string());
    }

    if service_is_running("Spooler")? != was_running {
        return Err("Print Spooler state did not return to its pre-operation value".into());
    }
    log.push(SPOOLER_POSTCONDITION_TOKEN.into());

    std::mem::forget(restore_guard);
    Ok(log.join("\n"))
}

struct SpoolerRestoreGuard {
    was_running: bool,
}

impl Drop for SpoolerRestoreGuard {
    fn drop(&mut self) {
        if self.was_running {
            let _ = start_service("Spooler");
        }
    }
}

fn service_is_running(service: &str) -> Result<bool, String> {
    Ok(services::inspect(service)?
        .status
        .eq_ignore_ascii_case("Running"))
}

fn stop_service(service: &str) -> Result<(), String> {
    services::control_service(service, "stop")
}

fn start_service(service: &str) -> Result<(), String> {
    services::control_service(service, "start")
}

// -- Windows Update Component Rebuild ------------------------

/// Rebuild the Windows Update cache components.
///
/// Instead of running fragile batch scripts, this function manages the
/// update services and cache directories programmatically:
///
/// 1. Stop: wuauserv, cryptsvc, bits, msiserver
/// 2. Rename: %systemroot%\SoftwareDistribution -> SoftwareDistribution.old
/// 3. Rename: %systemroot%\System32\catroot2 -> catroot2.old
/// 4. Start: wuauserv, cryptsvc, bits, msiserver
///
/// After this, Windows Update will rebuild clean cache structures on next check.
pub fn rebuild_windows_update() -> Result<String, String> {
    let mut log = Vec::new();
    let services = ["wuauserv", "cryptsvc", "bits", "msiserver"];
    let snapshots = services
        .iter()
        .map(|name| query_service_snapshot(name))
        .collect::<Result<Vec<_>, _>>()?;
    let restore_guard = ServiceRestoreGuard {
        snapshots: snapshots.clone(),
    };

    // Step 1: Stop all update-related services
    log.push("Stopping Windows Update services...".to_string());
    for snapshot in &snapshots {
        if !snapshot.was_running {
            log.push(format!("  {} (already stopped)", snapshot.name));
            continue;
        }
        stop_service(snapshot.name)?;
        log.push(format!("  Stopped: {}", snapshot.name));
    }

    // Step 2: Rename SoftwareDistribution
    let sysroot = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());

    let sd_path = format!(r"{}\SoftwareDistribution", sysroot);
    let sd_old = format!(r"{}\SoftwareDistribution.old", sysroot);

    log.push("Renaming SoftwareDistribution...".to_string());
    if let Err(e) = rename_dir_if_exists(&sd_path, &sd_old, "SoftwareDistribution") {
        return Err(format!("Failed to rename SoftwareDistribution: {}", e));
    }
    log.push("  SoftwareDistribution renamed.".to_string());

    // Step 3: Rename catroot2
    let catroot_path = format!(r"{}\System32\catroot2", sysroot);
    let catroot_old = format!(r"{}\System32\catroot2.old", sysroot);

    log.push("Renaming catroot2...".to_string());
    if let Err(e) = rename_dir_if_exists(&catroot_path, &catroot_old, "catroot2") {
        return Err(format!("Failed to rename catroot2: {}", e));
    }
    log.push("  catroot2 renamed.".to_string());

    // Step 4: Restore the original service configuration and running state.
    restore_services(&snapshots, &mut log)?;
    if !service_states_match(&snapshots)? {
        return Err(
            "Windows Update service state did not return to its pre-operation values".into(),
        );
    }
    log.push(WINDOWS_UPDATE_POSTCONDITION_TOKEN.into());
    std::mem::forget(restore_guard);

    Ok(log.join("\n"))
}

#[derive(Clone)]
struct ServiceSnapshot {
    name: &'static str,
    was_running: bool,
    start_type: String,
}

struct ServiceRestoreGuard {
    snapshots: Vec<ServiceSnapshot>,
}

impl Drop for ServiceRestoreGuard {
    fn drop(&mut self) {
        let mut log = Vec::new();
        let _ = restore_services(&self.snapshots, &mut log);
    }
}

fn query_service_snapshot(name: &'static str) -> Result<ServiceSnapshot, String> {
    let current = services::inspect(name)?;
    let start_type = normalize_service_start_type(&current.start_type)
        .ok_or_else(|| format!("Could not determine start type for {}", name))?
        .to_string();
    Ok(ServiceSnapshot {
        name,
        was_running: current.status.eq_ignore_ascii_case("Running"),
        start_type,
    })
}

fn normalize_service_start_type(start_type: &str) -> Option<&'static str> {
    if start_type.eq_ignore_ascii_case("auto") {
        Some("auto")
    } else if start_type.eq_ignore_ascii_case("manual") || start_type.eq_ignore_ascii_case("demand")
    {
        Some("demand")
    } else if start_type.eq_ignore_ascii_case("disabled") {
        Some("disabled")
    } else {
        None
    }
}

fn restore_services(snapshots: &[ServiceSnapshot], log: &mut Vec<String>) -> Result<(), String> {
    log.push("Restoring Windows Update service state...".to_string());
    for snapshot in snapshots {
        services::set_start_type(snapshot.name, &snapshot.start_type)?;
        if snapshot.was_running {
            start_service(snapshot.name)?;
            log.push(format!("  Restored running: {}", snapshot.name));
        } else {
            log.push(format!("  Preserved stopped: {}", snapshot.name));
        }
    }
    Ok(())
}

fn service_states_match(snapshots: &[ServiceSnapshot]) -> Result<bool, String> {
    for snapshot in snapshots {
        let current = services::inspect(snapshot.name)?;
        let current_start_type = normalize_service_start_type(&current.start_type)
            .ok_or_else(|| format!("Could not determine start type for {}", snapshot.name))?;
        if current.status.eq_ignore_ascii_case("Running") != snapshot.was_running
            || current_start_type != snapshot.start_type
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Rename a directory if it exists. If the old directory already exists,
/// remove it first. Returns Ok(()) on success or if source doesn't exist.
fn rename_dir_if_exists(path: &str, old_path: &str, label: &str) -> Result<(), String> {
    let p = Path::new(path);
    if !p.is_dir() {
        return Ok(()); // Nothing to rename
    }

    let old = Path::new(old_path);
    if old.exists() {
        // Remove old backup first
        if old.is_dir() {
            std::fs::remove_dir_all(old)
                .map_err(|e| format!("Failed to remove old {}.old: {}", label, e))?;
        } else {
            std::fs::remove_file(old)
                .map_err(|e| format!("Failed to remove old {}.old: {}", label, e))?;
        }
    }

    std::fs::rename(p, old).map_err(|e| format!("Failed to rename {}: {}", label, e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_dir_if_exists_handles_missing_path() {
        let result = rename_dir_if_exists(
            r"C:\Definitely\Does\Not\Exist_12345",
            r"C:\Definitely\Does\Not\Exist_12345.old",
            "TestDir",
        );
        assert!(
            result.is_ok(),
            "Missing path should return Ok: {:?}",
            result
        );
    }

    #[test]
    fn normalizes_service_start_type_labels() {
        assert_eq!(normalize_service_start_type("Auto"), Some("auto"));
        assert_eq!(normalize_service_start_type("Manual"), Some("demand"));
        assert_eq!(normalize_service_start_type("Disabled"), Some("disabled"));
        assert_eq!(normalize_service_start_type("Boot"), None);
    }
}
