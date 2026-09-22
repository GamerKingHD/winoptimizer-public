//! Context menu inventory for Windows Explorer right-click entries.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextMenuEntry {
    pub name: String,
    pub command: String,
    pub location: String,
    pub enabled: bool,
}

/// List custom context menu entries.
pub fn list() -> Vec<ContextMenuEntry> {
    use windows::core::*;
    use windows::Win32::System::Registry::*;

    let mut entries = Vec::new();
    let paths = [
        (r"Software\Classes\*\shell", "File"),
        (r"Software\Classes\Directory\shell", "Directory"),
        (r"Software\Classes\Directory\Background\shell", "Desktop"),
        (r"Software\Classes\Drive\shell", "Drive"),
    ];

    for &(key_path, location) in &paths {
        unsafe {
            let mut key = std::mem::zeroed();
            if RegOpenKeyExW(
                HKEY_CURRENT_USER,
                &HSTRING::from(key_path),
                Some(0),
                KEY_READ,
                &mut key,
            )
            .is_ok()
            {
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
                    let shell_name = String::from_utf16_lossy(&name_buf[..name_len as usize]);
                    entries.push(ContextMenuEntry {
                        name: shell_name,
                        command: String::new(),
                        location: location.to_string(),
                        enabled: true,
                    });
                    index += 1;
                    name_len = name_buf.len() as u32;
                }
                let _ = RegCloseKey(key);
            }
        }
    }

    entries
}

/// Validate a context menu entry name against registry path injection.
#[cfg(test)]
fn validate_context_menu_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 256 {
        return Err("Invalid context menu entry name length".into());
    }
    if name.contains('\\')
        || name.contains('/')
        || name.contains('\0')
        || name.contains("..")
        || name.chars().any(|c| c.is_control())
    {
        return Err("Context menu name contains invalid characters".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_name_accepts_normal() {
        assert!(validate_context_menu_name("Open with Notepad").is_ok());
        assert!(validate_context_menu_name("Git Bash").is_ok());
    }

    #[test]
    fn validate_name_rejects_empty() {
        assert!(validate_context_menu_name("").is_err());
    }

    #[test]
    fn validate_name_rejects_backslash() {
        assert!(validate_context_menu_name("Foo\\..\\Bar").is_err());
    }

    #[test]
    fn validate_name_rejects_forward_slash() {
        assert!(validate_context_menu_name("Foo/Bar").is_err());
    }

    #[test]
    fn validate_name_rejects_traversal() {
        assert!(validate_context_menu_name("..\\..\\Windows").is_err());
    }

    #[test]
    fn validate_name_rejects_null_byte() {
        assert!(validate_context_menu_name("Foo\0Bar").is_err());
    }

    #[test]
    fn validate_name_rejects_control_chars() {
        assert!(validate_context_menu_name("Foo\nBar").is_err());
        assert!(validate_context_menu_name("Foo\rBar").is_err());
    }

    #[test]
    fn validate_name_rejects_too_long() {
        let long_name = "A".repeat(257);
        assert!(validate_context_menu_name(&long_name).is_err());
    }
}
