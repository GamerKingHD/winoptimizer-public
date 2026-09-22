//! Startup programs manager - reads and controls auto-starting programs.
//! Sources: HKCU\Software\Microsoft\Windows\CurrentVersion\Run
//!          HKLM\Software\Microsoft\Windows\CurrentVersion\Run
//!          %APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartupProgram {
    pub id: String,
    pub name: String,
    pub command: String,
    pub location: String, // "HKCU_Run", "HKLM_Run", "StartupFolder"
    pub value_type: Option<String>,
    pub enabled: bool,
    #[serde(default)]
    pub protected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_action_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StartupMutationResult {
    pub status: String,
    pub action_id: String,
    pub message: String,
    pub recovery_available: bool,
    pub verified: bool,
}

fn program_id(name: &str, command: &str, location: &str) -> String {
    blake3::hash(format!("{location}\0{name}\0{command}").as_bytes())
        .to_hex()
        .to_string()
}

pub fn startup_folder_path() -> Option<PathBuf> {
    std::env::var("APPDATA").ok().map(|app_data| {
        PathBuf::from(app_data).join(r"Microsoft\Windows\Start Menu\Programs\Startup")
    })
}

pub fn is_startup_folder_path(path: &Path) -> bool {
    let Some(folder) = startup_folder_path().and_then(|p| p.canonicalize().ok()) else {
        return false;
    };
    let candidate = if path.exists() {
        path.canonicalize().ok()
    } else {
        path.parent()
            .and_then(|parent| parent.canonicalize().ok())
            .and_then(|parent| path.file_name().map(|name| parent.join(name)))
    };
    candidate.is_some_and(|candidate| candidate.starts_with(folder))
}

/// Enumerate all startup programs.
pub fn list() -> Vec<StartupProgram> {
    let mut programs = Vec::new();

    // Registry-based startup entries
    programs.extend(list_registry_run(
        r"Software\Microsoft\Windows\CurrentVersion\Run",
        "HKCU_Run",
        true,
    ));
    programs.extend(list_registry_run(
        r"Software\Microsoft\Windows\CurrentVersion\RunOnce",
        "HKCU_RunOnce",
        true,
    ));

    // HKLM requires elevation
    if crate::core::elevation::is_elevated() {
        programs.extend(list_registry_run(
            r"Software\Microsoft\Windows\CurrentVersion\Run",
            "HKLM_Run",
            false,
        ));
    }

    // Startup folder
    if let Some(startup_folder) = startup_folder_path() {
        if startup_folder.exists() {
            if let Ok(entries) = std::fs::read_dir(startup_folder) {
                for entry in entries.flatten() {
                    if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                        programs.push(StartupProgram {
                            id: program_id(
                                &entry.file_name().to_string_lossy(),
                                &entry.path().to_string_lossy(),
                                "StartupFolder",
                            ),
                            name: entry.file_name().to_string_lossy().to_string(),
                            command: entry.path().to_string_lossy().to_string(),
                            location: "StartupFolder".to_string(),
                            value_type: None,
                            enabled: true,
                            protected: false,
                            recovery_action_id: None,
                        });
                    }
                }
            }
        }
    }

    mark_protected(&mut programs);
    programs
}

fn list_registry_run(key_path: &str, location: &str, is_hkcu: bool) -> Vec<StartupProgram> {
    use windows::core::*;
    use windows::Win32::System::Registry::*;

    let mut programs = Vec::new();
    let hive = if is_hkcu {
        HKEY_CURRENT_USER
    } else {
        HKEY_LOCAL_MACHINE
    };

    unsafe {
        let mut key = std::mem::zeroed();
        if RegOpenKeyExW(hive, &HSTRING::from(key_path), Some(0), KEY_READ, &mut key).is_err() {
            return programs;
        }

        let mut index: u32 = 0;
        let mut name_buf = [0u16; 4096];
        let mut name_len = name_buf.len() as u32;
        let mut value_buf = [0u16; 16384];
        let mut value_len = value_buf.len() as u32;
        let mut _type: u32 = 0;

        while RegEnumValueW(
            key,
            index,
            Some(PWSTR(name_buf.as_mut_ptr())),
            &mut name_len,
            None,
            Some(&mut _type),
            Some(value_buf.as_mut_ptr() as *mut u8),
            Some(&mut value_len),
        )
        .is_ok()
        {
            if _type == REG_SZ.0 || _type == REG_EXPAND_SZ.0 {
                let name = String::from_utf16_lossy(&name_buf[..name_len as usize]);
                let val_len = (value_len as usize / 2).saturating_sub(1);
                let command = String::from_utf16_lossy(&value_buf[..val_len]);

                programs.push(StartupProgram {
                    id: program_id(&name, &command, location),
                    name,
                    command,
                    location: location.to_string(),
                    value_type: Some(
                        if _type == REG_EXPAND_SZ.0 {
                            "REG_EXPAND_SZ"
                        } else {
                            "REG_SZ"
                        }
                        .into(),
                    ),
                    enabled: true,
                    protected: false,
                    recovery_action_id: None,
                });
            }

            index += 1;
            name_len = name_buf.len() as u32;
            value_len = value_buf.len() as u32;
        }

        let _ = RegCloseKey(key);
    }

    programs
}

fn mark_protected(programs: &mut [StartupProgram]) {
    for program in programs {
        let protected = is_protected(program);
        program.protected = protected;
    }
}

pub fn find_by_id(id: &str) -> crate::core::error::Result<StartupProgram> {
    use crate::core::error::WinOptimizerError;

    if id.len() != 64 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(WinOptimizerError::Other(
            "Invalid startup program identifier".into(),
        ));
    }

    list()
        .into_iter()
        .find(|program| program.id == id)
        .ok_or_else(|| WinOptimizerError::Other("Startup program is no longer available".into()))
}

pub fn list_with_recovery(
    recovery_store: &crate::core::startup_recovery::StartupRecoveryStore,
) -> crate::core::error::Result<Vec<StartupProgram>> {
    let entries = recovery_store
        .list()
        .map_err(crate::core::error::WinOptimizerError::Database)?;
    let mut programs = merge_disabled_programs(list(), &entries);
    mark_protected(&mut programs);
    Ok(programs)
}

pub fn find_by_id_with_recovery(
    id: &str,
    recovery_store: &crate::core::startup_recovery::StartupRecoveryStore,
) -> crate::core::error::Result<StartupProgram> {
    list_with_recovery(recovery_store)?
        .into_iter()
        .find(|program| program.id == id)
        .ok_or_else(|| {
            crate::core::error::WinOptimizerError::Other(
                "Startup program is no longer available".into(),
            )
        })
}

pub fn find_recovery_entry_for_program(
    program: &StartupProgram,
    recovery_store: &crate::core::startup_recovery::StartupRecoveryStore,
) -> crate::core::error::Result<crate::core::undo_log::UndoEntry> {
    use crate::core::error::WinOptimizerError;

    let entry_id = program
        .recovery_action_id
        .as_deref()
        .ok_or_else(|| WinOptimizerError::Other("Startup program has no recovery record".into()))?;
    let entry = recovery_store
        .list()
        .map_err(WinOptimizerError::Database)?
        .into_iter()
        .find(|entry| entry.id == entry_id && entry.rolled_back_at.is_none())
        .ok_or_else(|| WinOptimizerError::Other("Startup recovery record is stale".into()))?;
    let reconstructed = startup_program_from_undo_entry(&entry)
        .ok_or_else(|| WinOptimizerError::Other("Startup recovery record is invalid".into()))?;
    if reconstructed != *program {
        return Err(WinOptimizerError::Other(
            "Startup recovery record changed after preview".into(),
        ));
    }
    Ok(entry)
}

pub(crate) fn merge_disabled_programs(
    mut active: Vec<StartupProgram>,
    undo_entries: &[crate::core::undo_log::UndoEntry],
) -> Vec<StartupProgram> {
    let active_ids: HashSet<String> = active.iter().map(|program| program.id.clone()).collect();
    let mut seen_disabled = HashSet::new();

    for entry in undo_entries {
        if entry.kind != "startup_change" || entry.rolled_back_at.is_some() {
            continue;
        }
        let Some(program) = startup_program_from_undo_entry(entry) else {
            continue;
        };
        if active_ids.contains(&program.id) || !seen_disabled.insert(program.id.clone()) {
            continue;
        }
        active.push(program);
    }

    active
}

fn startup_program_from_undo_entry(
    entry: &crate::core::undo_log::UndoEntry,
) -> Option<StartupProgram> {
    if entry.kind != "startup_change" || entry.rolled_back_at.is_some() {
        return None;
    }
    let reversal: serde_json::Value = serde_json::from_str(&entry.reversal_command).ok()?;
    match reversal["action"].as_str()? {
        "restore_startup_registry" => {
            let hive = reversal["hive"].as_str()?;
            let key_path = reversal["key_path"].as_str()?;
            let location = match (hive, key_path) {
                ("HKCU", r"Software\Microsoft\Windows\CurrentVersion\Run") => "HKCU_Run",
                ("HKCU", r"Software\Microsoft\Windows\CurrentVersion\RunOnce") => "HKCU_RunOnce",
                ("HKLM", r"Software\Microsoft\Windows\CurrentVersion\Run") => "HKLM_Run",
                _ => return None,
            };
            let name = reversal["value_name"].as_str()?;
            let command = reversal["value_data"].as_str()?;
            let value_type = reversal["value_type"].as_str().unwrap_or("REG_SZ");
            if !valid_startup_value_name(name)
                || command.is_empty()
                || !matches!(value_type, "REG_SZ" | "REG_EXPAND_SZ")
            {
                return None;
            }
            Some(StartupProgram {
                id: program_id(name, command, location),
                name: name.into(),
                command: command.into(),
                location: location.into(),
                value_type: Some(value_type.into()),
                enabled: false,
                protected: false,
                recovery_action_id: Some(entry.id.clone()),
            })
        }
        "restore_file" => {
            let original_path = reversal["original_path"].as_str()?;
            let path = Path::new(original_path);
            let name = path.file_name()?.to_string_lossy().to_string();
            if !valid_startup_value_name(&name)
                || entry.target != name
                || !is_startup_folder_path(path)
            {
                return None;
            }
            let backup_path = reversal["backup_path"].as_str()?;
            if backup_path.is_empty() || !Path::new(backup_path).is_file() {
                return None;
            }
            Some(StartupProgram {
                id: program_id(&name, original_path, "StartupFolder"),
                name,
                command: original_path.into(),
                location: "StartupFolder".into(),
                value_type: None,
                enabled: false,
                protected: false,
                recovery_action_id: Some(entry.id.clone()),
            })
        }
        _ => None,
    }
}

fn valid_startup_value_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 260
        && !name.chars().any(|character| {
            character == '\0' || character == '\r' || character == '\n' || character == '\\'
        })
}

/// Disable a startup program and persist its recovery record before mutation.
pub fn disable(
    program: &StartupProgram,
    recovery_store: &crate::core::startup_recovery::StartupRecoveryStore,
    session_id: &str,
    backup_mgr: &crate::core::backup::BackupManager,
) -> crate::core::error::Result<StartupMutationResult> {
    use crate::core::error::WinOptimizerError;
    use windows::core::*;
    use windows::Win32::System::Registry::*;

    if is_protected(program) {
        return Err(WinOptimizerError::Other(format!(
            "Protected startup entry cannot be disabled: {}",
            program.name
        )));
    }

    // Security: validate name doesn't contain dangerous characters
    if program.name.is_empty() || program.name.len() > 260 {
        return Err(WinOptimizerError::Other(
            "Invalid startup program name".into(),
        ));
    }
    if program
        .name
        .chars()
        .any(|c| c == '\0' || c == '\r' || c == '\n')
    {
        return Err(WinOptimizerError::Other(
            "Startup program name contains invalid characters".into(),
        ));
    }

    match program.location.as_str() {
        "HKCU_Run" | "HKCU_RunOnce" | "HKLM_Run" => {
            let (hive, hive_name, key_path, is_hkcu) = match program.location.as_str() {
                "HKCU_Run" => (
                    HKEY_CURRENT_USER,
                    "HKCU",
                    r"Software\Microsoft\Windows\CurrentVersion\Run",
                    true,
                ),
                "HKCU_RunOnce" => (
                    HKEY_CURRENT_USER,
                    "HKCU",
                    r"Software\Microsoft\Windows\CurrentVersion\RunOnce",
                    true,
                ),
                "HKLM_Run" => {
                    if !crate::core::elevation::is_elevated() {
                        return Err(WinOptimizerError::ElevationRequired);
                    }
                    (
                        HKEY_LOCAL_MACHINE,
                        "HKLM",
                        r"Software\Microsoft\Windows\CurrentVersion\Run",
                        false,
                    )
                }
                _ => unreachable!(),
            };

            // Save existing value for undo
            let reversal = serde_json::json!({
                "action": "restore_startup_registry",
                "hive": hive_name,
                "key_path": key_path,
                "value_name": program.name,
                "value_data": program.command,
                "value_type": program.value_type.as_deref().unwrap_or("REG_SZ"),
            })
            .to_string();

            let recovery = recovery_store.record(
                session_id,
                "startup_change",
                &program.name,
                &reversal,
                None,
            )?;

            unsafe {
                let mut key = std::mem::zeroed();
                if RegOpenKeyExW(
                    hive,
                    &HSTRING::from(key_path),
                    Some(0),
                    KEY_SET_VALUE,
                    &mut key,
                )
                .is_ok()
                {
                    let result = RegDeleteValueW(key, &HSTRING::from(&program.name));
                    let _ = RegCloseKey(key);
                    if result.is_err() {
                        return Err(WinOptimizerError::Other(
                            "Failed to delete startup registry value".into(),
                        ));
                    }
                } else {
                    return Err(WinOptimizerError::Other(
                        "Failed to open startup registry key".into(),
                    ));
                }
            }
            let verified = !list_registry_run(key_path, &program.location, is_hkcu)
                .iter()
                .any(|entry| entry.name == program.name);
            if !verified {
                return Err(WinOptimizerError::Other(
                    "Startup entry was not removed".into(),
                ));
            }
            Ok(StartupMutationResult {
                status: "completed".into(),
                action_id: recovery.id,
                message: format!("{} will no longer start with Windows", program.name),
                recovery_available: true,
                verified,
            })
        }
        "StartupFolder" => {
            let path = std::path::Path::new(&program.command);
            // Security: validate the path is actually inside the startup folder
            // to prevent arbitrary file deletion via forged IPC calls
            if !is_startup_folder_path(path) {
                return Err(WinOptimizerError::Other(
                    "Path is not inside the startup folder".into(),
                ));
            }
            if path.exists() {
                // Backup shortcut
                let backup_path = backup_mgr
                    .backup_file(path)
                    .map_err(|e| WinOptimizerError::Other(format!("Backup failed: {}", e)))?;

                let reversal = serde_json::json!({
                    "action": "restore_file",
                    "backup_path": backup_path.to_string_lossy(),
                    "original_path": program.command,
                })
                .to_string();

                let recovery = recovery_store.record(
                    session_id,
                    "startup_change",
                    &program.name,
                    &reversal,
                    None,
                )?;
                if let Err(error) = std::fs::remove_file(path) {
                    return Err(WinOptimizerError::Other(error.to_string()));
                }
                let verified = !path.exists();
                if !verified {
                    return Err(WinOptimizerError::Other(
                        "Startup file was not removed".into(),
                    ));
                }
                return Ok(StartupMutationResult {
                    status: "completed".into(),
                    action_id: recovery.id,
                    message: format!("{} will no longer start with Windows", program.name),
                    recovery_available: true,
                    verified,
                });
            }
            Err(WinOptimizerError::Other(
                "Startup file is no longer available".into(),
            ))
        }
        _ => Err(WinOptimizerError::Other(format!(
            "Unknown location: {}",
            program.location
        ))),
    }
}

pub fn is_protected(program: &StartupProgram) -> bool {
    let identity = format!("{} {}", program.name, program.command).to_ascii_lowercase();
    [
        "securityhealth",
        "windows security",
        "windows defender",
        "defender",
        "msmpeng",
        "nissrv",
        "mssense",
        "mpdefendercore",
        "winoptimizer",
    ]
    .iter()
    .any(|protected| identity.contains(protected))
}

#[cfg(test)]
mod opaque_id_tests {
    use super::program_id;

    #[test]
    fn startup_program_ids_are_opaque_and_bound_to_the_listed_record() {
        let id = program_id("Example", r"C:\Apps\example.exe", "HKCU_Run");

        assert_eq!(id.len(), 64);
        assert!(id.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(
            id,
            program_id("Example", r"C:\Apps\other.exe", "HKCU_Run"),
            "the command is part of the server-issued identifier"
        );
        assert_ne!(
            id,
            program_id("Example", r"C:\Apps\example.exe", "StartupFolder"),
            "the location is part of the server-issued identifier"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn undo_entry(
        id: &str,
        reversal: &str,
        target: &str,
        rolled_back_at: Option<&str>,
    ) -> crate::core::undo_log::UndoEntry {
        crate::core::undo_log::UndoEntry {
            id: id.into(),
            session_id: "session".into(),
            kind: "startup_change".into(),
            target: target.into(),
            reversal_command: reversal.into(),
            original_size_bytes: None,
            created_at: "2026-07-31T00:00:00Z".into(),
            rolled_back_at: rolled_back_at.map(str::to_string),
        }
    }

    fn registry_reversal(name: &str, command: &str) -> String {
        serde_json::json!({
            "action": "restore_startup_registry",
            "hive": "HKCU",
            "key_path": r"Software\Microsoft\Windows\CurrentVersion\Run",
            "value_name": name,
            "value_data": command,
            "value_type": "REG_SZ",
        })
        .to_string()
    }

    #[test]
    fn merge_reconstructs_disabled_registry_entries_from_undo_records() {
        let entry = undo_entry(
            "undo-1",
            &registry_reversal("Fixture", r"C:\fixture.exe"),
            "Fixture",
            None,
        );

        let programs = merge_disabled_programs(Vec::new(), &[entry]);

        assert_eq!(programs.len(), 1);
        assert_eq!(programs[0].name, "Fixture");
        assert_eq!(programs[0].location, "HKCU_Run");
        assert!(!programs[0].enabled);
        assert_eq!(programs[0].recovery_action_id.as_deref(), Some("undo-1"));
    }

    #[test]
    fn merge_skips_rolled_back_stale_and_already_active_records() {
        let active = StartupProgram {
            id: program_id("Fixture", r"C:\fixture.exe", "HKCU_Run"),
            name: "Fixture".into(),
            command: r"C:\fixture.exe".into(),
            location: "HKCU_Run".into(),
            value_type: Some("REG_SZ".into()),
            enabled: true,
            protected: false,
            recovery_action_id: None,
        };
        let rolled_back = undo_entry(
            "undo-rolled-back",
            &registry_reversal("Old", r"C:\old.exe"),
            "Old",
            Some("2026-07-31T00:01:00Z"),
        );
        let stale = undo_entry(
            "undo-stale",
            r#"{"action":"restore_startup_registry","hive":"HKCU","key_path":"Software\Bad","value_name":"Bad","value_data":"C:\\bad.exe"}"#,
            "Bad",
            None,
        );
        let duplicate = undo_entry(
            "undo-active",
            &registry_reversal("Fixture", r"C:\fixture.exe"),
            "Fixture",
            None,
        );

        let programs = merge_disabled_programs(vec![active], &[rolled_back, stale, duplicate]);

        assert_eq!(programs.len(), 1);
        assert!(programs[0].enabled);
        assert!(programs[0].recovery_action_id.is_none());
    }

    #[test]
    fn protected_identity_detects_windows_security_and_self() {
        let mut program = StartupProgram {
            id: "fixture".into(),
            name: "Windows Security notification icon".into(),
            command: r"C:\Windows\System32\SecurityHealthSystray.exe".into(),
            location: "HKCU_Run".into(),
            value_type: Some("REG_SZ".into()),
            enabled: true,
            protected: false,
            recovery_action_id: None,
        };
        assert!(is_protected(&program));
        program.name = "WinOptimizer".into();
        program.command = r"C:\Apps\winoptimizer.exe".into();
        assert!(is_protected(&program));
    }

    #[test]
    fn protected_identity_detects_defender_engine_targets() {
        let mut program = StartupProgram {
            id: "fixture".into(),
            name: "Antimalware Service Executable".into(),
            command: r"C:\Windows\System32\MsMpEng.exe".into(),
            location: "HKCU_Run".into(),
            value_type: Some("REG_SZ".into()),
            enabled: true,
            protected: false,
            recovery_action_id: None,
        };
        assert!(is_protected(&program));
        program.name = "Microsoft Network Realtime Inspection Service".into();
        program.command = r"C:\Windows\System32\NisSrv.exe".into();
        assert!(is_protected(&program));
    }

    #[test]
    fn startup_folder_path_is_some() {
        // Should resolve on any Windows system
        let path = startup_folder_path();
        assert!(path.is_some());
    }

    #[test]
    fn is_startup_folder_path_rejects_nonexistent() {
        assert!(!is_startup_folder_path(std::path::Path::new(
            r"C:\nonexistent\path\file.lnk"
        )));
    }
}
