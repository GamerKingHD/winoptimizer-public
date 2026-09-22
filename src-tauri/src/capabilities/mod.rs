//! Machine-readable command capability metadata and mutation invariants.
//!
//! The Tauri registration macro is intentionally kept simple.  This registry
//! is the reviewable source of truth that makes a newly registered command
//! visible to the architecture audit before it can be treated as safe.
use crate::core::operation::ExposureLevel;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    ReadOnly,
    Unclassified,
    PlanCreator,
    PlanExecutor,
    Internal,
    Legacy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofStatus {
    Proven,
    PartiallyProven,
    Experimental,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    None,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryLevel {
    Exact,
    BestEffort,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Entitlement {
    Free,
    Paid(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OnlineRequirement {
    NotRequired,
    Optional,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ArchitectureSupport {
    pub x64: bool,
    pub arm64: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CapabilityDescriptor {
    pub id: &'static str,
    pub name: &'static str,
    pub exposure: ExposureLevel,
    pub proof: ProofStatus,
    pub risk: RiskLevel,
    pub recovery: RecoveryLevel,
    pub entitlement: Entitlement,
    pub online_requirement: OnlineRequirement,
    pub architectures: ArchitectureSupport,
    pub route: Option<&'static str>,
    pub operation_ids: &'static [&'static str],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandDescriptor {
    pub command: &'static str,
    pub kind: CommandKind,
    pub operation_id: Option<String>,
    pub capability_id: String,
    pub proof_status: ProofStatus,
    pub risk: RiskLevel,
    pub recovery: RecoveryLevel,
    pub mutates_system: bool,
    pub uses_operation_kernel: bool,
    pub requires_confirmation: bool,
    pub requires_revalidation: bool,
    pub requires_verification: bool,
    pub requires_journal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditViolation {
    pub command: &'static str,
    pub message: &'static str,
}

const REGISTERED_COMMANDS: &[&str] = &[
    "scan_cleanup_targets",
    "create_cleanup_plan",
    "execute_cleanup_plan",
    "cancel_cleanup",
    "scan_temp_files",
    "scan_browser_cache",
    "detect_browsers",
    "scan_windows_update_cache",
    "scan_system_logs",
    "list_startup_programs",
    "create_startup_program_plan",
    "execute_startup_program_plan",
    "create_startup_program_enable_plan",
    "execute_startup_program_enable_plan",
    "create_service_plan",
    "execute_service_plan",
    "create_services_manual_plan",
    "execute_services_manual_plan",
    "create_services_guidance_plan",
    "execute_services_guidance_plan",
    "list_services",
    "list_startup_tasks",
    "scan_scheduled_tasks",
    "create_scheduled_task_plan",
    "execute_scheduled_task_plan",
    "create_scheduled_tasks_guidance_plan",
    "execute_scheduled_tasks_guidance_plan",
    "list_system_tweak_targets",
    "create_system_tweak_plan",
    "execute_system_tweak_plan",
    "list_tweak_presets",
    "create_tweak_preset_plan",
    "execute_tweak_preset_plan",
    "list_context_menu_entries",
    "list_hosts_entries",
    "create_hosts_plan",
    "execute_hosts_plan",
    "create_appx_plan",
    "execute_appx_plan",
    "list_appx_candidates",
    "get_appx_policy_status",
    "scan_disk_usage",
    "cancel_scan",
    "cancel_repair",
    "export_disk_usage_json",
    "list_disk_children",
    "get_disk_entry_details",
    "export_disk_snapshot_json",
    "list_largest_disk_files",
    "usn_scan_volume",
    "usn_find_files",
    "process_snapshot",
    "create_process_plan",
    "execute_process_plan",
    "get_hardware_info",
    "get_os_info",
    "get_installed_updates",
    "get_battery_info",
    "list_env_vars",
    "create_env_plan",
    "execute_env_plan",
    "get_network_info",
    "get_undo_log",
    "get_settings",
    "update_settings",
    "record_telemetry_event",
    "check_for_updates",
    "install_update",
    "list_performance_experiments",
    "save_performance_experiment",
    "prepare_performance_experiment_revert",
    "record_performance_experiment_decision",
    "check_elevated",
    "has_resume_operation_args",
    "confirm_and_request_elevation",
    "execute_resumed_operation",
    "list_repair_targets",
    "list_repair_guides",
    "create_repair_plan",
    "execute_repair_plan",
    "list_repair_actions",
    "preview_transaction",
    "backup_registry_key_hive",
    "get_operation_kernel_contract",
    "get_capability_manifest",
    "get_operation_journal",
    "get_unresolved_operations",
    "create_retry_plan",
    "export_support_bundle",
    "preview_operation_undo",
    "execute_operation_undo",
];

const PLAN_CREATORS: &[&str] = &[
    "create_cleanup_plan",
    "create_startup_program_plan",
    "create_startup_program_enable_plan",
    "create_service_plan",
    "create_services_manual_plan",
    "create_services_guidance_plan",
    "create_scheduled_task_plan",
    "create_scheduled_tasks_guidance_plan",
    "create_system_tweak_plan",
    "prepare_performance_experiment_revert",
    "create_tweak_preset_plan",
    "create_hosts_plan",
    "create_appx_plan",
    "create_process_plan",
    "create_env_plan",
    "create_repair_plan",
    "create_retry_plan",
];

const PLAN_EXECUTORS: &[&str] = &[
    "execute_cleanup_plan",
    "execute_startup_program_plan",
    "execute_startup_program_enable_plan",
    "execute_service_plan",
    "execute_services_manual_plan",
    "execute_services_guidance_plan",
    "execute_scheduled_task_plan",
    "execute_scheduled_tasks_guidance_plan",
    "execute_system_tweak_plan",
    "execute_tweak_preset_plan",
    "execute_hosts_plan",
    "execute_appx_plan",
    "execute_process_plan",
    "execute_env_plan",
    "execute_repair_plan",
    "execute_resumed_operation",
    "execute_operation_undo",
];

const LEGACY_MUTATIONS: &[&str] = &[];

const INTERNAL_COMMANDS: &[&str] = &[
    "cancel_cleanup",
    "cancel_scan",
    "cancel_repair",
    "check_elevated",
    "confirm_and_request_elevation",
    "export_support_bundle",
    "install_update",
];

const READ_ONLY_COMMANDS: &[&str] = &[
    "has_resume_operation_args",
    "scan_cleanup_targets",
    "scan_temp_files",
    "scan_browser_cache",
    "detect_browsers",
    "scan_windows_update_cache",
    "scan_system_logs",
    "list_startup_programs",
    "list_services",
    "list_startup_tasks",
    "scan_scheduled_tasks",
    "list_system_tweak_targets",
    "list_tweak_presets",
    "list_context_menu_entries",
    "list_hosts_entries",
    "list_appx_candidates",
    "get_appx_policy_status",
    "scan_disk_usage",
    "export_disk_usage_json",
    "list_disk_children",
    "get_disk_entry_details",
    "export_disk_snapshot_json",
    "list_largest_disk_files",
    "usn_scan_volume",
    "usn_find_files",
    "process_snapshot",
    "get_hardware_info",
    "get_os_info",
    "get_installed_updates",
    "get_battery_info",
    "list_env_vars",
    "get_network_info",
    "get_undo_log",
    "get_settings",
    "update_settings",
    "record_telemetry_event",
    "check_for_updates",
    "list_performance_experiments",
    "save_performance_experiment",
    "record_performance_experiment_decision",
    "list_repair_targets",
    "list_repair_guides",
    "list_repair_actions",
    "preview_transaction",
    "backup_registry_key_hive",
    "get_operation_kernel_contract",
    "get_capability_manifest",
    "get_operation_journal",
    "get_unresolved_operations",
    "preview_operation_undo",
];

const COMMAND_CLASSIFICATIONS: &[(&[&str], CommandKind)] = &[
    (PLAN_CREATORS, CommandKind::PlanCreator),
    (PLAN_EXECUTORS, CommandKind::PlanExecutor),
    (LEGACY_MUTATIONS, CommandKind::Legacy),
    (INTERNAL_COMMANDS, CommandKind::Internal),
    (READ_ONLY_COMMANDS, CommandKind::ReadOnly),
];

const ALL_ARCHITECTURES: ArchitectureSupport = ArchitectureSupport {
    x64: true,
    arm64: false,
};

const CAPABILITY_DESCRIPTORS: &[CapabilityDescriptor] = &[
    CapabilityDescriptor {
        id: "cleanup.temp",
        name: "Clean Up",
        exposure: ExposureLevel::Core,
        proof: ProofStatus::PartiallyProven,
        risk: RiskLevel::Medium,
        recovery: RecoveryLevel::None,
        entitlement: Entitlement::Free,
        online_requirement: OnlineRequirement::NotRequired,
        architectures: ALL_ARCHITECTURES,
        route: Some("/disk-cleaner"),
        operation_ids: &["cleanup.reference_slice"],
    },
    CapabilityDescriptor {
        id: "startup.programs",
        name: "Startup Programs",
        exposure: ExposureLevel::Core,
        proof: ProofStatus::PartiallyProven,
        risk: RiskLevel::Medium,
        recovery: RecoveryLevel::Exact,
        entitlement: Entitlement::Free,
        online_requirement: OnlineRequirement::NotRequired,
        architectures: ALL_ARCHITECTURES,
        route: Some("/startup-manager"),
        operation_ids: &["startup.disable_program", "startup.enable_program"],
    },
    CapabilityDescriptor {
        id: "startup.services",
        name: "Services",
        exposure: ExposureLevel::Advanced,
        proof: ProofStatus::Experimental,
        risk: RiskLevel::High,
        recovery: RecoveryLevel::BestEffort,
        entitlement: Entitlement::Free,
        online_requirement: OnlineRequirement::NotRequired,
        architectures: ALL_ARCHITECTURES,
        route: Some("/startup-manager"),
        operation_ids: &[
            "startup.service_change",
            "startup.services_set_manual",
            "startup.services_guidance",
        ],
    },
    CapabilityDescriptor {
        id: "startup.tasks",
        name: "Scheduled Tasks",
        exposure: ExposureLevel::Advanced,
        proof: ProofStatus::Experimental,
        risk: RiskLevel::High,
        recovery: RecoveryLevel::BestEffort,
        entitlement: Entitlement::Free,
        online_requirement: OnlineRequirement::NotRequired,
        architectures: ALL_ARCHITECTURES,
        route: Some("/startup-manager"),
        operation_ids: &[
            "startup.scheduled_task_change",
            "startup.scheduled_tasks_guidance",
        ],
    },
    CapabilityDescriptor {
        id: "tweaks.system",
        name: "System Tweaks",
        exposure: ExposureLevel::Advanced,
        proof: ProofStatus::Experimental,
        risk: RiskLevel::High,
        recovery: RecoveryLevel::None,
        entitlement: Entitlement::Free,
        online_requirement: OnlineRequirement::NotRequired,
        architectures: ALL_ARCHITECTURES,
        route: Some("/system-tweaks"),
        operation_ids: &["system_tweak.change", "preset.dynamic"],
    },
    CapabilityDescriptor {
        id: "repair.guided",
        name: "Repair Center",
        exposure: ExposureLevel::Core,
        proof: ProofStatus::PartiallyProven,
        risk: RiskLevel::Medium,
        recovery: RecoveryLevel::None,
        entitlement: Entitlement::Free,
        online_requirement: OnlineRequirement::NotRequired,
        architectures: ALL_ARCHITECTURES,
        route: Some("/repair-center"),
        operation_ids: &["repair.verified_diagnostic", "repair.maintenance"],
    },
    CapabilityDescriptor {
        id: "disk.usage",
        name: "Disk Analyzer",
        exposure: ExposureLevel::Core,
        proof: ProofStatus::Experimental,
        risk: RiskLevel::Low,
        recovery: RecoveryLevel::None,
        entitlement: Entitlement::Free,
        online_requirement: OnlineRequirement::NotRequired,
        architectures: ALL_ARCHITECTURES,
        route: Some("/disk-analyzer"),
        operation_ids: &[],
    },
    CapabilityDescriptor {
        id: "hosts.editor",
        name: "Hosts Editor",
        exposure: ExposureLevel::Advanced,
        proof: ProofStatus::Experimental,
        risk: RiskLevel::High,
        recovery: RecoveryLevel::BestEffort,
        entitlement: Entitlement::Free,
        online_requirement: OnlineRequirement::NotRequired,
        architectures: ALL_ARCHITECTURES,
        route: Some("/hosts-editor"),
        operation_ids: &["hosts.save"],
    },
    CapabilityDescriptor {
        id: "appx.manager",
        name: "Preinstalled Apps Policy",
        exposure: ExposureLevel::Advanced,
        proof: ProofStatus::Experimental,
        risk: RiskLevel::High,
        recovery: RecoveryLevel::BestEffort,
        entitlement: Entitlement::Free,
        online_requirement: OnlineRequirement::NotRequired,
        architectures: ALL_ARCHITECTURES,
        route: Some("/appx-manager"),
        operation_ids: &["appx.policy_change"],
    },
    CapabilityDescriptor {
        id: "environment.variables",
        name: "Environment Variables",
        exposure: ExposureLevel::Advanced,
        proof: ProofStatus::PartiallyProven,
        risk: RiskLevel::Medium,
        recovery: RecoveryLevel::Exact,
        entitlement: Entitlement::Free,
        online_requirement: OnlineRequirement::NotRequired,
        architectures: ALL_ARCHITECTURES,
        route: Some("/env-vars"),
        operation_ids: &["environment.change"],
    },
    CapabilityDescriptor {
        id: "process.manager",
        name: "Process Manager",
        exposure: ExposureLevel::Advanced,
        proof: ProofStatus::PartiallyProven,
        risk: RiskLevel::High,
        recovery: RecoveryLevel::None,
        entitlement: Entitlement::Free,
        online_requirement: OnlineRequirement::NotRequired,
        architectures: ALL_ARCHITECTURES,
        route: Some("/process-manager"),
        operation_ids: &["process.terminate"],
    },
    CapabilityDescriptor {
        id: "operation.kernel",
        name: "Operation Kernel",
        exposure: ExposureLevel::Internal,
        proof: ProofStatus::PartiallyProven,
        risk: RiskLevel::None,
        recovery: RecoveryLevel::BestEffort,
        entitlement: Entitlement::Free,
        online_requirement: OnlineRequirement::NotRequired,
        architectures: ALL_ARCHITECTURES,
        route: None,
        operation_ids: &["synthetic.operation-kernel"],
    },
    CapabilityDescriptor {
        id: "support.bundle",
        name: "Support Bundle",
        exposure: ExposureLevel::Supporting,
        proof: ProofStatus::Proven,
        risk: RiskLevel::Low,
        recovery: RecoveryLevel::None,
        entitlement: Entitlement::Free,
        online_requirement: OnlineRequirement::NotRequired,
        architectures: ALL_ARCHITECTURES,
        route: Some("/settings"),
        operation_ids: &["support.bundle"],
    },
    CapabilityDescriptor {
        id: "privacy.telemetry",
        name: "Privacy-Minimized Telemetry",
        exposure: ExposureLevel::Supporting,
        proof: ProofStatus::Proven,
        risk: RiskLevel::None,
        recovery: RecoveryLevel::None,
        entitlement: Entitlement::Free,
        online_requirement: OnlineRequirement::Optional,
        architectures: ALL_ARCHITECTURES,
        route: Some("/privacy"),
        operation_ids: &[],
    },
    CapabilityDescriptor {
        id: "updates",
        name: "Authenticated Updates",
        exposure: ExposureLevel::Supporting,
        proof: ProofStatus::PartiallyProven,
        risk: RiskLevel::Medium,
        recovery: RecoveryLevel::BestEffort,
        entitlement: Entitlement::Free,
        online_requirement: OnlineRequirement::Optional,
        architectures: ALL_ARCHITECTURES,
        route: Some("/settings"),
        operation_ids: &["update.check", "update.install"],
    },
    CapabilityDescriptor {
        id: "history",
        name: "History & Undo",
        exposure: ExposureLevel::Core,
        proof: ProofStatus::PartiallyProven,
        risk: RiskLevel::Medium,
        recovery: RecoveryLevel::BestEffort,
        entitlement: Entitlement::Free,
        online_requirement: OnlineRequirement::NotRequired,
        architectures: ALL_ARCHITECTURES,
        route: Some("/history"),
        operation_ids: &["operation.undo", "operation.retry", "operation.resumed"],
    },
    CapabilityDescriptor {
        id: "network.monitor",
        name: "Network Monitor",
        exposure: ExposureLevel::Supporting,
        proof: ProofStatus::Proven,
        risk: RiskLevel::Low,
        recovery: RecoveryLevel::None,
        entitlement: Entitlement::Free,
        online_requirement: OnlineRequirement::NotRequired,
        architectures: ALL_ARCHITECTURES,
        route: Some("/network"),
        operation_ids: &[],
    },
    CapabilityDescriptor {
        id: "capability.manifest",
        name: "Capability Manifest",
        exposure: ExposureLevel::Internal,
        proof: ProofStatus::Proven,
        risk: RiskLevel::None,
        recovery: RecoveryLevel::None,
        entitlement: Entitlement::Free,
        online_requirement: OnlineRequirement::NotRequired,
        architectures: ALL_ARCHITECTURES,
        route: None,
        operation_ids: &["capability.manifest"],
    },
];

pub fn capability_descriptors() -> &'static [CapabilityDescriptor] {
    CAPABILITY_DESCRIPTORS
}

fn capability_for_operation(operation_id: &str) -> Option<&'static CapabilityDescriptor> {
    CAPABILITY_DESCRIPTORS
        .iter()
        .find(|capability| capability.operation_ids.contains(&operation_id))
}

#[tauri::command]
pub fn get_capability_manifest() -> Vec<CapabilityDescriptor> {
    CAPABILITY_DESCRIPTORS.to_vec()
}

pub fn audit_capability_manifest() -> Vec<AuditViolation> {
    let mut violations = Vec::new();
    for capability in CAPABILITY_DESCRIPTORS {
        for operation_id in capability.operation_ids {
            let has_command_mapping = all_descriptors()
                .iter()
                .any(|command| command.operation_id.as_deref() == Some(operation_id))
                // create_repair_plan and execute_repair_plan select the
                // operation ID from the immutable action in the submitted
                // plan, so their static descriptor exposes the diagnostic
                // variant while also routing maintenance plans here.
                || (*operation_id == "repair.maintenance"
                    && ["create_repair_plan", "execute_repair_plan"]
                        .iter()
                        .all(|command| REGISTERED_COMMANDS.contains(command)));
            if !has_command_mapping {
                violations.push(AuditViolation {
                    command: capability.id,
                    message: "capability operation has no registered command mapping",
                });
            }
        }
    }
    for command in all_descriptors() {
        if let Some(operation_id) = command.operation_id.as_deref() {
            if !CAPABILITY_DESCRIPTORS
                .iter()
                .any(|capability| capability.operation_ids.contains(&operation_id))
            {
                violations.push(AuditViolation {
                    command: command.command,
                    message: "registered operation has no capability mapping",
                });
            }
        }
    }
    violations
}

pub fn registered_commands() -> &'static [&'static str] {
    REGISTERED_COMMANDS
}

fn command_kind(command: &'static str) -> Option<CommandKind> {
    COMMAND_CLASSIFICATIONS
        .iter()
        .find_map(|(commands, kind)| commands.contains(&command).then_some(*kind))
}

fn classification_count(command: &str) -> usize {
    COMMAND_CLASSIFICATIONS
        .iter()
        .filter(|(commands, _)| commands.contains(&command))
        .count()
}

pub fn descriptor(command: &'static str) -> CommandDescriptor {
    let kind = match command_kind(command) {
        Some(kind) => kind,
        None => CommandKind::Unclassified,
    };
    let mutates_system = matches!(kind, CommandKind::PlanExecutor | CommandKind::Legacy);
    let operation_id = match command {
        "create_cleanup_plan" | "execute_cleanup_plan" => Some("cleanup.reference_slice"),
        "create_startup_program_plan" | "execute_startup_program_plan" => {
            Some("startup.disable_program")
        }
        "create_startup_program_enable_plan" | "execute_startup_program_enable_plan" => {
            Some("startup.enable_program")
        }
        "create_service_plan" | "execute_service_plan" => Some("startup.service_change"),
        "create_services_manual_plan" | "execute_services_manual_plan" => {
            Some("startup.services_set_manual")
        }
        "create_services_guidance_plan" | "execute_services_guidance_plan" => {
            Some("startup.services_guidance")
        }
        "create_scheduled_task_plan" | "execute_scheduled_task_plan" => {
            Some("startup.scheduled_task_change")
        }
        "create_scheduled_tasks_guidance_plan" | "execute_scheduled_tasks_guidance_plan" => {
            Some("startup.scheduled_tasks_guidance")
        }
        "create_system_tweak_plan"
        | "prepare_performance_experiment_revert"
        | "execute_system_tweak_plan" => Some("system_tweak.change"),
        "create_tweak_preset_plan" | "execute_tweak_preset_plan" => Some("preset.dynamic"),
        "create_hosts_plan" | "execute_hosts_plan" => Some("hosts.save"),
        "create_appx_plan" | "execute_appx_plan" => Some("appx.policy_change"),
        "create_process_plan" | "execute_process_plan" => Some("process.terminate"),
        "create_env_plan" | "execute_env_plan" => Some("environment.change"),
        "create_repair_plan" | "execute_repair_plan" => Some("repair.verified_diagnostic"),
        "check_for_updates" => Some("update.check"),
        "install_update" => Some("update.install"),
        "execute_resumed_operation" => Some("operation.resumed"),
        "preview_operation_undo" | "execute_operation_undo" => Some("operation.undo"),
        "get_operation_kernel_contract" => Some("synthetic.operation-kernel"),
        "get_capability_manifest" => Some("capability.manifest"),
        "create_retry_plan" => Some("operation.retry"),
        "export_support_bundle" => Some("support.bundle"),
        _ => None,
    }
    .map(str::to_owned);
    let operation_capability = operation_id.as_deref().and_then(capability_for_operation);
    let capability_id = operation_capability
        .map(|capability| capability.id.to_owned())
        .unwrap_or_else(|| format!("command.{command}"));
    let uses_operation_kernel = matches!(kind, CommandKind::PlanExecutor);
    let (proof_status, risk, recovery) = operation_capability
        .map(|capability| (capability.proof, capability.risk, capability.recovery))
        .unwrap_or_else(|| match command {
            _ if mutates_system => (
                ProofStatus::PartiallyProven,
                RiskLevel::Medium,
                RecoveryLevel::BestEffort,
            ),
            _ => (ProofStatus::Proven, RiskLevel::None, RecoveryLevel::None),
        });
    CommandDescriptor {
        command,
        kind,
        operation_id,
        capability_id,
        proof_status,
        risk,
        recovery,
        mutates_system,
        uses_operation_kernel,
        requires_confirmation: mutates_system,
        requires_revalidation: mutates_system,
        requires_verification: mutates_system,
        requires_journal: mutates_system,
    }
}

pub fn all_descriptors() -> Vec<CommandDescriptor> {
    REGISTERED_COMMANDS
        .iter()
        .copied()
        .map(descriptor)
        .collect()
}

fn audit_descriptor(descriptor: &CommandDescriptor) -> Vec<AuditViolation> {
    let mut violations = Vec::new();
    if descriptor.kind == CommandKind::Unclassified {
        violations.push(AuditViolation {
            command: descriptor.command,
            message: "registered command has no explicit classification",
        });
        return violations;
    }
    if !matches!(
        descriptor.kind,
        CommandKind::PlanExecutor | CommandKind::Legacy
    ) {
        return violations;
    }
    if descriptor.operation_id.is_none() {
        violations.push(AuditViolation {
            command: descriptor.command,
            message: "mutation has no operation ID",
        });
    }
    if descriptor.capability_id.is_empty() {
        violations.push(AuditViolation {
            command: descriptor.command,
            message: "mutation has no capability ID",
        });
    }
    if !descriptor.requires_journal {
        violations.push(AuditViolation {
            command: descriptor.command,
            message: "mutation does not require the durable journal",
        });
    }
    if !descriptor.requires_confirmation {
        violations.push(AuditViolation {
            command: descriptor.command,
            message: "mutation does not require explicit confirmation",
        });
    }
    if !descriptor.uses_operation_kernel {
        violations.push(AuditViolation {
            command: descriptor.command,
            message: "mutation does not use the operation kernel",
        });
    }
    if !descriptor.requires_revalidation {
        violations.push(AuditViolation {
            command: descriptor.command,
            message: "mutation does not require revalidation",
        });
    }
    if !descriptor.requires_verification {
        violations.push(AuditViolation {
            command: descriptor.command,
            message: "mutation does not require postcondition verification",
        });
    }
    violations
}

fn audit_commands(commands: &[&'static str]) -> Vec<AuditViolation> {
    let mut violations = Vec::new();
    for &command in commands {
        match classification_count(command) {
            0 => {
                violations.push(AuditViolation {
                    command,
                    message: "registered command has no explicit classification",
                });
                continue;
            }
            1 => {}
            _ => violations.push(AuditViolation {
                command,
                message: "registered command has multiple classifications",
            }),
        }
        violations.extend(audit_descriptor(&descriptor(command)));
    }
    violations
}

pub fn audit_registered_commands() -> Vec<AuditViolation> {
    audit_commands(REGISTERED_COMMANDS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names_from_registration_source() -> Vec<&'static str> {
        let source = include_str!("../lib.rs");
        let start = source
            .find("tauri::generate_handler![")
            .expect("Tauri command registration must exist");
        let end = source[start..]
            .find("])")
            .map(|offset| start + offset)
            .expect("Tauri command registration must be closed");
        source[start..end]
            .lines()
            .filter_map(|line| {
                let name = line.trim().trim_end_matches(',');
                if name.is_empty() || name.ends_with('[') || name.starts_with("//") {
                    None
                } else {
                    Some(name)
                }
            })
            .collect()
    }

    #[test]
    fn production_crate_does_not_enable_tauri_test_feature() {
        let cargo = include_str!("../../Cargo.toml");
        assert!(
            !cargo.contains("\"protocol-asset\", \"test\""),
            "production crate must not enable the Tauri test feature"
        );
        assert!(
            !cargo.contains("features = [\"custom-protocol\", \"protocol-asset\", \"test\"]"),
            "production crate must not enable the Tauri test feature"
        );
    }

    #[test]
    fn descriptor_registry_matches_tauri_registration() {
        let source_names = names_from_registration_source();
        assert_eq!(source_names, REGISTERED_COMMANDS);
        assert_eq!(all_descriptors().len(), REGISTERED_COMMANDS.len());
    }

    #[test]
    fn every_registered_command_has_one_explicit_classification() {
        for &command in registered_commands() {
            assert_eq!(
                classification_count(command),
                1,
                "classification for {command}"
            );
            assert_ne!(descriptor(command).kind, CommandKind::Unclassified);
        }
    }

    #[test]
    fn audit_rejects_unclassified_commands() {
        assert_eq!(
            audit_commands(&["synthetic_unclassified"]),
            vec![AuditViolation {
                command: "synthetic_unclassified",
                message: "registered command has no explicit classification",
            }]
        );
    }

    #[test]
    fn audit_rejects_mutations_without_safety_metadata() {
        let violations = audit_descriptor(&CommandDescriptor {
            command: "synthetic_mutation",
            kind: CommandKind::PlanExecutor,
            operation_id: None,
            capability_id: String::new(),
            proof_status: ProofStatus::Proven,
            risk: RiskLevel::None,
            recovery: RecoveryLevel::None,
            mutates_system: true,
            uses_operation_kernel: false,
            requires_confirmation: false,
            requires_revalidation: false,
            requires_verification: false,
            requires_journal: false,
        });
        let messages: Vec<_> = violations
            .iter()
            .map(|violation| violation.message)
            .collect();
        for expected in [
            "mutation has no operation ID",
            "mutation has no capability ID",
            "mutation does not use the operation kernel",
            "mutation does not require explicit confirmation",
            "mutation does not require revalidation",
            "mutation does not require postcondition verification",
            "mutation does not require the durable journal",
        ] {
            assert!(messages.contains(&expected), "missing audit: {expected}");
        }
    }

    #[test]
    fn every_registered_mutation_has_safety_metadata() {
        assert!(audit_registered_commands().is_empty());
    }

    #[test]
    fn resumed_execution_has_only_the_authenticated_mutation_entrypoint() {
        assert!(!registered_commands().contains(&"get_resumed_confirmed_plan"));
        assert!(!registered_commands().contains(&"request_elevation"));
        assert!(!registered_commands().contains(&"request_admin_startup"));
        let resumed = descriptor("execute_resumed_operation");
        assert_eq!(resumed.kind, CommandKind::PlanExecutor);
        assert!(resumed.uses_operation_kernel);
        assert!(resumed.requires_journal);
        assert!(resumed.requires_confirmation);
        assert!(resumed.requires_revalidation);
        assert!(resumed.requires_verification);
    }

    #[test]
    fn capability_manifest_has_unique_ids_and_complete_operation_mappings() {
        let mut ids = std::collections::HashSet::new();
        for capability in capability_descriptors() {
            assert!(ids.insert(capability.id), "duplicate capability id");
        }
        assert!(
            audit_capability_manifest().is_empty(),
            "capability audit violations: {:?}",
            audit_capability_manifest()
        );
    }

    #[test]
    fn command_metadata_uses_the_mapped_capability_contract() {
        for descriptor in all_descriptors() {
            let Some(operation_id) = descriptor.operation_id.as_deref() else {
                continue;
            };
            let capability = capability_for_operation(operation_id)
                .unwrap_or_else(|| panic!("missing capability for {operation_id}"));

            assert_eq!(
                descriptor.capability_id, capability.id,
                "{}",
                descriptor.command
            );
            assert_eq!(
                descriptor.proof_status, capability.proof,
                "{}",
                descriptor.command
            );
            assert_eq!(descriptor.risk, capability.risk, "{}", descriptor.command);
            assert_eq!(
                descriptor.recovery, capability.recovery,
                "{}",
                descriptor.command
            );
        }
    }

    #[test]
    fn legacy_mutations_are_explicit_migration_debt() {
        assert!(
            LEGACY_MUTATIONS.is_empty(),
            "direct legacy mutation commands must not be registered"
        );
        assert!(LEGACY_MUTATIONS
            .iter()
            .all(|command| descriptor(command).kind == CommandKind::Legacy));
        assert!(LEGACY_MUTATIONS
            .iter()
            .all(|command| descriptor(command).requires_journal));
    }
}
