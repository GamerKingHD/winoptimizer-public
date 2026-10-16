//! Shared registry primitives for the privacy, performance and customization
//! tweak modules.
//!
//! These three modules used to carry private copies of the same wrappers. The
//! copies agreed everywhere except one rule: whether deleting a value under a
//! key that does not exist is an error. That rule is now an explicit
//! [`MissingKey`] argument so each module keeps its historical behavior.
//!
//! Only `HKLM` and `HKCU` are accepted. Everything goes through
//! `core::registry_helpers`.

use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

use crate::core::registry_helpers as helpers;
use crate::core::system_tweak_operation::TweakPhysicalResource;

/// How deleting a value behaves when its parent key cannot be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MissingKey {
    /// A missing parent key is an error.
    Error,
    /// A missing parent key means there is nothing to delete.
    Ignore,
}

/// Split `HIVE\path\to\key` into the hive name and the sub-path.
pub(super) fn split_hive(full_key: &str) -> (&str, String) {
    full_key
        .split_once('\\')
        .map_or((full_key, String::new()), |(hive, path)| {
            (hive, path.to_string())
        })
}

fn resolve(full_key: &str) -> Result<(HKEY, String), String> {
    let (hive, path) = split_hive(full_key);
    let handle = match hive {
        "HKLM" => HKEY_LOCAL_MACHINE,
        "HKCU" => HKEY_CURRENT_USER,
        _ => return Err(format!("Unknown hive: {hive}")),
    };
    Ok((handle, path))
}

pub(super) fn set_dword(full_key: &str, value_name: &str, data: u32) -> Result<(), String> {
    let (hive, path) = resolve(full_key)?;
    unsafe {
        let key = helpers::create_key_write(hive, &path)
            .ok_or_else(|| format!("Failed to create/open key {full_key}"))?;
        let result = helpers::set_reg_dword(key, value_name, data);
        helpers::close_key(key);
        result.map_err(|error| {
            format!("Failed to set {value_name} = {data} at {full_key}: {error:?}")
        })
    }
}

pub(super) fn set_raw(
    full_key: &str,
    value_name: &str,
    registry_type: u32,
    raw_value: &[u8],
) -> Result<(), String> {
    let (hive, path) = resolve(full_key)?;
    unsafe {
        let key = helpers::create_key_write(hive, &path)
            .ok_or_else(|| format!("Failed to create/open key {full_key}"))?;
        let result = helpers::set_reg_raw(key, value_name, registry_type, raw_value);
        helpers::close_key(key);
        result.map_err(|error| format!("Failed to restore {value_name} at {full_key}: {error:?}"))
    }
}

pub(super) fn delete_value(
    full_key: &str,
    value_name: &str,
    missing_key: MissingKey,
) -> Result<(), String> {
    let (hive, path) = resolve(full_key)?;
    unsafe {
        let Some(key) = helpers::open_key_write(hive, &path) else {
            return match missing_key {
                MissingKey::Ignore => Ok(()),
                MissingKey::Error => Err(format!("Failed to open key {full_key} for deletion")),
            };
        };
        let result = helpers::delete_value(key, value_name);
        helpers::close_key(key);
        result.map_err(|error| format!("Failed to delete {value_name} at {full_key}: {error:?}"))
    }
}

/// Delete `full_key` when it exists. The underlying call removes only keys
/// without subkeys, so a key that gained children since capture is reported as
/// an error instead of being wiped.
pub(super) fn delete_key(full_key: &str) -> Result<(), String> {
    let (hive, path) = resolve(full_key)?;
    unsafe {
        if let Some(key) = helpers::open_key_read(hive, &path) {
            helpers::close_key(key);
            helpers::delete_key_recursive(hive, &path).map_err(|error| {
                format!("Failed to delete empty registry key {full_key}: {error}")
            })?;
        }
    }
    Ok(())
}

fn absent_resource(full_key: &str, value_name: &str) -> TweakPhysicalResource {
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

/// Record the physical state of one registry value. An unknown hive or a key
/// that cannot be opened is captured as absent.
pub(super) fn capture_resource(full_key: &str, value_name: &str) -> TweakPhysicalResource {
    let Ok((hive, path)) = resolve(full_key) else {
        return absent_resource(full_key, value_name);
    };
    unsafe {
        let Some(key) = helpers::open_key_read(hive, &path) else {
            return absent_resource(full_key, value_name);
        };
        let value = helpers::read_reg_dword(key, value_name);
        let raw = helpers::read_reg_raw(key, value_name);
        helpers::close_key(key);
        TweakPhysicalResource::RegistryDword {
            full_key: full_key.into(),
            value_name: value_name.into(),
            key_existed: true,
            value_existed: raw.is_some(),
            value,
            registry_type: raw.as_ref().map(|(registry_type, _)| *registry_type),
            raw_value: raw.map(|(_, raw_value)| raw_value),
        }
    }
}

/// Put a value back exactly as [`capture_resource`] recorded it.
pub(super) fn restore_resource(
    resource: &TweakPhysicalResource,
    missing_key: MissingKey,
) -> Result<(), String> {
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
            delete_value(full_key, value_name, missing_key)?;
        } else {
            delete_key(full_key)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRATCH_ROOT: &str = "HKCU\\Software\\WinOptimizerTests";
    const REG_SZ: u32 = 1;
    const REG_DWORD: u32 = 4;

    /// A uniquely named key under a private HKCU root. Tests never touch any
    /// other registry location, and the key is removed when the guard drops.
    struct Scratch {
        key: String,
    }

    impl Scratch {
        fn new(label: &str) -> Self {
            let key = format!("{SCRATCH_ROOT}\\{label}-{}", uuid::Uuid::new_v4().simple());
            assert!(key.starts_with(SCRATCH_ROOT));
            Self { key }
        }

        fn key(&self) -> &str {
            &self.key
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = delete_key(&self.key);
            let _ = delete_key(SCRATCH_ROOT);
        }
    }

    struct Snapshot {
        key_existed: bool,
        value_existed: bool,
        value: Option<u32>,
        registry_type: Option<u32>,
        raw_value: Option<Vec<u8>>,
    }

    fn snapshot(key: &str, value_name: &str) -> Snapshot {
        match capture_resource(key, value_name) {
            TweakPhysicalResource::RegistryDword {
                key_existed,
                value_existed,
                value,
                registry_type,
                raw_value,
                ..
            } => Snapshot {
                key_existed,
                value_existed,
                value,
                registry_type,
                raw_value,
            },
            other => panic!("unexpected resource: {other:?}"),
        }
    }

    #[test]
    fn split_hive_separates_hive_from_path() {
        assert_eq!(
            split_hive("HKLM\\SOFTWARE\\Policies\\X"),
            ("HKLM", "SOFTWARE\\Policies\\X".to_string())
        );
        assert_eq!(split_hive("HKCU"), ("HKCU", String::new()));
        assert_eq!(split_hive(""), ("", String::new()));
        assert_eq!(split_hive("HKCU\\"), ("HKCU", String::new()));
    }

    #[test]
    fn unknown_hive_is_an_error_and_never_a_panic() {
        assert!(set_dword("HKXX\\a", "v", 1).is_err());
        assert!(set_raw("HKXX\\a", "v", REG_DWORD, &[1, 0, 0, 0]).is_err());
        assert!(delete_value("HKXX\\a", "v", MissingKey::Ignore).is_err());
        assert!(delete_key("HKXX\\a").is_err());
        let snap = snapshot("HKXX\\a", "v");
        assert!(!snap.key_existed && !snap.value_existed);
    }

    #[test]
    fn capture_records_value_type_and_bytes() {
        let scratch = Scratch::new("capture");
        let absent = snapshot(scratch.key(), "Flag");
        assert!(!absent.key_existed && !absent.value_existed);

        set_dword(scratch.key(), "Flag", 5).unwrap();
        let present = snapshot(scratch.key(), "Flag");
        assert!(present.key_existed && present.value_existed);
        assert_eq!(present.value, Some(5));
        assert_eq!(present.registry_type, Some(REG_DWORD));
        assert_eq!(present.raw_value, Some(5u32.to_ne_bytes().to_vec()));
    }

    #[test]
    fn restore_removes_a_key_that_did_not_exist_before() {
        let scratch = Scratch::new("absent-key");
        let before = capture_resource(scratch.key(), "Flag");

        set_dword(scratch.key(), "Flag", 1).unwrap();
        assert!(snapshot(scratch.key(), "Flag").value_existed);

        restore_resource(&before, MissingKey::Error).unwrap();
        let after = snapshot(scratch.key(), "Flag");
        assert!(!after.key_existed && !after.value_existed);
    }

    #[test]
    fn restore_resets_a_preexisting_value_and_keeps_its_key() {
        let scratch = Scratch::new("preexisting");
        set_dword(scratch.key(), "Flag", 7).unwrap();
        let before = capture_resource(scratch.key(), "Flag");

        set_dword(scratch.key(), "Flag", 9).unwrap();
        assert_eq!(snapshot(scratch.key(), "Flag").value, Some(9));

        restore_resource(&before, MissingKey::Error).unwrap();
        let after = snapshot(scratch.key(), "Flag");
        assert!(after.key_existed);
        assert_eq!(after.value, Some(7));
    }

    #[test]
    fn restore_deletes_a_new_value_but_keeps_an_existing_key() {
        let scratch = Scratch::new("new-value");
        set_dword(scratch.key(), "Other", 1).unwrap();
        let before = capture_resource(scratch.key(), "Flag");

        set_dword(scratch.key(), "Flag", 3).unwrap();
        restore_resource(&before, MissingKey::Error).unwrap();

        let after = snapshot(scratch.key(), "Flag");
        assert!(after.key_existed, "the key held other values and must stay");
        assert!(!after.value_existed);
        assert_eq!(snapshot(scratch.key(), "Other").value, Some(1));
    }

    #[test]
    fn restore_preserves_a_non_dword_value_type_and_bytes() {
        let scratch = Scratch::new("raw-type");
        let text: Vec<u8> = "abc"
            .encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        set_raw(scratch.key(), "Name", REG_SZ, &text).unwrap();
        let before = capture_resource(scratch.key(), "Name");

        set_dword(scratch.key(), "Name", 42).unwrap();
        assert_eq!(
            snapshot(scratch.key(), "Name").registry_type,
            Some(REG_DWORD)
        );

        restore_resource(&before, MissingKey::Error).unwrap();
        let after = snapshot(scratch.key(), "Name");
        assert_eq!(after.registry_type, Some(REG_SZ));
        assert_eq!(after.raw_value, Some(text));
    }

    #[test]
    fn delete_value_under_a_missing_key_follows_the_requested_mode() {
        let scratch = Scratch::new("missing-key");
        assert!(delete_value(scratch.key(), "Flag", MissingKey::Ignore).is_ok());
        assert!(delete_value(scratch.key(), "Flag", MissingKey::Error).is_err());
    }

    #[test]
    fn deleting_a_missing_value_is_an_error_in_both_modes() {
        let scratch = Scratch::new("missing-value");
        set_dword(scratch.key(), "Other", 1).unwrap();
        assert!(delete_value(scratch.key(), "Flag", MissingKey::Ignore).is_err());
        assert!(delete_value(scratch.key(), "Flag", MissingKey::Error).is_err());
    }

    #[test]
    fn delete_key_refuses_to_remove_a_key_that_gained_subkeys() {
        let scratch = Scratch::new("has-children");
        let child = format!("{}\\child", scratch.key());
        set_dword(&child, "Flag", 1).unwrap();

        assert!(delete_key(scratch.key()).is_err());
        assert!(snapshot(&child, "Flag").value_existed);

        delete_key(&child).unwrap();
    }

    #[test]
    fn restore_rejects_a_resource_that_is_not_a_registry_value() {
        let service = TweakPhysicalResource::Service {
            name: "Spooler".into(),
            existed: true,
            start_type: None,
            running: None,
            delayed_auto_start: None,
        };
        assert!(restore_resource(&service, MissingKey::Error).is_err());
    }
}
