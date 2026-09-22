//! Performance tweaks - visual effects, processor scheduling,
//! power plan optimization, system responsiveness settings,
//! kernel clock source (HPET/TSC), timer coalescing, and
//! IFEO process priority overrides.
//!
//! The governed operation journal captures the target state; exact automated
//! restoration remains disabled for this domain until it is proven.
//!
//! ## Safety tiers (from researches.txt Section 5):
//! - **Safe:** visual effects, power plan, processor scheduling
//! - **Advanced:** HPET/TSC clock source, dynamic tick, timer coalescing
//!   (require restart; may increase idle power on mobile devices)

use crate::core::command_helpers::create_command;
use crate::core::system_tweak_operation::{TweakPhysicalResource, TweakPhysicalState};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfTweak {
    pub id: String,
    pub name: String,
    pub description: String,
    pub risk: String,
    pub is_applied: bool,
}

pub fn list() -> Vec<PerfTweak> {
    let mut tweaks = vec![
        PerfTweak {
            id: "adjust_visual_effects".into(),
            name: "Adjust for Best Performance".into(),
            description: "Disables animations, transparency, shadows for faster UI".into(),
            risk: "low".into(),
            is_applied: is_applied("adjust_visual_effects"),
        },
        PerfTweak {
            id: "processor_scheduling".into(),
            name: "Optimize for Programs".into(),
            description: "Sets processor scheduling to favor foreground programs".into(),
            risk: "low".into(),
            is_applied: is_applied("processor_scheduling"),
        },
        PerfTweak {
            id: "high_performance_power".into(),
            name: "High Performance Power Plan".into(),
            description: "Activates the high-performance power plan".into(),
            risk: "low".into(),
            is_applied: is_applied("high_performance_power"),
        },
        PerfTweak {
            id: "disable_startup_delay".into(),
            name: "Disable Startup Delay".into(),
            description: "Disables the startup application delay for faster boot".into(),
            risk: "medium".into(),
            is_applied: is_applied("disable_startup_delay"),
        },
        PerfTweak {
            id: "disable_indexing".into(),
            name: "Disable Windows Search Indexing".into(),
            description: "Disables content indexing for faster disk access".into(),
            risk: "medium".into(),
            is_applied: is_applied("disable_indexing"),
        },
        PerfTweak {
            id: "disable_hibernation".into(),
            name: "Disable Hibernation".into(),
            description: "Turns off hibernation and removes its disk image; use this on desktops or when you explicitly accept losing hibernation and Fast Startup.".into(),
            risk: "medium".into(),
            is_applied: is_applied("disable_hibernation"),
        },
        PerfTweak {
            id: "disable_fast_startup".into(),
            name: "Disable Fast Startup".into(),
            description: "Disables Windows Fast Startup without changing the hibernation feature itself; this can increase cold-boot time but avoids hybrid-shutdown surprises.".into(),
            risk: "medium".into(),
            is_applied: is_applied("disable_fast_startup"),
        },
        PerfTweak {
            id: "enable_game_mode".into(),
            name: "Enable Game Mode".into(),
            description: "Enables Windows Game Mode for a gaming-focused scheduling preference; benefits vary by workload.".into(),
            risk: "low".into(),
            is_applied: is_applied("enable_game_mode"),
        },
        PerfTweak {
            id: "disable_game_dvr".into(),
            name: "Disable Game DVR Background Recording".into(),
            description: "Disables Windows background game capture. Existing Game Bar recording workflows may stop working.".into(),
            risk: "medium".into(),
            is_applied: is_applied("disable_game_dvr"),
        },
        PerfTweak {
            id: "disable_mouse_acceleration".into(),
            name: "Disable Mouse Acceleration".into(),
            description: "Uses a direct pointer response instead of Enhance Pointer Precision; this changes mouse feel and is intended as a gaming preference.".into(),
            risk: "low".into(),
            is_applied: is_applied("disable_mouse_acceleration"),
        },
        PerfTweak {
            id: "ultimate_performance_power".into(),
            name: "Ultimate Performance Power Plan".into(),
            description: "Activates the Ultimate Performance power plan (bypasses all power-saving throttling)".into(),
            risk: "advanced".into(),
            is_applied: is_applied("ultimate_performance_power"),
        },
    ];
    if crate::system_info::battery::gather().is_laptop {
        tweaks.retain(|tweak| tweak.id != "disable_hibernation");
    }
    tweaks
}

fn is_applied(tweak_id: &str) -> bool {
    match tweak_id {
        "adjust_visual_effects" => has_dwords(&[(
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\VisualEffects",
            "VisualFXSetting",
            2,
        )]),
        "processor_scheduling" => has_dwords(&[(
            r"HKLM\SYSTEM\CurrentControlSet\Control\PriorityControl",
            "Win32PrioritySeparation",
            38,
        )]),
        "high_performance_power" => active_power_plan_is("8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c"),
        "disable_startup_delay" => has_dwords(&[(
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Serialize",
            "StartupDelayInMSec",
            0,
        )]),
        "disable_indexing" => service_is_disabled("WSearch"),
        "disable_hibernation" => has_dwords(&[
            (
                r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Power",
                "HibernateEnabled",
                0,
            ),
            (
                r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\FlyoutMenuSettings",
                "ShowHibernateOption",
                0,
            ),
        ]),
        "disable_fast_startup" => has_dwords(&[(
            r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Power",
            "HiberbootEnabled",
            0,
        )]),
        "enable_game_mode" => has_dwords(&[
            (r"HKCU\SOFTWARE\Microsoft\GameBar", "AllowAutoGameMode", 1),
            (r"HKCU\SOFTWARE\Microsoft\GameBar", "AutoGameModeEnabled", 1),
        ]),
        "disable_game_dvr" => has_dwords(&[
            (
                r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\GameDVR",
                "AppCaptureEnabled",
                0,
            ),
            (r"HKCU\System\GameConfigStore", "GameDVR_Enabled", 0),
        ]),
        "disable_mouse_acceleration" => has_dwords(&[
            (r"HKCU\Control Panel\Mouse", "MouseSpeed", 0),
            (r"HKCU\Control Panel\Mouse", "MouseThreshold1", 0),
            (r"HKCU\Control Panel\Mouse", "MouseThreshold2", 0),
        ]),
        "ultimate_performance_power" => {
            active_power_plan_is("e9a42b02-d5df-448d-aa00-03f14749eb61")
        }
        "disable_hpet" => has_dwords(&[(
            r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Kernel",
            "UsePlatformClock",
            0,
        )]),
        "enable_tsc_sync" => has_dwords(&[(
            r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Kernel",
            "TscSyncPolicy",
            1,
        )]),
        "disable_dynamic_tick" => has_dwords(&[(
            r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Kernel",
            "EnableDynamicTick",
            0,
        )]),
        "force_tsc_calibration" => has_dwords(&[(
            r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Kernel",
            "ForceTscCalibration",
            1,
        )]),
        "disable_timer_coalescing" => has_dwords(&[
            (
                r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Power",
                "PerfEnablePackageIdle",
                0,
            ),
            (
                r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Power",
                "CoalescingTimerInterval",
                1,
            ),
        ]),
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

fn active_power_plan_is(guid: &str) -> bool {
    query_active_power_plan().is_some_and(|current| current.eq_ignore_ascii_case(guid))
}

pub fn capture_state(tweak_id: &str) -> Result<TweakPhysicalState, String> {
    let resources = match tweak_id {
        "adjust_visual_effects" => registry_resources(&[(
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\VisualEffects",
            "VisualFXSetting",
        )]),
        "processor_scheduling" => registry_resources(&[(
            r"HKLM\SYSTEM\CurrentControlSet\Control\PriorityControl",
            "Win32PrioritySeparation",
        )]),
        "high_performance_power" | "ultimate_performance_power" => {
            vec![TweakPhysicalResource::ActivePowerPlan {
                guid: query_active_power_plan(),
            }]
        }
        "disable_startup_delay" => registry_resources(&[(
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Serialize",
            "StartupDelayInMSec",
        )]),
        "disable_indexing" => vec![capture_service_resource("WSearch")?],
        "disable_hibernation" => registry_resources(&[
            (
                r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Power",
                "HibernateEnabled",
            ),
            (
                r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\FlyoutMenuSettings",
                "ShowHibernateOption",
            ),
            (
                r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Power",
                "HiberbootEnabled",
            ),
        ]),
        "disable_fast_startup" => registry_resources(&[(
            r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Power",
            "HiberbootEnabled",
        )]),
        "enable_game_mode" => registry_resources(&[
            (r"HKCU\SOFTWARE\Microsoft\GameBar", "AllowAutoGameMode"),
            (r"HKCU\SOFTWARE\Microsoft\GameBar", "AutoGameModeEnabled"),
        ]),
        "disable_game_dvr" => registry_resources(&[
            (
                r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\GameDVR",
                "AppCaptureEnabled",
            ),
            (r"HKCU\System\GameConfigStore", "GameDVR_Enabled"),
        ]),
        "disable_mouse_acceleration" => registry_resources(&[
            (r"HKCU\Control Panel\Mouse", "MouseSpeed"),
            (r"HKCU\Control Panel\Mouse", "MouseThreshold1"),
            (r"HKCU\Control Panel\Mouse", "MouseThreshold2"),
        ]),
        "disable_hpet" => registry_resources(&[(
            r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Kernel",
            "UsePlatformClock",
        )]),
        "enable_tsc_sync" => registry_resources(&[(
            r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Kernel",
            "TscSyncPolicy",
        )]),
        "disable_dynamic_tick" => registry_resources(&[(
            r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Kernel",
            "EnableDynamicTick",
        )]),
        "force_tsc_calibration" => registry_resources(&[(
            r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Kernel",
            "ForceTscCalibration",
        )]),
        "disable_timer_coalescing" => registry_resources(&[
            (
                r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Power",
                "PerfEnablePackageIdle",
            ),
            (
                r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Power",
                "CoalescingTimerInterval",
            ),
        ]),
        _ => return Err(format!("Unknown tweak: {tweak_id}")),
    };
    Ok(TweakPhysicalState { resources })
}

pub fn restore_state(tweak_id: &str, state: &TweakPhysicalState) -> Result<(), String> {
    match tweak_id {
        "high_performance_power" | "ultimate_performance_power" => {
            let guid = state
                .resources
                .iter()
                .find_map(|resource| match resource {
                    TweakPhysicalResource::ActivePowerPlan { guid } => guid.as_deref(),
                    _ => None,
                })
                .ok_or_else(|| "Missing active power-plan capture".to_string())?;
            activate_power_plan(guid)
        }
        "disable_indexing" => restore_service_resource(state, "WSearch"),
        "disable_hibernation" => {
            let was_enabled = state.resources.iter().any(|resource| {
                matches!(
                    resource,
                    TweakPhysicalResource::RegistryDword {
                        full_key,
                        value_name,
                        value: Some(value),
                        ..
                    } if full_key.ends_with(r"Session Manager\Power")
                        && value_name == "HibernateEnabled"
                        && *value != 0
                )
            });
            set_hibernation(was_enabled)?;
            restore_registry_resources(state, tweak_id)
        }
        _ => {
            for resource in &state.resources {
                match resource {
                    TweakPhysicalResource::RegistryDword { .. } => {
                        restore_registry_resource(resource)?
                    }
                    _ => {
                        return Err(format!(
                            "Unexpected resource in performance tweak {tweak_id}"
                        ))
                    }
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

fn capture_registry_resource(full_key: &str, value_name: &str) -> TweakPhysicalResource {
    let (hive_str, path) = split_hive(full_key);
    let hive = match hive_str {
        "HKLM" => windows::Win32::System::Registry::HKEY_LOCAL_MACHINE,
        "HKCU" => windows::Win32::System::Registry::HKEY_CURRENT_USER,
        _ => {
            return TweakPhysicalResource::RegistryDword {
                full_key: full_key.into(),
                value_name: value_name.into(),
                key_existed: false,
                value_existed: false,
                value: None,
                registry_type: None,
                raw_value: None,
            }
        }
    };
    use crate::core::registry_helpers;
    use windows::Win32::System::Registry::RegCloseKey;
    unsafe {
        if let Some(key) = registry_helpers::open_key_read(hive, &path) {
            let value = registry_helpers::read_reg_dword(key, value_name);
            let raw = registry_helpers::read_reg_raw(key, value_name);
            let _ = RegCloseKey(key);
            return TweakPhysicalResource::RegistryDword {
                full_key: full_key.into(),
                value_name: value_name.into(),
                key_existed: true,
                value_existed: raw.is_some(),
                value,
                registry_type: raw.as_ref().map(|(registry_type, _)| *registry_type),
                raw_value: raw.map(|(_, raw_value)| raw_value),
            };
        }
    }
    TweakPhysicalResource::RegistryDword {
        full_key: full_key.into(),
        value_name: value_name.into(),
        key_existed: false,
        value_existed: false,
        value: None,
        registry_type: None,
        raw_value: None,
    }
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
    let current = crate::startup::services::inspect(service)?;
    let should_run = running.unwrap_or(false);
    if should_run && !current.status.eq_ignore_ascii_case("running") {
        crate::startup::services::control_service(service, "start")?;
    } else if !should_run && current.status.eq_ignore_ascii_case("running") {
        crate::startup::services::control_service(service, "stop")?;
    }
    Ok(())
}

fn restore_registry_resource(resource: &TweakPhysicalResource) -> Result<(), String> {
    let TweakPhysicalResource::RegistryDword {
        full_key,
        value_name,
        key_existed,
        value_existed,
        value,
        registry_type,
        raw_value,
    } = resource
    else {
        return Err("Expected registry resource".into());
    };
    if *value_existed {
        if let (Some(registry_type), Some(raw_value)) = (registry_type, raw_value) {
            set_registry_raw(full_key, value_name, *registry_type, raw_value)
        } else {
            set_registry_dword(full_key, value_name, value.unwrap_or_default())
        }
    } else {
        if *key_existed {
            delete_registry_value(full_key, value_name)?;
        }
        if !*key_existed {
            delete_registry_key(full_key)?;
        }
        Ok(())
    }
}

fn restore_registry_resources(state: &TweakPhysicalState, tweak_id: &str) -> Result<(), String> {
    for resource in &state.resources {
        match resource {
            TweakPhysicalResource::RegistryDword { .. } => restore_registry_resource(resource)?,
            _ => {
                return Err(format!(
                    "Unexpected resource in performance tweak {tweak_id}"
                ))
            }
        }
    }
    Ok(())
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

/// Apply registry values after the operation layer has captured the current
/// target state.
fn apply_registry_ops(ops: &[(&str, &str, u32)]) -> Result<(), String> {
    for &(full_key, value_name, new_val) in ops {
        set_registry_dword(full_key, value_name, new_val)?;
    }

    Ok(())
}

pub fn apply(tweak_id: &str) -> Result<(), String> {
    match tweak_id {
        "adjust_visual_effects" => {
            apply_registry_ops(&[(
                r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\VisualEffects",
                "VisualFXSetting",
                2,
            )])?;
        }
        "processor_scheduling" => {
            apply_registry_ops(&[(
                r"HKLM\SYSTEM\CurrentControlSet\Control\PriorityControl",
                "Win32PrioritySeparation",
                38,
            )])?;
        }
        "high_performance_power" => {
            ensure_ac_power_for_high_performance()?;
            activate_power_plan("8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c")?;
        }
        "disable_startup_delay" => {
            apply_registry_ops(&[(
                r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Serialize",
                "StartupDelayInMSec",
                0,
            )])?;
        }
        "disable_indexing" => {
            let current = crate::startup::services::inspect("WSearch")?;
            if !current.status.eq_ignore_ascii_case("Stopped") {
                crate::startup::services::control_service("WSearch", "stop")?;
            }
            crate::startup::services::set_start_type("WSearch", "disabled")?;
        }
        "disable_hibernation" => {
            set_hibernation(false)?;
            apply_registry_ops(&[
                (
                    r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Power",
                    "HibernateEnabled",
                    0,
                ),
                (
                    r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\FlyoutMenuSettings",
                    "ShowHibernateOption",
                    0,
                ),
                (
                    r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Power",
                    "HiberbootEnabled",
                    0,
                ),
            ])?;
        }
        "disable_fast_startup" => apply_registry_ops(&[(
            r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Power",
            "HiberbootEnabled",
            0,
        )])?,
        "enable_game_mode" => apply_registry_ops(&[
            (r"HKCU\SOFTWARE\Microsoft\GameBar", "AllowAutoGameMode", 1),
            (r"HKCU\SOFTWARE\Microsoft\GameBar", "AutoGameModeEnabled", 1),
        ])?,
        "disable_game_dvr" => apply_registry_ops(&[
            (
                r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\GameDVR",
                "AppCaptureEnabled",
                0,
            ),
            (r"HKCU\System\GameConfigStore", "GameDVR_Enabled", 0),
        ])?,
        "disable_mouse_acceleration" => apply_registry_ops(&[
            (r"HKCU\Control Panel\Mouse", "MouseSpeed", 0),
            (r"HKCU\Control Panel\Mouse", "MouseThreshold1", 0),
            (r"HKCU\Control Panel\Mouse", "MouseThreshold2", 0),
        ])?,
        // -- Kernel Clock (HPET/TSC) - Section 5 --------------
        "ultimate_performance_power" => {
            activate_power_plan("e9a42b02-d5df-448d-aa00-03f14749eb61")?;
        }
        "disable_hpet" => {
            apply_registry_ops(&[(
                r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Kernel",
                "UsePlatformClock",
                0,
            )])?;
        }
        "enable_tsc_sync" => {
            apply_registry_ops(&[(
                r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Kernel",
                "TscSyncPolicy",
                1,
            )])?;
        }
        "disable_dynamic_tick" => {
            apply_registry_ops(&[(
                r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Kernel",
                "EnableDynamicTick",
                0,
            )])?;
        }
        "force_tsc_calibration" => {
            apply_registry_ops(&[(
                r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Kernel",
                "ForceTscCalibration",
                1,
            )])?;
        }
        "disable_timer_coalescing" => {
            apply_registry_ops(&[
                (
                    r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Power",
                    "PerfEnablePackageIdle",
                    0,
                ),
                (
                    r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Power",
                    "CoalescingTimerInterval",
                    1,
                ),
            ])?;
        }
        _ => return Err(format!("Unknown tweak: {}", tweak_id)),
    }
    Ok(())
}

fn ensure_ac_power_for_high_performance() -> Result<(), String> {
    let power = crate::system_info::battery::gather();
    if power.is_laptop && !power.charger_connected {
        return Err(
            "High Performance is available only while a laptop is connected to AC power.".into(),
        );
    }
    Ok(())
}

pub fn revert(tweak_id: &str) -> Result<(), String> {
    match tweak_id {
        "adjust_visual_effects" => {
            apply_registry_ops(&[(
                r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\VisualEffects",
                "VisualFXSetting",
                1,
            )])?;
        }
        "high_performance_power" => {
            return Err("Use captured-state restoration for power-plan changes".into());
        }
        "disable_startup_delay" => {
            delete_registry_value(
                r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Serialize",
                "StartupDelayInMSec",
            )?;
        }
        "disable_indexing" => {
            crate::startup::services::set_start_type("WSearch", "auto")?;
            let current = crate::startup::services::inspect("WSearch")?;
            if current.status.eq_ignore_ascii_case("Stopped") {
                crate::startup::services::control_service("WSearch", "start")?;
            }
        }
        // -- Kernel Clock reverts - delete values to restore defaults -
        "ultimate_performance_power" => {
            return Err("Use captured-state restoration for power-plan changes".into());
        }
        "disable_hpet" => {
            revert_registry_value(
                r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Kernel",
                "UsePlatformClock",
            )?;
        }
        "enable_tsc_sync" => {
            revert_registry_value(
                r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Kernel",
                "TscSyncPolicy",
            )?;
        }
        "disable_dynamic_tick" => {
            revert_registry_value(
                r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Kernel",
                "EnableDynamicTick",
            )?;
        }
        "force_tsc_calibration" => {
            revert_registry_value(
                r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Kernel",
                "ForceTscCalibration",
            )?;
        }
        "disable_timer_coalescing" => {
            revert_registry_value(
                r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Power",
                "PerfEnablePackageIdle",
            )?;
            revert_registry_value(
                r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Power",
                "CoalescingTimerInterval",
            )?;
        }
        _ => {}
    }
    Ok(())
}

fn set_registry_dword(key: &str, value: &str, data: u32) -> Result<(), String> {
    let (hive_str, path) = split_hive(key);
    let hive = match hive_str {
        "HKLM" => windows::Win32::System::Registry::HKEY_LOCAL_MACHINE,
        "HKCU" => windows::Win32::System::Registry::HKEY_CURRENT_USER,
        _ => return Err(format!("Unknown hive: {}", hive_str)),
    };

    use windows::core::*;
    use windows::Win32::System::Registry::*;

    unsafe {
        let mut hkey = std::mem::zeroed();
        let result = RegCreateKeyExW(
            hive,
            &HSTRING::from(&path),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut hkey,
            None,
        );
        if result.is_err() {
            let _ = RegCloseKey(hkey);
            return Err(format!("Failed to open key {}: {:?}", key, result));
        }

        let data_bytes = data.to_ne_bytes();
        let result = RegSetValueExW(
            hkey,
            &HSTRING::from(value),
            None,
            REG_DWORD,
            Some(&data_bytes),
        );
        let _ = RegCloseKey(hkey);

        if result.is_err() {
            return Err(format!(
                "Failed to set {} = {} at {}: {:?}",
                value, data, key, result
            ));
        }
    }

    Ok(())
}

fn set_registry_raw(
    key: &str,
    value: &str,
    registry_type: u32,
    raw_value: &[u8],
) -> Result<(), String> {
    let (hive_str, path) = split_hive(key);
    let hive = match hive_str {
        "HKLM" => windows::Win32::System::Registry::HKEY_LOCAL_MACHINE,
        "HKCU" => windows::Win32::System::Registry::HKEY_CURRENT_USER,
        _ => return Err(format!("Unknown hive: {hive_str}")),
    };
    unsafe {
        let hkey = crate::core::registry_helpers::create_key_write(hive, &path)
            .ok_or_else(|| format!("Failed to create/open key {path}"))?;
        let result =
            crate::core::registry_helpers::set_reg_raw(hkey, value, registry_type, raw_value);
        crate::core::registry_helpers::close_key(hkey);
        result.map_err(|error| format!("Failed to restore {value} at {key}: {error:?}"))
    }
}

fn delete_registry_value(key: &str, value: &str) -> Result<(), String> {
    let (hive_str, path) = split_hive(key);
    let hive = match hive_str {
        "HKLM" => windows::Win32::System::Registry::HKEY_LOCAL_MACHINE,
        "HKCU" => windows::Win32::System::Registry::HKEY_CURRENT_USER,
        _ => return Err(format!("Unknown hive: {}", hive_str)),
    };

    use windows::core::*;
    use windows::Win32::System::Registry::*;

    unsafe {
        let mut hkey = std::mem::zeroed();
        let result = RegOpenKeyExW(
            hive,
            &HSTRING::from(&path),
            Some(0),
            KEY_SET_VALUE,
            &mut hkey,
        );
        if result.is_err() {
            // Key doesn't exist - not an error for deletion
            return Ok(());
        }
        let result = RegDeleteValueW(hkey, &HSTRING::from(value));
        let _ = RegCloseKey(hkey);
        if result.is_err() {
            return Err(format!(
                "Failed to delete {} at {}: {:?}",
                value, key, result
            ));
        }
    }

    Ok(())
}

fn delete_registry_key(full_key: &str) -> Result<(), String> {
    let (hive_str, path) = split_hive(full_key);
    let hive = match hive_str {
        "HKLM" => windows::Win32::System::Registry::HKEY_LOCAL_MACHINE,
        "HKCU" => windows::Win32::System::Registry::HKEY_CURRENT_USER,
        _ => return Err(format!("Unknown hive: {}", hive_str)),
    };
    use crate::core::registry_helpers;
    use windows::Win32::System::Registry::RegCloseKey;
    unsafe {
        if let Some(key) = registry_helpers::open_key_read(hive, &path) {
            let _ = RegCloseKey(key);
            registry_helpers::delete_key_recursive(hive, &path).map_err(|error| {
                format!("Failed to delete empty registry key {full_key}: {error}")
            })?;
        }
    }
    Ok(())
}

/// Delete a registry value to restore Windows default behavior.
fn revert_registry_value(full_key: &str, value_name: &str) -> Result<(), String> {
    delete_registry_value(full_key, value_name)
}

fn split_hive(full_key: &str) -> (&str, String) {
    if let Some(pos) = full_key.find('\\') {
        (&full_key[..pos], full_key[pos + 1..].to_string())
    } else {
        (full_key, String::new())
    }
}

/// Query the currently active power plan GUID from powercfg.
fn activate_power_plan(guid: &str) -> Result<(), String> {
    let output = create_command("powercfg")
        .args(["/setactive", guid])
        .output()
        .map_err(|e| format!("powercfg error: {e}"))?;
    if !output.status.success() {
        return Err(format!("Failed to activate power plan {guid}"));
    }
    Ok(())
}

fn set_hibernation(enabled: bool) -> Result<(), String> {
    let state = if enabled { "on" } else { "off" };
    let output = create_command("powercfg")
        .args(["/hibernate", state])
        .output()
        .map_err(|error| format!("powercfg hibernation command failed: {error}"))?;
    if !output.status.success() {
        return Err(format!("Failed to turn hibernation {state}"));
    }
    Ok(())
}

fn query_active_power_plan() -> Option<String> {
    let output = create_command("powercfg")
        .args(["/getactivescheme"])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Output format: "Power Scheme GUID: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx  (Scheme Name)"
    for part in stdout.split_whitespace() {
        if part.len() == 36 && part.chars().filter(|&c| c == '-').count() == 4 {
            return Some(part.to_string());
        }
    }
    None
}
