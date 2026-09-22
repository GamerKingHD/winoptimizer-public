use crate::*;

// ============================================================
// Tauri Commands - Undo Log
// ============================================================

#[tauri::command]
pub(crate) async fn get_undo_log(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<core::undo_log::UndoEntry>> {
    let Some(undo_log) = state
        .ensure_undo_log()
        .map_err(|error| core::error::WinOptimizerError::Other(error.to_string()))?
    else {
        return Ok(Vec::new());
    };
    tauri::async_runtime::spawn_blocking(move || {
        undo_log
            .list()
            .map_err(core::error::WinOptimizerError::Database)
    })
    .await
    .map_err(|e| core::error::WinOptimizerError::Other(e.to_string()))?
}

pub(crate) fn restore_startup_undo_entry(
    backup_mgr: &core::backup::BackupManager,
    entry: &core::undo_log::UndoEntry,
    reversal: &serde_json::Value,
) -> Result<()> {
    if entry.kind != "startup_change" {
        return Err(core::error::WinOptimizerError::Other(
            "Startup undo action cannot be used for this entry type".into(),
        ));
    }

    match reversal["action"].as_str().unwrap_or("") {
        "restore_startup_registry" => {
            let hive = reversal["hive"].as_str().unwrap_or("");
            let key_path = reversal["key_path"].as_str().unwrap_or("");
            let value_name = reversal["value_name"].as_str().unwrap_or("");
            let value_data = reversal["value_data"].as_str().unwrap_or("");
            let value_type = reversal["value_type"].as_str().unwrap_or("REG_SZ");
            validate_startup_registry_undo(hive, key_path, value_name)?;
            write_startup_registry_value(hive, key_path, value_name, value_data, value_type)?;

            let location = match (hive, key_path) {
                ("HKCU", r"Software\Microsoft\Windows\CurrentVersion\Run") => "HKCU_Run",
                ("HKCU", r"Software\Microsoft\Windows\CurrentVersion\RunOnce") => "HKCU_RunOnce",
                ("HKLM", r"Software\Microsoft\Windows\CurrentVersion\Run") => "HKLM_Run",
                _ => unreachable!("startup registry target was validated above"),
            };
            let verified = startup::programs::list().into_iter().any(|program| {
                program.location == location
                    && program.name == value_name
                    && program.command == value_data
            });
            if !verified {
                return Err(core::error::WinOptimizerError::Other(
                    "Startup registry entry restoration could not be verified".into(),
                ));
            }
        }
        "restore_file" => {
            let backup_path = std::path::Path::new(reversal["backup_path"].as_str().unwrap_or(""));
            let original_path =
                std::path::Path::new(reversal["original_path"].as_str().unwrap_or(""));
            if !startup::programs::is_startup_folder_path(original_path) {
                return Err(core::error::WinOptimizerError::Other(
                    "Startup file restore target is not allowed".into(),
                ));
            }
            backup_mgr
                .restore_file_best_effort(backup_path, original_path)
                .map_err(core::error::WinOptimizerError::Io)?;
            if !original_path.is_file() {
                return Err(core::error::WinOptimizerError::Other(
                    "Startup file restoration could not be verified".into(),
                ));
            }
        }
        _ => {
            return Err(core::error::WinOptimizerError::Other(
                "Unsupported startup undo action".into(),
            ));
        }
    }

    Ok(())
}

fn validate_startup_registry_undo(hive: &str, key_path: &str, value_name: &str) -> Result<()> {
    let allowed_key = matches!(
        (hive, key_path),
        ("HKCU", r"Software\Microsoft\Windows\CurrentVersion\Run")
            | ("HKCU", r"Software\Microsoft\Windows\CurrentVersion\RunOnce")
            | ("HKLM", r"Software\Microsoft\Windows\CurrentVersion\Run")
    );
    if !allowed_key {
        return Err(core::error::WinOptimizerError::Other(
            "Startup registry undo target is not allowed".into(),
        ));
    }
    if value_name.is_empty()
        || value_name.len() > 260
        || value_name
            .chars()
            .any(|c| c == '\0' || c == '\r' || c == '\n' || c == '\\')
    {
        return Err(core::error::WinOptimizerError::Other(
            "Invalid startup registry value name in undo entry".into(),
        ));
    }
    if hive == "HKLM" && !core::elevation::is_elevated() {
        return Err(core::error::WinOptimizerError::ElevationRequired);
    }
    Ok(())
}

fn write_startup_registry_value(
    hive: &str,
    key_path: &str,
    value_name: &str,
    value_data: &str,
    value_type: &str,
) -> Result<()> {
    use windows::core::*;
    use windows::Win32::System::Registry::*;

    let hive = match hive {
        "HKCU" => HKEY_CURRENT_USER,
        "HKLM" => HKEY_LOCAL_MACHINE,
        _ => {
            return Err(core::error::WinOptimizerError::Other(
                "Invalid startup registry hive".into(),
            ))
        }
    };

    unsafe {
        let mut key: HKEY = std::mem::zeroed();
        if RegCreateKeyExW(
            hive,
            &HSTRING::from(key_path),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
        .is_err()
        {
            return Err(core::error::WinOptimizerError::Other(
                "Failed to create startup registry key during undo".into(),
            ));
        }
        let data: Vec<u8> = value_data
            .encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(|c| c.to_le_bytes())
            .collect();
        let registry_type = match value_type {
            "REG_SZ" => REG_SZ,
            "REG_EXPAND_SZ" => REG_EXPAND_SZ,
            _ => {
                let _ = RegCloseKey(key);
                return Err(core::error::WinOptimizerError::Other(
                    "Unsupported startup registry value type".into(),
                ));
            }
        };
        let result = RegSetValueExW(
            key,
            &HSTRING::from(value_name),
            None,
            registry_type,
            Some(&data),
        );
        let _ = RegCloseKey(key);
        if result.is_err() {
            return Err(core::error::WinOptimizerError::Other(
                "Failed to write startup registry value during undo".into(),
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod trust_tests {
    use super::*;

    #[test]
    fn allows_known_startup_registry_undo_targets() {
        assert!(validate_startup_registry_undo(
            "HKCU",
            r"Software\Microsoft\Windows\CurrentVersion\Run",
            "Example"
        )
        .is_ok());
    }

    #[test]
    fn rejects_unknown_startup_registry_undo_targets() {
        assert!(validate_startup_registry_undo("HKCU", r"Software\Bad", "Example").is_err());
        assert!(validate_startup_registry_undo(
            "HKCU",
            r"Software\Microsoft\Windows\CurrentVersion\Run",
            "Bad\\Name"
        )
        .is_err());
    }
}
