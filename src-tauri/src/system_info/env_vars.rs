//! Environment variables editor - read/edit Windows environment
//! variables from the registry (user + system scope).
//!
//! All mutations record undo entries so changes are reversible.

use serde::{Deserialize, Serialize};

use crate::core::registry_helpers;

pub const USER_ENVIRONMENT_KEY: &str = r"HKCU\Environment";
pub const SYSTEM_ENVIRONMENT_KEY: &str =
    r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
pub const ENVIRONMENT_SETTING_CHANGE: &str = "Environment";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
    pub scope: String,    // "user" or "system"
    pub var_type: String, // "REG_SZ" or "REG_EXPAND_SZ"
}

/// Read all environment variables from both user and system scope.
pub fn list() -> Result<Vec<EnvVar>, String> {
    let mut vars = Vec::new();

    use windows::Win32::System::Registry::*;

    // User variables: HKCU\Environment
    unsafe {
        match crate::core::registry_helpers::parse_hive_prefix(USER_ENVIRONMENT_KEY) {
            Ok((hive, path)) => {
                if let Some(key) = registry_helpers::open_key_read(hive, &path) {
                    registry_helpers::enum_values(key, |name, value_type| {
                        let Some(type_str) = registry_type_name(value_type) else {
                            return;
                        };
                        let Some(value) = registry_helpers::read_reg_string(key, &name) else {
                            return;
                        };
                        vars.push(EnvVar {
                            name,
                            value,
                            scope: "user".to_string(),
                            var_type: type_str.to_string(),
                        });
                    });
                    let _ = RegCloseKey(key);
                }
            }
            Err(e) => return Err(format!("Cannot read user env: {}", e)),
        }

        // System variables: HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment
        match crate::core::registry_helpers::parse_hive_prefix(SYSTEM_ENVIRONMENT_KEY) {
            Ok((hive, path)) => {
                if let Some(key) = registry_helpers::open_key_read(hive, &path) {
                    registry_helpers::enum_values(key, |name, value_type| {
                        let Some(type_str) = registry_type_name(value_type) else {
                            return;
                        };
                        let Some(value) = registry_helpers::read_reg_string(key, &name) else {
                            return;
                        };
                        vars.push(EnvVar {
                            name,
                            value,
                            scope: "system".to_string(),
                            var_type: type_str.to_string(),
                        });
                    });
                    let _ = RegCloseKey(key);
                }
            }
            Err(e) => return Err(format!("Cannot read system env: {}", e)),
        }
    }

    vars.sort_by(|a, b| a.scope.cmp(&b.scope).then(a.name.cmp(&b.name)));
    Ok(vars)
}

/// Read one variable from exactly one registry scope.
///
/// This is used for execution and undo read-back so a same-named value in the
/// other scope cannot satisfy verification.
pub fn read_scoped(name: &str, scope: &str) -> Result<Option<EnvVar>, String> {
    validate_scope(scope)?;
    validate_name(name)?;
    let hive_str = if scope == "system" {
        SYSTEM_ENVIRONMENT_KEY
    } else {
        USER_ENVIRONMENT_KEY
    };
    let (hive, path) = crate::core::registry_helpers::parse_hive_prefix(hive_str)
        .map_err(|e| format!("Bad hive: {}", e))?;

    unsafe {
        let Some(key) = registry_helpers::open_key_read(hive, &path) else {
            return Ok(None);
        };
        let result = read_registry_value(key, name, scope);
        let _ = windows::Win32::System::Registry::RegCloseKey(key);
        result
    }
}

unsafe fn read_registry_value(
    key: windows::Win32::System::Registry::HKEY,
    name: &str,
    scope: &str,
) -> Result<Option<EnvVar>, String> {
    use windows::core::HSTRING;
    use windows::Win32::System::Registry::{RegQueryValueExW, REG_VALUE_TYPE};

    let mut value_type = REG_VALUE_TYPE::default();
    let mut value_size = 0u32;
    match RegQueryValueExW(
        key,
        &HSTRING::from(name),
        None,
        Some(&mut value_type as *mut REG_VALUE_TYPE),
        None,
        Some(&mut value_size),
    )
    .ok()
    {
        Ok(()) => {}
        Err(error) if error.code() == windows::core::HRESULT::from_win32(2) => return Ok(None),
        Err(error) => return Err(format!("Failed to read environment variable: {error}")),
    }

    let var_type = registry_type_name(value_type.0)
        .ok_or_else(|| format!("Unsupported registry type for environment variable {name}"))?;
    let value = registry_helpers::read_reg_string(key, name).ok_or_else(|| {
        format!("Environment variable {scope}\\{name} is not a valid string value")
    })?;
    Ok(Some(EnvVar {
        name: name.to_string(),
        value,
        scope: scope.to_string(),
        var_type: var_type.to_string(),
    }))
}

fn registry_type_name(value_type: u32) -> Option<&'static str> {
    match value_type {
        1 => Some("REG_SZ"),
        2 => Some("REG_EXPAND_SZ"),
        _ => None,
    }
}

/// Set or add an environment variable.
///
/// The operation journal owns the captured pre-state and undo record.
pub fn set(var: &EnvVar) -> Result<(), String> {
    validate_env_var(var)?;
    if var.scope == "system" && !crate::core::elevation::is_elevated() {
        return Err(crate::core::error::WinOptimizerError::ElevationRequired.to_string());
    }

    let hive_str = if var.scope == "system" {
        SYSTEM_ENVIRONMENT_KEY
    } else {
        USER_ENVIRONMENT_KEY
    };

    let (hive, path) = crate::core::registry_helpers::parse_hive_prefix(hive_str)
        .map_err(|e| format!("Bad hive: {}", e))?;

    // Write the new value with the correct type
    unsafe {
        let key = registry_helpers::create_key_write(hive, &path)
            .ok_or_else(|| "Cannot open registry key for write".to_string())?;
        let reg_type = match var.var_type.as_str() {
            "REG_EXPAND_SZ" => windows::Win32::System::Registry::REG_EXPAND_SZ,
            _ => windows::Win32::System::Registry::REG_SZ,
        };
        registry_helpers::set_reg_string_with_type(key, &var.name, &var.value, reg_type)
            .map_err(|e| format!("Failed to set env var: {:?}", e))?;
        let _ = windows::Win32::System::Registry::RegCloseKey(key);
    }

    broadcast_environment_change()?;
    Ok(())
}

/// Restore an environment variable without creating a new undo entry.
/// Used only by the undo dispatcher after the original state was recorded.
pub fn restore(var: &EnvVar) -> Result<(), String> {
    validate_env_var(var)?;
    if var.scope == "system" && !crate::core::elevation::is_elevated() {
        return Err(crate::core::error::WinOptimizerError::ElevationRequired.to_string());
    }

    let hive_str = if var.scope == "system" {
        SYSTEM_ENVIRONMENT_KEY
    } else {
        USER_ENVIRONMENT_KEY
    };
    let (hive, path) = crate::core::registry_helpers::parse_hive_prefix(hive_str)
        .map_err(|e| format!("Bad hive: {}", e))?;

    unsafe {
        let key = registry_helpers::create_key_write(hive, &path)
            .ok_or_else(|| "Cannot open registry key for restore".to_string())?;
        let reg_type = match var.var_type.as_str() {
            "REG_EXPAND_SZ" => windows::Win32::System::Registry::REG_EXPAND_SZ,
            _ => windows::Win32::System::Registry::REG_SZ,
        };
        let result =
            registry_helpers::set_reg_string_with_type(key, &var.name, &var.value, reg_type)
                .map_err(|e| format!("Failed to restore env var: {:?}", e));
        let _ = windows::Win32::System::Registry::RegCloseKey(key);
        result
    }?;
    broadcast_environment_change()
}

/// Delete an environment variable without creating a new undo entry.
/// An already-absent value is considered successfully restored.
pub fn delete_without_undo(name: &str, scope: &str) -> Result<(), String> {
    validate_scope(scope)?;
    validate_name(name)?;
    if scope == "system" && !crate::core::elevation::is_elevated() {
        return Err(crate::core::error::WinOptimizerError::ElevationRequired.to_string());
    }

    let hive_str = if scope == "system" {
        SYSTEM_ENVIRONMENT_KEY
    } else {
        USER_ENVIRONMENT_KEY
    };
    let (hive, path) = crate::core::registry_helpers::parse_hive_prefix(hive_str)
        .map_err(|e| format!("Bad hive: {}", e))?;

    unsafe {
        let Some(key) = registry_helpers::open_key_write(hive, &path) else {
            return Ok(());
        };
        let result = registry_helpers::delete_value(key, name);
        let _ = windows::Win32::System::Registry::RegCloseKey(key);
        match result {
            Ok(()) => Ok(()),
            Err(error) if error.code() == windows::core::HRESULT::from_win32(2) => Ok(()),
            Err(error) => Err(format!("Failed to delete env var during undo: {:?}", error)),
        }?;
    }
    broadcast_environment_change()
}

/// Delete an environment variable.
///
/// The operation journal owns the captured pre-state and undo record.
pub fn delete(name: &str, scope: &str) -> Result<(), String> {
    validate_scope(scope)?;
    validate_name(name)?;
    if scope == "system" && !crate::core::elevation::is_elevated() {
        return Err(crate::core::error::WinOptimizerError::ElevationRequired.to_string());
    }

    let hive_str = if scope == "system" {
        SYSTEM_ENVIRONMENT_KEY
    } else {
        USER_ENVIRONMENT_KEY
    };

    let (hive, path) = crate::core::registry_helpers::parse_hive_prefix(hive_str)
        .map_err(|e| format!("Bad hive: {}", e))?;

    // Delete the value
    unsafe {
        let key = registry_helpers::open_key_write(hive, &path)
            .ok_or_else(|| "Cannot open registry key for write".to_string())?;
        registry_helpers::delete_value(key, name)
            .map_err(|e| format!("Failed to delete env var: {:?}", e))?;
        let _ = windows::Win32::System::Registry::RegCloseKey(key);
    }

    broadcast_environment_change()?;
    Ok(())
}

/// Broadcast `WM_SETTINGCHANGE` with the documented `"Environment"` setting
/// name after a registry mutation. A notification failure is returned so the
/// caller cannot report a fully completed, verified operation without noting it.
pub fn broadcast_environment_change() -> Result<(), String> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };

    let environment = HSTRING::from(ENVIRONMENT_SETTING_CHANGE);
    let mut result = 0usize;
    unsafe {
        let send_result = SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(environment.as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            5_000,
            Some(&mut result),
        );
        if send_result.0 == 0 {
            return Err(format!(
                "Environment was changed but notification failed: {}",
                windows::core::Error::from_win32()
            ));
        }
    }
    Ok(())
}

fn validate_env_var(var: &EnvVar) -> Result<(), String> {
    validate_scope(&var.scope)?;
    validate_name(&var.name)?;
    match var.var_type.as_str() {
        "REG_SZ" | "REG_EXPAND_SZ" => {}
        _ => {
            return Err(format!(
                "Unsupported environment variable type: {}",
                var.var_type
            ))
        }
    }
    if var.value.contains('\0') || var.value.contains('\r') || var.value.contains('\n') {
        return Err("Environment variable value cannot contain control characters".to_string());
    }
    Ok(())
}

fn validate_scope(scope: &str) -> Result<(), String> {
    match scope {
        "user" | "system" => Ok(()),
        _ => Err(format!("Unsupported environment variable scope: {}", scope)),
    }
}

fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 255 {
        return Err("Environment variable name length is invalid".to_string());
    }
    if name.starts_with('=')
        || name.contains('\0')
        || name.contains('=')
        || name.chars().any(|c| c.is_control())
    {
        return Err("Environment variable name contains unsupported characters".to_string());
    }
    if is_protected_name(name) {
        return Err(format!(
            "Protected environment variable cannot be modified: {name}"
        ));
    }
    Ok(())
}

pub(crate) fn is_protected_name(name: &str) -> bool {
    ["SystemRoot", "windir", "ComSpec", "PATHEXT"]
        .iter()
        .any(|protected| name.eq_ignore_ascii_case(protected))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_supported_string_registry_types_are_exposed() {
        assert_eq!(registry_type_name(1), Some("REG_SZ"));
        assert_eq!(registry_type_name(2), Some("REG_EXPAND_SZ"));
        assert_eq!(registry_type_name(4), None);
    }

    #[test]
    fn setting_change_contract_names_the_environment_namespace() {
        assert_eq!(ENVIRONMENT_SETTING_CHANGE, "Environment");
    }

    #[test]
    fn protected_environment_names_are_case_insensitive() {
        assert!(is_protected_name("systemroot"));
        assert!(is_protected_name("PATHEXT"));
        assert!(!is_protected_name("Path"));
    }
}
