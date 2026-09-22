//! Hosts file editor - read, edit, and manage entries in
//! C:\Windows\System32\drivers\etc\hosts.
//!
//! Every save backs up the original file and records an undo entry.
//! Supports multiple hostnames per line (lossless round-trip).

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::IpAddr;
use std::path::Path;

/// Path to the Windows hosts file.
const HOSTS_PATH: &str = r"C:\Windows\System32\drivers\etc\hosts";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostEntry {
    pub id: usize, // line number in the parsed file
    pub ip: String,
    pub hostnames: Vec<String>, // all hostnames on this line
    pub comment: Option<String>,
    pub enabled: bool,
    #[serde(default)]
    pub raw_line: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostsSnapshot {
    pub bytes: Vec<u8>,
    pub digest: String,
    pub encoding: String,
    pub attributes: u32,
    pub creation_time: u64,
    pub last_access_time: u64,
    pub last_write_time: u64,
    /// The exact file DACL in SDDL form. Missing ACL evidence is not safe for
    /// a mutation plan and is rejected at the operation boundary.
    #[serde(default)]
    pub acl: Option<String>,
    pub entries: Vec<HostEntry>,
}

pub fn snapshot() -> Result<HostsSnapshot, String> {
    snapshot_at(Path::new(HOSTS_PATH))
}

fn snapshot_at(path: &Path) -> Result<HostsSnapshot, String> {
    use std::os::windows::fs::MetadataExt;

    let mut file = std::fs::File::open(path).map_err(|e| format!("Cannot read hosts file: {e}"))?;
    let metadata = file
        .metadata()
        .map_err(|e| format!("Cannot read hosts metadata: {e}"))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| format!("Cannot read hosts file: {e}"))?;
    let (content, encoding) = decode_hosts_bytes(&bytes)?;
    let entries = parse_entries(content.strip_prefix('\u{feff}').unwrap_or(content))?;
    Ok(HostsSnapshot {
        digest: blake3::hash(&bytes).to_hex().to_string(),
        bytes,
        encoding: encoding.into(),
        attributes: metadata.file_attributes(),
        creation_time: metadata.creation_time(),
        last_access_time: metadata.last_access_time(),
        last_write_time: metadata.last_write_time(),
        acl: Some(capture_acl(path)?),
        entries,
    })
}

/// Read the hosts file into a list of entries.
pub fn list() -> Result<Vec<HostEntry>, String> {
    Ok(snapshot()?.entries)
}

fn decode_hosts_bytes(bytes: &[u8]) -> Result<(&str, &'static str), String> {
    let encoding = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        "utf-8-bom"
    } else {
        "utf-8"
    };
    let content = std::str::from_utf8(bytes)
        .map_err(|_| "The hosts file uses an unsupported or unsafe encoding.".to_string())?;
    Ok((content, encoding))
}

fn parse_entries(content: &str) -> Result<Vec<HostEntry>, String> {
    let mut entries = Vec::new();
    let mut id = 0;

    for line in content.lines() {
        let trimmed = line.trim();
        id += 1;

        // Skip empty lines
        if trimmed.is_empty() {
            continue;
        }

        // Pure comment line
        if let Some(stripped) = trimmed.strip_prefix('#') {
            let rest = stripped.trim();
            // Check if it's a commented-out entry or just a comment
            if rest.is_empty() || !looks_like_ip(rest) {
                continue;
            }
            // It's a disabled entry - parse it but mark as disabled
            let (entry_part, comment) = match rest.find('#') {
                Some(pos) => (rest[..pos].trim(), Some(rest[pos + 1..].trim().to_string())),
                None => (rest, None),
            };
            if let Some(mut entry) = parse_line(entry_part, id) {
                entry.enabled = false;
                entry.comment = comment;
                entry.raw_line = Some(line.to_string());
                entries.push(entry);
            }
            continue;
        }

        // Active entry
        let (ip_part, comment) = match trimmed.find('#') {
            Some(pos) => (
                trimmed[..pos].trim(),
                Some(trimmed[pos + 1..].trim().to_string()),
            ),
            None => (trimmed, None),
        };

        if looks_like_ip(ip_part) {
            if let Some(mut entry) = parse_line(ip_part, id) {
                entry.comment = comment;
                entry.raw_line = Some(line.to_string());
                entries.push(entry);
            }
        }
    }

    Ok(entries)
}

/// Check if a trimmed string looks like it starts with an IP address.
fn looks_like_ip(s: &str) -> bool {
    let first = s.split_whitespace().next().unwrap_or("");
    first.contains('.') || first.contains(':')
}

/// Parse a single line (without comment) into a HostEntry with all hostnames.
fn parse_line(line: &str, id: usize) -> Option<HostEntry> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }

    Some(HostEntry {
        id,
        ip: parts[0].to_string(),
        hostnames: parts[1..].iter().map(|s| s.to_string()).collect(),
        comment: None,
        enabled: true,
        raw_line: None,
    })
}

/// Save entries back to the hosts file.
///
/// The governed operation journal captures the original state before this
/// mutation. Exact automatic undo remains disabled until ACL and metadata
/// restoration are proven.
pub fn save(expected: &HostsSnapshot, entries: &[HostEntry]) -> Result<(), String> {
    if !crate::core::elevation::is_elevated() {
        return Err(crate::core::error::WinOptimizerError::ElevationRequired.to_string());
    }
    validate_entries(entries)?;

    // Revalidate the same snapshot that was confirmed by the operation plan.
    let current = snapshot()?;
    if !snapshot_matches(expected, &current) {
        return Err("The hosts file changed while the save was being prepared.".into());
    }
    let output = render_snapshot_bytes(expected, entries)?;

    atomic_replace(Path::new(HOSTS_PATH), expected, &output)?;

    let readback = snapshot()?;
    if readback.bytes != output || !basic_metadata_matches(expected, &readback) {
        return Err(
            "Hosts replacement did not preserve the confirmed content, DACL, attributes, creation time, and last-write time."
                .into(),
        );
    }

    Ok(())
}

fn atomic_replace(path: &Path, original: &HostsSnapshot, bytes: &[u8]) -> Result<(), String> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Storage::FileSystem::{ReplaceFileW, REPLACE_FILE_FLAGS};

    let parent = path
        .parent()
        .ok_or_else(|| "Hosts file has no parent directory".to_string())?;
    let temp = parent.join(format!(".winoptimizer-hosts-{}.tmp", uuid::Uuid::new_v4()));
    let write_result = (|| -> Result<(), String> {
        let original_acl = original.acl.as_deref().ok_or_else(|| {
            "Exact hosts DACL evidence is unavailable; refusing to replace the file.".to_string()
        })?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|error| format!("Cannot stage hosts replacement: {error}"))?;
        file.write_all(bytes)
            .map_err(|error| format!("Cannot stage hosts content: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("Cannot flush hosts replacement: {error}"))?;
        drop(file);
        restore_basic_metadata(&temp, original)?;
        ensure_acl(&temp, original_acl)?;

        let current = snapshot_at(path)
            .map_err(|error| format!("Cannot revalidate hosts file before replacement: {error}"))?;
        if !snapshot_matches(original, &current) {
            return Err("The hosts file changed while the replacement was being staged.".into());
        }

        let target_wide = HSTRING::from(path.as_os_str());
        let temp_wide = HSTRING::from(temp.to_string_lossy().as_ref());
        unsafe {
            ReplaceFileW(
                &target_wide,
                &temp_wide,
                PCWSTR::null(),
                REPLACE_FILE_FLAGS(0),
                None,
                None,
            )
            .map_err(|error| {
                format!(
                    "Cannot atomically replace hosts file: {error}. The original file was preserved."
                )
            })
        }?;
        restore_basic_metadata(path, original)?;
        ensure_acl(path, original_acl)?;
        let readback = snapshot_at(path)?;
        if readback.bytes != bytes || !basic_metadata_matches(original, &readback) {
            return Err(
                "Hosts replacement readback did not match the staged bytes, DACL, and basic metadata."
                    .into(),
            );
        }
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    write_result
}

fn restore_basic_metadata(path: &Path, metadata: &HostsSnapshot) -> Result<(), String> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FileBasicInfo, SetFileInformationByHandle, FILE_ATTRIBUTE_NORMAL,
        FILE_BASIC_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        FILE_WRITE_ATTRIBUTES, OPEN_EXISTING,
    };

    let path_wide = HSTRING::from(path.as_os_str());
    let handle = unsafe {
        CreateFileW(
            &path_wide,
            FILE_WRITE_ATTRIBUTES.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    }
    .map_err(|error| format!("Cannot open hosts metadata for restoration: {error}"))?;
    let info = FILE_BASIC_INFO {
        CreationTime: metadata.creation_time as i64,
        LastAccessTime: metadata.last_access_time as i64,
        LastWriteTime: metadata.last_write_time as i64,
        ChangeTime: 0,
        FileAttributes: metadata.attributes,
    };
    let result = unsafe {
        SetFileInformationByHandle(
            handle,
            FileBasicInfo,
            std::ptr::from_ref(&info).cast(),
            std::mem::size_of::<FILE_BASIC_INFO>() as u32,
        )
    }
    .map_err(|error| format!("Cannot restore hosts attributes and timestamps: {error}"));
    let close_result = unsafe { CloseHandle(handle) }
        .map_err(|error| format!("Cannot close hosts metadata handle: {error}"));
    result.and(close_result)
}

fn free_local(pointer: *mut std::ffi::c_void) {
    if !pointer.is_null() {
        unsafe {
            let _ = windows::Win32::Foundation::LocalFree(Some(
                windows::Win32::Foundation::HLOCAL(pointer),
            ));
        }
    }
}

fn capture_acl(path: &Path) -> Result<String, String> {
    use windows::core::{HSTRING, PWSTR};
    use windows::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetNamedSecurityInfoW, SE_FILE_OBJECT,
    };
    use windows::Win32::Security::{DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};

    let path_wide = HSTRING::from(path.as_os_str());
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let error = unsafe {
        GetNamedSecurityInfoW(
            &path_wide,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            None,
            None,
            &mut descriptor as *mut PSECURITY_DESCRIPTOR,
        )
    };
    if error.0 != 0 {
        return Err(format!(
            "Cannot capture hosts DACL (Windows error {})",
            error.0
        ));
    }
    if descriptor.is_invalid() {
        return Err("Cannot capture hosts DACL: Windows returned no descriptor.".into());
    }

    let mut string_descriptor = PWSTR(std::ptr::null_mut());
    let mut length = 0u32;
    let result = (|| -> Result<String, String> {
        unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                1,
                DACL_SECURITY_INFORMATION,
                &mut string_descriptor,
                Some(&mut length),
            )
        }
        .map_err(|error| format!("Cannot serialize hosts DACL: {error}"))?;
        if string_descriptor.0.is_null() || length == 0 || length > 1_048_576 {
            return Err("Cannot serialize hosts DACL: Windows returned invalid data.".into());
        }
        let wide = unsafe { std::slice::from_raw_parts(string_descriptor.0, length as usize) };
        let end = wide
            .iter()
            .position(|character| *character == 0)
            .unwrap_or(wide.len());
        let value = String::from_utf16(&wide[..end])
            .map_err(|_| "Cannot serialize hosts DACL: invalid UTF-16 data.".to_string())?;
        if value.trim().is_empty() {
            return Err(
                "Cannot serialize hosts DACL: Windows returned an empty descriptor.".into(),
            );
        }
        Ok(value)
    })();
    free_local(string_descriptor.0.cast());
    free_local(descriptor.0.cast());
    result
}

fn restore_acl(path: &Path, acl: &str) -> Result<(), String> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Security::Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW;
    use windows::Win32::Security::{
        SetFileSecurityW, DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        UNPROTECTED_DACL_SECURITY_INFORMATION,
    };

    if acl.trim().is_empty() {
        return Err("Exact hosts DACL evidence is empty; refusing to replace the file.".into());
    }
    let acl_wide: Vec<u16> = acl.encode_utf16().chain(std::iter::once(0)).collect();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let conversion = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(acl_wide.as_ptr()),
            1,
            &mut descriptor as *mut PSECURITY_DESCRIPTOR,
            None,
        )
    };
    if let Err(error) = conversion {
        free_local(descriptor.0.cast());
        return Err(format!("Cannot parse hosts DACL: {error}"));
    }
    if descriptor.is_invalid() {
        return Err("Cannot parse hosts DACL: Windows returned no descriptor.".into());
    }

    let path_wide = HSTRING::from(path.as_os_str());
    let result = unsafe {
        SetFileSecurityW(
            &path_wide,
            DACL_SECURITY_INFORMATION | UNPROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        )
    };
    free_local(descriptor.0.cast());
    if !result.as_bool() {
        return Err(format!(
            "Cannot restore hosts DACL: {}",
            windows::core::Error::from_win32()
        ));
    }
    Ok(())
}

fn ensure_acl(path: &Path, expected: &str) -> Result<(), String> {
    if capture_acl(path)?.as_str() == expected {
        return Ok(());
    }
    restore_acl(path, expected)?;
    let actual = capture_acl(path)?;
    if actual != expected {
        return Err(
            "Windows did not preserve the exact hosts DACL; refusing to replace the original file."
                .into(),
        );
    }
    Ok(())
}

pub(crate) fn basic_metadata_matches(expected: &HostsSnapshot, actual: &HostsSnapshot) -> bool {
    let acl_matches = match (&expected.acl, &actual.acl) {
        (Some(expected), Some(actual)) => !expected.trim().is_empty() && expected == actual,
        _ => false,
    };
    expected.attributes == actual.attributes
        && expected.creation_time == actual.creation_time
        && expected.last_write_time == actual.last_write_time
        && acl_matches
}

pub(crate) fn snapshot_matches(expected: &HostsSnapshot, actual: &HostsSnapshot) -> bool {
    expected.digest == actual.digest
        && expected.bytes == actual.bytes
        && expected.encoding == actual.encoding
        && basic_metadata_matches(expected, actual)
}

pub(crate) fn render_snapshot_bytes(
    original: &HostsSnapshot,
    entries: &[HostEntry],
) -> Result<Vec<u8>, String> {
    let (content, encoding) = decode_hosts_bytes(&original.bytes)?;
    if encoding != original.encoding {
        return Err("Hosts snapshot encoding metadata does not match its bytes.".into());
    }
    Ok(render_hosts_file(content, entries).into_bytes())
}

fn render_hosts_file(original: &str, entries: &[HostEntry]) -> String {
    let mut output = String::new();
    if original.starts_with('\u{feff}') {
        output.push('\u{feff}');
    }
    let body = original.strip_prefix('\u{feff}').unwrap_or(original);
    let original_lines = lines_with_endings(body);
    let append_line_ending = original_lines
        .iter()
        .rev()
        .find_map(|(_, ending)| (!ending.is_empty()).then_some(*ending))
        .unwrap_or("\n");
    let mut entry_ids_written = std::collections::HashSet::new();
    let mut wrote_body = false;

    for (line_idx, (line, line_ending)) in original_lines.iter().enumerate() {
        let trimmed = line.trim();
        let id = line_idx + 1;

        let parsed_original = if let Some(stripped) = trimmed.strip_prefix('#') {
            let rest = stripped.trim();
            if looks_like_ip(rest) {
                let (entry_part, comment) = match rest.find('#') {
                    Some(pos) => (rest[..pos].trim(), Some(rest[pos + 1..].trim().to_string())),
                    None => (rest, None),
                };
                parse_line(entry_part, id).map(|mut entry| {
                    entry.enabled = false;
                    entry.comment = comment;
                    entry
                })
            } else {
                None
            }
        } else {
            let ip_part = match trimmed.find('#') {
                Some(pos) => trimmed[..pos].trim(),
                None => trimmed,
            };
            if looks_like_ip(ip_part) {
                parse_line(ip_part, id).map(|mut entry| {
                    entry.comment = trimmed
                        .find('#')
                        .map(|pos| trimmed[pos + 1..].trim().to_string());
                    entry
                })
            } else {
                None
            }
        };

        if parsed_original.is_none() {
            output.push_str(line);
            output.push_str(line_ending);
            wrote_body = true;
            continue;
        }

        if let Some(entry) = entries.iter().find(|e| e.id == id) {
            let unchanged = parsed_original.as_ref().is_some_and(|original_entry| {
                entry.ip == original_entry.ip
                    && entry.hostnames == original_entry.hostnames
                    && entry.comment == original_entry.comment
                    && entry.enabled == original_entry.enabled
            });
            if unchanged && entry.raw_line.as_deref() == Some(*line) {
                output.push_str(line);
                output.push_str(line_ending);
            } else {
                write_entry_line(&mut output, entry, line_ending);
            }
            entry_ids_written.insert(id);
            wrote_body = true;
        }
    }

    for entry in entries {
        if !entry_ids_written.contains(&entry.id) {
            if wrote_body && !output.ends_with('\n') {
                output.push_str(append_line_ending);
            }
            write_entry_line(&mut output, entry, append_line_ending);
            wrote_body = true;
        }
    }

    output
}

fn lines_with_endings(content: &str) -> Vec<(&str, &str)> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in content.bytes().enumerate() {
        if byte != b'\n' {
            continue;
        }
        if index > start && content.as_bytes()[index - 1] == b'\r' {
            lines.push((&content[start..index - 1], "\r\n"));
        } else {
            lines.push((&content[start..index], "\n"));
        }
        start = index + 1;
    }
    if start < content.len() {
        lines.push((&content[start..], ""));
    }
    lines
}

/// Write a single entry line to the output buffer.
fn write_entry_line(output: &mut String, entry: &HostEntry, line_ending: &str) {
    if entry.enabled {
        output.push_str(&entry.ip);
        for hostname in &entry.hostnames {
            output.push(' ');
            output.push_str(hostname);
        }
    } else {
        output.push_str("# ");
        output.push_str(&entry.ip);
        for hostname in &entry.hostnames {
            output.push(' ');
            output.push_str(hostname);
        }
    }
    if let Some(ref comment) = entry.comment {
        if !comment.is_empty() {
            output.push_str(" # ");
            output.push_str(comment);
        }
    }
    output.push_str(line_ending);
}

pub(crate) fn validate_entries(entries: &[HostEntry]) -> Result<(), String> {
    let mut seen = HashSet::new();
    for entry in entries {
        if has_line_breaks(&entry.ip)
            || entry.comment.as_deref().is_some_and(has_line_breaks)
            || entry
                .hostnames
                .iter()
                .any(|hostname| has_line_breaks(hostname))
        {
            return Err("Hosts entries cannot contain line breaks or NUL characters".to_string());
        }
        entry
            .ip
            .parse::<IpAddr>()
            .map_err(|_| format!("Invalid hosts IP address: {}", entry.ip))?;
        if entry.hostnames.is_empty() {
            return Err(format!(
                "Hosts entry must have at least one hostname: {}",
                entry.ip
            ));
        }
        for hostname in &entry.hostnames {
            validate_hostname(hostname)?;
        }
        for hostname in &entry.hostnames {
            let key = format!(
                "{}|{}",
                entry.ip.to_ascii_lowercase(),
                hostname.to_ascii_lowercase()
            );
            if !seen.insert(key) {
                return Err(format!("Duplicate hosts entry: {} {}", entry.ip, hostname));
            }
        }
    }
    Ok(())
}

fn validate_hostname(hostname: &str) -> Result<(), String> {
    if hostname.is_empty() || hostname.len() > 253 {
        return Err("Hostname length is invalid".to_string());
    }
    let hostname_without_root = hostname.strip_suffix('.').unwrap_or(hostname);
    if hostname_without_root.is_empty() {
        return Err("Hostname length is invalid".to_string());
    }
    for label in hostname_without_root.split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err(format!("Invalid hostname label: {}", hostname));
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err(format!(
                "Hostname label cannot start or end with '-': {}",
                hostname
            ));
        }
        if !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(format!(
                "Hostname contains unsupported characters: {}",
                hostname
            ));
        }
    }
    Ok(())
}

fn has_line_breaks(value: &str) -> bool {
    value.contains('\n') || value.contains('\r') || value.contains('\0')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: usize, raw_line: Option<&str>) -> HostEntry {
        HostEntry {
            id,
            ip: "127.0.0.1".into(),
            hostnames: vec!["localhost".into()],
            comment: Some("keep".into()),
            enabled: true,
            raw_line: raw_line.map(str::to_string),
        }
    }

    #[test]
    fn render_preserves_unparsed_lines_and_unchanged_entries() {
        let original = "# managed comment\n!include custom.conf\n127.0.0.1 localhost # keep\n";
        let rendered = render_hosts_file(original, &[entry(3, Some("127.0.0.1 localhost # keep"))]);
        assert_eq!(rendered, original);
    }

    #[test]
    fn render_rewrites_changed_entries_but_keeps_directives() {
        let original = "!include custom.conf\n127.0.0.1 localhost # keep\n";
        let mut changed = entry(2, Some("127.0.0.1 localhost # keep"));
        changed.hostnames = vec!["local.test".into()];
        let rendered = render_hosts_file(original, &[changed]);
        assert_eq!(
            rendered,
            "!include custom.conf\n127.0.0.1 local.test # keep\n"
        );
    }

    #[test]
    fn render_omits_removed_parsed_entries() {
        let original = "127.0.0.1 localhost\n!include custom.conf\n";
        let rendered = render_hosts_file(original, &[]);
        assert_eq!(rendered, "!include custom.conf\n");
    }

    #[test]
    fn render_preserves_utf8_bom_and_crlf() {
        let original = "\u{feff}# managed\r\n127.0.0.1 localhost # keep\r\n";
        let entries = parse_entries(original.strip_prefix('\u{feff}').unwrap()).unwrap();
        let rendered = render_hosts_file(original, &entries);
        assert_eq!(rendered.as_bytes(), original.as_bytes());
    }

    #[test]
    fn render_preserves_mixed_endings_and_missing_final_newline() {
        let original = "# comment\r\n!include custom.conf\n127.0.0.1 localhost # keep\r\nunknown";
        let entries = parse_entries(original).unwrap();
        assert_eq!(render_hosts_file(original, &entries), original);
    }

    #[test]
    fn render_changes_only_the_target_line_ending() {
        let original = "# comment\r\n127.0.0.1 localhost\nunknown";
        let mut entries = parse_entries(original).unwrap();
        entries[0].hostnames = vec!["local.test".into()];
        assert_eq!(
            render_hosts_file(original, &entries),
            "# comment\r\n127.0.0.1 local.test\nunknown"
        );
    }

    #[test]
    fn render_appends_using_the_last_line_ending_in_a_mixed_file() {
        let original = "# first\r\n# second\r\n127.0.0.1 localhost\n";
        let mut entries = parse_entries(original).unwrap();
        entries.push(entry(4, None));

        assert_eq!(
            render_hosts_file(original, &entries),
            "# first\r\n# second\r\n127.0.0.1 localhost\n127.0.0.1 localhost # keep\n"
        );
    }

    #[test]
    fn disabled_entry_comment_is_not_parsed_as_a_hostname() {
        let entries = parse_entries("# 127.0.0.1 localhost # sample comment\n").unwrap();
        assert_eq!(entries[0].hostnames, vec!["localhost"]);
        assert_eq!(entries[0].comment.as_deref(), Some("sample comment"));
        assert!(!entries[0].enabled);
    }

    #[test]
    fn absolute_hostname_with_root_dot_is_valid() {
        assert!(validate_hostname("example.test.").is_ok());
    }

    #[test]
    fn validation_rejects_injection_and_invalid_addresses() {
        let mut candidate = entry(1, None);
        candidate.comment = Some("comment\r\n1.2.3.4 injected.test".into());
        assert!(validate_entries(&[candidate.clone()]).is_err());

        candidate.comment = None;
        candidate.ip = "127.0.0.1\n1.2.3.4".into();
        assert!(validate_entries(&[candidate.clone()]).is_err());

        candidate.ip = "127.0.0.1".into();
        candidate.hostnames = vec!["localhost\r\ninjected.test".into()];
        assert!(validate_entries(&[candidate.clone()]).is_err());

        candidate.hostnames = vec!["localhost".into()];
        candidate.ip = "999.1.1.1".into();
        assert!(validate_entries(&[candidate.clone()]).is_err());

        candidate.ip = "127.0.0.1".into();
        candidate.hostnames = vec!["-invalid.test".into()];
        assert!(validate_entries(&[candidate]).is_err());
    }

    #[test]
    fn unsupported_encoding_is_rejected() {
        assert!(decode_hosts_bytes(&[0xFF, 0xFE, 0x41, 0x00]).is_err());
    }

    #[test]
    fn atomic_replace_preserves_basic_metadata_and_changes_complete_file() {
        use windows::core::HSTRING;
        use windows::Win32::Storage::FileSystem::{
            SetFileAttributesW, FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_HIDDEN,
        };

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("hosts");
        std::fs::write(&target, b"before").unwrap();
        unsafe {
            SetFileAttributesW(
                &HSTRING::from(target.as_os_str()),
                FILE_ATTRIBUTE_ARCHIVE | FILE_ATTRIBUTE_HIDDEN,
            )
        }
        .unwrap();
        let before = snapshot_at(&target).unwrap();
        atomic_replace(&target, &before, b"after").unwrap();
        let after = snapshot_at(&target).unwrap();
        assert_eq!(after.bytes, b"after");
        assert!(basic_metadata_matches(&before, &after));
        assert_eq!(after.acl, before.acl);

        let mut missing_acl = after.clone();
        missing_acl.acl = None;
        assert!(!basic_metadata_matches(&before, &missing_acl));
    }

    #[test]
    fn atomic_replace_rejects_stale_metadata_without_replacing_file() {
        use windows::core::HSTRING;
        use windows::Win32::Storage::FileSystem::{SetFileAttributesW, FILE_ATTRIBUTE_HIDDEN};

        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("hosts");
        std::fs::write(&target, b"before").unwrap();
        let before = snapshot_at(&target).unwrap();
        unsafe { SetFileAttributesW(&HSTRING::from(target.as_os_str()), FILE_ATTRIBUTE_HIDDEN) }
            .unwrap();

        let error = atomic_replace(&target, &before, b"after").unwrap_err();
        assert!(error.contains("changed"));
        assert_eq!(std::fs::read(&target).unwrap(), b"before");
        assert_eq!(
            std::fs::read_dir(&dir)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".winoptimizer-hosts-")
                })
                .count(),
            0
        );
    }
}
