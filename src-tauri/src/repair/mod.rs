//! Guided Windows repair actions.
//!
//! Repair actions are allowlisted and logged. The frontend can preview the
//! exact command before execution.
//!
//! Includes programmatic DISM (DismAPI.dll) and SFC (sfc_os.dll) actions
//! alongside traditional CLI-based repair actions.

pub mod dism_sfc;
pub mod troubleshoot;

use crate::core::bounded_process::{run_bounded, ProcessTermination, RunControl, TreeContainment};
use crate::core::error::{Result, WinOptimizerError};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairVerificationScope {
    ExecutionAcknowledgement,
    StatePostcondition,
    HealthPostcondition,
    UserSymptom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairAction {
    pub id: String,
    pub category: String,
    pub name: String,
    pub description: String,
    pub command: String,
    pub args: Vec<String>,
    pub estimated_minutes: u32,
    pub requires_admin: bool,
    pub mutates_system: bool,
    pub restart_required: bool,
    pub risk: String,
    #[serde(default)]
    pub limitations: String,
}

/// Structured Job Object containment state for a repair command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairTreeContainment {
    /// A Job Object owned the process tree; cancellation/timeout terminates it.
    Contained,
    /// Job Object assignment failed; only the direct child is guaranteed to be
    /// terminated, so the result must not claim full tree termination.
    Unavailable,
    /// The action was not a native child process (for example a DLL/API call).
    NotApplicable,
}

/// Structured reason a repair command stopped. Never inferred from text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairProcessTermination {
    Completed,
    NonZeroExit,
    Cancelled,
    TimedOut,
}

fn default_repair_termination() -> RepairProcessTermination {
    RepairProcessTermination::Completed
}

fn default_repair_containment() -> RepairTreeContainment {
    // Older serialized logs carry no containment evidence, so the honest
    // default is that containment is not known rather than assumed.
    RepairTreeContainment::Unavailable
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairExecutionLog {
    pub id: String,
    pub action_id: String,
    pub command: String,
    pub args: Vec<String>,
    pub started_at: String,
    pub ended_at: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub requires_admin: bool,
    pub mutates_system: bool,
    pub restart_required: bool,
    /// Structured termination reason, independent of captured output text.
    #[serde(default = "default_repair_termination")]
    pub termination: RepairProcessTermination,
    /// Structured process-tree containment state.
    #[serde(default = "default_repair_containment")]
    pub tree_containment: RepairTreeContainment,
}

fn map_termination(termination: ProcessTermination) -> RepairProcessTermination {
    match termination {
        ProcessTermination::Completed => RepairProcessTermination::Completed,
        ProcessTermination::NonZeroExit => RepairProcessTermination::NonZeroExit,
        ProcessTermination::Cancelled => RepairProcessTermination::Cancelled,
        ProcessTermination::TimedOut => RepairProcessTermination::TimedOut,
    }
}

fn map_containment(containment: TreeContainment) -> RepairTreeContainment {
    match containment {
        TreeContainment::Contained => RepairTreeContainment::Contained,
        TreeContainment::Unavailable => RepairTreeContainment::Unavailable,
        TreeContainment::NotApplicable => RepairTreeContainment::NotApplicable,
    }
}

pub fn list_actions() -> Vec<RepairAction> {
    vec![
        action(
            "sfc_verify",
            "System Files",
            "Verify system files",
            "Runs System File Checker in read-only mode.",
            "sfc",
            &["/verifyonly"],
            10,
            true,
            false,
            false,
            "low",
        ),
        action(
            "dism_check_health",
            "System Image",
            "Check component store health",
            "Checks whether the Windows image is flagged as healthy, repairable, or non-repairable.",
            "DISM",
            &["/Online", "/Cleanup-Image", "/CheckHealth"],
            5,
            true,
            false,
            false,
            "low",
        ),
        action(
            "sfc_scan",
            "System Files",
            "Check protected system files",
            "Runs System File Checker (sfc /scannow) to scan protected Windows files and attempt repair when possible. Completion means the command and its configured verification step finished without the failure signal WinOptimizer checks; it does not prove that specific files were repaired or that system integrity was restored.",
            "sfc",
            &["/scannow"],
            30,
            true,
            true,
            false,
            "medium",
        ),
        action(
            "dism_restore_health",
            "System Image",
            "Service the component store",
            "Runs DISM RestoreHealth for the online Windows image. Completion means the DISM operation finished; component-store health is reported separately by the follow-up health check.",
            "DISM",
            &["/Online", "/Cleanup-Image", "/RestoreHealth"],
            45,
            true,
            true,
            false,
            "medium",
        ),
        action(
            "windows_update_status",
            "Windows Update",
            "Check Windows Update services",
            "Queries the Windows Update service state.",
            "SCM",
            &["query", "wuauserv"],
            1,
            false,
            false,
            false,
            "low",
        ),
        action(
            "network_status",
            "Network",
            "Check network configuration",
            "Collects the current network configuration without changing networking state.",
            "powershell",
            &[
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                "$ErrorActionPreference = 'Stop'; try { $network_config = Get-NetIPConfiguration | Select-Object -First 1 } catch { Write-Error $_; exit 1 }; if ($null -eq $network_config) { 'NetworkConfiguration=Missing' } else { 'NetworkConfiguration=Present'; 'InterfaceAlias=' + $network_config.InterfaceAlias; 'IPv4DefaultGateway=' + (($network_config.IPv4DefaultGateway | Select-Object -First 1).NextHop); 'DNSServer=' + (($network_config.DNSServer.ServerAddresses) -join ',') }",
            ],
            1,
            false,
            false,
            false,
            "low",
        ),
        action(
            "printing_status",
            "Printing",
            "Check Print Spooler status",
            "Queries the Print Spooler service state without changing the service or its queue.",
            "SCM",
            &["query", "Spooler"],
            1,
            false,
            false,
            false,
            "low",
        ),
        action(
            "dns_flush",
            "Network",
            "Flush DNS resolver cache",
            "Clears the local DNS resolver cache.",
            "ipconfig",
            &["/flushdns"],
            1,
            false,
            true,
            false,
            "low",
        ),
        action(
            "winsock_reset",
            "Network",
            "Reset Winsock catalog",
            "Resets Winsock providers. A restart is required.",
            "netsh",
            &["winsock", "reset"],
            2,
            true,
            true,
            true,
            "medium",
        ),
        action(
            "tcpip_reset",
            "Network",
            "Reset TCP/IP stack",
            "Resets TCP/IP configuration. A restart is required.",
            "netsh",
            &["int", "ip", "reset"],
            2,
            true,
            true,
            true,
            "medium",
        ),
        action(
            "defender_status",
            "Security",
            "Read Defender status",
            "Shows Microsoft Defender protection and signature status.",
            "powershell",
            &["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", "Get-MpComputerStatus | Select-Object AMServiceEnabled,AntivirusEnabled,RealTimeProtectionEnabled,AntispywareSignatureLastUpdated,NISSignatureLastUpdated | Format-List"],
            1,
            false,
            false,
            false,
            "low",
        ),
        action(
            "defender_update",
            "Security",
            "Update Defender signatures",
            "Updates Microsoft Defender security intelligence.",
            "powershell",
            &["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", "Update-MpSignature"],
            5,
            false,
            true,
            false,
            "low",
        ),
        action(
            "defender_quick_scan",
            "Security",
            "Run Defender quick scan",
            "Starts a Microsoft Defender quick scan.",
            "powershell",
            &["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", "Start-MpScan -ScanType QuickScan"],
            15,
            false,
            true,
            false,
            "medium",
        ),
        action(
            "appx_status",
            "Microsoft Store / AppX",
            "Check Microsoft Store package status",
            "Checks whether the current user has a registered Microsoft Store AppX package and reports its package fields.",
            "powershell",
            &[
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                "$ErrorActionPreference = 'Stop'; try { $package = Get-AppxPackage -Name Microsoft.WindowsStore -ErrorAction Stop | Select-Object -First 1 } catch { Write-Error $_; exit 1 }; if ($null -eq $package) { 'MicrosoftStorePackage=Missing' } else { 'MicrosoftStorePackage=Present'; 'PackageName=' + $package.Name; 'PackageFullName=' + $package.PackageFullName; 'PackageStatus=' + $package.Status }",
            ],
            1,
            false,
            false,
            false,
            "low",
        ),
        action(
            "search_indexing_status",
            "Search / indexing",
            "Check Windows Search status",
            "Queries the Windows Search service state without changing indexing configuration.",
            "SCM",
            &["query", "WSearch"],
            1,
            false,
            false,
            false,
            "low",
        ),
        action(
            "recovery_settings",
            "Recovery",
            "Open Windows recovery settings",
            "Opens the official Windows recovery settings page. WinOptimizer does not automate reset or reinstall flows.",
            "cmd",
            &["/C", "start", "ms-settings:recovery"],
            1,
            false,
            false,
            false,
            "low",
        ),
        // -- Programmatic DISM + SFC (Section 2) --------------
        action(
            "dism_check_health_api",
            "System Image",
            "Check component store health (API)",
            "Uses DismAPI.dll directly - no CLI. Checks whether the Windows image is flagged as healthy, repairable, or non-repairable.",
            "dism_api",
            &["check_health"],
            2,
            true,
            false,
            false,
            "low",
        ),
        action(
            "dism_restore_health_api",
            "System Image",
            "Service the component store (API)",
            "Uses DismAPI.dll directly. Restores the Windows component store from Windows Update or local repair sources. Completion means the DISM operation finished; component-store health is reported separately by the follow-up health check.",
            "dism_api",
            &["restore_health"],
            30,
            true,
            true,
            false,
            "medium",
        ),
        // -- Scriptless Troubleshooting (Section 4) -----------
        action(
            "dns_flush_api",
            "Network",
            "Flush DNS cache (API)",
            "Uses DnsFlushResolverCache from dnsapi.dll directly - no ipconfig shell spawn.",
            "dns_api",
            &["flush"],
            1,
            false,
            true,
            false,
            "low",
        ),
        action(
            "recover_print_spooler",
            "Printing",
            "Recover Print Spooler",
            "Stops the Spooler service, purges queued print files from %systemroot%\\System32\\spool\\PRINTERS, then restarts the service cleanly.",
            "troubleshoot",
            &["spooler"],
            3,
            true,
            true,
            false,
            "medium",
        ),
        action(
            "rebuild_windows_update",
            "Windows Update",
            "Rebuild Windows Update cache",
            "Stops wuauserv/cryptsvc/bits/msiserver, renames SoftwareDistribution and catroot2 to .old, then restarts services. Windows rebuilds clean cache on next check.",
            "troubleshoot",
            &["wu_rebuild"],
            10,
            true,
            true,
            false,
            "advanced",
        ),
    ]
}

pub(crate) fn diagnostic_verification_method(action_id: &str) -> Option<&'static str> {
    match action_id {
        "sfc_verify" => Some(
            "`sfc /verifyonly` exits 0 and prints diagnostic output. This acknowledges a completed verification pass; Windows does not document SFC exit codes, so it does not certify that no integrity violation exists.",
        ),
        "dism_check_health" => Some("DISM returns a recognized component-store health state."),
        "windows_update_status" => Some("SCM returns the wuauserv service identity and state."),
        "network_status" => Some(
            "Output contains NetworkConfiguration=Present with InterfaceAlias, IPv4DefaultGateway, and DNSServer markers, or NetworkConfiguration=Missing.",
        ),
        "printing_status" => Some("Output contains SERVICE_NAME: Spooler and a STATE field."),
        "defender_status" => Some("Defender returns named protection-state fields."),
        "appx_status" => Some(
            "Output contains MicrosoftStorePackage=Present with PackageName, PackageFullName, and PackageStatus markers, or MicrosoftStorePackage=Missing.",
        ),
        "search_indexing_status" => {
            Some("Output contains SERVICE_NAME: WSearch and a STATE field.")
        },
        "dism_check_health_api" => {
            Some("DismAPI returns a recognized component-store health state.")
        }
        _ => None,
    }
}

pub(crate) fn diagnostic_verification_scope(action_id: &str) -> Option<RepairVerificationScope> {
    match action_id {
        "sfc_verify" | "dism_check_health" | "dism_check_health_api" => {
            Some(RepairVerificationScope::HealthPostcondition)
        }
        "windows_update_status"
        | "network_status"
        | "printing_status"
        | "defender_status"
        | "appx_status"
        | "search_indexing_status" => Some(RepairVerificationScope::StatePostcondition),
        _ => None,
    }
}

pub(crate) fn maintenance_verification_scope(action_id: &str) -> Option<RepairVerificationScope> {
    match action_id {
        "sfc_scan" | "dism_restore_health" | "dism_restore_health_api" => {
            Some(RepairVerificationScope::HealthPostcondition)
        }
        "recover_print_spooler" | "rebuild_windows_update" => {
            Some(RepairVerificationScope::StatePostcondition)
        }
        "dns_flush"
        | "winsock_reset"
        | "tcpip_reset"
        | "defender_update"
        | "defender_quick_scan"
        | "dns_flush_api" => Some(RepairVerificationScope::ExecutionAcknowledgement),
        _ => None,
    }
}

pub(crate) fn maintenance_verification_method(action_id: &str) -> Option<&'static str> {
    match action_id {
        "sfc_scan" => Some(
            "The repair command exits 0 and a controlled follow-up `sfc /verifyonly` pass also exits 0. This proves the verification pass completed without reporting failure; Windows does not document SFC exit codes, so it does not prove that any protected file was repaired.",
        ),
        "dism_restore_health" | "dism_restore_health_api" => {
            Some("A follow-up DISM health check reports Healthy after RestoreHealth.")
        }
        "recover_print_spooler" => Some(
            "The Print Spooler state is checked after recovery and must match the state captured before the operation.",
        ),
        "rebuild_windows_update" => Some(
            "All four Windows Update service states and start types are checked after the cache rebuild and must match their captured pre-operation values.",
        ),
        "dns_flush" => Some(
            "ipconfig /flushdns exits successfully; this acknowledges the command only and does not prove connectivity or symptom resolution.",
        ),
        "winsock_reset" => Some(
            "netsh winsock reset exits successfully; this acknowledges the reset only and requires the documented restart.",
        ),
        "tcpip_reset" => Some(
            "netsh int ip reset exits successfully; this acknowledges the reset only and requires the documented restart.",
        ),
        "defender_update" => Some(
            "Update-MpSignature exits successfully; this acknowledges signature-update execution and does not prove protection or threat resolution.",
        ),
        "defender_quick_scan" => Some(
            "Start-MpScan exits successfully; this acknowledges scan execution and does not claim that the user symptom or threat state is fixed.",
        ),
        "dns_flush_api" => Some(
            "DnsFlushResolverCache returns TRUE; this acknowledges cache-flush execution only.",
        ),
        _ => None,
    }
}

pub(crate) fn verification_method(action_id: &str) -> Option<&'static str> {
    diagnostic_verification_method(action_id).or_else(|| maintenance_verification_method(action_id))
}

pub(crate) fn verification_scope(action_id: &str) -> Option<RepairVerificationScope> {
    diagnostic_verification_scope(action_id).or_else(|| maintenance_verification_scope(action_id))
}

#[cfg(test)]
pub(crate) fn supported_diagnostic_actions() -> Vec<RepairAction> {
    list_actions()
        .into_iter()
        .filter(|action| {
            !action.mutates_system && diagnostic_verification_method(&action.id).is_some()
        })
        .collect()
}

pub(crate) fn supported_repair_actions() -> Vec<RepairAction> {
    list_actions()
        .into_iter()
        .filter(|action| verification_method(&action.id).is_some())
        .collect()
}

pub fn run_action(action_id: &str) -> Result<RepairExecutionLog> {
    run_action_with_control(action_id, &RunControl::unbounded_cancel())
}

/// Run a read-only repair diagnostic under an explicit cancellation/time bound.
pub(crate) fn run_action_with_control(
    action_id: &str,
    control: &RunControl,
) -> Result<RepairExecutionLog> {
    run_action_internal(action_id, false, control)
}

/// Execute a repair only after its immutable operation plan has passed the
/// kernel validation and durable journal gates, under an explicit
/// cancellation/time bound so a hung SFC/DISM process cannot leave the
/// operation executing indefinitely.
pub(crate) fn run_action_through_kernel_with_control(
    action_id: &str,
    control: &RunControl,
) -> Result<RepairExecutionLog> {
    run_action_internal(action_id, true, control)
}

fn run_action_internal(
    action_id: &str,
    allow_mutation: bool,
    control: &RunControl,
) -> Result<RepairExecutionLog> {
    let action = list_actions()
        .into_iter()
        .find(|a| a.id == action_id)
        .ok_or_else(|| WinOptimizerError::Other(format!("Unknown repair action: {}", action_id)))?;

    validate_action(&action)?;
    if action.mutates_system
        && (!allow_mutation || maintenance_verification_method(&action.id).is_none())
    {
        return Err(WinOptimizerError::Other(if allow_mutation {
            "Repair action has no implemented maintenance verification contract".into()
        } else {
            "Mutating repair actions must run through the operation kernel".into()
        }));
    }
    if !action.mutates_system && diagnostic_verification_method(&action.id).is_none() {
        return Err(WinOptimizerError::Other(
            "Repair action is not a supported read-only diagnostic".into(),
        ));
    }
    if action.requires_admin && !crate::core::elevation::is_elevated() {
        return Err(WinOptimizerError::ElevationRequired);
    }

    let started_at = Utc::now().to_rfc3339();

    let mut termination = RepairProcessTermination::Completed;
    let mut tree_containment = RepairTreeContainment::NotApplicable;

    // Handle programmatic API actions
    let (exit_code, stdout, stderr): (Option<i32>, String, String) = match action.command.as_str() {
        "dism_api" => {
            let sub_action = action.args.first().map(|s| s.as_str()).unwrap_or("");
            match sub_action {
                "check_health" => match dism_sfc::dism_check_health() {
                    Ok(state) => (
                        Some(0),
                        format!("Component store health: {:?}", state),
                        String::new(),
                    ),
                    Err(e) => (Some(1), String::new(), e),
                },
                "restore_health" => match dism_sfc::dism_restore_health() {
                    Ok(()) => (
                        Some(0),
                        "DISM RestoreHealth completed successfully.".into(),
                        String::new(),
                    ),
                    Err(e) => (Some(1), String::new(), e),
                },
                _ => (
                    Some(1),
                    String::new(),
                    format!("Unknown dism_api sub-action: {}", sub_action),
                ),
            }
        }
        "dns_api" => match troubleshoot::dns_flush() {
            Ok(()) => (
                Some(0),
                "DNS resolver cache flushed successfully.".into(),
                String::new(),
            ),
            Err(e) => (Some(1), String::new(), e),
        },
        "SCM" => {
            let service_name = action.args.get(1).map(String::as_str).unwrap_or("");
            match service_status_diagnostic(service_name) {
                Ok(output) => (Some(0), output, String::new()),
                Err(e) => (Some(1), String::new(), e),
            }
        }
        "troubleshoot" => {
            let sub_action = action.args.first().map(|s| s.as_str()).unwrap_or("");
            match sub_action {
                "spooler" => match troubleshoot::recover_print_spooler() {
                    Ok(log) => (Some(0), log, String::new()),
                    Err(e) => (Some(1), String::new(), e),
                },
                "wu_rebuild" => match troubleshoot::rebuild_windows_update() {
                    Ok(log) => (Some(0), log, String::new()),
                    Err(e) => (Some(1), String::new(), e),
                },
                _ => (
                    Some(1),
                    String::new(),
                    format!("Unknown troubleshoot sub-action: {}", sub_action),
                ),
            }
        }
        _ => {
            // Standard CLI execution with process-tree ownership so a hung
            // SFC/DISM command is terminated instead of running forever.
            let output = run_bounded(&action.command, &action.args, control).map_err(|e| {
                WinOptimizerError::Other(format!("Failed to run {}: {}", action.command, e))
            })?;
            termination = map_termination(output.termination());
            tree_containment = map_containment(output.containment);
            let stderr = if output.timed_out {
                format!("{}\nCommandTimedOut=true", trim_output(&output.stderr))
            } else if output.cancelled {
                format!("{}\nCommandCancelled=true", trim_output(&output.stderr))
            } else if matches!(output.containment, TreeContainment::Unavailable) {
                format!(
                    "{}\nProcessTreeContainment=unavailable",
                    trim_output(&output.stderr)
                )
            } else {
                trim_output(&output.stderr)
            };
            (output.exit_code, trim_output(&output.stdout), stderr)
        }
    };

    if matches!(tree_containment, RepairTreeContainment::NotApplicable) {
        termination = if exit_code == Some(0) {
            RepairProcessTermination::Completed
        } else {
            RepairProcessTermination::NonZeroExit
        };
    }

    let (stdout, stderr) = if action.mutates_system {
        append_maintenance_verification(&action.id, exit_code, stdout, stderr, control)
    } else {
        (stdout, stderr)
    };

    let ended_at = Utc::now().to_rfc3339();

    let log = RepairExecutionLog {
        id: uuid::Uuid::new_v4().to_string(),
        action_id: action.id.clone(),
        command: action.command.clone(),
        args: action.args.clone(),
        started_at,
        ended_at,
        exit_code,
        stdout,
        stderr,
        requires_admin: action.requires_admin,
        mutates_system: action.mutates_system,
        restart_required: action.restart_required,
        termination,
        tree_containment,
    };

    append_log(&log)?;
    Ok(log)
}

/// Result of running one follow-up verification command:
/// `(exit code, stdout, stderr)`.
pub(crate) type MaintenanceVerificationOutput = (Option<i32>, String, String);

fn run_verification_command(
    command: &str,
    args: &[&str],
    control: &RunControl,
) -> std::io::Result<MaintenanceVerificationOutput> {
    let owned: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    let output = run_bounded(command, &owned, control)?;
    if output.timed_out {
        return Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "verification command exceeded its execution bound",
        ));
    }
    if output.cancelled {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "verification command was cancelled",
        ));
    }
    Ok((
        output.exit_code,
        trim_output(&output.stdout),
        trim_output(&output.stderr),
    ))
}

fn append_maintenance_verification(
    action_id: &str,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    control: &RunControl,
) -> (String, String) {
    append_maintenance_verification_with(action_id, exit_code, stdout, stderr, |command, args| {
        run_verification_command(command, args, control)
    })
}

/// Emitter for maintenance verification evidence.
///
/// The follow-up command runner is injectable so the emitter → verifier
/// contract can be exercised without invoking a real repair command.
pub(crate) fn append_maintenance_verification_with<F>(
    action_id: &str,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    mut run: F,
) -> (String, String)
where
    F: FnMut(&str, &[&str]) -> std::io::Result<MaintenanceVerificationOutput>,
{
    if exit_code != Some(0) {
        return (
            stdout,
            format!(
                "{stderr}\nVerificationSkipped=command_failed",
                stderr = stderr
            ),
        );
    }

    match action_id {
        // `sfc /verifyonly` output is localized, so only the structured
        // follow-up process exit code may be treated as verification evidence.
        // Windows does not document SFC exit codes; exit 0 is reported as a
        // completed verification pass, not as certified repair.
        "sfc_scan" => match run("sfc", &["/verifyonly"]) {
            Ok((verification_exit_code, verification_stdout, verification_stderr)) => (
                format!("{stdout}\nVerificationExitCode={verification_exit_code:?}\n{verification_stdout}"),
                format!("{stderr}\n{verification_stderr}"),
            ),
            Err(error) => (stdout, format!("{stderr}\nVerificationError={error}")),
        },
        "dism_restore_health" | "dism_restore_health_api" => {
            match dism_sfc::dism_check_health() {
                Ok(state) => (
                    format!("{stdout}\nVerificationComponentStoreHealth={state:?}"),
                    stderr,
                ),
                Err(error) => (stdout, format!("{stderr}\nVerificationError={error}")),
            }
        }
        _ => (
            format!(
                "{stdout}\nVerificationScope=execution_acknowledgement",
                stdout = stdout
            ),
            stderr,
        ),
    }
}

fn service_status_diagnostic(service_name: &str) -> std::result::Result<String, String> {
    if service_name.is_empty() {
        return Err("SCM diagnostic requires a service name".into());
    }
    let service = crate::startup::services::inspect(service_name)?;
    Ok(format!(
        "SERVICE_NAME: {}\nSTATE: {}\nSTART_TYPE: {}",
        service.name.to_ascii_uppercase(),
        service.status.to_ascii_uppercase(),
        service.start_type.to_ascii_uppercase()
    ))
}

#[allow(clippy::too_many_arguments)]
fn action(
    id: &str,
    category: &str,
    name: &str,
    description: &str,
    command: &str,
    args: &[&str],
    estimated_minutes: u32,
    requires_admin: bool,
    mutates_system: bool,
    restart_required: bool,
    risk: &str,
) -> RepairAction {
    RepairAction {
        id: id.to_string(),
        category: category.to_string(),
        name: name.to_string(),
        description: description.to_string(),
        command: command.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        estimated_minutes,
        requires_admin,
        mutates_system,
        restart_required,
        risk: risk.to_string(),
        limitations: limitations(id, mutates_system).into(),
    }
}

fn limitations(action_id: &str, mutates_system: bool) -> &'static str {
    if mutates_system {
        return maintenance_verification_method(action_id).map_or(
            "No maintenance verification contract is implemented; this action remains Experimental and unavailable from the guarded Repair Center.",
            |_| "This action mutates Windows and has no exact automatic undo. Review its explicit verification scope; a verified postcondition does not prove the user symptom is fixed.",
        );
    }

    match action_id {
        "network_status" => {
            "Reports one current network configuration only; it does not test reachability, DNS resolution, proxy or VPN behavior, or change networking state."
        },
        "printing_status" => {
            "Reports only Print Spooler service state; it does not inspect printer drivers, queues, ports, or validate that a print job can complete."
        },
        "appx_status" => {
            "Checks only the current user's Microsoft Store package registration; it does not repair, reinstall, provision, or validate Store launch behavior."
        },
        "search_indexing_status" => {
            "Reports only Windows Search service state; it does not inspect the index database, indexed locations, catalog health, or search-client behavior."
        },
        "recovery_settings" => {
            "Opens Windows settings only; WinOptimizer does not automate reset or reinstall flows."
        }
        _ => "Read-only evidence can describe the current state but does not repair Windows.",
    }
}

fn validate_action(action: &RepairAction) -> Result<()> {
    if action.id.is_empty() {
        return Err(WinOptimizerError::Other("Repair action id is empty".into()));
    }
    if !matches!(
        action.command.as_str(),
        "sfc"
            | "DISM"
            | "SCM"
            | "ipconfig"
            | "netsh"
            | "powershell"
            | "cmd"
            | "dism_api"
            | "dns_api"
            | "troubleshoot"
    ) {
        return Err(WinOptimizerError::Other(
            "Repair command is not allowlisted".into(),
        ));
    }
    if action
        .args
        .iter()
        .any(|a| a.contains('\0') || a.contains('\n') || a.contains('\r'))
    {
        return Err(WinOptimizerError::Other(
            "Repair command contains invalid arguments".into(),
        ));
    }
    Ok(())
}

fn append_log(log: &RepairExecutionLog) -> Result<()> {
    let path = log_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut line = serde_json::to_string(log)?;
    line.push('\n');
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(line.as_bytes())?;
    Ok(())
}

fn log_path() -> Result<PathBuf> {
    Ok(crate::core::paths::AppPaths::installed()?
        .logs
        .join("repair")
        .join("repair_logs.jsonl"))
}

fn trim_output(output: &str) -> String {
    const MAX: usize = 20_000;
    if output.len() <= MAX {
        return output.to_string();
    }
    let start = output.len().saturating_sub(MAX);
    format!("[truncated]\n{}", &output[start..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_builtin_repair_actions_are_allowlisted() {
        for action in list_actions() {
            validate_action(&action).unwrap();
        }
    }

    #[test]
    fn rejects_unknown_repair_command() {
        let mut action = list_actions().remove(0);
        action.command = "bad.exe".into();
        assert!(validate_action(&action).is_err());
    }

    #[test]
    fn rejects_control_characters_in_arguments() {
        let mut action = list_actions().remove(0);
        action.args.push("bad\narg".into());
        assert!(validate_action(&action).is_err());
    }

    #[test]
    fn direct_mutating_repair_dispatch_is_blocked() {
        let error = run_action("sfc_scan").unwrap_err();
        assert!(error
            .to_string()
            .contains("must run through the operation kernel"));
    }

    #[test]
    fn direct_non_diagnostic_repair_dispatch_is_blocked() {
        let error = run_action("recovery_settings").unwrap_err();
        assert!(error
            .to_string()
            .contains("not a supported read-only diagnostic"));
    }

    #[test]
    fn supported_diagnostic_listing_excludes_mutating_and_unverified_actions() {
        let actions = supported_diagnostic_actions();
        let ids: Vec<_> = actions.iter().map(|action| action.id.as_str()).collect();

        assert_eq!(
            ids,
            vec![
                "sfc_verify",
                "dism_check_health",
                "windows_update_status",
                "network_status",
                "printing_status",
                "defender_status",
                "appx_status",
                "search_indexing_status",
                "dism_check_health_api",
            ]
        );
        assert!(actions.iter().all(|action| !action.mutates_system));
        assert!(!ids.contains(&"recovery_settings"));
    }

    #[test]
    fn phase_21_diagnostics_have_read_only_allowlisted_metadata() {
        for action_id in [
            "network_status",
            "printing_status",
            "appx_status",
            "search_indexing_status",
        ] {
            let action = list_actions()
                .into_iter()
                .find(|action| action.id == action_id)
                .unwrap();
            assert!(!action.mutates_system);
            assert!(!action.requires_admin);
            assert!(!action.restart_required);
            assert_eq!(action.risk, "low");
            assert!(action.limitations.contains("does not"));
            validate_action(&action).unwrap();
            assert!(diagnostic_verification_method(action_id).is_some());
        }
    }

    #[test]
    fn service_diagnostics_use_native_scm_metadata() {
        for (action_id, service_name) in [
            ("windows_update_status", "wuauserv"),
            ("printing_status", "Spooler"),
            ("search_indexing_status", "WSearch"),
        ] {
            let action = list_actions()
                .into_iter()
                .find(|action| action.id == action_id)
                .unwrap();
            assert_eq!(action.command, "SCM");
            assert_eq!(
                action.args,
                vec!["query".to_string(), service_name.to_string()]
            );
        }
    }
}
