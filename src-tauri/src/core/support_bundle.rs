//! Support bundles export the current operation journal only.
//!
//! Retained legacy undo history remains in its canonical database for
//! compatibility readers. Exporting a bundle does not import, rewrite, or
//! delete that history.
//!
//! Privacy boundary:
//! - This module is the single authoritative redaction layer for anything the
//!   product presents as safe to share. `redact_value` must be applied to any
//!   shareable artifact.
//! - Operation journal, undo history, settings, scan snapshots, and repair logs
//!   are internal persistent data; they stay unredacted in their own stores and
//!   are not copied into the bundle.
//! - `export_disk_usage_json` / `export_disk_snapshot_json` are raw,
//!   user-controlled exports of the user's own scan; they are documented as
//!   unsanitized and are not support-sharing artifacts.

use crate::core::operation::{OperationError, OperationErrorCode};
use crate::core::operation_journal::OperationJournal;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupportBundleOptions {
    #[serde(default = "default_true")]
    pub redact_user_profile_path: bool,
    #[serde(default = "default_true")]
    pub redact_network_addresses: bool,
    #[serde(default = "default_true")]
    pub redact_hostnames: bool,
    #[serde(default)]
    pub include_exact_file_paths: bool,
    #[serde(default)]
    pub include_command_output: bool,
}

impl Default for SupportBundleOptions {
    fn default() -> Self {
        Self {
            redact_user_profile_path: true,
            redact_network_addresses: true,
            redact_hostnames: true,
            include_exact_file_paths: false,
            include_command_output: false,
        }
    }
}

const REQUIRED_DIRECTORIES: &[&str] = &["diagnostics", "repair", "relevant-log-excerpts"];

const REQUIRED_FILES: &[&str] = &[
    "manifest.json",
    "operations.json",
    "system-summary.json",
    "diagnostics/README.txt",
    "repair/README.txt",
    "relevant-log-excerpts/README.txt",
];

#[derive(Debug, Serialize, Deserialize)]
struct SupportBundleManifest {
    schema_version: u32,
    created_at: String,
    files: Vec<String>,
    options: SupportBundleOptions,
}

/// Export current journal records and redacted diagnostic metadata.
///
/// Legacy undo records are intentionally not copied into the bundle because
/// they remain retained in place for compatibility/history access.
pub fn export(
    journal: &OperationJournal,
    options: SupportBundleOptions,
) -> Result<PathBuf, OperationError> {
    let paths = crate::core::paths::AppPaths::installed()
        .map_err(|_| internal("LOCALAPPDATA is unavailable"))?;
    let root = paths
        .support_bundles()
        .join(format!("support-{}", uuid::Uuid::new_v4()));
    write_bundle(&root, journal, options)?;
    Ok(root)
}

fn write_bundle(
    root: &Path,
    journal: &OperationJournal,
    options: SupportBundleOptions,
) -> Result<(), OperationError> {
    for directory in REQUIRED_DIRECTORIES {
        std::fs::create_dir_all(root.join(directory)).map_err(io_error)?;
    }

    let mut operations = serde_json::to_value(journal.list()?).map_err(serialization_error)?;
    let profile = std::env::var("USERPROFILE").ok();
    redact_value(&mut operations, None, &options, profile.as_deref());
    write_json(&root.join("operations.json"), &operations)?;

    let mut system_summary = serde_json::json!({
        "os": crate::system_info::os::gather(),
        "support_bundle": "conservative-redaction-default",
    });
    redact_value(&mut system_summary, None, &options, profile.as_deref());
    write_json(&root.join("system-summary.json"), &system_summary)?;

    let note = if options.include_command_output {
        "Command output was requested where available; no raw diagnostic log files are collected by this export.\n"
    } else {
        "No command output or raw diagnostic logs are included by default.\n"
    };
    for directory in REQUIRED_DIRECTORIES {
        std::fs::write(root.join(directory).join("README.txt"), note).map_err(io_error)?;
    }

    let created_at = chrono::Utc::now().to_rfc3339();
    let manifest = SupportBundleManifest {
        schema_version: 1,
        created_at,
        files: REQUIRED_FILES
            .iter()
            .map(|file| (*file).to_owned())
            .collect(),
        options,
    };
    write_json(&root.join("manifest.json"), &manifest)?;
    Ok(())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), OperationError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(serialization_error)?;
    std::fs::write(path, bytes).map_err(io_error)
}

fn redact_value(
    value: &mut Value,
    key: Option<&str>,
    options: &SupportBundleOptions,
    profile: Option<&str>,
) {
    match value {
        Value::Object(fields) => {
            for (name, child) in fields {
                if !options.include_command_output && is_command_output_key(name) {
                    *child = Value::String("<redacted-command-output>".into());
                } else {
                    redact_value(child, Some(name), options, profile);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_value(item, key, options, profile);
            }
        }
        Value::String(text) => {
            if let Some(token) = key.and_then(|name| identity_key_token(name, options)) {
                *text = token.into();
                return;
            }
            *text = redact_string(text, options, profile);
        }
        _ => {}
    }
}

/// Compiled redaction patterns. Built once; the support bundle is the single
/// authoritative layer for anything presented as safe to share.
struct RedactionPatterns {
    email: Regex,
    domain_account: Regex,
    registry_path: Regex,
    unc_path: Regex,
    drive_path: Regex,
    forward_slash_path: Regex,
    mac: Regex,
}

fn redaction_patterns() -> &'static RedactionPatterns {
    static PATTERNS: OnceLock<RedactionPatterns> = OnceLock::new();
    PATTERNS.get_or_init(|| RedactionPatterns {
        email: Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b")
            .expect("email pattern"),
        domain_account: Regex::new(
            r"(^|[^A-Za-z0-9._:/\\-])([A-Za-z0-9][A-Za-z0-9._-]*\\[A-Za-z0-9._$-]+)",
        )
        .expect("domain account pattern"),
        registry_path: Regex::new(
            r#"(?i)\b(?:HKEY_[A-Z_]+|HKLM|HKCU|HKCR|HKU)(?:\\[^\s"'<>|,;]*)+"#,
        )
        .expect("registry path pattern"),
        unc_path: Regex::new(r#"\\\\[^\s"'<>|,;]+"#).expect("unc path pattern"),
        drive_path: Regex::new(r#"(?i)\b[a-z]:[\\/][^\s"'<>|,;]*"#).expect("drive path pattern"),
        forward_slash_path: Regex::new(r#"(^|[\s"'(=:,])((?:/[A-Za-z0-9._-]+){2,})"#)
            .expect("forward slash path pattern"),
        mac: Regex::new(r"(?i)\b[0-9a-f]{2}(?:[:-][0-9a-f]{2}){5}\b").expect("mac pattern"),
    })
}

fn is_command_output_key(key: &str) -> bool {
    matches!(key, "command_output" | "stdout" | "stderr")
}

/// Redact identity-bearing *keyed* scalar values that cannot be recognized from
/// their shape alone (for example a bare `JohnDoe` under `user_name`).
fn identity_key_token(key: &str, options: &SupportBundleOptions) -> Option<&'static str> {
    if options.redact_hostnames
        && matches!(
            key,
            "hostname" | "host_name" | "computer_name" | "machine_name" | "domain"
        )
    {
        return Some("<redacted-hostname>");
    }
    if options.redact_hostnames
        && matches!(key, "user_name" | "username" | "user" | "account" | "owner")
    {
        return Some("<redacted-identity>");
    }
    if options.redact_network_addresses && matches!(key, "ssid" | "network_name") {
        return Some("<redacted-network-name>");
    }
    None
}

fn is_registry_hive(segment: &str) -> bool {
    let upper = segment.to_ascii_uppercase();
    upper.starts_with("HKEY_") || matches!(upper.as_str(), "HKLM" | "HKCU" | "HKCR" | "HKU")
}

fn redact_string(text: &str, options: &SupportBundleOptions, profile: Option<&str>) -> String {
    let patterns = redaction_patterns();
    let mut result = text.to_string();

    // Paths first: a whole path value must be hidden entirely (not just its
    // profile prefix) when exact paths were not requested.
    if !options.include_exact_file_paths {
        result = patterns
            .registry_path
            .replace_all(&result, "<redacted-registry-path>")
            .into_owned();
        result = patterns
            .unc_path
            .replace_all(&result, "<redacted-file-path>")
            .into_owned();
        result = patterns
            .drive_path
            .replace_all(&result, "<redacted-file-path>")
            .into_owned();
        result = patterns
            .forward_slash_path
            .replace_all(&result, "$1<redacted-file-path>")
            .into_owned();
    }

    if options.redact_user_profile_path {
        if let Some(profile) = profile.filter(|profile| !profile.is_empty()) {
            let mut candidates = vec![profile.to_string()];
            let forward = profile.replace('\\', "/");
            if forward != profile {
                candidates.push(forward);
            }
            for candidate in candidates {
                result = result.replace(&candidate, "<redacted-user-profile>");
            }
        }
    }

    if options.redact_hostnames {
        result = patterns
            .email
            .replace_all(&result, "<redacted-email>")
            .into_owned();
        result = patterns
            .domain_account
            .replace_all(&result, |captures: &regex::Captures| {
                let boundary = captures
                    .get(1)
                    .map(|value| value.as_str())
                    .unwrap_or_default();
                let account = captures
                    .get(2)
                    .map(|value| value.as_str())
                    .unwrap_or_default();
                let domain = account.split('\\').next().unwrap_or_default();
                if is_registry_hive(domain) {
                    format!("{boundary}{account}")
                } else {
                    format!("{boundary}<redacted-domain-account>")
                }
            })
            .into_owned();
    }

    if options.redact_network_addresses {
        result = redact_network_tokens(&result);
    }

    result
}

fn is_network_token_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, ':' | '.' | '/' | '%')
}

fn classify_network_token(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    if redaction_patterns().mac.is_match(token) {
        return true;
    }
    let (base, prefix) = match token.split_once('/') {
        Some((base, prefix)) => (base, Some(prefix)),
        None => (token, None),
    };
    if let Some(prefix) = prefix {
        if prefix.is_empty() || !prefix.chars().all(|character| character.is_ascii_digit()) {
            return false;
        }
    }
    let address = base.split('%').next().unwrap_or(base);
    address.parse::<std::net::IpAddr>().is_ok()
}

/// Replace only the network-shaped substrings inside `text` so surrounding
/// diagnostic context survives.
fn redact_network_tokens(text: &str) -> String {
    let characters: Vec<char> = text.chars().collect();
    let mut result = String::with_capacity(text.len());
    let mut index = 0;
    while index < characters.len() {
        if is_network_token_char(characters[index]) {
            let start = index;
            while index < characters.len() && is_network_token_char(characters[index]) {
                index += 1;
            }
            let run: String = characters[start..index].iter().collect();
            result.push_str(&redact_network_run(&run));
        } else {
            result.push(characters[index]);
            index += 1;
        }
    }
    result
}

fn redact_network_run(run: &str) -> String {
    let trimmed = run.trim_end_matches(['.', ':', '/', '%']);
    let suffix = &run[trimmed.len()..];
    if !trimmed.is_empty() && classify_network_token(trimmed) {
        format!("<redacted-network-address>{suffix}")
    } else {
        run.to_string()
    }
}

fn default_true() -> bool {
    true
}

fn internal(message: impl Into<String>) -> OperationError {
    OperationError {
        code: OperationErrorCode::InternalError,
        message: message.into(),
        target_id: None,
        retryable: true,
    }
}

fn io_error(error: std::io::Error) -> OperationError {
    internal(error.to_string())
}

fn serialization_error(error: serde_json::Error) -> OperationError {
    OperationError {
        code: OperationErrorCode::SerializationFailed,
        message: error.to_string(),
        target_id: None,
        retryable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_options_are_conservative() {
        let options = SupportBundleOptions::default();
        assert!(options.redact_user_profile_path);
        assert!(options.redact_network_addresses);
        assert!(options.redact_hostnames);
        assert!(!options.include_exact_file_paths);
        assert!(!options.include_command_output);
    }

    #[test]
    fn redacts_paths_and_addresses_without_erasing_operation_shape() {
        let mut value = serde_json::json!({
            "path": r"C:\Users\Alice\file.txt",
            "address": "192.168.1.5",
            "ipv6": "2001:db8::1",
            "hostname": "WIN-ALICE",
            "command_output": "secret command output",
            "count": 2,
        });
        redact_value(
            &mut value,
            None,
            &SupportBundleOptions::default(),
            Some(r"C:\Users\Alice"),
        );
        assert_eq!(value["path"], "<redacted-file-path>");
        assert_eq!(value["address"], "<redacted-network-address>");
        assert_eq!(value["ipv6"], "<redacted-network-address>");
        assert_eq!(value["hostname"], "<redacted-hostname>");
        assert_eq!(value["command_output"], "<redacted-command-output>");
        assert_eq!(value["count"], 2);
    }

    #[test]
    fn explicit_options_allow_requested_details() {
        let mut value = serde_json::json!({
            "path": r"C:\Users\Alice\file.txt",
            "address": "192.168.1.5",
            "hostname": "WIN-ALICE",
            "command_output": "secret command output",
        });
        let options = SupportBundleOptions {
            redact_user_profile_path: false,
            redact_network_addresses: false,
            redact_hostnames: false,
            include_exact_file_paths: true,
            include_command_output: true,
        };

        redact_value(&mut value, None, &options, Some(r"C:\Users\Alice"));

        assert_eq!(value["path"], r"C:\Users\Alice\file.txt");
        assert_eq!(value["address"], "192.168.1.5");
        assert_eq!(value["hostname"], "WIN-ALICE");
        assert_eq!(value["command_output"], "secret command output");
    }

    #[test]
    fn export_contains_required_package_entries() {
        let temp = tempfile::tempdir().expect("temp directory");
        let journal = OperationJournal::open_at(&temp.path().join("journal.db")).expect("journal");
        let bundle = temp.path().join("support-bundle");

        write_bundle(&bundle, &journal, SupportBundleOptions::default()).expect("export");

        for directory in REQUIRED_DIRECTORIES {
            assert!(
                bundle.join(directory).is_dir(),
                "missing directory: {directory}"
            );
        }
        for file in REQUIRED_FILES {
            assert!(bundle.join(file).is_file(), "missing file: {file}");
        }
        assert!(serde_json::from_str::<Value>(
            &std::fs::read_to_string(bundle.join("operations.json")).expect("operations")
        )
        .expect("operations JSON")
        .is_array());
        assert!(serde_json::from_str::<Value>(
            &std::fs::read_to_string(bundle.join("system-summary.json")).expect("system summary")
        )
        .expect("system summary JSON")
        .is_object());
    }

    #[test]
    fn manifest_is_valid_and_enumerates_every_exported_file() {
        let temp = tempfile::tempdir().expect("temp directory");
        let journal = OperationJournal::open_at(&temp.path().join("journal.db")).expect("journal");
        let bundle = temp.path().join("support-bundle");

        write_bundle(&bundle, &journal, SupportBundleOptions::default()).expect("export");

        let manifest: SupportBundleManifest = serde_json::from_str(
            &std::fs::read_to_string(bundle.join("manifest.json")).expect("manifest"),
        )
        .expect("manifest JSON");
        assert_eq!(manifest.schema_version, 1);
        assert!(chrono::DateTime::parse_from_rfc3339(&manifest.created_at).is_ok());
        assert_eq!(
            manifest.files,
            REQUIRED_FILES
                .iter()
                .map(|file| (*file).to_owned())
                .collect::<Vec<_>>()
        );
        for file in &manifest.files {
            assert!(!std::path::Path::new(file).is_absolute());
            assert!(!file.split('/').any(|component| component == ".."));
            assert!(
                bundle.join(file).is_file(),
                "manifest file is missing: {file}"
            );
        }
        assert_eq!(manifest.options, SupportBundleOptions::default());
    }

    #[test]
    fn deserialized_missing_options_keep_conservative_defaults() {
        let options: SupportBundleOptions =
            serde_json::from_value(serde_json::json!({})).expect("default options");
        assert_eq!(options, SupportBundleOptions::default());
    }

    #[test]
    fn redacts_every_seeded_identity_form() {
        let options = SupportBundleOptions::default();
        let profile = Some(r"C:\Users\Alice");

        let cases: &[(&str, &str)] = &[
            (
                "contact john@example.com now",
                "contact <redacted-email> now",
            ),
            (
                "host 192.168.1.10 online",
                "host <redacted-network-address> online",
            ),
            (
                "host 2001:db8::1 online",
                "host <redacted-network-address> online",
            ),
            ("link fe80::1%eth0 up", "link <redacted-network-address> up"),
            (
                "mac 00:1A:2B:3C:4D:5E seen",
                "mac <redacted-network-address> seen",
            ),
            (
                "net 10.0.0.0/24 routed",
                "net <redacted-network-address> routed",
            ),
            (
                r"open \\fileserver\share\secret.txt now",
                "open <redacted-file-path> now",
            ),
            (
                r"open C:\Users\Alice\file.txt now",
                "open <redacted-file-path> now",
            ),
            (
                r"note DOMAIN\JohnDoe signed in",
                "note <redacted-domain-account> signed in",
            ),
            (
                r"read HKLM\SOFTWARE\Microsoft\Windows now",
                "read <redacted-registry-path> now",
            ),
        ];

        for (input, expected) in cases {
            assert_eq!(
                redact_string(input, &options, profile),
                *expected,
                "input: {input}"
            );
        }
    }

    #[test]
    fn redacts_forward_slash_profile_paths_and_keyed_identities() {
        let options = SupportBundleOptions::default();
        let profile = Some(r"C:\Users\Alice");

        assert_eq!(
            redact_string("open C:/Users/Alice/file.txt", &options, profile),
            "open <redacted-file-path>"
        );
        assert_eq!(
            redact_string("home /Users/Alice/file.txt", &options, profile),
            "home <redacted-file-path>"
        );

        for (key, value, expected) in [
            ("user_name", "JohnDoe", "<redacted-identity>"),
            ("hostname", "WIN-ALICE", "<redacted-hostname>"),
            ("computer_name", "WIN-ALICE", "<redacted-hostname>"),
            ("ssid", "HomeWiFi-5G", "<redacted-network-name>"),
        ] {
            let mut node = serde_json::json!({ key: value });
            redact_value(&mut node, None, &options, profile);
            assert_eq!(node[key], expected, "key: {key}");
        }
    }

    #[test]
    fn mixed_text_keeps_useful_context_while_removing_identifiers() {
        let options = SupportBundleOptions::default();
        let input = r"Alice on 10.0.0.5 at DOMAIN\JohnDoe (john@example.com)";
        let output = redact_string(input, &options, Some(r"C:\Users\Alice"));

        for leaked in ["10.0.0.5", r"DOMAIN\JohnDoe", "john@example.com"] {
            assert!(!output.contains(leaked), "leaked {leaked}: {output}");
        }
        assert!(output.contains("Alice on"), "context lost: {output}");
        assert!(output.contains("at "), "context lost: {output}");
    }

    #[test]
    fn raw_output_is_preserved_when_exact_details_are_explicitly_requested() {
        let options = SupportBundleOptions {
            redact_user_profile_path: false,
            redact_network_addresses: false,
            redact_hostnames: false,
            include_exact_file_paths: true,
            include_command_output: true,
        };
        let input = r"open C:\Users\Alice\file.txt from 10.0.0.5";

        assert_eq!(
            redact_string(input, &options, Some(r"C:\Users\Alice")),
            input
        );
    }

    #[test]
    fn operations_export_redacts_embedded_paths_and_hosts() {
        let options = SupportBundleOptions::default();
        let mut operations = serde_json::json!([
            {
                "properties": {
                    "path": r"C:\Users\Alice\secret.bin",
                    "size_bytes": "1024"
                },
                "host_name": "WIN-ALICE",
                "note": r"reachable at 172.16.5.4 over \\fileserver\share"
            }
        ]);

        redact_value(&mut operations, None, &options, Some(r"C:\Users\Alice"));

        let text = serde_json::to_string(&operations).unwrap();
        for leaked in [r"C:\Users\Alice", "WIN-ALICE", "172.16.5.4", "fileserver"] {
            assert!(!text.contains(leaked), "leaked {leaked}: {text}");
        }
        assert!(text.contains("size_bytes"), "diagnostic shape lost");
        assert!(text.contains("1024"), "diagnostic value lost");
    }
}
