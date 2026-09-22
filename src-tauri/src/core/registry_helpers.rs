//! Shared registry helper wrappers for windows-rs 0.60 API.
//! All other modules should use these instead of calling the raw Windows API.

use windows::core::*;
use windows::Win32::Foundation::ERROR_MORE_DATA;
use windows::Win32::System::Registry::*;

/// Open a registry key with read access. Returns None if the key doesn't exist.
///
/// # Safety
/// The caller must pass a valid registry hive handle and use the returned handle
/// only until it is closed.
pub unsafe fn open_key_read(hive: HKEY, subkey: &str) -> Option<HKEY> {
    let mut key: HKEY = std::mem::zeroed();
    RegOpenKeyExW(hive, &HSTRING::from(subkey), Some(0), KEY_READ, &mut key)
        .ok()
        .ok()?;
    Some(key)
}

/// Open a registry key with write access. Returns None if the key doesn't exist.
///
/// # Safety
/// The caller must pass a valid registry hive handle and close the returned handle.
pub unsafe fn open_key_write(hive: HKEY, subkey: &str) -> Option<HKEY> {
    let mut key: HKEY = std::mem::zeroed();
    RegOpenKeyExW(
        hive,
        &HSTRING::from(subkey),
        Some(0),
        KEY_SET_VALUE,
        &mut key,
    )
    .ok()
    .ok()?;
    Some(key)
}

/// Create or open a registry key with write access.
///
/// # Safety
/// The caller must pass a valid registry hive handle and close the returned handle.
pub unsafe fn create_key_write(hive: HKEY, subkey: &str) -> Option<HKEY> {
    let mut key: HKEY = std::mem::zeroed();
    RegCreateKeyExW(
        hive,
        &HSTRING::from(subkey),
        None,
        None,
        REG_OPTION_NON_VOLATILE,
        KEY_SET_VALUE,
        None,
        &mut key,
        None,
    )
    .ok()
    .ok()?;
    Some(key)
}

/// Read a REG_SZ string value from an open key.
///
/// # Safety
/// `key` must be a valid open registry key handle.
pub unsafe fn read_reg_string(key: HKEY, value_name: &str) -> Option<String> {
    let value_name = HSTRING::from(value_name);
    let mut buf_size = 0u32;
    let mut value_type = REG_VALUE_TYPE::default();

    RegQueryValueExW(
        key,
        &value_name,
        None,
        Some(&mut value_type as *mut REG_VALUE_TYPE),
        None,
        Some(&mut buf_size),
    )
    .ok()
    .ok()?;

    if value_type != REG_SZ && value_type != REG_EXPAND_SZ {
        return None;
    }

    let mut bytes = vec![0u8; buf_size as usize];
    RegQueryValueExW(
        key,
        &value_name,
        None,
        Some(&mut value_type as *mut REG_VALUE_TYPE),
        Some(bytes.as_mut_ptr()),
        Some(&mut buf_size),
    )
    .ok()
    .ok()?;
    bytes.truncate(buf_size as usize);
    let mut utf16 = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    if utf16.last() == Some(&0) {
        utf16.pop();
    }
    String::from_utf16(&utf16).ok()
}

/// Read a REG_SZ string from a path like "HKLM\Some\Key".
///
/// # Safety
/// `hive` must be a valid registry hive handle.
pub unsafe fn read_reg_string_path(hive: HKEY, subkey: &str, value_name: &str) -> Option<String> {
    let key = open_key_read(hive, subkey)?;
    let result = read_reg_string(key, value_name);
    let _ = RegCloseKey(key);
    result
}

/// Write a REG_DWORD value.
///
/// # Safety
/// `key` must be a valid open registry key handle with write access.
pub unsafe fn set_reg_dword(
    key: HKEY,
    value_name: &str,
    data: u32,
) -> std::result::Result<(), windows::core::Error> {
    let data_bytes = data.to_ne_bytes();
    RegSetValueExW(
        key,
        &HSTRING::from(value_name),
        None,
        REG_DWORD,
        Some(&data_bytes),
    )
    .ok()?;
    Ok(())
}

/// Write a REG_SZ value.
///
/// # Safety
/// `key` must be a valid open registry key handle with write access.
pub unsafe fn set_reg_string(
    key: HKEY,
    value_name: &str,
    data: &str,
) -> std::result::Result<(), windows::core::Error> {
    set_reg_string_with_type(key, value_name, data, REG_SZ)
}

/// Write a string value with a specific registry type (REG_SZ or REG_EXPAND_SZ).
///
/// # Safety
/// `key` must be a valid open registry key handle with write access.
pub unsafe fn set_reg_string_with_type(
    key: HKEY,
    value_name: &str,
    data: &str,
    reg_type: REG_VALUE_TYPE,
) -> std::result::Result<(), windows::core::Error> {
    let data_bytes: Vec<u8> = data
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(|c| c.to_le_bytes())
        .collect();
    RegSetValueExW(
        key,
        &HSTRING::from(value_name),
        None,
        reg_type,
        Some(&data_bytes),
    )
    .ok()?;
    Ok(())
}

/// Delete a registry value.
///
/// # Safety
/// `key` must be a valid open registry key handle with write access.
pub unsafe fn delete_value(
    key: HKEY,
    value_name: &str,
) -> std::result::Result<(), windows::core::Error> {
    RegDeleteValueW(key, &HSTRING::from(value_name)).ok()?;
    Ok(())
}

/// Enumerate subkeys of an open key. Calls `f` with the name of each subkey.
///
/// # Safety
/// `key` must be a valid open registry key handle.
pub unsafe fn enum_subkeys(key: HKEY, mut f: impl FnMut(String)) {
    let mut index: u32 = 0;
    let mut name_buf = vec![0u16; 1024];

    loop {
        let mut name_len = name_buf.len() as u32;
        let status = RegEnumKeyExW(
            key,
            index,
            Some(PWSTR(name_buf.as_mut_ptr())),
            &mut name_len,
            None,
            None,
            None,
            None,
        );
        if status.is_ok() {
            f(String::from_utf16_lossy(&name_buf[..name_len as usize]));
            index += 1;
        } else if status == ERROR_MORE_DATA {
            name_buf.resize((name_len as usize).saturating_add(1), 0);
        } else {
            break;
        }
    }
}

/// Enumerate values of an open key. Calls `f` with (name, type).
///
/// # Safety
/// `key` must be a valid open registry key handle.
pub unsafe fn enum_values(key: HKEY, mut f: impl FnMut(String, u32)) {
    let mut index: u32 = 0;
    let mut name_buf = vec![0u16; 4096];
    let mut value_buf = vec![0u8; 16384 * 2];
    let mut value_type: u32 = 0;

    loop {
        let mut name_len = name_buf.len() as u32;
        let mut value_len = value_buf.len() as u32;
        let status = RegEnumValueW(
            key,
            index,
            Some(PWSTR(name_buf.as_mut_ptr())),
            &mut name_len,
            None,
            Some(&mut value_type),
            Some(value_buf.as_mut_ptr()),
            Some(&mut value_len),
        );
        if status.is_ok() {
            f(
                String::from_utf16_lossy(&name_buf[..name_len as usize]),
                value_type,
            );
            index += 1;
        } else if status == ERROR_MORE_DATA {
            name_buf.resize((name_len as usize).saturating_add(1), 0);
            value_buf.resize(value_len as usize, 0);
        } else {
            break;
        }
    }
}

/// Read a REG_DWORD value from an open key.
///
/// # Safety
/// `key` must be a valid open registry key handle.
pub unsafe fn read_reg_dword(key: HKEY, value_name: &str) -> Option<u32> {
    let mut buf = [0u8; 4];
    let mut buf_size = buf.len() as u32;
    let mut value_type = REG_VALUE_TYPE::default();

    RegQueryValueExW(
        key,
        &HSTRING::from(value_name),
        None,
        Some(&mut value_type as *mut REG_VALUE_TYPE),
        Some(buf.as_mut_ptr()),
        Some(&mut buf_size),
    )
    .ok()
    .ok()?;

    if value_type != REG_DWORD || buf_size != std::mem::size_of::<u32>() as u32 {
        return None;
    }

    Some(u32::from_ne_bytes(buf))
}

/// Read a registry value without discarding its type or raw bytes.
///
/// # Safety
/// `key` must be a valid open registry key handle.
pub unsafe fn read_reg_raw(key: HKEY, value_name: &str) -> Option<(u32, Vec<u8>)> {
    let mut value_type = REG_VALUE_TYPE::default();
    let mut buf_size = 0u32;
    RegQueryValueExW(
        key,
        &HSTRING::from(value_name),
        None,
        Some(&mut value_type as *mut REG_VALUE_TYPE),
        None,
        Some(&mut buf_size),
    )
    .ok()
    .ok()?;
    let mut bytes = vec![0u8; buf_size as usize];
    RegQueryValueExW(
        key,
        &HSTRING::from(value_name),
        None,
        Some(&mut value_type as *mut REG_VALUE_TYPE),
        Some(bytes.as_mut_ptr()),
        Some(&mut buf_size),
    )
    .ok()
    .ok()?;
    bytes.truncate(buf_size as usize);
    Some((value_type.0, bytes))
}

/// Restore registry bytes with their original registry type.
///
/// # Safety
/// `key` must be a valid open registry key handle with write access.
pub unsafe fn set_reg_raw(
    key: HKEY,
    value_name: &str,
    registry_type: u32,
    raw_value: &[u8],
) -> std::result::Result<(), windows::core::Error> {
    RegSetValueExW(
        key,
        &HSTRING::from(value_name),
        None,
        REG_VALUE_TYPE(registry_type),
        Some(raw_value),
    )
    .ok()?;
    Ok(())
}

/// Close a registry key.
///
/// # Safety
/// `key` must be a valid registry key handle or the default null handle.
pub unsafe fn close_key(key: HKEY) {
    let _ = RegCloseKey(key);
}

/// Delete a registry key (including subkeys).
///
/// # Safety
/// `hive` must be a valid registry hive handle and `subkey` must identify the
/// intended key within that hive.
pub unsafe fn delete_key_recursive(
    hive: HKEY,
    subkey: &str,
) -> std::result::Result<(), windows::core::Error> {
    RegDeleteKeyExW(hive, &HSTRING::from(subkey), 0, Some(0)).ok()?;
    Ok(())
}

/// Convert an optional string from the registry to display-friendly form.
pub fn parse_hive_prefix(full_key: &str) -> std::result::Result<(HKEY, String), String> {
    let pos = full_key.find('\\').unwrap_or(full_key.len());
    let hive_str = &full_key[..pos];
    let path = if pos < full_key.len() {
        &full_key[pos + 1..]
    } else {
        ""
    };

    let hive = match hive_str {
        "HKLM" => HKEY_LOCAL_MACHINE,
        "HKCU" => HKEY_CURRENT_USER,
        "HKCR" => HKEY_CLASSES_ROOT,
        "HKU" => HKEY_USERS,
        _ => return Err(format!("Unknown hive: {}", hive_str)),
    };

    Ok((hive, path.to_string()))
}
