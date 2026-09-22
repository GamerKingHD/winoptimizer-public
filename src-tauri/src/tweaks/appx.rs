//! Native AppX de-provisioning via Windows policy registry keys.
//!
//! Instead of destructive PowerShell (Remove-AppxPackage / Get-AppxPackage),
//! this module writes to the native `RemoveDefaultMicrosoftStorePackages`
//! policy path. Windows processes these keys on next login and cleanly
//! unprovisions the targeted packages for all users.
//!
//! Benefits over PowerShell:
//! - Doesn't corrupt the app dependency tree
//! - Doesn't break Microsoft Store or Xbox services
//! - Windows handles cleanup natively (moves to DeletedAllUserPackages)
//! - Works for all users (machine-wide policy)
//!
//! The exclusion matrix protects essential system packages from removal.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use windows::core::HSTRING;
use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteKeyExW, RegDeleteValueW, RegEnumKeyExW, HKEY,
    HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, REG_DWORD, REG_OPTION_NON_VOLATILE,
};

// -- Policy registry paths -----------------------------------

const APPX_POLICY_ROOT: &str =
    r"SOFTWARE\Policies\Microsoft\Windows\Appx\RemoveDefaultMicrosoftStorePackages";

// -- Exclusion matrix (Section 6) -----------------------------

/// Packages that MUST NOT be removed - doing so breaks critical
/// Windows functionality (Store, Gaming, Xbox, in-app purchases).
const PROTECTED_PACKAGES: &[&str] = &[
    "Microsoft.WindowsStore",
    "Microsoft.GamingServices",
    "Microsoft.XboxIdentityProvider",
    "Microsoft.StorePurchaseApp",
    "Microsoft.SecHealthUI",
    "Microsoft.Windows.SecHealthUI",
];

fn is_protected_package(family_name: &str) -> bool {
    PROTECTED_PACKAGES.iter().any(|prefix| {
        family_name.len() >= prefix.len()
            && family_name[..prefix.len()].eq_ignore_ascii_case(prefix)
    })
}

fn package_risk(family_name: &str) -> &'static str {
    if family_name.starts_with("Microsoft.Xbox")
        || matches!(
            family_name,
            "Microsoft.Messaging"
                | "Microsoft.Office.OneNote"
                | "Microsoft.People"
                | "Microsoft.SkypeApp"
                | "Microsoft.WindowsCamera"
                | "Microsoft.WindowsCommunicationsApps"
                | "Microsoft.YourPhone"
        )
    {
        "caution"
    } else if family_name.starts_with("Microsoft.") || family_name.starts_with("Spotify.") {
        "safe"
    } else {
        "advanced"
    }
}

// -- Common bloatware candidates ------------------------------

/// Well-known pre-installed bloatware packages that are safe to remove.
/// These are the most common candidates. The frontend should allow
/// users to pick which ones to de-provision.
const BLOATWARE_CANDIDATES: &[(&str, &str)] = &[
    ("Microsoft.BingWeather", "MSN Weather"),
    ("Microsoft.BingNews", "MSN News"),
    ("Microsoft.GetHelp", "Get Help"),
    ("Microsoft.Getstarted", "Tips"),
    ("Microsoft.Messaging", "Messaging"),
    ("Microsoft.Microsoft3DViewer", "3D Viewer"),
    ("Microsoft.MicrosoftOfficeHub", "Office Hub"),
    (
        "Microsoft.MicrosoftSolitaireCollection",
        "Solitaire Collection",
    ),
    ("Microsoft.MixedReality.Portal", "Mixed Reality Portal"),
    ("Microsoft.Office.OneNote", "OneNote"),
    ("Microsoft.People", "People"),
    ("Microsoft.SkypeApp", "Skype"),
    ("Microsoft.Wallet", "Wallet"),
    ("Microsoft.WindowsAlarms", "Alarms & Clock"),
    ("Microsoft.WindowsCamera", "Camera"),
    ("Microsoft.WindowsCommunicationsApps", "Mail & Calendar"),
    ("Microsoft.WindowsFeedbackHub", "Feedback Hub"),
    ("Microsoft.WindowsMaps", "Maps"),
    ("Microsoft.WindowsSoundRecorder", "Voice Recorder"),
    ("Microsoft.YourPhone", "Phone Link"),
    ("Microsoft.ZuneMusic", "Groove Music"),
    ("Microsoft.ZuneVideo", "Movies & TV"),
    ("Microsoft.MicrosoftPowerBIDesktopForWindows", "Power BI"),
    ("Microsoft.Todos", "Microsoft To Do"),
    ("SpotifyAB.SpotifyMusic", "Spotify"),
    ("Disney.37853FC22B2CE", "Disney+"),
    ("Microsoft.Xbox.TCUI", "Xbox TCUI"),
    ("Microsoft.XboxGameOverlay", "Xbox Game Overlay"),
    ("Microsoft.XboxGamingOverlay", "Xbox Gaming Overlay"),
    ("Microsoft.XboxSpeechToTextOverlay", "Xbox Speech To Text"),
];

// -- Input validation -----------------------------------------

/// Validate a package family name against registry path injection.
fn validate_family_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 256 {
        return Err("Invalid package family name length".into());
    }
    // AppX family names: alphanumeric, dots, underscores, hyphens
    // e.g. "Microsoft.BingWeather_8wekyb3d8bbwe"
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Err(format!(
            "Invalid characters in package family name: {}",
            name
        ));
    }
    if name.contains("..") {
        return Err("Package family name contains path traversal".into());
    }
    Ok(())
}

// -- Public types ---------------------------------------------

/// A package that can be de-provisioned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppxPackage {
    /// Package Family Name (e.g. "Microsoft.BingWeather_8wekyb3d8bbwe").
    pub family_name: String,
    /// Human-readable display name.
    pub display_name: String,
    /// Is this package protected (cannot be removed)?
    pub is_protected: bool,
    /// Is it already registered for de-provisioning?
    pub is_marked_for_removal: bool,
    /// Full package identity captured from the installed package inventory.
    #[serde(default)]
    pub installed_package_full_name: Option<String>,
    /// Provisioned package identity captured from the online image inventory.
    #[serde(default)]
    pub provisioned_package_name: Option<String>,
    /// Conservative user-facing removal classification.
    #[serde(default = "default_package_risk")]
    pub risk: String,
}

fn default_package_risk() -> String {
    "advanced".into()
}

/// Result of a de-provisioning operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppxDeProvisionResult {
    pub packages_marked: Vec<String>,
    pub packages_skipped: Vec<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppxPolicySnapshot {
    pub enabled: bool,
    pub marked_packages: Vec<String>,
}

// -- Query currently marked packages --------------------------

/// List all packages currently registered for de-provisioning.
pub fn list_marked_packages() -> Result<Vec<String>, String> {
    let mut result = Vec::new();
    unsafe {
        let mut hkey: HKEY = std::mem::zeroed();
        let open = windows::Win32::System::Registry::RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(APPX_POLICY_ROOT),
            Some(0),
            KEY_READ,
            &mut hkey,
        );
        if open == ERROR_FILE_NOT_FOUND {
            return Ok(result);
        }
        if open.is_err() {
            return Err(format!("Failed to read AppX policy packages: {open:?}"));
        }

        let mut index: u32 = 0;
        let mut name_buf = [0u16; 256];
        let mut name_len = name_buf.len() as u32;

        while RegEnumKeyExW(
            hkey,
            index,
            Some(windows::core::PWSTR(name_buf.as_mut_ptr())),
            &mut name_len,
            None,
            None,
            None,
            None,
        )
        .is_ok()
        {
            let name = String::from_utf16_lossy(&name_buf[..name_len as usize]);
            result.push(name);
            index += 1;
            name_len = name_buf.len() as u32;
        }

        let _ = RegCloseKey(hkey);
    }
    Ok(result)
}

/// Check if the root policy key is enabled.
pub fn is_policy_enabled() -> bool {
    policy_snapshot()
        .map(|snapshot| snapshot.enabled)
        .unwrap_or(false)
}

pub fn policy_snapshot() -> Result<AppxPolicySnapshot, String> {
    let mut marked_packages = list_marked_packages()?;
    marked_packages.sort();
    unsafe {
        let mut hkey: HKEY = std::mem::zeroed();
        let open = windows::Win32::System::Registry::RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(APPX_POLICY_ROOT),
            Some(0),
            KEY_READ,
            &mut hkey,
        );
        if open == ERROR_FILE_NOT_FOUND {
            return Ok(AppxPolicySnapshot {
                enabled: false,
                marked_packages,
            });
        }
        if open.is_err() {
            return Err(format!("Failed to read AppX policy state: {open:?}"));
        }

        let enabled = crate::core::registry_helpers::read_reg_dword(hkey, "Enabled").unwrap_or(0);
        let _ = RegCloseKey(hkey);
        Ok(AppxPolicySnapshot {
            enabled: enabled != 0,
            marked_packages,
        })
    }
}

// -- Get available packages -----------------------------------

/// Get the list of candidate bloatware packages with their protection status
/// and whether they are already marked for removal.
pub fn list_candidates() -> Vec<AppxPackage> {
    let marked = list_marked_packages().unwrap_or_default();
    let evidence = discover_package_evidence();

    let mut candidates = BLOATWARE_CANDIDATES
        .iter()
        .map(|(family_name, display_name)| {
            let package_evidence = evidence.get(*family_name);
            let resolved_family_name = package_evidence
                .and_then(|evidence| evidence.installed_family_name.clone())
                .or_else(|| {
                    package_evidence
                        .and_then(|evidence| evidence.provisioned_package_name.as_deref())
                        .and_then(provisioned_family_name)
                })
                .unwrap_or_else(|| family_name.to_string());
            let is_protected = is_protected_package(family_name);
            AppxPackage {
                family_name: resolved_family_name.clone(),
                display_name: display_name.to_string(),
                is_protected,
                is_marked_for_removal: marked.iter().any(|marked_name| {
                    marked_name == &resolved_family_name || marked_name == family_name
                }),
                installed_package_full_name: package_evidence
                    .and_then(|evidence| evidence.installed_package_full_name.clone()),
                provisioned_package_name: package_evidence
                    .and_then(|evidence| evidence.provisioned_package_name.clone()),
                risk: package_risk(family_name).into(),
            }
        })
        .collect::<Vec<_>>();

    // Keep policy entries visible so users can undo a prior selection even if
    // Windows has already processed the package on a later sign-in.
    for marked_name in marked {
        if !candidates
            .iter()
            .any(|candidate| candidate.family_name == marked_name)
        {
            let is_protected = is_protected_package(&marked_name);
            candidates.push(AppxPackage {
                display_name: marked_name.clone(),
                family_name: marked_name.clone(),
                is_protected,
                is_marked_for_removal: true,
                installed_package_full_name: None,
                provisioned_package_name: None,
                risk: package_risk(&marked_name).into(),
            });
        }
    }
    candidates
}

#[derive(Debug, Default)]
struct PackageEvidence {
    installed_family_name: Option<String>,
    installed_package_full_name: Option<String>,
    provisioned_package_name: Option<String>,
}

/// Read-only package discovery. A failed or unavailable query yields no
/// evidence, which makes the corresponding static candidate non-selectable.
fn discover_package_evidence() -> HashMap<String, PackageEvidence> {
    let script = r#"
$ErrorActionPreference = 'SilentlyContinue'
Get-AppxPackage | ForEach-Object { 'I|' + $_.Name + '|' + $_.PackageFullName + '|' + $_.PackageFamilyName }
Get-AppxProvisionedPackage -Online | ForEach-Object { 'P|' + (($_.PackageName -split '_')[0]) + '|' + $_.PackageName }
"#;
    let output = match crate::core::command_helpers::create_command("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return HashMap::new(),
    };

    let mut evidence: HashMap<String, PackageEvidence> = HashMap::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let fields = line.splitn(4, '|').collect::<Vec<_>>();
        match fields.as_slice() {
            ["I", package_name, full_name, family_name] => {
                evidence
                    .entry((*package_name).to_string())
                    .or_default()
                    .installed_family_name = Some((*family_name).to_string());
                evidence
                    .entry((*package_name).to_string())
                    .or_default()
                    .installed_package_full_name = Some((*full_name).to_string());
            }
            ["P", package_name, provisioned_name] => {
                evidence
                    .entry((*package_name).to_string())
                    .or_default()
                    .provisioned_package_name = Some((*provisioned_name).to_string());
            }
            _ => {}
        }
    }
    evidence
}

// -- Register packages for de-provisioning --------------------

/// Register one or more package family names for de-provisioning.
///
/// This writes the `RemovedPackage = 1` value under each package subkey
/// and sets `Enabled = 1` on the root policy key. Windows processes
/// these on next login/profile creation.
pub fn mark_packages_for_removal(family_names: &[String]) -> Result<AppxDeProvisionResult, String> {
    // Security: require elevation for HKLM writes
    if !crate::core::elevation::is_elevated() {
        return Err("Elevation required for AppX de-provisioning".into());
    }

    let mut marked = Vec::new();
    let mut skipped = Vec::new();

    unsafe {
        // Ensure the root key exists
        let mut root_key: HKEY = std::mem::zeroed();
        RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(APPX_POLICY_ROOT),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut root_key,
            None,
        )
        .ok()
        .ok()
        .ok_or_else(|| "Failed to create AppX policy key".to_string())?;

        // Enable the policy
        let enabled: u32 = 1;
        let data = enabled.to_ne_bytes();
        let result = windows::Win32::System::Registry::RegSetValueExW(
            root_key,
            &HSTRING::from("Enabled"),
            None,
            REG_DWORD,
            Some(&data),
        );
        if result != windows::Win32::Foundation::ERROR_SUCCESS {
            let _ = RegCloseKey(root_key);
            return Err(format!("Failed to enable AppX policy: {:?}", result));
        }

        let _ = RegCloseKey(root_key);
    }

    for name in family_names {
        // Security: validate family name against path injection
        if let Err(e) = validate_family_name(name) {
            skipped.push(format!("{} ({})", name, e));
            continue;
        }

        // Check protection
        if is_protected_package(name) {
            skipped.push(format!("{} (protected)", name));
            continue;
        }

        unsafe {
            let full_path = format!("{}\\{}", APPX_POLICY_ROOT, name);
            let mut pkg_key: HKEY = std::mem::zeroed();
            if RegCreateKeyExW(
                HKEY_LOCAL_MACHINE,
                &HSTRING::from(&full_path),
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &mut pkg_key,
                None,
            )
            .is_err()
            {
                skipped.push(format!("{} (failed to create key)", name));
                continue;
            }

            let removed: u32 = 1;
            let data = removed.to_ne_bytes();
            let result = windows::Win32::System::Registry::RegSetValueExW(
                pkg_key,
                &HSTRING::from("RemovedPackage"),
                None,
                REG_DWORD,
                Some(&data),
            );
            let _ = RegCloseKey(pkg_key);
            if result == windows::Win32::Foundation::ERROR_SUCCESS {
                marked.push(name.clone());
            } else {
                skipped.push(format!("{} (failed to mark package: {:?})", name, result));
            }
        }
    }

    Ok(AppxDeProvisionResult {
        packages_marked: marked,
        packages_skipped: skipped,
        enabled: true,
    })
}

/// Remove a package from the de-provisioning list.
pub fn unmark_package(family_name: &str) -> Result<(), String> {
    validate_family_name(family_name)?;
    let full_path = format!("{}\\{}", APPX_POLICY_ROOT, family_name);
    unsafe {
        let mut pkg_key: HKEY = std::mem::zeroed();
        let open = windows::Win32::System::Registry::RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(&full_path),
            Some(0),
            KEY_SET_VALUE,
            &mut pkg_key,
        );
        if open != windows::Win32::Foundation::ERROR_SUCCESS {
            if open == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND {
                return Ok(());
            }
            return Err(format!(
                "Failed to open AppX package {}: {:?}",
                family_name, open
            ));
        }

        // Delete the RemovedPackage value
        let value_result = RegDeleteValueW(pkg_key, &HSTRING::from("RemovedPackage"));
        let _ = RegCloseKey(pkg_key);
        if value_result != windows::Win32::Foundation::ERROR_SUCCESS
            && value_result != windows::Win32::Foundation::ERROR_FILE_NOT_FOUND
        {
            return Err(format!(
                "Failed to unmark package {}: {:?}",
                family_name, value_result
            ));
        }

        // Delete the package subkey
        let key_result =
            RegDeleteKeyExW(HKEY_LOCAL_MACHINE, &HSTRING::from(&full_path), 0, Some(0));
        if key_result != windows::Win32::Foundation::ERROR_SUCCESS
            && key_result != windows::Win32::Foundation::ERROR_FILE_NOT_FOUND
        {
            return Err(format!(
                "Failed to remove AppX policy key {}: {:?}",
                family_name, key_result
            ));
        }
    }
    Ok(())
}

/// Disable the entire de-provisioning policy (but keep the package list).
pub fn disable_policy() -> Result<(), String> {
    unsafe {
        let mut root_key: HKEY = std::mem::zeroed();
        let open = windows::Win32::System::Registry::RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(APPX_POLICY_ROOT),
            Some(0),
            KEY_SET_VALUE,
            &mut root_key,
        );
        if open != windows::Win32::Foundation::ERROR_SUCCESS {
            if open == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND {
                return Ok(()); // Key doesn't exist - nothing to disable
            }
            return Err(format!("Failed to open AppX policy: {:?}", open));
        }

        let disabled: u32 = 0;
        let data = disabled.to_ne_bytes();
        let result = windows::Win32::System::Registry::RegSetValueExW(
            root_key,
            &HSTRING::from("Enabled"),
            None,
            REG_DWORD,
            Some(&data),
        );
        let _ = RegCloseKey(root_key);
        if result != windows::Win32::Foundation::ERROR_SUCCESS {
            return Err(format!("Failed to disable AppX policy: {:?}", result));
        }
    }
    Ok(())
}

/// Restore an AppX policy snapshot without recording another undo entry.
pub fn restore_policy(enabled: bool, desired_packages: &[String]) -> Result<(), String> {
    for package in desired_packages {
        validate_family_name(package)?;
    }

    let current_packages = list_marked_packages()?;
    for package in current_packages {
        if !desired_packages.iter().any(|desired| desired == &package) {
            unmark_package(&package)?;
        }
    }

    let current_packages = list_marked_packages()?;
    let missing: Vec<String> = desired_packages
        .iter()
        .filter(|desired| !current_packages.iter().any(|current| current == *desired))
        .cloned()
        .collect();
    if !missing.is_empty() || enabled {
        mark_packages_for_removal(&missing)?;
    }
    if !enabled {
        disable_policy()?;
    }
    Ok(())
}

fn provisioned_family_name(full_name: &str) -> Option<String> {
    let parts = full_name.split('_').collect::<Vec<_>>();
    if parts.len() < 2 || parts[0].is_empty() || parts.last().is_some_and(|part| part.is_empty()) {
        return None;
    }

    Some(format!("{}_{}", parts[0], parts.last()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_packages_not_empty() {
        assert!(!PROTECTED_PACKAGES.is_empty());
    }

    #[test]
    fn bloatware_candidates_are_defined() {
        assert!(!BLOATWARE_CANDIDATES.is_empty());
    }

    #[test]
    fn store_is_protected() {
        assert!(PROTECTED_PACKAGES.contains(&"Microsoft.WindowsStore"));
    }

    #[test]
    fn gaming_services_are_protected() {
        assert!(PROTECTED_PACKAGES.contains(&"Microsoft.GamingServices"));
        assert!(PROTECTED_PACKAGES.contains(&"Microsoft.XboxIdentityProvider"));
    }

    #[test]
    fn windows_security_is_protected() {
        assert!(PROTECTED_PACKAGES.contains(&"Microsoft.SecHealthUI"));
        assert!(PROTECTED_PACKAGES.contains(&"Microsoft.Windows.SecHealthUI"));
        assert!(is_protected_package("microsoft.sechealthui_8wekyb3d8bbwe"));
    }

    #[test]
    fn candidates_excludes_protected() {
        let candidates = list_candidates();
        for c in &candidates {
            let should_be_protected = is_protected_package(&c.family_name);
            // If it IS a protected package, is_protected should be true
            if should_be_protected {
                assert!(
                    c.is_protected,
                    "Package {} matches protected prefix but is_protected is false",
                    c.family_name
                );
            }
        }
    }

    #[test]
    fn validate_family_name_accepts_valid() {
        assert!(validate_family_name("Microsoft.BingWeather_8wekyb3d8bbwe").is_ok());
        assert!(validate_family_name("SpotifyAB.SpotifyMusic").is_ok());
    }

    #[test]
    fn validate_family_name_rejects_empty() {
        assert!(validate_family_name("").is_err());
    }

    #[test]
    fn validate_family_name_rejects_path_separator() {
        assert!(validate_family_name("Microsoft.Foo\\..\\Bar").is_err());
        assert!(validate_family_name("Microsoft.Foo/Bar").is_err());
    }

    #[test]
    fn validate_family_name_rejects_traversal() {
        assert!(validate_family_name("Microsoft.Foo..Bar").is_err());
    }

    #[test]
    fn provisioned_identity_derives_package_family_name() {
        assert_eq!(
            provisioned_family_name("Microsoft.BingWeather_4.53.0.0_neutral_8wekyb3d8bbwe"),
            Some("Microsoft.BingWeather_8wekyb3d8bbwe".into())
        );
        assert_eq!(provisioned_family_name("invalid"), None);
    }

    #[test]
    fn validate_family_name_rejects_too_long() {
        let long_name = "A".repeat(257);
        assert!(validate_family_name(&long_name).is_err());
    }
}
