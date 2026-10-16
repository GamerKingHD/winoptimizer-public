//! Privacy tweaks - disable telemetry, Cortana, activity history,
//! advertising ID, location tracking, and other data-collection features.
//!
//! Includes deep telemetry mitigation (Section 7): DiagTrack service,
//! WMI Autologger decommissioning, and scheduled telemetry task disabling.
//!
//! The governed operation journal captures the target state; exact automated
//! restoration remains disabled for this domain until it is proven.
//!
//! ## Safety tiers:
//! - **Low:** Registry policy keys that don't break functionality
//! - **Extreme:** Service stops, autologger disabling, task disabling
//!   (may impact enterprise device management, Insider updates, support diagnostics)

use super::registry_ops::{self as ops, MissingKey};
use serde::{Deserialize, Serialize};

use crate::core::command_helpers::create_command;
use crate::core::system_tweak_operation::{TweakPhysicalResource, TweakPhysicalState};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacyTweak {
    pub id: String,
    pub name: String,
    pub description: String,
    pub risk: String, // "low", "medium", "high"
    pub is_applied: bool,
}

/// List available privacy tweaks.
pub fn list() -> Vec<PrivacyTweak> {
    let mut tweaks = vec![
        PrivacyTweak {
            id: "disable_telemetry".into(),
            name: "Minimize Windows Diagnostic Data".into(),
            description: "Applies the documented diagnostic-data policy values available to this Windows edition; it does not disable every diagnostic signal.".into(),
            risk: "low".into(),
            is_applied: is_applied("disable_telemetry"),
        },
        PrivacyTweak {
            id: "disable_cortana".into(),
            name: "Disable Cortana".into(),
            description: "Disables Cortana voice assistant and web search".into(),
            risk: "low".into(),
            is_applied: is_applied("disable_cortana"),
        },
        PrivacyTweak {
            id: "disable_activity_history".into(),
            name: "Disable Activity History".into(),
            description: "Prevents Windows from collecting activity history".into(),
            risk: "low".into(),
            is_applied: is_applied("disable_activity_history"),
        },
        PrivacyTweak {
            id: "disable_advertising_id".into(),
            name: "Disable Advertising ID".into(),
            description: "Resets and disables the unique advertising ID".into(),
            risk: "low".into(),
            is_applied: is_applied("disable_advertising_id"),
        },
        PrivacyTweak {
            id: "disable_location_tracking".into(),
            name: "Disable Location Tracking".into(),
            description: "Disables location services and sensor tracking".into(),
            risk: "low".into(),
            is_applied: is_applied("disable_location_tracking"),
        },
        PrivacyTweak {
            id: "disable_web_search".into(),
            name: "Disable Bing and Search Suggestions".into(),
            description: "Disables Bing web results and the supported Windows search-box suggestion policy; this does not disable local file search.".into(),
            risk: "low".into(),
            is_applied: is_applied("disable_web_search"),
        },
        PrivacyTweak {
            id: "disable_feedback".into(),
            name: "Disable Feedback Requests".into(),
            description: "Disables periodic feedback and survey requests".into(),
            risk: "low".into(),
            is_applied: is_applied("disable_feedback"),
        },
        PrivacyTweak {
            id: "disable_tailored_experiences".into(),
            name: "Disable Tailored Experiences".into(),
            description: "Stops Windows from using diagnostic data for tailored experiences.".into(),
            risk: "low".into(),
            is_applied: is_applied("disable_tailored_experiences"),
        },
        PrivacyTweak {
            id: "disable_app_launch_tracking".into(),
            name: "Disable App Launch Tracking".into(),
            description: "Stops Windows from tracking application launches for Start and search improvements.".into(),
            risk: "low".into(),
            is_applied: is_applied("disable_app_launch_tracking"),
        },
        PrivacyTweak {
            id: "disable_delivery_optimization_p2p".into(),
            name: "Disable Delivery Optimization Peer-to-Peer Sharing".into(),
            description: "Prevents peer-to-peer content sharing while leaving Delivery Optimization itself available.".into(),
            risk: "low".into(),
            is_applied: is_applied("disable_delivery_optimization_p2p"),
        },
        PrivacyTweak {
            id: "disable_find_my_device".into(),
            name: "Disable Find My Device".into(),
            description: "Stops Windows from registering this device and its location for Find My Device. This is separate from app location access.".into(),
            risk: "medium".into(),
            is_applied: is_applied("disable_find_my_device"),
        },
        // -- Deep Telemetry (Section 7) -----------------------
        PrivacyTweak {
            id: "stop_diagtrack_service".into(),
            name: "Stop DiagTrack Service".into(),
            description: "Stops and disables the Connected User Experiences and Telemetry (DiagTrack) service. Eliminates background telemetry transmission. May break Diagnostic Data Viewer and some Insider updates.".into(),
            risk: "extreme".into(),
            is_applied: is_applied("stop_diagtrack_service"),
        },
        PrivacyTweak {
            id: "disable_telemetry_autologger".into(),
            name: "Disable Telemetry Autologger".into(),
            description: "Disables the Diagtrack-Listener WMI Autologger session. Stops early-boot event capture and continuous ETL trace writing. Disables boot-phase system logging.".into(),
            risk: "extreme".into(),
            is_applied: is_applied("disable_telemetry_autologger"),
        },
        PrivacyTweak {
            id: "disable_telemetry_tasks".into(),
            name: "Disable Telemetry Scheduled Tasks".into(),
            description: "Disables Microsoft Compatibility Appraiser and CEIP Consolidator scheduled tasks. Prevents background application inventory and usage metric collection.".into(),
            risk: "extreme".into(),
            is_applied: is_applied("disable_telemetry_tasks"),
        },
    ];
    if optional_feature_state("Recall").ok().flatten().is_some() {
        tweaks.push(PrivacyTweak {
            id: "disable_recall".into(),
            name: "Disable Windows Recall".into(),
            description: "Disables the Recall optional Windows feature when it is present; no package removal is performed.".into(),
            risk: "medium".into(),
            is_applied: is_applied("disable_recall"),
        });
    }
    if supports_windows_ai() {
        tweaks.push(PrivacyTweak {
            id: "disable_copilot".into(),
            name: "Disable Windows Copilot".into(),
            description: "Applies the user-scoped Windows Copilot policy where supported; this does not remove the Copilot package.".into(),
            risk: "medium".into(),
            is_applied: is_applied("disable_copilot"),
        });
    }
    if supports_click_to_do() {
        tweaks.push(PrivacyTweak {
            id: "disable_click_to_do".into(),
            name: "Disable Click to Do".into(),
            description: "Applies the Windows AI policy that removes Click to Do entry points; it does not remove Recall or AI packages.".into(),
            risk: "medium".into(),
            is_applied: is_applied("disable_click_to_do"),
        });
    }
    if supports_modern_standby() {
        tweaks.push(PrivacyTweak {
            id: "disable_modern_standby_networking".into(),
            name: "Disable Modern Standby Networking".into(),
            description: "Disables network connectivity during Modern Standby on S0 Low Power Idle systems; this can affect notifications and Remote Desktop wake behavior.".into(),
            risk: "medium".into(),
            is_applied: is_applied("disable_modern_standby_networking"),
        });
    }
    if crate::startup::services::inspect("WSAIFabricSvc").is_ok() {
        tweaks.push(PrivacyTweak {
            id: "disable_ai_service_autostart".into(),
            name: "Disable Windows AI Service Auto-Start".into(),
            description: "Stops and disables WSAIFabricSvc when this Windows AI service exists; service identity and behavior vary by Windows build.".into(),
            risk: "high".into(),
            is_applied: is_applied("disable_ai_service_autostart"),
        });
    }
    tweaks
}

fn is_applied(tweak_id: &str) -> bool {
    match tweak_id {
        "disable_telemetry" => has_dwords(&[
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\DataCollection",
                "AllowTelemetry",
                0,
            ),
            (
                r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\DataCollection",
                "AllowTelemetry",
                0,
            ),
            (
                r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Diagnostics\DiagTrack",
                "ShowedToastAtSession",
                1,
            ),
        ]),
        "disable_cortana" => has_dwords(&[
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\Windows Search",
                "AllowCortana",
                0,
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\Windows Search",
                "AllowSearchToUseLocation",
                0,
            ),
        ]),
        "disable_activity_history" => has_dwords(&[
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\System",
                "EnableActivityFeed",
                0,
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\System",
                "PublishUserActivities",
                0,
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\System",
                "UploadUserActivities",
                0,
            ),
        ]),
        "disable_advertising_id" => has_dwords(&[(
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\AdvertisingInfo",
            "Enabled",
            0,
        )]),
        "disable_location_tracking" => has_dwords(&[
            (
                r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\location",
                "Value",
                0,
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\LocationAndSensors",
                "DisableLocation",
                1,
            ),
        ]),
        "disable_web_search" => has_dwords(&[
            (
                r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Search",
                "BingSearchEnabled",
                0,
            ),
            (
                r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Search",
                "AllowSearchToUseLocation",
                0,
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\Explorer",
                "DisableSearchBoxSuggestions",
                1,
            ),
        ]),
        "disable_feedback" => has_dwords(&[
            (
                r"HKCU\SOFTWARE\Microsoft\Siuf\Rules",
                "NumberOfSIUFInPeriod",
                0,
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\DataCollection",
                "DoNotShowFeedbackNotifications",
                1,
            ),
        ]),
        "disable_tailored_experiences" => has_dwords(&[(
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows\CloudContent",
            "DisableTailoredExperiencesWithDiagnosticData",
            1,
        )]),
        "disable_app_launch_tracking" => has_dwords(&[(
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "Start_TrackProgs",
            0,
        )]),
        "disable_delivery_optimization_p2p" => has_dwords(&[(
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows\DeliveryOptimization",
            "DODownloadMode",
            0,
        )]),
        "disable_copilot" => has_dwords(&[(
            r"HKCU\SOFTWARE\Policies\Microsoft\Windows\WindowsCopilot",
            "TurnOffWindowsCopilot",
            1,
        )]),
        "disable_find_my_device" => has_dwords(&[(
            r"HKLM\SOFTWARE\Policies\Microsoft\FindMyDevice",
            "AllowFindMyDevice",
            0,
        )]),
        "disable_recall" => optional_feature_state("Recall")
            .ok()
            .flatten()
            .is_some_and(|enabled| !enabled),
        "disable_ai_service_autostart" => service_is_disabled("WSAIFabricSvc"),
        "disable_click_to_do" => has_dwords(&[(
            r"HKCU\SOFTWARE\Policies\Microsoft\Windows\WindowsAI",
            "DisableClickToDo",
            1,
        )]),
        "disable_modern_standby_networking" => has_dwords(&[
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Power\PowerSettings\f15576e8-98b7-4186-b944-eafa664402d9",
                "ACSettingIndex",
                0,
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Power\PowerSettings\f15576e8-98b7-4186-b944-eafa664402d9",
                "DCSettingIndex",
                0,
            ),
        ]),
        "stop_diagtrack_service" => service_is_disabled("DiagTrack"),
        "disable_telemetry_autologger" => has_dwords(&[(
            r"HKLM\SYSTEM\CurrentControlSet\Control\WMI\Autologger\Diagtrack-Listener",
            "Start",
            0,
        )]),
        "disable_telemetry_tasks" => {
            task_is_disabled(
                r"\Microsoft\Windows\Application Experience\Microsoft Compatibility Appraiser",
            ) && task_is_disabled(
                r"\Microsoft\Windows\Customer Experience Improvement Program\Consolidator",
            )
        }
        _ => false,
    }
}

fn has_dwords(expected: &[(&str, &str, u32)]) -> bool {
    expected
        .iter()
        .all(|(key, value, data)| read_current_dword_with_existence(key, value) == (true, *data))
}

fn service_is_disabled(service: &str) -> bool {
    crate::startup::services::inspect(service)
        .map(|current| current.start_type.eq_ignore_ascii_case("disabled"))
        .unwrap_or(false)
}

fn task_is_disabled(task: &str) -> bool {
    crate::startup::scheduler::find_task(task)
        .map(|current| !current.definition.enabled)
        .unwrap_or(false)
}

pub fn capture_state(tweak_id: &str) -> Result<TweakPhysicalState, String> {
    let resources = match tweak_id {
        "disable_telemetry" => registry_resources(&[
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\DataCollection",
                "AllowTelemetry",
            ),
            (
                r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\DataCollection",
                "AllowTelemetry",
            ),
            (
                r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Diagnostics\DiagTrack",
                "ShowedToastAtSession",
            ),
        ]),
        "disable_cortana" => registry_resources(&[
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\Windows Search",
                "AllowCortana",
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\Windows Search",
                "AllowSearchToUseLocation",
            ),
        ]),
        "disable_activity_history" => registry_resources(&[
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\System",
                "EnableActivityFeed",
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\System",
                "PublishUserActivities",
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\System",
                "UploadUserActivities",
            ),
        ]),
        "disable_advertising_id" => registry_resources(&[(
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\AdvertisingInfo",
            "Enabled",
        )]),
        "disable_location_tracking" => registry_resources(&[
            (
                r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\location",
                "Value",
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\LocationAndSensors",
                "DisableLocation",
            ),
        ]),
        "disable_web_search" => registry_resources(&[
            (
                r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Search",
                "BingSearchEnabled",
            ),
            (
                r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Search",
                "AllowSearchToUseLocation",
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\Explorer",
                "DisableSearchBoxSuggestions",
            ),
        ]),
        "disable_feedback" => registry_resources(&[
            (
                r"HKCU\SOFTWARE\Microsoft\Siuf\Rules",
                "NumberOfSIUFInPeriod",
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\DataCollection",
                "DoNotShowFeedbackNotifications",
            ),
        ]),
        "disable_tailored_experiences" => registry_resources(&[(
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows\CloudContent",
            "DisableTailoredExperiencesWithDiagnosticData",
        )]),
        "disable_app_launch_tracking" => registry_resources(&[(
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "Start_TrackProgs",
        )]),
        "disable_delivery_optimization_p2p" => registry_resources(&[(
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows\DeliveryOptimization",
            "DODownloadMode",
        )]),
        "disable_copilot" => registry_resources(&[(
            r"HKCU\SOFTWARE\Policies\Microsoft\Windows\WindowsCopilot",
            "TurnOffWindowsCopilot",
        )]),
        "disable_find_my_device" => registry_resources(&[(
            r"HKLM\SOFTWARE\Policies\Microsoft\FindMyDevice",
            "AllowFindMyDevice",
        )]),
        "disable_recall" => vec![capture_optional_feature_resource("Recall")?],
        "disable_ai_service_autostart" => vec![capture_service_resource("WSAIFabricSvc")?],
        "disable_click_to_do" => registry_resources(&[(
            r"HKCU\SOFTWARE\Policies\Microsoft\Windows\WindowsAI",
            "DisableClickToDo",
        )]),
        "disable_modern_standby_networking" => registry_resources(&[
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Power\PowerSettings\f15576e8-98b7-4186-b944-eafa664402d9",
                "ACSettingIndex",
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Power\PowerSettings\f15576e8-98b7-4186-b944-eafa664402d9",
                "DCSettingIndex",
            ),
        ]),
        "stop_diagtrack_service" => vec![capture_service_resource("DiagTrack")?],
        "disable_telemetry_autologger" => registry_resources(&[(
            r"HKLM\SYSTEM\CurrentControlSet\Control\WMI\Autologger\Diagtrack-Listener",
            "Start",
        )]),
        "disable_telemetry_tasks" => vec![
            capture_task_resource(
                r"\Microsoft\Windows\Application Experience\Microsoft Compatibility Appraiser",
            )?,
            capture_task_resource(
                r"\Microsoft\Windows\Customer Experience Improvement Program\Consolidator",
            )?,
        ],
        _ => return Err(format!("Unknown tweak: {tweak_id}")),
    };
    Ok(TweakPhysicalState { resources })
}

pub fn restore_state(tweak_id: &str, state: &TweakPhysicalState) -> Result<(), String> {
    match tweak_id {
        "stop_diagtrack_service" => restore_service_resource(state, "DiagTrack"),
        "disable_telemetry_tasks" => {
            restore_task_resource(
                state,
                r"\Microsoft\Windows\Application Experience\Microsoft Compatibility Appraiser",
            )?;
            restore_task_resource(
                state,
                r"\Microsoft\Windows\Customer Experience Improvement Program\Consolidator",
            )
        }
        "disable_recall" => restore_optional_feature_resource(state, "Recall"),
        "disable_ai_service_autostart" => restore_service_resource(state, "WSAIFabricSvc"),
        _ => {
            for resource in &state.resources {
                match resource {
                    TweakPhysicalResource::RegistryDword { .. } => {
                        restore_registry_resource(resource)?
                    }
                    _ => return Err(format!("Unexpected resource in privacy tweak {tweak_id}")),
                }
            }
            Ok(())
        }
    }
}

fn registry_resources(entries: &[(&str, &str)]) -> Vec<TweakPhysicalResource> {
    entries
        .iter()
        .map(|(full_key, value_name)| capture_registry_resource(full_key, value_name))
        .collect()
}

fn optional_feature_state(name: &str) -> Result<Option<bool>, String> {
    let script = format!(
        "$feature = Get-WindowsOptionalFeature -Online -FeatureName '{name}' -ErrorAction SilentlyContinue; if ($null -eq $feature) {{ exit 3 }}; $feature.State"
    );
    let output = match create_command("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .output()
    {
        Ok(output) => output,
        Err(_) => return Ok(None),
    };
    if !output.status.success() {
        return Ok(None);
    }
    let state = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_ascii_lowercase();
    match state.as_str() {
        "enabled" => Ok(Some(true)),
        "disabled" | "enable pending" | "disable pending" => Ok(Some(false)),
        _ => Ok(None),
    }
}

fn supports_windows_ai() -> bool {
    crate::system_info::os::gather()
        .build
        .parse::<u32>()
        .is_ok_and(|build| build >= 22000)
        && crate::startup::services::inspect("WSAIFabricSvc").is_ok()
}

fn supports_click_to_do() -> bool {
    crate::system_info::os::gather()
        .build
        .parse::<u32>()
        .is_ok_and(|build| build >= 26100)
        && supports_windows_ai()
}

fn supports_modern_standby() -> bool {
    let output = match create_command("powercfg.exe").args(["/a"]).output() {
        Ok(output) if output.status.success() => output,
        _ => return false,
    };
    String::from_utf8_lossy(&output.stdout)
        .to_ascii_lowercase()
        .contains("s0 low power idle")
}

fn capture_optional_feature_resource(name: &str) -> Result<TweakPhysicalResource, String> {
    let enabled = optional_feature_state(name)?.ok_or_else(|| {
        format!("Optional feature {name} is not available or its state is unknown")
    })?;
    Ok(TweakPhysicalResource::OptionalFeature {
        name: name.into(),
        existed: true,
        enabled,
    })
}

fn restore_optional_feature_resource(state: &TweakPhysicalState, name: &str) -> Result<(), String> {
    let resource = state
        .resources
        .iter()
        .find(|resource| {
            matches!(
                resource,
                TweakPhysicalResource::OptionalFeature {
                    name: resource_name,
                    ..
                } if resource_name == name
            )
        })
        .ok_or_else(|| format!("Missing optional-feature capture for {name}"))?;
    let TweakPhysicalResource::OptionalFeature {
        existed, enabled, ..
    } = resource
    else {
        unreachable!();
    };
    if !existed {
        return Err(format!(
            "Optional feature {name} was not present in the captured state"
        ));
    }
    set_optional_feature(name, *enabled)
}

fn set_optional_feature(name: &str, enabled: bool) -> Result<(), String> {
    let command = if enabled {
        format!("Enable-WindowsOptionalFeature -Online -FeatureName '{name}' -NoRestart")
    } else {
        format!("Disable-WindowsOptionalFeature -Online -FeatureName '{name}' -NoRestart")
    };
    let output = create_command("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &command,
        ])
        .output()
        .map_err(|error| format!("Optional feature command failed: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Failed to {} optional feature {name}",
            if enabled { "enable" } else { "disable" }
        ));
    }
    Ok(())
}

fn capture_registry_resource(full_key: &str, value_name: &str) -> TweakPhysicalResource {
    ops::capture_resource(full_key, value_name)
}

fn capture_service_resource(service: &str) -> Result<TweakPhysicalResource, String> {
    let current = crate::startup::services::inspect(service)?;
    Ok(TweakPhysicalResource::Service {
        name: service.into(),
        existed: true,
        start_type: Some(current.start_type),
        running: Some(current.status.eq_ignore_ascii_case("running")),
        delayed_auto_start: current.delayed_auto_start,
    })
}

fn capture_task_resource(task: &str) -> Result<TweakPhysicalResource, String> {
    match crate::startup::scheduler::find_task(task) {
        Ok(current) => Ok(TweakPhysicalResource::ScheduledTask {
            path: task.into(),
            existed: true,
            enabled: Some(current.definition.enabled),
            current_state: Some(current.definition.current_state.clone()),
            definition: Some(current.definition),
        }),
        Err(crate::startup::scheduler::SchedulerError::NotFound(_)) => {
            Ok(TweakPhysicalResource::ScheduledTask {
                path: task.into(),
                existed: false,
                enabled: None,
                current_state: None,
                definition: None,
            })
        }
        Err(error) => Err(error.to_string()),
    }
}

fn restore_registry_resource(resource: &TweakPhysicalResource) -> Result<(), String> {
    ops::restore_resource(resource, MissingKey::Error)
}

fn restore_service_resource(state: &TweakPhysicalState, service: &str) -> Result<(), String> {
    let resource = state
        .resources
        .iter()
        .find(|resource| {
            matches!(resource, TweakPhysicalResource::Service { name, .. } if name == service)
        })
        .ok_or_else(|| format!("Missing service capture for {service}"))?;
    let TweakPhysicalResource::Service {
        existed,
        start_type,
        running,
        ..
    } = resource
    else {
        unreachable!();
    };
    if !existed {
        return Err(format!(
            "Service {service} was not present in the captured state"
        ));
    }
    let start_type = start_type.as_deref().unwrap_or("unknown");
    let start_type = if start_type.eq_ignore_ascii_case("auto") {
        "auto"
    } else if start_type.eq_ignore_ascii_case("manual") || start_type.eq_ignore_ascii_case("demand")
    {
        "demand"
    } else if start_type.eq_ignore_ascii_case("disabled") {
        "disabled"
    } else {
        return Err(format!(
            "Unsupported captured startup mode {start_type} for {service}"
        ));
    };
    crate::startup::services::set_start_type(service, start_type)?;
    if let Some(delayed_auto_start) = resource_delayed_auto_start(state, service) {
        crate::startup::services::set_delayed_auto_start(service, delayed_auto_start)?;
    }
    let current = crate::startup::services::inspect(service)?;
    let should_run = running.unwrap_or(false);
    if should_run && !current.status.eq_ignore_ascii_case("running") {
        crate::startup::services::control_service(service, "start")?;
    } else if !should_run && current.status.eq_ignore_ascii_case("running") {
        crate::startup::services::control_service(service, "stop")?;
    }
    Ok(())
}

fn resource_delayed_auto_start(state: &TweakPhysicalState, service: &str) -> Option<bool> {
    state.resources.iter().find_map(|resource| match resource {
        TweakPhysicalResource::Service {
            name,
            delayed_auto_start,
            ..
        } if name == service => *delayed_auto_start,
        _ => None,
    })
}

fn restore_task_resource(state: &TweakPhysicalState, task: &str) -> Result<(), String> {
    let resource = state
        .resources
        .iter()
        .find(|resource| {
            matches!(resource, TweakPhysicalResource::ScheduledTask { path, .. } if path == task)
        })
        .ok_or_else(|| format!("Missing scheduled-task capture for {task}"))?;
    let TweakPhysicalResource::ScheduledTask {
        existed,
        enabled,
        definition,
        ..
    } = resource
    else {
        unreachable!();
    };
    if !existed {
        if crate::startup::scheduler::find_task(task).is_ok() {
            return Err(format!("Scheduled task {task} appeared after capture"));
        }
        return Ok(());
    }
    if let Some(definition) = definition {
        crate::startup::scheduler::restore_task_definition(definition)
            .map(|_| ())
            .map_err(|error| error.to_string())
    } else {
        crate::startup::scheduler::set_task_state(task, enabled.unwrap_or(false))
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

/// Read the current DWORD value and whether it existed.
/// Returns (existed, value).
fn read_current_dword_with_existence(full_key: &str, value_name: &str) -> (bool, u32) {
    let (hive_str, path) = split_hive(full_key);
    let hive = match hive_str {
        "HKLM" => windows::Win32::System::Registry::HKEY_LOCAL_MACHINE,
        "HKCU" => windows::Win32::System::Registry::HKEY_CURRENT_USER,
        _ => return (false, 0),
    };

    use crate::core::registry_helpers;
    use windows::Win32::System::Registry::RegCloseKey;

    unsafe {
        let key = registry_helpers::open_key_read(hive, &path);
        if let Some(k) = key {
            match registry_helpers::read_reg_dword(k, value_name) {
                Some(val) => {
                    let _ = RegCloseKey(k);
                    return (true, val);
                }
                None => {
                    let _ = RegCloseKey(k);
                    return (false, 0);
                }
            }
        }
    }
    (false, 0)
}

/// Apply a set of registry values after the operation layer has captured the
/// current target state.
fn apply_registry_ops(new_values: Vec<(&str, &str, u32)>) -> Result<(), String> {
    for &(full_key, value_name, new_val) in &new_values {
        set_registry_dword(full_key, value_name, new_val)?;
    }

    Ok(())
}

/// Apply a privacy tweak by writing the corresponding registry key.
pub fn apply(tweak_id: &str) -> Result<(), String> {
    let _name = match tweak_id {
        "disable_telemetry" => "Disable Telemetry",
        "disable_cortana" => "Disable Cortana",
        "disable_activity_history" => "Disable Activity History",
        "disable_advertising_id" => "Disable Advertising ID",
        "disable_location_tracking" => "Disable Location Tracking",
        "disable_web_search" => "Disable Bing and Search Suggestions",
        "disable_feedback" => "Disable Feedback Requests",
        "disable_tailored_experiences" => "Disable Tailored Experiences",
        "disable_app_launch_tracking" => "Disable App Launch Tracking",
        "disable_delivery_optimization_p2p" => "Disable Delivery Optimization Peer-to-Peer Sharing",
        "disable_copilot" => "Disable Windows Copilot",
        "disable_find_my_device" => "Disable Find My Device",
        "disable_recall" => "Disable Windows Recall",
        "disable_click_to_do" => "Disable Click to Do",
        "disable_ai_service_autostart" => "Disable Windows AI Service Auto-Start",
        "disable_modern_standby_networking" => "Disable Modern Standby Networking",
        // Deep telemetry
        "stop_diagtrack_service" => "Stop DiagTrack Service",
        "disable_telemetry_autologger" => "Disable Telemetry Autologger",
        "disable_telemetry_tasks" => "Disable Telemetry Tasks",
        _ => return Err(format!("Unknown tweak: {}", tweak_id)),
    };

    let ops: Vec<(&str, &str, u32)> = match tweak_id {
        "disable_telemetry" => vec![
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\DataCollection",
                "AllowTelemetry",
                0,
            ),
            (
                r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\DataCollection",
                "AllowTelemetry",
                0,
            ),
            (
                r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Diagnostics\DiagTrack",
                "ShowedToastAtSession",
                1,
            ),
        ],
        "disable_cortana" => vec![
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\Windows Search",
                "AllowCortana",
                0,
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\Windows Search",
                "AllowSearchToUseLocation",
                0,
            ),
        ],
        "disable_activity_history" => vec![
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\System",
                "EnableActivityFeed",
                0,
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\System",
                "PublishUserActivities",
                0,
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\System",
                "UploadUserActivities",
                0,
            ),
        ],
        "disable_advertising_id" => vec![(
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\AdvertisingInfo",
            "Enabled",
            0,
        )],
        "disable_location_tracking" => vec![
            (
                r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\location",
                "Value",
                0,
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\LocationAndSensors",
                "DisableLocation",
                1,
            ),
        ],
        "disable_web_search" => vec![
            (
                r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Search",
                "BingSearchEnabled",
                0,
            ),
            (
                r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Search",
                "AllowSearchToUseLocation",
                0,
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\Explorer",
                "DisableSearchBoxSuggestions",
                1,
            ),
        ],
        "disable_feedback" => vec![
            (
                r"HKCU\SOFTWARE\Microsoft\Siuf\Rules",
                "NumberOfSIUFInPeriod",
                0,
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\DataCollection",
                "DoNotShowFeedbackNotifications",
                1,
            ),
        ],
        "disable_tailored_experiences" => vec![(
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows\CloudContent",
            "DisableTailoredExperiencesWithDiagnosticData",
            1,
        )],
        "disable_app_launch_tracking" => vec![(
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "Start_TrackProgs",
            0,
        )],
        "disable_delivery_optimization_p2p" => vec![(
            r"HKLM\SOFTWARE\Policies\Microsoft\Windows\DeliveryOptimization",
            "DODownloadMode",
            0,
        )],
        "disable_copilot" => vec![(
            r"HKCU\SOFTWARE\Policies\Microsoft\Windows\WindowsCopilot",
            "TurnOffWindowsCopilot",
            1,
        )],
        "disable_find_my_device" => vec![(
            r"HKLM\SOFTWARE\Policies\Microsoft\FindMyDevice",
            "AllowFindMyDevice",
            0,
        )],
        "disable_recall" | "disable_ai_service_autostart" => vec![],
        "disable_click_to_do" => vec![(
            r"HKCU\SOFTWARE\Policies\Microsoft\Windows\WindowsAI",
            "DisableClickToDo",
            1,
        )],
        "disable_modern_standby_networking" => vec![
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Power\PowerSettings\f15576e8-98b7-4186-b944-eafa664402d9",
                "ACSettingIndex",
                0,
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Power\PowerSettings\f15576e8-98b7-4186-b944-eafa664402d9",
                "DCSettingIndex",
                0,
            ),
        ],
        // Deep telemetry - no registry ops (service/task changes below)
        "stop_diagtrack_service" | "disable_telemetry_tasks" => vec![],
        "disable_telemetry_autologger" => vec![(
            r"HKLM\SYSTEM\CurrentControlSet\Control\WMI\Autologger\Diagtrack-Listener",
            "Start",
            0,
        )],
        _ => return Err(format!("Unknown tweak: {}", tweak_id)),
    };

    if !ops.is_empty() {
        apply_registry_ops(ops)?;
    }

    // Handle non-registry tweaks (service + task changes)
    match tweak_id {
        "stop_diagtrack_service" => {
            let current = crate::startup::services::inspect("DiagTrack")?;
            if !current.status.eq_ignore_ascii_case("Stopped") {
                crate::startup::services::control_service("DiagTrack", "stop")?;
            }
            crate::startup::services::set_start_type("DiagTrack", "disabled")?;
        }
        "disable_telemetry_tasks" => {
            let tasks = [
                (
                    r"\Microsoft\Windows\Application Experience",
                    "Microsoft Compatibility Appraiser",
                ),
                (
                    r"\Microsoft\Windows\Customer Experience Improvement Program",
                    "Consolidator",
                ),
            ];
            for (folder, task_name) in &tasks {
                let full_name = format!(r"{}\{}", folder, task_name);
                crate::startup::scheduler::set_task_state(&full_name, false)
                    .map_err(|error| error.to_string())?;
            }
        }
        "disable_recall" => set_optional_feature("Recall", false)?,
        "disable_ai_service_autostart" => {
            let current = crate::startup::services::inspect("WSAIFabricSvc")?;
            if !current.status.eq_ignore_ascii_case("Stopped") {
                crate::startup::services::control_service("WSAIFabricSvc", "stop")?;
            }
            crate::startup::services::set_start_type("WSAIFabricSvc", "disabled")?;
        }
        "disable_click_to_do" => {}
        _ => {}
    }

    Ok(())
}

/// Revert a privacy tweak (delete the registry keys).
pub fn revert(tweak_id: &str) -> Result<(), String> {
    let _name = match tweak_id {
        "disable_telemetry" => "Disable Telemetry",
        "disable_cortana" => "Disable Cortana",
        "disable_advertising_id" => "Disable Advertising ID",
        "disable_web_search" => "Disable Web Search in Start",
        // Deep telemetry
        "stop_diagtrack_service" => "Stop DiagTrack Service",
        "disable_telemetry_autologger" => "Disable Telemetry Autologger",
        "disable_telemetry_tasks" => "Disable Telemetry Tasks",
        _ => {
            // Some tweaks don't support revert (activity_history, location, feedback
            // don't have revert logic - they just skip)
            return Ok(());
        }
    };

    let ops: Vec<(&str, &str)> = match tweak_id {
        "disable_telemetry" => vec![
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\DataCollection",
                "AllowTelemetry",
            ),
            (
                r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\DataCollection",
                "AllowTelemetry",
            ),
        ],
        "disable_cortana" => vec![
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\Windows Search",
                "AllowCortana",
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\Windows Search",
                "AllowSearchToUseLocation",
            ),
        ],
        "disable_advertising_id" => vec![(
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\AdvertisingInfo",
            "Enabled",
        )],
        "disable_web_search" => vec![
            (
                r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Search",
                "BingSearchEnabled",
            ),
            (
                r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Search",
                "AllowSearchToUseLocation",
            ),
        ],
        "disable_telemetry_autologger" => vec![(
            r"HKLM\SYSTEM\CurrentControlSet\Control\WMI\Autologger\Diagtrack-Listener",
            "Start",
        )],
        _ => vec![],
    };

    // Process registry reversions if there are any
    if !ops.is_empty() {
        for &(full_key, value_name) in &ops {
            delete_registry_value(full_key, value_name)?;
        }
    }

    // Handle non-registry reversions
    match tweak_id {
        "stop_diagtrack_service" => {
            crate::startup::services::set_start_type("DiagTrack", "auto")?;
            let current = crate::startup::services::inspect("DiagTrack")?;
            if current.status.eq_ignore_ascii_case("Stopped") {
                crate::startup::services::control_service("DiagTrack", "start")?;
            }
        }
        "disable_telemetry_tasks" => {
            let tasks = [
                (
                    r"\Microsoft\Windows\Application Experience",
                    "Microsoft Compatibility Appraiser",
                ),
                (
                    r"\Microsoft\Windows\Customer Experience Improvement Program",
                    "Consolidator",
                ),
            ];
            for (folder, task_name) in &tasks {
                let full_name = format!(r"{}\{}", folder, task_name);
                if let Err(error) = crate::startup::scheduler::set_task_state(&full_name, true) {
                    log::warn!("Failed to enable scheduled task {}: {}", full_name, error);
                }
            }
        }
        _ => {}
    }

    Ok(())
}

fn set_registry_dword(key: &str, value: &str, data: u32) -> Result<(), String> {
    ops::set_dword(key, value, data)
}

fn delete_registry_value(key: &str, value: &str) -> Result<(), String> {
    ops::delete_value(key, value, MissingKey::Error)
}

fn split_hive(full_key: &str) -> (&str, String) {
    ops::split_hive(full_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// "extreme" and "advanced" are legacy labels that the operation kernel
    /// maps to its most severe level through the catch-all in
    /// `system_tweak_operation::risk`. Pinning the vocabulary makes a typo fail
    /// here instead of silently becoming Critical.
    const RISK_LABELS: [&str; 5] = ["low", "medium", "high", "extreme", "advanced"];

    #[test]
    fn listed_tweaks_have_unique_described_ids() {
        let tweaks = list();
        assert!(!tweaks.is_empty(), "the tweak list must not be empty");
        let mut seen = HashSet::new();
        for tweak in &tweaks {
            assert!(
                !tweak.id.is_empty()
                    && tweak
                        .id
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "malformed tweak id {:?}",
                tweak.id
            );
            assert!(!tweak.name.trim().is_empty(), "{} has no name", tweak.id);
            assert!(
                !tweak.description.trim().is_empty(),
                "{} has no description",
                tweak.id
            );
            assert!(
                RISK_LABELS.contains(&tweak.risk.as_str()),
                "{} has unknown risk {:?}",
                tweak.id,
                tweak.risk
            );
            assert!(seen.insert(tweak.id.clone()), "duplicate id {}", tweak.id);
        }
    }
}
