//! Offline guidance catalog for Windows services and scheduled tasks.
//!
//! The catalog is packaged with the app, does not perform network lookups, and
//! never turns an unknown target into a bulk recommendation.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use crate::core::evidence::{
    EvidenceConfidence, EvidenceConflict, EvidenceMatchingRule, EvidenceMetadata,
    EvidenceSourceType,
};

pub const CATALOG_VERSION: &str = "1.0.0";
pub const DESKTOP_OS_SCOPE: &str = "Windows 10/11 desktop";
pub const WINDOWS_SERVICES_URL: &str =
    "https://learn.microsoft.com/en-us/windows/win32/services/automatically-starting-services";
pub const TASK_MAINTENANCE_URL: &str =
    "https://learn.microsoft.com/en-us/windows/win32/taskschd/task-maintenence";
pub const SERVER_SERVICES_URL: &str =
    "https://learn.microsoft.com/en-us/windows-server/security/windows-services/security-guidelines-for-disabling-system-services-in-windows-server";
pub const BLACK_VIPER_URL: &str = "https://www.blackviper.com/contact-bv/";
pub const WINUTIL_URL: &str = "https://github.com/ChrisTitusTech/winutil";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum GuidanceAction {
    KeepEnabled,
    SetManual,
    DisableIfUnused,
    #[default]
    ReviewFirst,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum GuidanceSourceType {
    Microsoft,
    Vendor,
    Community,
    Policy,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum GuidanceMatch {
    Protected,
    Exact,
    Pattern,
    OutOfScope,
    Conflict,
    #[default]
    None,
}

/// Shared service/task guidance shown by the frontend and captured in plans.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuidanceInfo {
    /// UI grouping. It mirrors `action` so clients do not need a second policy vocabulary.
    pub group: GuidanceAction,
    pub action: GuidanceAction,
    pub rationale: String,
    pub conditions: Vec<String>,
    pub source_type: GuidanceSourceType,
    pub source_title: String,
    pub source_url: String,
    pub os_scope: String,
    pub catalog_version: String,
    pub protected: bool,
    pub matched_by: GuidanceMatch,
}

impl Default for GuidanceInfo {
    fn default() -> Self {
        unknown_guidance()
    }
}

impl GuidanceInfo {
    /// Converts the startup catalog's legacy metadata into the shared evidence model.
    pub fn evidence(&self) -> EvidenceMetadata {
        let source_type = match self.source_type {
            GuidanceSourceType::Microsoft => EvidenceSourceType::MicrosoftGuidance,
            GuidanceSourceType::Vendor => EvidenceSourceType::VendorGuidance,
            GuidanceSourceType::Community => EvidenceSourceType::CommunityGuidance,
            GuidanceSourceType::Policy => EvidenceSourceType::WinOptimizerDeterministicRule,
            GuidanceSourceType::Unknown => EvidenceSourceType::Unknown,
        };
        let matching_rule = match self.matched_by {
            GuidanceMatch::Protected => EvidenceMatchingRule::DeterministicRule,
            GuidanceMatch::Exact => EvidenceMatchingRule::Exact,
            GuidanceMatch::Pattern => EvidenceMatchingRule::Pattern,
            GuidanceMatch::OutOfScope | GuidanceMatch::Conflict | GuidanceMatch::None => {
                EvidenceMatchingRule::Unknown
            }
        };
        let conflict = if self.matched_by == GuidanceMatch::Conflict {
            EvidenceConflict::Conflicting
        } else if self.matched_by == GuidanceMatch::None {
            EvidenceConflict::Unknown
        } else {
            EvidenceConflict::None
        };
        let confidence = match (self.source_type, self.matched_by) {
            (_, GuidanceMatch::Conflict | GuidanceMatch::OutOfScope | GuidanceMatch::None) => {
                EvidenceConfidence::Unknown
            }
            (GuidanceSourceType::Microsoft | GuidanceSourceType::Policy, _) => {
                EvidenceConfidence::High
            }
            (GuidanceSourceType::Vendor | GuidanceSourceType::Community, _) => {
                EvidenceConfidence::Medium
            }
            (GuidanceSourceType::Unknown, _) => EvidenceConfidence::Unknown,
        };

        EvidenceMetadata {
            source_type,
            source_name: self.source_title.clone(),
            source_version: Some(self.catalog_version.clone()),
            scope: self.os_scope.clone(),
            matching_rule,
            confidence,
            rationale: self.rationale.clone(),
            tradeoffs: self.conditions.clone(),
            conflict,
            last_bundled_update: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetKind {
    Service,
    ScheduledTask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchKind {
    Exact,
    Prefix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceTier {
    Official,
    Community,
}

#[derive(Debug, Clone, Copy)]
struct CatalogEntry {
    target: TargetKind,
    pattern: &'static str,
    match_kind: MatchKind,
    action: GuidanceAction,
    rationale: &'static str,
    conditions: &'static [&'static str],
    source_type: GuidanceSourceType,
    source_title: &'static str,
    source_url: &'static str,
    os_scope: &'static str,
    source_tier: SourceTier,
}

const SERVICE_DEPENDENCY_CONDITION: &[&str] =
    &["Check dependent services and the feature that starts this service before changing it."];
const OPTIONAL_FEATURE_CONDITION: &[&str] =
    &["Use this only when the related Windows feature or device is not used."];
const COMMUNITY_CONDITION: &[&str] = &[
    "Community guidance is a starting point, not a guarantee; review the local feature and dependencies.",
];
const BLACK_VIPER_CONDITION: &[&str] = &[
    "Historical source: Windows services information is no longer updated and is not maintained for Windows 11.",
    "Review the current service purpose locally before making any change.",
];
const SERVER_ONLY_CONDITION: &[&str] = &[
    "The source is scoped to Windows Server 2016 with Desktop Experience, not universal Windows 10/11 desktop guidance.",
];

const SERVICE_CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        target: TargetKind::Service,
        pattern: "BITS",
        match_kind: MatchKind::Exact,
        action: GuidanceAction::SetManual,
        rationale: "Background Intelligent Transfer Service is commonly started on demand by Windows Update and other features.",
        conditions: SERVICE_DEPENDENCY_CONDITION,
        source_type: GuidanceSourceType::Microsoft,
        source_title: "Microsoft guidance: Automatically Starting Services",
        source_url: WINDOWS_SERVICES_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Official,
    },
    CatalogEntry {
        target: TargetKind::Service,
        pattern: "AppXSvc",
        match_kind: MatchKind::Exact,
        action: GuidanceAction::SetManual,
        rationale: "The AppX Deployment Service starts on demand for Microsoft Store application deployment.",
        conditions: SERVICE_DEPENDENCY_CONDITION,
        source_type: GuidanceSourceType::Microsoft,
        source_title: "Microsoft guidance: Windows service startup and dependencies",
        source_url: WINDOWS_SERVICES_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Official,
    },
    CatalogEntry {
        target: TargetKind::Service,
        pattern: "ClipSVC",
        match_kind: MatchKind::Exact,
        action: GuidanceAction::SetManual,
        rationale: "The Microsoft Store licensing service is an on-demand service; disabling it can break Store applications.",
        conditions: SERVICE_DEPENDENCY_CONDITION,
        source_type: GuidanceSourceType::Microsoft,
        source_title: "Microsoft guidance: Windows service startup and dependencies",
        source_url: WINDOWS_SERVICES_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Official,
    },
    CatalogEntry {
        target: TargetKind::Service,
        pattern: "msiserver",
        match_kind: MatchKind::Exact,
        action: GuidanceAction::SetManual,
        rationale: "Windows Installer is normally started when an installation or repair needs it.",
        conditions: SERVICE_DEPENDENCY_CONDITION,
        source_type: GuidanceSourceType::Microsoft,
        source_title: "Microsoft guidance: Windows service startup and dependencies",
        source_url: WINDOWS_SERVICES_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Official,
    },
    CatalogEntry {
        target: TargetKind::Service,
        pattern: "Dhcp",
        match_kind: MatchKind::Exact,
        action: GuidanceAction::KeepEnabled,
        rationale: "DHCP Client supplies dynamic addressing and DNS registration; disabling it can prevent network configuration.",
        conditions: SERVICE_DEPENDENCY_CONDITION,
        source_type: GuidanceSourceType::Microsoft,
        source_title: "Microsoft guidance: Windows service dependencies",
        source_url: WINDOWS_SERVICES_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Official,
    },
    CatalogEntry {
        target: TargetKind::Service,
        pattern: "LanmanWorkstation",
        match_kind: MatchKind::Exact,
        action: GuidanceAction::KeepEnabled,
        rationale: "Workstation supports SMB client connections and other Windows networking features.",
        conditions: SERVICE_DEPENDENCY_CONDITION,
        source_type: GuidanceSourceType::Microsoft,
        source_title: "Microsoft guidance: Windows service dependencies",
        source_url: WINDOWS_SERVICES_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Official,
    },
    CatalogEntry {
        target: TargetKind::Service,
        pattern: "NlaSvc",
        match_kind: MatchKind::Exact,
        action: GuidanceAction::KeepEnabled,
        rationale: "Network Location Awareness supplies network profile information used by firewall and connectivity features.",
        conditions: SERVICE_DEPENDENCY_CONDITION,
        source_type: GuidanceSourceType::Microsoft,
        source_title: "Microsoft guidance: Windows service dependencies",
        source_url: WINDOWS_SERVICES_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Official,
    },
    // Server guidance is retained as provenance but is not actionable under
    // the Windows 10/11 desktop scope.
    CatalogEntry {
        target: TargetKind::Service,
        pattern: "Spooler",
        match_kind: MatchKind::Exact,
        action: GuidanceAction::DisableIfUnused,
        rationale: "The Print Spooler can be disabled when printing is not required in the scoped server deployment.",
        conditions: SERVER_ONLY_CONDITION,
        source_type: GuidanceSourceType::Microsoft,
        source_title: "Microsoft guidance: Security guidelines for Windows Server 2016 services",
        source_url: SERVER_SERVICES_URL,
        os_scope: "Windows Server 2016 Desktop Experience",
        source_tier: SourceTier::Official,
    },
    CatalogEntry {
        target: TargetKind::Service,
        pattern: "bthserv",
        match_kind: MatchKind::Exact,
        action: GuidanceAction::DisableIfUnused,
        rationale: "Bluetooth Support can be disabled when Bluetooth devices are not used in the scoped server deployment.",
        conditions: SERVER_ONLY_CONDITION,
        source_type: GuidanceSourceType::Microsoft,
        source_title: "Microsoft guidance: Security guidelines for Windows Server 2016 services",
        source_url: SERVER_SERVICES_URL,
        os_scope: "Windows Server 2016 Desktop Experience",
        source_tier: SourceTier::Official,
    },
    CatalogEntry {
        target: TargetKind::Service,
        pattern: "Steam Client Service",
        match_kind: MatchKind::Exact,
        action: GuidanceAction::DisableIfUnused,
        rationale: "Steam Client Service is only needed for Steam features that use it; Steam itself may need service support for updates or related features.",
        conditions: OPTIONAL_FEATURE_CONDITION,
        source_type: GuidanceSourceType::Vendor,
        source_title: "Valve Steam Support",
        source_url: "https://help.steampowered.com/",
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Official,
    },
    // Black Viper is retained as historical context only. The duplicate
    // WinUtil entry creates a same-tier conflict and therefore remains review.
    CatalogEntry {
        target: TargetKind::Service,
        pattern: "WSearch",
        match_kind: MatchKind::Exact,
        action: GuidanceAction::ReviewFirst,
        rationale: "Black Viper historically listed Windows Search as optional, but its service information is stale.",
        conditions: BLACK_VIPER_CONDITION,
        source_type: GuidanceSourceType::Community,
        source_title: "Black Viper (historical community guidance)",
        source_url: BLACK_VIPER_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Community,
    },
    CatalogEntry {
        target: TargetKind::Service,
        pattern: "WSearch",
        match_kind: MatchKind::Exact,
        action: GuidanceAction::DisableIfUnused,
        rationale: "Windows Search may be disabled when indexed search is not used, but search behavior and workload should be checked first.",
        conditions: COMMUNITY_CONDITION,
        source_type: GuidanceSourceType::Community,
        source_title: "Chris Titus Tech WinUtil",
        source_url: WINUTIL_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Community,
    },
    CatalogEntry {
        target: TargetKind::Service,
        pattern: "SysMain",
        match_kind: MatchKind::Exact,
        action: GuidanceAction::DisableIfUnused,
        rationale: "SysMain behavior depends on storage, workload, and Windows configuration; disable only after checking local impact.",
        conditions: COMMUNITY_CONDITION,
        source_type: GuidanceSourceType::Community,
        source_title: "Chris Titus Tech WinUtil",
        source_url: WINUTIL_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Community,
    },
    CatalogEntry {
        target: TargetKind::Service,
        pattern: "SysMain",
        match_kind: MatchKind::Exact,
        action: GuidanceAction::SetManual,
        rationale: "Historical community lists also suggest demand-start behavior for SysMain; the disagreement requires local review.",
        conditions: BLACK_VIPER_CONDITION,
        source_type: GuidanceSourceType::Community,
        source_title: "Black Viper (historical community guidance)",
        source_url: BLACK_VIPER_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Community,
    },
    CatalogEntry {
        target: TargetKind::Service,
        pattern: "DiagTrack",
        match_kind: MatchKind::Exact,
        action: GuidanceAction::DisableIfUnused,
        rationale: "Connected User Experiences and Telemetry is an optional privacy/performance tradeoff; review diagnostics and organization policy first.",
        conditions: COMMUNITY_CONDITION,
        source_type: GuidanceSourceType::Community,
        source_title: "Chris Titus Tech WinUtil",
        source_url: WINUTIL_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Community,
    },
    CatalogEntry {
        target: TargetKind::Service,
        pattern: "MapsBroker",
        match_kind: MatchKind::Exact,
        action: GuidanceAction::DisableIfUnused,
        rationale: "Downloaded map features may depend on MapsBroker; disable only when offline maps are not used.",
        conditions: OPTIONAL_FEATURE_CONDITION,
        source_type: GuidanceSourceType::Community,
        source_title: "Chris Titus Tech WinUtil",
        source_url: WINUTIL_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Community,
    },
    CatalogEntry {
        target: TargetKind::Service,
        pattern: "Fax",
        match_kind: MatchKind::Exact,
        action: GuidanceAction::DisableIfUnused,
        rationale: "Fax is optional when the device does not send or receive faxes.",
        conditions: OPTIONAL_FEATURE_CONDITION,
        source_type: GuidanceSourceType::Community,
        source_title: "Chris Titus Tech WinUtil",
        source_url: WINUTIL_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Community,
    },
    CatalogEntry {
        target: TargetKind::Service,
        pattern: "RemoteRegistry",
        match_kind: MatchKind::Exact,
        action: GuidanceAction::DisableIfUnused,
        rationale: "Remote Registry is not required for most personal desktop workflows, but administration tooling may depend on it.",
        conditions: OPTIONAL_FEATURE_CONDITION,
        source_type: GuidanceSourceType::Community,
        source_title: "Chris Titus Tech WinUtil",
        source_url: WINUTIL_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Community,
    },
    CatalogEntry {
        target: TargetKind::Service,
        pattern: "XboxGipSvc",
        match_kind: MatchKind::Exact,
        action: GuidanceAction::DisableIfUnused,
        rationale: "Xbox accessory support is optional when Xbox controllers or related features are not used.",
        conditions: OPTIONAL_FEATURE_CONDITION,
        source_type: GuidanceSourceType::Community,
        source_title: "Chris Titus Tech WinUtil",
        source_url: WINUTIL_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Community,
    },
    CatalogEntry {
        target: TargetKind::Service,
        pattern: "XblAuthManager",
        match_kind: MatchKind::Exact,
        action: GuidanceAction::DisableIfUnused,
        rationale: "Xbox identity support is optional when Xbox sign-in and games are not used.",
        conditions: OPTIONAL_FEATURE_CONDITION,
        source_type: GuidanceSourceType::Community,
        source_title: "Chris Titus Tech WinUtil",
        source_url: WINUTIL_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Community,
    },
    CatalogEntry {
        target: TargetKind::Service,
        pattern: "XboxNetApiSvc",
        match_kind: MatchKind::Exact,
        action: GuidanceAction::DisableIfUnused,
        rationale: "Xbox networking support is optional when Xbox networking and games are not used.",
        conditions: OPTIONAL_FEATURE_CONDITION,
        source_type: GuidanceSourceType::Community,
        source_title: "Chris Titus Tech WinUtil",
        source_url: WINUTIL_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Community,
    },
    CatalogEntry {
        target: TargetKind::ScheduledTask,
        pattern: "microsoft\\windows\\application experience\\",
        match_kind: MatchKind::Prefix,
        action: GuidanceAction::DisableIfUnused,
        rationale: "Application Experience compatibility assessment tasks are optional for users who accept reduced compatibility telemetry.",
        conditions: COMMUNITY_CONDITION,
        source_type: GuidanceSourceType::Community,
        source_title: "Chris Titus Tech WinUtil",
        source_url: WINUTIL_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Community,
    },
    CatalogEntry {
        target: TargetKind::ScheduledTask,
        pattern: "microsoft\\windows\\customer experience improvement program\\",
        match_kind: MatchKind::Prefix,
        action: GuidanceAction::DisableIfUnused,
        rationale: "Customer Experience Improvement Program tasks are optional telemetry tasks; review organizational policy first.",
        conditions: COMMUNITY_CONDITION,
        source_type: GuidanceSourceType::Community,
        source_title: "Chris Titus Tech WinUtil",
        source_url: WINUTIL_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Community,
    },
    CatalogEntry {
        target: TargetKind::ScheduledTask,
        pattern: "microsoft\\windows\\feedback\\siuf\\",
        match_kind: MatchKind::Prefix,
        action: GuidanceAction::DisableIfUnused,
        rationale: "Feedback tasks are optional when Windows feedback collection is not wanted, subject to policy.",
        conditions: COMMUNITY_CONDITION,
        source_type: GuidanceSourceType::Community,
        source_title: "Chris Titus Tech WinUtil",
        source_url: WINUTIL_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Community,
    },
    CatalogEntry {
        target: TargetKind::ScheduledTask,
        pattern: "microsoft\\windows\\maps\\",
        match_kind: MatchKind::Prefix,
        action: GuidanceAction::DisableIfUnused,
        rationale: "Map toast/update tasks are optional when Windows Maps is not used.",
        conditions: OPTIONAL_FEATURE_CONDITION,
        source_type: GuidanceSourceType::Community,
        source_title: "Chris Titus Tech WinUtil",
        source_url: WINUTIL_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Community,
    },
    CatalogEntry {
        target: TargetKind::ScheduledTask,
        pattern: "microsoft\\windows\\windows error reporting\\",
        match_kind: MatchKind::Prefix,
        action: GuidanceAction::DisableIfUnused,
        rationale: "Error-reporting tasks can be disabled only when the user accepts reduced diagnostic reporting.",
        conditions: COMMUNITY_CONDITION,
        source_type: GuidanceSourceType::Community,
        source_title: "Chris Titus Tech WinUtil",
        source_url: WINUTIL_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Community,
    },
    CatalogEntry {
        target: TargetKind::ScheduledTask,
        pattern: "microsoft\\windows\\windowsupdate\\",
        match_kind: MatchKind::Prefix,
        action: GuidanceAction::KeepEnabled,
        rationale: "Windows Update tasks participate in operating-system maintenance and update delivery.",
        conditions: SERVICE_DEPENDENCY_CONDITION,
        source_type: GuidanceSourceType::Microsoft,
        source_title: "Microsoft guidance: Automatic maintenance",
        source_url: TASK_MAINTENANCE_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Official,
    },
    CatalogEntry {
        target: TargetKind::ScheduledTask,
        pattern: "microsoft\\windows\\updateorchestrator\\",
        match_kind: MatchKind::Prefix,
        action: GuidanceAction::KeepEnabled,
        rationale: "Update Orchestrator tasks help Windows coordinate update maintenance.",
        conditions: SERVICE_DEPENDENCY_CONDITION,
        source_type: GuidanceSourceType::Microsoft,
        source_title: "Microsoft guidance: Automatic maintenance",
        source_url: TASK_MAINTENANCE_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Official,
    },
    CatalogEntry {
        target: TargetKind::ScheduledTask,
        pattern: "microsoft\\windows\\servicing\\",
        match_kind: MatchKind::Prefix,
        action: GuidanceAction::KeepEnabled,
        rationale: "Servicing tasks support Windows maintenance and component health.",
        conditions: SERVICE_DEPENDENCY_CONDITION,
        source_type: GuidanceSourceType::Microsoft,
        source_title: "Microsoft guidance: Automatic maintenance",
        source_url: TASK_MAINTENANCE_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Official,
    },
    CatalogEntry {
        target: TargetKind::ScheduledTask,
        pattern: "microsoft\\windows\\defender\\",
        match_kind: MatchKind::Prefix,
        action: GuidanceAction::KeepEnabled,
        rationale: "Security maintenance tasks should remain available for protection and malware scans.",
        conditions: SERVICE_DEPENDENCY_CONDITION,
        source_type: GuidanceSourceType::Microsoft,
        source_title: "Microsoft guidance: Automatic maintenance",
        source_url: TASK_MAINTENANCE_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Official,
    },
    CatalogEntry {
        target: TargetKind::ScheduledTask,
        pattern: "microsoft\\windows\\windows defender\\",
        match_kind: MatchKind::Prefix,
        action: GuidanceAction::KeepEnabled,
        rationale: "Windows Defender maintenance tasks should remain available for security work.",
        conditions: SERVICE_DEPENDENCY_CONDITION,
        source_type: GuidanceSourceType::Microsoft,
        source_title: "Microsoft guidance: Automatic maintenance",
        source_url: TASK_MAINTENANCE_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Official,
    },
    CatalogEntry {
        target: TargetKind::ScheduledTask,
        pattern: "microsoft\\windows\\maintenance\\",
        match_kind: MatchKind::Prefix,
        action: GuidanceAction::KeepEnabled,
        rationale: "Automatic maintenance runs health, update, and security work when the PC is idle and on AC power.",
        conditions: SERVICE_DEPENDENCY_CONDITION,
        source_type: GuidanceSourceType::Microsoft,
        source_title: "Microsoft guidance: Automatic maintenance",
        source_url: TASK_MAINTENANCE_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Official,
    },
    CatalogEntry {
        target: TargetKind::ScheduledTask,
        pattern: "microsoft\\windows\\bitlocker\\",
        match_kind: MatchKind::Prefix,
        action: GuidanceAction::KeepEnabled,
        rationale: "BitLocker maintenance tasks can be required for encryption and recovery workflows.",
        conditions: SERVICE_DEPENDENCY_CONDITION,
        source_type: GuidanceSourceType::Microsoft,
        source_title: "Microsoft guidance: Automatic maintenance",
        source_url: TASK_MAINTENANCE_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Official,
    },
    CatalogEntry {
        target: TargetKind::ScheduledTask,
        pattern: "microsoft\\windows\\systemrestore\\",
        match_kind: MatchKind::Prefix,
        action: GuidanceAction::KeepEnabled,
        rationale: "System Restore maintenance tasks should remain available when restore points are part of the recovery plan.",
        conditions: SERVICE_DEPENDENCY_CONDITION,
        source_type: GuidanceSourceType::Microsoft,
        source_title: "Microsoft guidance: Automatic maintenance",
        source_url: TASK_MAINTENANCE_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Official,
    },
    CatalogEntry {
        target: TargetKind::ScheduledTask,
        pattern: "microsoft\\windows\\time synchronization\\",
        match_kind: MatchKind::Prefix,
        action: GuidanceAction::KeepEnabled,
        rationale: "Time synchronization supports certificate validation, authentication, and reliable logs.",
        conditions: SERVICE_DEPENDENCY_CONDITION,
        source_type: GuidanceSourceType::Microsoft,
        source_title: "Microsoft guidance: Windows service dependencies",
        source_url: WINDOWS_SERVICES_URL,
        os_scope: DESKTOP_OS_SCOPE,
        source_tier: SourceTier::Official,
    },
    CatalogEntry {
        target: TargetKind::ScheduledTask,
        pattern: "microsoft\\xblgamesave\\",
        match_kind: MatchKind::Prefix,
        action: GuidanceAction::DisableIfUnused,
        rationale: "Xbox Game Save tasks are optional when Xbox games and synchronization are not used in the scoped deployment.",
        conditions: SERVER_ONLY_CONDITION,
        source_type: GuidanceSourceType::Microsoft,
        source_title: "Microsoft guidance: Security guidelines for Windows Server 2016 services",
        source_url: SERVER_SERVICES_URL,
        os_scope: "Windows Server 2016 Desktop Experience",
        source_tier: SourceTier::Official,
    },
];

fn normalize_name(kind: TargetKind, name: &str) -> String {
    let name = match kind {
        TargetKind::Service => name.trim(),
        TargetKind::ScheduledTask => name.trim().trim_start_matches('\\'),
    };
    name.to_ascii_lowercase()
}

fn entry_matches(entry: &CatalogEntry, kind: TargetKind, normalized_name: &str) -> bool {
    entry.target == kind
        && match entry.match_kind {
            MatchKind::Exact => normalized_name == entry.pattern.to_ascii_lowercase(),
            MatchKind::Prefix => normalized_name.starts_with(&entry.pattern.to_ascii_lowercase()),
        }
}

fn source_is_applicable(entry: &CatalogEntry, os_scope: &str) -> bool {
    entry.os_scope == os_scope || entry.os_scope == "Windows 10/11 desktop and Windows Server"
}

fn action_label(action: GuidanceAction) -> &'static str {
    match action {
        GuidanceAction::KeepEnabled => "Keep enabled",
        GuidanceAction::SetManual => "Set to Manual",
        GuidanceAction::DisableIfUnused => "Disable if unused",
        GuidanceAction::ReviewFirst => "Review first",
    }
}

fn entry_to_info(entry: &CatalogEntry, matched_by: GuidanceMatch) -> GuidanceInfo {
    let action = entry.action;
    GuidanceInfo {
        group: action,
        action,
        rationale: entry.rationale.into(),
        conditions: entry
            .conditions
            .iter()
            .map(|condition| (*condition).into())
            .collect(),
        source_type: entry.source_type,
        source_title: entry.source_title.into(),
        source_url: entry.source_url.into(),
        os_scope: entry.os_scope.into(),
        catalog_version: CATALOG_VERSION.into(),
        protected: false,
        matched_by,
    }
}

fn conflict_guidance(matches: &[&CatalogEntry]) -> GuidanceInfo {
    let mut conditions =
        vec!["Sources at the same precedence level disagree; no bulk action is offered.".into()];
    for entry in matches {
        conditions.push(format!(
            "{}: {}.",
            entry.source_title,
            action_label(entry.action)
        ));
    }
    GuidanceInfo {
        group: GuidanceAction::ReviewFirst,
        action: GuidanceAction::ReviewFirst,
        rationale:
            "Conflicting catalog entries require a local review before changing this target.".into(),
        conditions,
        source_type: GuidanceSourceType::Unknown,
        source_title: "Conflicting guidance sources".into(),
        source_url: String::new(),
        os_scope: DESKTOP_OS_SCOPE.into(),
        catalog_version: CATALOG_VERSION.into(),
        protected: false,
        matched_by: GuidanceMatch::Conflict,
    }
}

fn out_of_scope_guidance(entry: &CatalogEntry, os_scope: &str) -> GuidanceInfo {
    let mut guidance = entry_to_info(entry, GuidanceMatch::OutOfScope);
    guidance.group = GuidanceAction::ReviewFirst;
    guidance.action = GuidanceAction::ReviewFirst;
    guidance.rationale = format!(
        "{} This source is not applied as an actionable recommendation for {}.",
        entry.rationale, os_scope
    );
    guidance
        .conditions
        .push(format!("Current catalog scope: {os_scope}."));
    guidance
}

fn protected_guidance(kind: TargetKind) -> GuidanceInfo {
    let (rationale, url) = match kind {
        TargetKind::Service => (
            "This target is protected by WinOptimizer because changing it can affect core Windows dependencies or security.",
            WINDOWS_SERVICES_URL,
        ),
        TargetKind::ScheduledTask => (
            "This target is protected by WinOptimizer because it can provide Windows maintenance, updates, or security work.",
            TASK_MAINTENANCE_URL,
        ),
    };
    GuidanceInfo {
        group: GuidanceAction::KeepEnabled,
        action: GuidanceAction::KeepEnabled,
        rationale: rationale.into(),
        conditions: vec![
            "Existing protected-target rules override every catalog entry.".into(),
            "The target cannot be included in a bulk guidance plan.".into(),
        ],
        source_type: GuidanceSourceType::Policy,
        source_title: "WinOptimizer protected-target policy".into(),
        source_url: url.into(),
        os_scope: DESKTOP_OS_SCOPE.into(),
        catalog_version: CATALOG_VERSION.into(),
        protected: true,
        matched_by: GuidanceMatch::Protected,
    }
}

fn unknown_guidance() -> GuidanceInfo {
    GuidanceInfo {
        group: GuidanceAction::ReviewFirst,
        action: GuidanceAction::ReviewFirst,
        rationale: "No catalog source matches this target; review its owner, dependencies, and purpose first."
            .into(),
        conditions: vec![
            "Unknown and third-party targets are not eligible for bulk recommendations.".into(),
        ],
        source_type: GuidanceSourceType::Unknown,
        source_title: "No matching guidance source".into(),
        source_url: String::new(),
        os_scope: DESKTOP_OS_SCOPE.into(),
        catalog_version: CATALOG_VERSION.into(),
        protected: false,
        matched_by: GuidanceMatch::None,
    }
}

fn resolve(kind: TargetKind, name: &str, protected: bool, os_scope: &str) -> GuidanceInfo {
    if protected {
        return protected_guidance(kind);
    }

    let normalized_name = normalize_name(kind, name);
    let entries = SERVICE_CATALOG
        .iter()
        .filter(|entry| entry_matches(entry, kind, &normalized_name))
        .collect::<Vec<_>>();

    for (match_kind, source_tier, guidance_match) in [
        (MatchKind::Exact, SourceTier::Official, GuidanceMatch::Exact),
        (
            MatchKind::Exact,
            SourceTier::Community,
            GuidanceMatch::Exact,
        ),
        (
            MatchKind::Prefix,
            SourceTier::Official,
            GuidanceMatch::Pattern,
        ),
        (
            MatchKind::Prefix,
            SourceTier::Community,
            GuidanceMatch::Pattern,
        ),
    ] {
        let matches = entries
            .iter()
            .copied()
            .filter(|entry| {
                entry.match_kind == match_kind
                    && entry.source_tier == source_tier
                    && source_is_applicable(entry, os_scope)
            })
            .collect::<Vec<_>>();
        if !matches.is_empty() {
            let actions = matches
                .iter()
                .map(|entry| entry.action)
                .collect::<BTreeSet<_>>();
            return if actions.len() > 1 {
                conflict_guidance(&matches)
            } else {
                entry_to_info(matches[0], guidance_match)
            };
        }
    }

    // Preserve a precise source explanation when a server-only entry matched,
    // but do not turn that source into a desktop disable recommendation.
    if let Some(entry) = entries
        .iter()
        .copied()
        .find(|entry| entry.match_kind == MatchKind::Exact)
    {
        return out_of_scope_guidance(entry, os_scope);
    }
    if let Some(entry) = entries
        .iter()
        .copied()
        .find(|entry| entry.match_kind == MatchKind::Prefix)
    {
        return out_of_scope_guidance(entry, os_scope);
    }
    unknown_guidance()
}

pub fn service_guidance(service_name: &str) -> GuidanceInfo {
    resolve(
        TargetKind::Service,
        service_name,
        crate::startup::services::is_protected(service_name),
        DESKTOP_OS_SCOPE,
    )
}

pub fn scheduled_task_guidance(task_name: &str) -> GuidanceInfo {
    resolve(
        TargetKind::ScheduledTask,
        task_name,
        crate::startup::scheduler::is_protected(task_name),
        DESKTOP_OS_SCOPE,
    )
}

pub fn service_guidance_for_scope(service_name: &str, os_scope: &str) -> GuidanceInfo {
    resolve(
        TargetKind::Service,
        service_name,
        crate::startup::services::is_protected(service_name),
        os_scope,
    )
}

pub fn scheduled_task_guidance_for_scope(task_name: &str, os_scope: &str) -> GuidanceInfo {
    resolve(
        TargetKind::ScheduledTask,
        task_name,
        crate::startup::scheduler::is_protected(task_name),
        os_scope,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_targets_override_catalog_matches() {
        let guidance = service_guidance("RpcSs");
        assert_eq!(guidance.action, GuidanceAction::KeepEnabled);
        assert!(guidance.protected);
        assert_eq!(guidance.matched_by, GuidanceMatch::Protected);

        let task = scheduled_task_guidance(r"\Microsoft\Windows\WindowsUpdate\Scheduled Start");
        assert_eq!(task.action, GuidanceAction::KeepEnabled);
        assert!(task.protected);
    }

    #[test]
    fn exact_official_guidance_is_labeled() {
        let guidance = service_guidance("BITS");
        assert_eq!(guidance.action, GuidanceAction::SetManual);
        assert_eq!(guidance.source_type, GuidanceSourceType::Microsoft);
        assert_eq!(guidance.matched_by, GuidanceMatch::Exact);

        let evidence = guidance.evidence();
        assert_eq!(evidence.source_type, EvidenceSourceType::MicrosoftGuidance);
        assert_eq!(evidence.matching_rule, EvidenceMatchingRule::Exact);
        assert_eq!(evidence.confidence, EvidenceConfidence::High);
        assert_eq!(evidence.conflict, EvidenceConflict::None);
    }

    #[test]
    fn same_tier_conflicts_are_review_first() {
        let guidance = service_guidance("WSearch");
        assert_eq!(guidance.action, GuidanceAction::ReviewFirst);
        assert_eq!(guidance.matched_by, GuidanceMatch::Conflict);

        let sysmain = service_guidance("SysMain");
        assert_eq!(sysmain.action, GuidanceAction::ReviewFirst);
        assert_eq!(sysmain.matched_by, GuidanceMatch::Conflict);
        assert_eq!(sysmain.evidence().conflict, EvidenceConflict::Conflicting);
    }

    #[test]
    fn server_guidance_does_not_become_desktop_disable_advice() {
        let guidance = service_guidance("Spooler");
        assert_eq!(guidance.action, GuidanceAction::ReviewFirst);
        assert_eq!(guidance.matched_by, GuidanceMatch::OutOfScope);
        assert!(guidance.os_scope.contains("Server 2016"));
    }

    #[test]
    fn unknown_and_third_party_targets_are_review_first() {
        let service = service_guidance("AcmeVendorService");
        assert_eq!(service.action, GuidanceAction::ReviewFirst);
        assert_eq!(service.source_type, GuidanceSourceType::Unknown);

        let task = scheduled_task_guidance(r"\Vendor\Task");
        assert_eq!(task.action, GuidanceAction::ReviewFirst);
        assert_eq!(task.source_type, GuidanceSourceType::Unknown);
    }

    #[test]
    fn community_task_pattern_is_explicitly_labeled() {
        let guidance = scheduled_task_guidance(
            r"\Microsoft\Windows\Customer Experience Improvement Program\Consolidator",
        );
        assert_eq!(guidance.action, GuidanceAction::DisableIfUnused);
        assert_eq!(guidance.source_type, GuidanceSourceType::Community);
        assert_eq!(guidance.matched_by, GuidanceMatch::Pattern);
    }
}
