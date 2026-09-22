//! Windows OS version and edition information.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OsInfo {
    pub version: String,
    pub edition: String,
    pub build: String,
    pub arch: String,
    pub release_id: Option<String>,
    pub install_date: Option<String>,
    pub last_boot: Option<String>,
    pub user_name: String,
    pub domain: Option<String>,
}

pub fn gather() -> OsInfo {
    OsInfo {
        version: get_windows_version(),
        edition: get_windows_edition(),
        build: get_windows_build(),
        arch: get_architecture(),
        release_id: get_release_id(),
        install_date: None, // Would need WMI
        last_boot: None,
        user_name: std::env::var("USERNAME").unwrap_or_else(|_| "Unknown".into()),
        domain: std::env::var("USERDOMAIN").ok(),
    }
}

fn get_windows_version() -> String {
    // Prefer ProductName (e.g. "Windows 11 Enterprise") from registry.
    if let Some(product) = get_registry_string(
        r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion",
        "ProductName",
    ) {
        // Windows 11 upgraded from 10 sometimes keeps "Windows 10" in ProductName.
        // The reliable discriminator: build >= 22000 is Windows 11.
        if product.contains("Windows 10") {
            if let Some(build_str) = get_registry_string(
                r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion",
                "CurrentBuild",
            ) {
                if let Ok(build) = build_str.parse::<u32>() {
                    if build >= 22000 {
                        return product.replace("Windows 10", "Windows 11");
                    }
                }
            }
        }
        return product;
    }

    // Fallback: construct version from individual components.
    let major = get_registry_string(
        r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion",
        "CurrentMajorVersionNumber",
    );
    let minor = get_registry_string(
        r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion",
        "CurrentMinorVersionNumber",
    );
    let build = get_registry_string(
        r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion",
        "CurrentBuild",
    );

    match (&major, &minor, &build) {
        (Some(maj), Some(min), Some(bld)) => {
            format!("Windows {}.{}.{}", maj, min, bld)
        }
        _ => {
            // Last resort: try CurrentVersion REG_SZ (e.g. "6.3")
            if let Some(cv) = get_registry_string(
                r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion",
                "CurrentVersion",
            ) {
                let bld = build.as_deref().unwrap_or("");
                format!("Windows {} (build {})", cv, bld)
            } else {
                "Windows (unknown version)".to_string()
            }
        }
    }
}

fn get_windows_edition() -> String {
    // Try EditionID first, then parse from ProductName as fallback.
    if let Some(edition) = get_registry_string(
        r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion",
        "EditionID",
    ) {
        return edition;
    }
    // Parse edition from ProductName (e.g. "Windows 11 Enterprise" -> "Enterprise")
    if let Some(product) = get_registry_string(
        r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion",
        "ProductName",
    ) {
        for ed in &["Enterprise", "Professional", "Education", "Home", "Pro"] {
            if product.contains(ed) {
                return ed.to_string();
            }
        }
    }
    "Unknown".into()
}

fn get_windows_build() -> String {
    get_registry_string(
        r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion",
        "CurrentBuild",
    )
    .unwrap_or_else(|| "Unknown".into())
}

fn get_release_id() -> Option<String> {
    // Windows 10 22H2+ / Windows 11 uses DisplayVersion (e.g. "24H2").
    // Older builds use ReleaseId (e.g. "2009").
    get_registry_string(
        r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion",
        "DisplayVersion",
    )
    .or_else(|| {
        get_registry_string(
            r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion",
            "ReleaseId",
        )
    })
}

fn get_architecture() -> String {
    unsafe {
        let mut sys_info = std::mem::zeroed();
        windows::Win32::System::SystemInformation::GetNativeSystemInfo(&mut sys_info);
        match sys_info.Anonymous.Anonymous.wProcessorArchitecture.0 {
            0 => "x86",
            5 => "ARM",
            9 => "x64",
            12 => "ARM64",
            _ => "Unknown",
        }
        .to_string()
    }
}

fn get_registry_string(key: &str, value_name: &str) -> Option<String> {
    let (hive_str, path) = if let Some(pos) = key.find('\\') {
        (&key[..pos], key[pos + 1..].to_string())
    } else {
        return None;
    };

    use windows::core::*;
    use windows::Win32::System::Registry::*;

    let hive = match hive_str {
        "HKLM" => HKEY_LOCAL_MACHINE,
        "HKCU" => HKEY_CURRENT_USER,
        _ => return None,
    };

    unsafe {
        let mut hkey = std::mem::zeroed();
        if RegOpenKeyExW(hive, &HSTRING::from(&path), Some(0), KEY_READ, &mut hkey).is_err() {
            return None;
        }

        let mut buf = [0u16; 4096];
        let mut buf_size = (buf.len() * 2) as u32;
        let mut _type = REG_VALUE_TYPE::default();

        let result = RegQueryValueExW(
            hkey,
            &HSTRING::from(value_name),
            None,
            Some(&mut _type as *mut REG_VALUE_TYPE),
            Some(buf.as_mut_ptr() as *mut u8),
            Some(&mut buf_size),
        );

        let _ = RegCloseKey(hkey);

        if result.is_ok() && (_type == REG_SZ || _type == REG_EXPAND_SZ) {
            let len = (buf_size as usize / 2).saturating_sub(1);
            Some(String::from_utf16_lossy(&buf[..len]))
        } else {
            None
        }
    }
}
