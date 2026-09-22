//! Installed Windows updates list.
//! Reads from the Hotfix registry key, which stores KB patches directly
//! (same source used by `wmic qfe list` and Windows Update history).
//! Also reads from Uninstall keys for quality-of-life title display.

use serde::{Deserialize, Serialize};
use windows::core::*;
use windows::Win32::System::Registry::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub kb_number: Option<String>,
    pub title: String,
    pub installed_date: Option<String>,
}

const MAX_UPDATES: u32 = 2000;

pub fn list_installed_updates() -> Vec<UpdateInfo> {
    let mut updates = Vec::new();

    // Primary source: HKLM\...\Hotfix - stores KB patches directly as subkey names.
    // Each subkey is named "KBxxxxxxx" with InstalledDate and Description values.
    let hotfix_path = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Hotfix";
    updates.extend(scan_hotfix_key(hotfix_path));

    // The Hotfix key may also exist under the WOW6432Node on 64-bit systems.
    let wow_path = r"SOFTWARE\WOW6432Node\Microsoft\Windows NT\CurrentVersion\Hotfix";
    if wow_path != hotfix_path {
        updates.extend(scan_hotfix_key(wow_path));
    }

    // Secondary source: Uninstall entries with KB numbers for better titles.
    // These supplement the Hotfix list with more descriptive names.
    updates.extend(scan_uninstall_updates(
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
    ));
    updates.extend(scan_uninstall_updates(
        r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
    ));

    // Deduplicate by kb_number (prefer the entry with more info)
    let mut seen = std::collections::HashSet::new();
    updates.retain(|u| {
        let key = u.kb_number.clone().unwrap_or_default();
        if seen.contains(&key) {
            false
        } else {
            seen.insert(key);
            true
        }
    });

    // Sort newest first by date, then by KB number descending
    updates.sort_by(|a, b| {
        b.installed_date
            .as_deref()
            .unwrap_or("")
            .cmp(a.installed_date.as_deref().unwrap_or(""))
            .then_with(|| {
                b.kb_number
                    .as_deref()
                    .unwrap_or("")
                    .cmp(a.kb_number.as_deref().unwrap_or(""))
            })
    });

    updates
}

/// Scan the Hotfix registry key for KB patches.
fn scan_hotfix_key(key_path: &str) -> Vec<UpdateInfo> {
    let mut updates = Vec::new();

    unsafe {
        let mut key: HKEY = std::mem::zeroed();
        if RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(key_path),
            Some(0),
            KEY_READ,
            &mut key,
        )
        .is_err()
        {
            return updates;
        }

        let mut index: u32 = 0;
        let mut name_buf = [0u16; 512];
        let mut name_len = name_buf.len() as u32;

        while RegEnumKeyExW(
            key,
            index,
            Some(PWSTR(name_buf.as_mut_ptr())),
            &mut name_len,
            None,
            None,
            None,
            None,
        )
        .is_ok()
        {
            let name = String::from_utf16_lossy(&name_buf[..name_len as usize]);

            if !name.starts_with("KB") {
                index += 1;
                name_len = name_buf.len() as u32;
                continue;
            }

            // Open the subkey to read InstalledDate and Description
            let subkey_path = format!(r"{}\{}", key_path, name);
            let mut subkey: HKEY = std::mem::zeroed();
            let installed_date = if RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                &HSTRING::from(&subkey_path),
                Some(0),
                KEY_READ,
                &mut subkey,
            )
            .is_ok()
            {
                let date = read_reg_value_string(subkey, "InstalledDate");
                let _desc = read_reg_value_string(subkey, "Description");
                let _ = RegCloseKey(subkey);
                date
            } else {
                None
            };

            updates.push(UpdateInfo {
                kb_number: Some(name.clone()),
                title: name,
                installed_date,
            });

            index += 1;
            name_len = name_buf.len() as u32;

            if index >= MAX_UPDATES {
                break;
            }
        }
        let _ = RegCloseKey(key);
    }

    updates
}

/// Scan Uninstall keys for entries with DisplayName that looks like a Windows Update.
fn scan_uninstall_updates(key_path: &str) -> Vec<UpdateInfo> {
    let mut updates = Vec::new();

    unsafe {
        let mut key: HKEY = std::mem::zeroed();
        if RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(key_path),
            Some(0),
            KEY_READ,
            &mut key,
        )
        .is_err()
        {
            return updates;
        }

        let mut index: u32 = 0;
        let mut name_buf = [0u16; 512];
        let mut name_len = name_buf.len() as u32;

        while RegEnumKeyExW(
            key,
            index,
            Some(PWSTR(name_buf.as_mut_ptr())),
            &mut name_len,
            None,
            None,
            None,
            None,
        )
        .is_ok()
        {
            let subkey_name = String::from_utf16_lossy(&name_buf[..name_len as usize]);
            let subkey_path = format!(r"{}\{}", key_path, subkey_name);

            let mut subkey: HKEY = std::mem::zeroed();
            if RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                &HSTRING::from(&subkey_path),
                Some(0),
                KEY_READ,
                &mut subkey,
            )
            .is_ok()
            {
                let display_name = read_reg_value_string(subkey, "DisplayName");
                let install_date = read_reg_value_string(subkey, "InstallDate");

                if let Some(ref name) = display_name {
                    // Only include Windows updates (not regular installed apps)
                    let is_update = name.contains("KB")
                        || name.starts_with("Update for")
                        || name.starts_with("Security Update")
                        || name.starts_with("Hotfix")
                        || name.starts_with("Cumulative Update");

                    if is_update {
                        // Extract KB number
                        let kb = name.find('(').and_then(|start| {
                            let end = name[start..]
                                .find(')')
                                .map(|e| start + e + 1)
                                .unwrap_or(name.len());
                            let inner = &name[start..end];
                            // Look for KB inside parentheses like "(KB5048652)"
                            inner.find("KB").map(|k| {
                                let kb_end = inner[k..]
                                    .find(|c: char| !c.is_alphanumeric() && c != '-')
                                    .unwrap_or(inner.len() - k);
                                inner[k..k + kb_end].to_string()
                            })
                        });

                        updates.push(UpdateInfo {
                            kb_number: kb.or_else(|| {
                                // Fallback: search the whole title for KB
                                name.find("KB").map(|start| {
                                    let end = name[start..]
                                        .find(|c: char| !c.is_alphanumeric())
                                        .unwrap_or(name.len() - start);
                                    name[start..start + end].to_string()
                                })
                            }),
                            title: name.clone(),
                            installed_date: install_date,
                        });
                    }
                }

                let _ = RegCloseKey(subkey);
            }

            index += 1;
            name_len = name_buf.len() as u32;

            if index >= MAX_UPDATES {
                break;
            }
        }
        let _ = RegCloseKey(key);
    }

    updates
}

/// Read a REG_SZ value from an open registry key.
unsafe fn read_reg_value_string(key: HKEY, value_name: &str) -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut buf_size = (buf.len() * 2) as u32;
    let mut value_type = REG_VALUE_TYPE::default();

    let result = RegQueryValueExW(
        key,
        &HSTRING::from(value_name),
        None,
        Some(&mut value_type as *mut REG_VALUE_TYPE),
        Some(buf.as_mut_ptr() as *mut u8),
        Some(&mut buf_size),
    );

    if result.is_ok() && value_type == REG_SZ {
        let len = (buf_size as usize / 2).saturating_sub(1);
        Some(String::from_utf16_lossy(&buf[..len]))
    } else {
        None
    }
}
