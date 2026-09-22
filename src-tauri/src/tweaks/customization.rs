//! Low-risk Windows usability and Explorer customization tweaks.

use crate::core::system_tweak_operation::{TweakPhysicalResource, TweakPhysicalState};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomizationTweak {
    pub id: String,
    pub name: String,
    pub description: String,
    pub risk: String,
    pub is_applied: bool,
}

pub fn list() -> Vec<CustomizationTweak> {
    let mut tweaks = vec![
        CustomizationTweak {
            id: "show_known_file_extensions".into(),
            name: "Show Known File Extensions".into(),
            description: "Shows file extensions in Explorer so deceptive double extensions are easier to identify.".into(),
            risk: "low".into(),
            is_applied: is_applied("show_known_file_extensions"),
        },
        CustomizationTweak {
            id: "enable_long_paths".into(),
            name: "Enable Long Paths".into(),
            description: "Enables the Windows long-path policy for applications that also declare long-path support.".into(),
            risk: "low".into(),
            is_applied: is_applied("enable_long_paths"),
        },
        CustomizationTweak {
            id: "disable_promotional_content".into(),
            name: "Disable Windows Suggestions and Promoted Content".into(),
            description: "Reduces optional app suggestions, subscribed-content prompts, and silent promotional installs without removing Windows components.".into(),
            risk: "low".into(),
            is_applied: is_applied("disable_promotional_content"),
        },
        CustomizationTweak {
            id: "disable_lock_screen_spotlight".into(),
            name: "Disable Lock-screen Spotlight Tips".into(),
            description: "Turns off the Windows Spotlight lock-screen provider, including its tips and promotional cards; your chosen static or slideshow lock screen remains available.".into(),
            risk: "low".into(),
            is_applied: is_applied("disable_lock_screen_spotlight"),
        },
    ];
    if supports_windows_11() {
        tweaks.push(CustomizationTweak {
            id: "enable_end_task".into(),
            name: "Enable End Task on the Taskbar".into(),
            description: "Adds an End task action to supported Windows 11 taskbar context menus."
                .into(),
            risk: "low".into(),
            is_applied: is_applied("enable_end_task"),
        });
        tweaks.push(CustomizationTweak {
            id: "hide_widgets".into(),
            name: "Hide Taskbar Widgets".into(),
            description: "Hides the Widgets button from the Windows 11 taskbar without removing its packages.".into(),
            risk: "low".into(),
            is_applied: is_applied("hide_widgets"),
        });
    }
    if supports_windows_tips_policy() {
        tweaks.push(CustomizationTweak {
            id: "disable_windows_tips".into(),
            name: "Disable Windows Tips and Consumer Promotions".into(),
            description: "Uses the supported Windows Cloud Content policies to suppress Windows tips, soft landing, and consumer-feature promotions on editions that expose those policies.".into(),
            risk: "low".into(),
            is_applied: is_applied("disable_windows_tips"),
        });
    }
    tweaks
}

fn entries(tweak_id: &str) -> Result<Vec<(&'static str, &'static str, u32)>, String> {
    match tweak_id {
        "show_known_file_extensions" => Ok(vec![(
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "HideFileExt",
            0,
        )]),
        "enable_end_task" => Ok(vec![(
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Advanced\TaskbarDeveloperSettings",
            "TaskbarEndTask",
            1,
        )]),
        "enable_long_paths" => Ok(vec![(
            r"HKLM\SYSTEM\CurrentControlSet\Control\FileSystem",
            "LongPathsEnabled",
            1,
        )]),
        "disable_promotional_content" => Ok(vec![
            (
                r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\ContentDeliveryManager",
                "SystemPaneSuggestionsEnabled",
                0,
            ),
            (
                r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\ContentDeliveryManager",
                "SubscribedContentEnabled",
                0,
            ),
            (
                r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\ContentDeliveryManager",
                "SilentInstalledAppsEnabled",
                0,
            ),
        ]),
        "disable_lock_screen_spotlight" => Ok(vec![(
            r"HKCU\SOFTWARE\Policies\Microsoft\Windows\CloudContent",
            "ConfigureWindowsSpotlight",
            0,
        )]),
        "hide_widgets" => Ok(vec![(
            r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\Advanced",
            "TaskbarDa",
            0,
        )]),
        "disable_windows_tips" => Ok(vec![
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\CloudContent",
                "DisableSoftLanding",
                1,
            ),
            (
                r"HKLM\SOFTWARE\Policies\Microsoft\Windows\CloudContent",
                "DisableWindowsConsumerFeatures",
                1,
            ),
        ]),
        _ => Err(format!("Unknown customization tweak: {tweak_id}")),
    }
}

fn supports_windows_11() -> bool {
    crate::system_info::os::gather()
        .build
        .parse::<u32>()
        .is_ok_and(|build| build >= 22_000)
}

fn supports_windows_tips_policy() -> bool {
    let edition = crate::system_info::os::gather()
        .edition
        .to_ascii_lowercase();
    edition.contains("enterprise") || edition.contains("education") || edition.contains("iot")
}

fn is_applied(tweak_id: &str) -> bool {
    entries(tweak_id)
        .map(|values| {
            values.iter().all(|(key, name, value)| {
                read_dword(key, name).is_some_and(|current| current == *value)
            })
        })
        .unwrap_or(false)
}

pub fn capture_state(tweak_id: &str) -> Result<TweakPhysicalState, String> {
    let resources = entries(tweak_id)?
        .into_iter()
        .map(|(full_key, value_name, _)| capture_registry_resource(full_key, value_name))
        .collect();
    Ok(TweakPhysicalState { resources })
}

pub fn apply(tweak_id: &str) -> Result<(), String> {
    for (key, name, value) in entries(tweak_id)? {
        set_dword(key, name, value)?;
    }
    Ok(())
}

pub fn restore_state(tweak_id: &str, state: &TweakPhysicalState) -> Result<(), String> {
    let expected = entries(tweak_id)?;
    if state.resources.len() != expected.len()
        || state
            .resources
            .iter()
            .any(|resource| !matches!(resource, TweakPhysicalResource::RegistryDword { .. }))
    {
        return Err(format!(
            "Invalid captured state for customization tweak {tweak_id}"
        ));
    }
    for resource in &state.resources {
        restore_registry_resource(resource)?;
    }
    Ok(())
}

fn capture_registry_resource(full_key: &str, value_name: &str) -> TweakPhysicalResource {
    let (hive, path) = split_hive(full_key);
    let hive = match hive {
        "HKLM" => windows::Win32::System::Registry::HKEY_LOCAL_MACHINE,
        "HKCU" => windows::Win32::System::Registry::HKEY_CURRENT_USER,
        _ => unreachable!(),
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
            set_raw(full_key, value_name, *registry_type, raw_value)
        } else {
            set_dword(full_key, value_name, value.unwrap_or_default())
        }
    } else {
        if *key_existed {
            delete_value(full_key, value_name)?;
        } else {
            delete_key(full_key)?;
        }
        Ok(())
    }
}

fn read_dword(full_key: &str, value_name: &str) -> Option<u32> {
    let (hive, path) = split_hive(full_key);
    let hive = match hive {
        "HKLM" => windows::Win32::System::Registry::HKEY_LOCAL_MACHINE,
        "HKCU" => windows::Win32::System::Registry::HKEY_CURRENT_USER,
        _ => return None,
    };
    use crate::core::registry_helpers;
    use windows::Win32::System::Registry::RegCloseKey;
    unsafe {
        let key = registry_helpers::open_key_read(hive, &path)?;
        let value = registry_helpers::read_reg_dword(key, value_name);
        let _ = RegCloseKey(key);
        value
    }
}

fn set_dword(full_key: &str, value_name: &str, value: u32) -> Result<(), String> {
    let (hive, path) = split_hive(full_key);
    let hive = match hive {
        "HKLM" => windows::Win32::System::Registry::HKEY_LOCAL_MACHINE,
        "HKCU" => windows::Win32::System::Registry::HKEY_CURRENT_USER,
        _ => return Err(format!("Unknown registry hive in {full_key}")),
    };
    use windows::core::HSTRING;
    use windows::Win32::System::Registry::*;
    unsafe {
        let mut key = std::mem::zeroed();
        let result = RegCreateKeyExW(
            hive,
            &HSTRING::from(&path),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        );
        if result.is_err() {
            let _ = RegCloseKey(key);
            return Err(format!("Failed to open {full_key}: {result:?}"));
        }
        let bytes = value.to_ne_bytes();
        let result = RegSetValueExW(
            key,
            &HSTRING::from(value_name),
            None,
            REG_DWORD,
            Some(&bytes),
        );
        let _ = RegCloseKey(key);
        if result.is_err() {
            return Err(format!(
                "Failed to set {value_name} at {full_key}: {result:?}"
            ));
        }
        Ok(())
    }
}

fn set_raw(
    full_key: &str,
    value_name: &str,
    registry_type: u32,
    raw_value: &[u8],
) -> Result<(), String> {
    let (hive_str, path) = split_hive(full_key);
    let hive = match hive_str {
        "HKLM" => windows::Win32::System::Registry::HKEY_LOCAL_MACHINE,
        "HKCU" => windows::Win32::System::Registry::HKEY_CURRENT_USER,
        _ => return Err(format!("Unknown registry hive in {full_key}")),
    };
    unsafe {
        let key = crate::core::registry_helpers::create_key_write(hive, &path)
            .ok_or_else(|| format!("Failed to open {full_key}"))?;
        let result =
            crate::core::registry_helpers::set_reg_raw(key, value_name, registry_type, raw_value);
        crate::core::registry_helpers::close_key(key);
        result.map_err(|error| format!("Failed to restore {value_name} at {full_key}: {error:?}"))
    }
}

fn delete_value(full_key: &str, value_name: &str) -> Result<(), String> {
    let (hive, path) = split_hive(full_key);
    let hive = match hive {
        "HKLM" => windows::Win32::System::Registry::HKEY_LOCAL_MACHINE,
        "HKCU" => windows::Win32::System::Registry::HKEY_CURRENT_USER,
        _ => return Err(format!("Unknown registry hive in {full_key}")),
    };
    use windows::core::HSTRING;
    use windows::Win32::System::Registry::*;
    unsafe {
        let mut key = std::mem::zeroed();
        if RegOpenKeyExW(
            hive,
            &HSTRING::from(&path),
            Some(0),
            KEY_SET_VALUE,
            &mut key,
        )
        .is_err()
        {
            return Ok(());
        }
        let result = RegDeleteValueW(key, &HSTRING::from(value_name));
        let _ = RegCloseKey(key);
        if result.is_err() {
            return Err(format!(
                "Failed to delete {value_name} at {full_key}: {result:?}"
            ));
        }
        Ok(())
    }
}

fn delete_key(full_key: &str) -> Result<(), String> {
    let (hive, path) = split_hive(full_key);
    let hive = match hive {
        "HKLM" => windows::Win32::System::Registry::HKEY_LOCAL_MACHINE,
        "HKCU" => windows::Win32::System::Registry::HKEY_CURRENT_USER,
        _ => return Err(format!("Unknown registry hive in {full_key}")),
    };
    use crate::core::registry_helpers;
    use windows::Win32::System::Registry::RegCloseKey;
    unsafe {
        if let Some(key) = registry_helpers::open_key_read(hive, &path) {
            let _ = RegCloseKey(key);
            registry_helpers::delete_key_recursive(hive, &path)
                .map_err(|error| format!("Failed to delete empty key {full_key}: {error}"))?;
        }
    }
    Ok(())
}

fn split_hive(full_key: &str) -> (&str, String) {
    full_key
        .split_once('\\')
        .map_or((full_key, String::new()), |(hive, path)| {
            (hive, path.into())
        })
}
