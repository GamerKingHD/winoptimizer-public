//! Consent-gated transport, verification, and atomic installation primitives
//! for opt-in updates.
//!
//! Core workflows never call the transport. A caller must pass the persisted
//! feature-specific consent, and downloaded content is still rejected unless
//! the existing manifest signature and payload digest checks succeed.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[cfg(windows)]
use std::cmp::min;

use crate::core::command_helpers::create_command;
use crate::core::settings::AppSettings;

const FORMAT_VERSION: u32 = 1;
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_PAYLOAD_BYTES: usize = 512 * 1024 * 1024;
const MAX_READ_CHUNK: usize = 64 * 1024;
const REQUEST_TIMEOUT_MS: i32 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateKind {
    Release,
    Guidance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateDistribution {
    Portable,
    Msi,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedUpdateManifest {
    pub format_version: u32,
    pub kind: UpdateKind,
    pub channel: String,
    pub version: String,
    pub payload_blake3: String,
    pub signature: String,
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("update manifest is not valid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("update verification key is invalid")]
    InvalidVerificationKey,
    #[error("update verification key is not configured")]
    VerificationKeyNotConfigured,
    #[error("update signature is not valid hexadecimal")]
    InvalidSignatureEncoding,
    #[error("update signature is invalid")]
    InvalidSignature,
    #[error("update format version is unsupported")]
    UnsupportedFormat,
    #[error("update channel is empty")]
    EmptyChannel,
    #[error("update version is empty")]
    EmptyVersion,
    #[error("update service is not configured")]
    ConfigurationUnavailable,
    #[error("update payload digest is invalid")]
    InvalidPayloadDigest,
    #[error("update payload does not match its signed digest")]
    PayloadDigestMismatch,
    #[error("network access for this update feature was not explicitly enabled")]
    NetworkAccessDenied,
    #[error("update endpoint must be a valid HTTPS URL")]
    InvalidEndpoint,
    #[error("update endpoints must use the same HTTPS origin")]
    MismatchedEndpointOrigin,
    #[error("update network access is unavailable on this platform")]
    NetworkUnavailable,
    #[error("update network request failed: {0}")]
    Network(String),
    #[error("update server returned HTTP status {0}")]
    UnexpectedHttpStatus(u16),
    #[error("update {kind} response exceeds {max_bytes} bytes")]
    ResponseTooLarge {
        kind: &'static str,
        max_bytes: usize,
    },
    #[error("downloaded update kind does not match the requested feature")]
    KindMismatch,
    #[error("downloaded update channel does not match the requested channel")]
    ChannelMismatch,
    #[error("update {candidate} is older than the installed version {current}")]
    Downgrade { current: String, candidate: String },
    #[error("the verified release payload is not a valid {distribution} artifact")]
    InvalidDistributionPayload { distribution: &'static str },
    #[error("no verified update is staged")]
    NoStagedUpdate,
    #[error("update installation is unavailable on this platform")]
    InstallationUnavailable,
    #[error("update installer could not be started: {0}")]
    Installer(io::Error),
    #[error("update installation failed: {0}")]
    Io(#[from] io::Error),
}

/// The two independent online permissions described by Phase 11.
///
/// This is intentionally not a global Internet switch. The persisted
/// `AppSettings` value is checked immediately before any network operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateKindConsent {
    ReleaseChecks,
    GuidanceUpdates,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateRequest {
    pub kind: UpdateKind,
    pub channel: String,
    pub manifest_url: String,
    pub payload_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadedUpdate {
    manifest: SignedUpdateManifest,
    payload: Vec<u8>,
}

impl DownloadedUpdate {
    pub fn manifest(&self) -> &SignedUpdateManifest {
        &self.manifest
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuidanceSource {
    Bundled,
    Updated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidanceSelection {
    pub source: GuidanceSource,
    pub payload: Vec<u8>,
    pub warning: Option<String>,
}

/// Keep the packaged guidance usable when an optional update fails.
///
/// The candidate is produced only after `fetch_signed_update` has verified its
/// manifest and payload. Any transport, parsing, kind, or verification error
/// is reported in `warning` while the bundled bytes remain active.
pub fn select_guidance_or_bundled(
    bundled: &[u8],
    candidate: Result<DownloadedUpdate, UpdateError>,
) -> GuidanceSelection {
    match candidate {
        Ok(update) if update.manifest.kind == UpdateKind::Guidance => GuidanceSelection {
            source: GuidanceSource::Updated,
            payload: update.payload,
            warning: None,
        },
        Ok(_) => GuidanceSelection {
            source: GuidanceSource::Bundled,
            payload: bundled.to_vec(),
            warning: Some(UpdateError::KindMismatch.to_string()),
        },
        Err(error) => GuidanceSelection {
            source: GuidanceSource::Bundled,
            payload: bundled.to_vec(),
            warning: Some(error.to_string()),
        },
    }
}

#[derive(Debug, Clone)]
pub struct UpdateVerifier {
    public_key: VerifyingKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SecureEndpoint {
    host: String,
    port: u16,
    path: String,
    origin: String,
}

impl UpdateRequest {
    /// Load release/guidance endpoints supplied by the signed-release build
    /// environment. A missing endpoint fails closed instead of silently
    /// choosing a developer or test URL.
    pub fn configured(kind: UpdateKind) -> Result<Self, UpdateError> {
        let (manifest_key, payload_key) = match kind {
            UpdateKind::Release => (
                "WINOPTIMIZER_UPDATE_MANIFEST_URL",
                "WINOPTIMIZER_UPDATE_PAYLOAD_URL",
            ),
            UpdateKind::Guidance => (
                "WINOPTIMIZER_GUIDANCE_MANIFEST_URL",
                "WINOPTIMIZER_GUIDANCE_PAYLOAD_URL",
            ),
        };
        let manifest_url = std::env::var(manifest_key)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .ok_or(UpdateError::ConfigurationUnavailable)?;
        let payload_url = std::env::var(payload_key)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .ok_or(UpdateError::ConfigurationUnavailable)?;
        let channel = std::env::var("WINOPTIMIZER_UPDATE_CHANNEL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "stable".into());
        Ok(Self {
            kind,
            channel,
            manifest_url,
            payload_url,
        })
    }

    fn consent(&self) -> UpdateKindConsent {
        match self.kind {
            UpdateKind::Release => UpdateKindConsent::ReleaseChecks,
            UpdateKind::Guidance => UpdateKindConsent::GuidanceUpdates,
        }
    }

    fn consent_enabled(&self, settings: &AppSettings) -> bool {
        match self.consent() {
            UpdateKindConsent::ReleaseChecks => settings.update_preferences.check_automatically,
            UpdateKindConsent::GuidanceUpdates => {
                settings.recommendation_preferences.update_guidance
            }
        }
    }

    fn endpoints(&self) -> Result<(SecureEndpoint, SecureEndpoint), UpdateError> {
        if self.channel.trim().is_empty() {
            return Err(UpdateError::EmptyChannel);
        }
        let manifest = SecureEndpoint::parse(&self.manifest_url)?;
        let payload = SecureEndpoint::parse(&self.payload_url)?;
        if manifest.origin != payload.origin {
            return Err(UpdateError::MismatchedEndpointOrigin);
        }
        Ok((manifest, payload))
    }
}

impl SecureEndpoint {
    fn parse(value: &str) -> Result<Self, UpdateError> {
        if value.len() > 8192
            || !value.starts_with("https://")
            || value.contains('#')
            || value.contains('@')
            || value
                .bytes()
                .any(|byte| byte == 0 || byte.is_ascii_whitespace() || byte == 0x7f)
        {
            return Err(UpdateError::InvalidEndpoint);
        }

        let remainder = &value["https://".len()..];
        let authority_end = remainder.find(['/', '?']).unwrap_or(remainder.len());
        let authority = &remainder[..authority_end];
        if authority.is_empty() || authority.contains(['[', ']', '%']) {
            return Err(UpdateError::InvalidEndpoint);
        }

        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) if authority.matches(':').count() == 1 => {
                let port = port
                    .parse::<u16>()
                    .map_err(|_| UpdateError::InvalidEndpoint)?;
                if port == 0 {
                    return Err(UpdateError::InvalidEndpoint);
                }
                (host, port)
            }
            Some(_) => return Err(UpdateError::InvalidEndpoint),
            None => (authority, 443),
        };
        if host.is_empty() || host.contains(['/', '?', ':']) {
            return Err(UpdateError::InvalidEndpoint);
        }

        let path = if authority_end == remainder.len() {
            "/".to_string()
        } else if remainder[authority_end..].starts_with('?') {
            format!("/{}", &remainder[authority_end..])
        } else {
            remainder[authority_end..].to_string()
        };

        Ok(Self {
            origin: format!("{}:{}", host.to_ascii_lowercase(), port),
            host: host.to_string(),
            port,
            path,
        })
    }
}

impl UpdateVerifier {
    pub fn from_public_key_bytes(bytes: [u8; 32]) -> Result<Self, UpdateError> {
        let public_key =
            VerifyingKey::from_bytes(&bytes).map_err(|_| UpdateError::InvalidVerificationKey)?;
        Ok(Self { public_key })
    }

    pub fn from_public_key_hex(value: &str) -> Result<Self, UpdateError> {
        let bytes = decode_hex(value, 32).ok_or(UpdateError::InvalidVerificationKey)?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| UpdateError::InvalidVerificationKey)?;
        Self::from_public_key_bytes(bytes)
    }

    pub fn production() -> Result<Self, UpdateError> {
        let value = option_env!("WINOPTIMIZER_UPDATE_PUBLIC_KEY_HEX")
            .ok_or(UpdateError::VerificationKeyNotConfigured)?;
        Self::from_public_key_hex(value)
    }

    /// Download and verify one explicitly opted-in release or guidance update.
    ///
    /// This function performs no work when the corresponding persisted setting
    /// is disabled. It validates both endpoints before opening WinHTTP, uses a
    /// bounded HTTPS GET, and returns the verified bytes without installing
    /// them. Callers may then choose the bundled guidance fallback or invoke
    /// `install_verified` for a release through their existing confirmation
    /// flow.
    pub fn fetch_signed_update(
        &self,
        request: &UpdateRequest,
        settings: &AppSettings,
    ) -> Result<DownloadedUpdate, UpdateError> {
        self.fetch_signed_update_with_settings(request, settings, download_https)
    }

    /// Fetch a signed update after a direct user action. Manual checking is a
    /// request-specific consent and therefore does not require the automatic
    /// update preference to be enabled.
    pub fn fetch_signed_update_explicit(
        &self,
        request: &UpdateRequest,
    ) -> Result<DownloadedUpdate, UpdateError> {
        let (manifest_endpoint, payload_endpoint) = request.endpoints()?;
        self.fetch_signed_update_with(
            request,
            &manifest_endpoint,
            &payload_endpoint,
            download_https,
        )
    }

    /// Fetch and authenticate only the signed manifest. This is the bounded
    /// lightweight path used by automatic checks; it does not download the
    /// release package.
    pub fn fetch_signed_manifest(
        &self,
        request: &UpdateRequest,
        settings: &AppSettings,
    ) -> Result<SignedUpdateManifest, UpdateError> {
        if !request.consent_enabled(settings) {
            return Err(UpdateError::NetworkAccessDenied);
        }
        self.fetch_signed_manifest_explicit(request)
    }

    pub fn fetch_signed_manifest_explicit(
        &self,
        request: &UpdateRequest,
    ) -> Result<SignedUpdateManifest, UpdateError> {
        self.fetch_signed_manifest_explicit_with(request, download_https)
    }

    fn fetch_signed_manifest_explicit_with<F>(
        &self,
        request: &UpdateRequest,
        mut download: F,
    ) -> Result<SignedUpdateManifest, UpdateError>
    where
        F: FnMut(&SecureEndpoint, usize, &'static str) -> Result<Vec<u8>, UpdateError>,
    {
        let (manifest_endpoint, _) = request.endpoints()?;
        let manifest_bytes = download(&manifest_endpoint, MAX_MANIFEST_BYTES, "manifest")?;
        let manifest: SignedUpdateManifest = serde_json::from_slice(&manifest_bytes)?;
        if manifest.kind != request.kind {
            return Err(UpdateError::KindMismatch);
        }
        if manifest.channel != request.channel {
            return Err(UpdateError::ChannelMismatch);
        }
        self.verify_manifest_signature(&manifest)?;
        Ok(manifest)
    }

    fn fetch_signed_update_with_settings<F>(
        &self,
        request: &UpdateRequest,
        settings: &AppSettings,
        download: F,
    ) -> Result<DownloadedUpdate, UpdateError>
    where
        F: FnMut(&SecureEndpoint, usize, &'static str) -> Result<Vec<u8>, UpdateError>,
    {
        if !request.consent_enabled(settings) {
            return Err(UpdateError::NetworkAccessDenied);
        }
        let (manifest_endpoint, payload_endpoint) = request.endpoints()?;
        self.fetch_signed_update_with(request, &manifest_endpoint, &payload_endpoint, download)
    }

    fn fetch_signed_update_with<F>(
        &self,
        request: &UpdateRequest,
        manifest_endpoint: &SecureEndpoint,
        payload_endpoint: &SecureEndpoint,
        mut download: F,
    ) -> Result<DownloadedUpdate, UpdateError>
    where
        F: FnMut(&SecureEndpoint, usize, &'static str) -> Result<Vec<u8>, UpdateError>,
    {
        let manifest_bytes = download(manifest_endpoint, MAX_MANIFEST_BYTES, "manifest")?;
        let manifest: SignedUpdateManifest = serde_json::from_slice(&manifest_bytes)?;
        if manifest.kind != request.kind {
            return Err(UpdateError::KindMismatch);
        }
        if manifest.channel != request.channel {
            return Err(UpdateError::ChannelMismatch);
        }

        let payload = download(payload_endpoint, MAX_PAYLOAD_BYTES, "payload")?;
        self.verify(&manifest, &payload)?;
        Ok(DownloadedUpdate { manifest, payload })
    }

    pub fn verify(
        &self,
        manifest: &SignedUpdateManifest,
        payload: &[u8],
    ) -> Result<(), UpdateError> {
        self.validate_manifest(manifest)?;
        let expected_digest =
            decode_hex(&manifest.payload_blake3, 32).ok_or(UpdateError::InvalidPayloadDigest)?;
        if blake3::hash(payload).as_bytes() != expected_digest.as_slice() {
            return Err(UpdateError::PayloadDigestMismatch);
        }
        self.verify_manifest_signature(manifest)
    }

    fn validate_manifest(&self, manifest: &SignedUpdateManifest) -> Result<(), UpdateError> {
        if manifest.format_version != FORMAT_VERSION {
            return Err(UpdateError::UnsupportedFormat);
        }
        if manifest.channel.trim().is_empty() {
            return Err(UpdateError::EmptyChannel);
        }
        if manifest.version.trim().is_empty() {
            return Err(UpdateError::EmptyVersion);
        }
        decode_hex(&manifest.payload_blake3, 32).ok_or(UpdateError::InvalidPayloadDigest)?;
        Ok(())
    }

    fn verify_manifest_signature(
        &self,
        manifest: &SignedUpdateManifest,
    ) -> Result<(), UpdateError> {
        self.validate_manifest(manifest)?;
        let signature_bytes =
            decode_hex(&manifest.signature, 64).ok_or(UpdateError::InvalidSignatureEncoding)?;
        let signature = Signature::from_slice(&signature_bytes)
            .map_err(|_| UpdateError::InvalidSignatureEncoding)?;
        self.public_key
            .verify(&signed_payload(manifest)?, &signature)
            .map_err(|_| UpdateError::InvalidSignature)
    }

    /// Reject a signed package that would silently downgrade the installed
    /// application. Equal versions are valid and are reported as up to date by
    /// the command layer.
    pub fn verify_not_downgrade(
        &self,
        manifest: &SignedUpdateManifest,
        current_version: &str,
    ) -> Result<(), UpdateError> {
        self.validate_manifest(manifest)?;
        if current_version.trim().is_empty() {
            return Err(UpdateError::EmptyVersion);
        }
        if compare_versions(&manifest.version, current_version).is_lt() {
            return Err(UpdateError::Downgrade {
                current: current_version.into(),
                candidate: manifest.version.clone(),
            });
        }
        Ok(())
    }

    pub fn is_newer_version(&self, manifest: &SignedUpdateManifest, current_version: &str) -> bool {
        compare_versions(&manifest.version, current_version).is_gt()
    }

    /// Verify the signed manifest and replace `destination` atomically.
    pub fn install_verified(
        &self,
        manifest: &SignedUpdateManifest,
        payload: &[u8],
        destination: impl AsRef<Path>,
    ) -> Result<(), UpdateError> {
        self.verify(manifest, payload)?;
        write_atomic(destination.as_ref(), payload)
    }

    /// Persist a verified release in the application-owned staging directory.
    /// The running executable is never overwritten by this method.
    pub fn stage_release(
        &self,
        update: &DownloadedUpdate,
        staging_dir: impl AsRef<Path>,
        distribution: UpdateDistribution,
    ) -> Result<StagedUpdate, UpdateError> {
        if update.manifest.kind != UpdateKind::Release {
            return Err(UpdateError::KindMismatch);
        }
        validate_distribution_payload(distribution, &update.payload)?;
        let staging_dir = staging_dir.as_ref();
        fs::create_dir_all(staging_dir)?;
        let extension = match distribution {
            UpdateDistribution::Portable => "exe",
            UpdateDistribution::Msi => "msi",
        };
        let version = safe_version_component(&update.manifest.version)?;
        let artifact_path = staging_dir.join(format!("winoptimizer-{version}.{extension}"));
        self.install_verified(&update.manifest, &update.payload, &artifact_path)?;
        let metadata = StagedUpdateMetadata {
            manifest: update.manifest.clone(),
            distribution,
        };
        let metadata_path = staging_dir.join(STAGED_METADATA_FILE);
        let metadata_bytes = serde_json::to_vec_pretty(&metadata)?;
        write_atomic(&metadata_path, &metadata_bytes)?;
        Ok(StagedUpdate {
            manifest: update.manifest.clone(),
            distribution,
            artifact_path,
        })
    }

    pub fn load_staged_release(
        &self,
        staging_dir: impl AsRef<Path>,
        distribution: UpdateDistribution,
    ) -> Result<StagedUpdate, UpdateError> {
        let staging_dir = staging_dir.as_ref();
        let metadata_path = staging_dir.join(STAGED_METADATA_FILE);
        let metadata: StagedUpdateMetadata =
            serde_json::from_slice(&fs::read(&metadata_path).map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    UpdateError::NoStagedUpdate
                } else {
                    UpdateError::Io(error)
                }
            })?)?;
        if metadata.distribution != distribution {
            return Err(UpdateError::InvalidDistributionPayload {
                distribution: distribution.as_str(),
            });
        }
        let extension = match distribution {
            UpdateDistribution::Portable => "exe",
            UpdateDistribution::Msi => "msi",
        };
        let artifact_path = staging_dir.join(format!(
            "winoptimizer-{}.{}",
            safe_version_component(&metadata.manifest.version)?,
            extension
        ));
        let payload = fs::read(&artifact_path).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                UpdateError::NoStagedUpdate
            } else {
                UpdateError::Io(error)
            }
        })?;
        self.verify(&metadata.manifest, &payload)?;
        validate_distribution_payload(distribution, &payload)?;
        Ok(StagedUpdate {
            manifest: metadata.manifest,
            distribution,
            artifact_path,
        })
    }

    /// Install only a previously staged, re-verified release after explicit
    /// user confirmation. Portable updates use a delayed helper replacement;
    /// MSI updates launch the standard Windows Installer UI.
    pub fn install_staged_release(
        &self,
        staged: &StagedUpdate,
        current_version: &str,
    ) -> Result<UpdateInstallResult, UpdateError> {
        self.verify_not_downgrade(&staged.manifest, current_version)?;
        let payload = fs::read(&staged.artifact_path)?;
        self.verify(&staged.manifest, &payload)?;
        validate_distribution_payload(staged.distribution, &payload)?;
        match staged.distribution {
            UpdateDistribution::Portable => schedule_portable_replacement(&staged.artifact_path),
            UpdateDistribution::Msi => launch_msi_installer(&staged.artifact_path),
        }
        .map(|status| UpdateInstallResult {
            status,
            distribution: staged.distribution,
            version: staged.manifest.version.clone(),
            artifact_path: staged.artifact_path.clone(),
            requires_restart: matches!(staged.distribution, UpdateDistribution::Portable),
        })
    }
}

const STAGED_METADATA_FILE: &str = "staged-update.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StagedUpdateMetadata {
    manifest: SignedUpdateManifest,
    distribution: UpdateDistribution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedUpdate {
    pub manifest: SignedUpdateManifest,
    pub distribution: UpdateDistribution,
    pub artifact_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateInstallStatus {
    PortableReplacementScheduled,
    MsiInstallerLaunched,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateInstallResult {
    pub status: UpdateInstallStatus,
    pub distribution: UpdateDistribution,
    pub version: String,
    pub artifact_path: PathBuf,
    pub requires_restart: bool,
}

impl UpdateDistribution {
    fn as_str(self) -> &'static str {
        match self {
            Self::Portable => "portable",
            Self::Msi => "MSI",
        }
    }
}

fn safe_version_component(version: &str) -> Result<String, UpdateError> {
    if version.is_empty()
        || version.len() > 64
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'+'))
    {
        return Err(UpdateError::EmptyVersion);
    }
    Ok(version.into())
}

fn compare_versions(candidate: &str, current: &str) -> std::cmp::Ordering {
    let candidate_parts = version_parts(candidate);
    let current_parts = version_parts(current);
    let count = candidate_parts.len().max(current_parts.len());
    (0..count)
        .map(|index| {
            (
                candidate_parts.get(index).copied().unwrap_or(0),
                current_parts.get(index).copied().unwrap_or(0),
            )
        })
        .find_map(|(candidate, current)| (candidate != current).then_some(candidate.cmp(&current)))
        .unwrap_or_else(|| candidate.cmp(current))
}

fn version_parts(value: &str) -> Vec<u64> {
    value
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<u64>().unwrap_or(u64::MAX))
        .collect()
}

fn validate_distribution_payload(
    distribution: UpdateDistribution,
    payload: &[u8],
) -> Result<(), UpdateError> {
    let valid = match distribution {
        UpdateDistribution::Portable => payload.starts_with(b"MZ"),
        UpdateDistribution::Msi => {
            payload.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1])
        }
    };
    if valid {
        Ok(())
    } else {
        Err(UpdateError::InvalidDistributionPayload {
            distribution: distribution.as_str(),
        })
    }
}

fn launch_msi_installer(path: &Path) -> Result<UpdateInstallStatus, UpdateError> {
    #[cfg(windows)]
    {
        create_command("msiexec.exe")
            .arg("/i")
            .arg(path)
            .spawn()
            .map_err(UpdateError::Installer)?;
        Ok(UpdateInstallStatus::MsiInstallerLaunched)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err(UpdateError::InstallationUnavailable)
    }
}

fn schedule_portable_replacement(path: &Path) -> Result<UpdateInstallStatus, UpdateError> {
    #[cfg(windows)]
    {
        let current = std::env::current_exe().map_err(UpdateError::Installer)?;
        if current == path {
            return Err(UpdateError::Installer(io::Error::new(
                io::ErrorKind::InvalidInput,
                "staged portable update must not be the running executable",
            )));
        }
        let helper = path
            .parent()
            .ok_or_else(|| {
                UpdateError::Installer(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "staged update has no parent",
                ))
            })?
            .join(format!(".apply-update-{}.cmd", uuid::Uuid::new_v4()));
        let script = r#"@echo off
set "pid=%~1"
:wait_for_exit
tasklist /FI "PID eq %pid%" /NH | findstr /C:"%pid%" >nul
if not errorlevel 1 (
  timeout /t 1 /nobreak >nul
  goto wait_for_exit
)
move /Y "%~2" "%~3" >nul
if errorlevel 1 exit /b 1
start "" "%~3"
del "%~f0"
"#;
        fs::write(&helper, script).map_err(UpdateError::Installer)?;
        let pid = std::process::id().to_string();
        let command_line = format!(
            "\"{}\" {} \"{}\" \"{}\"",
            helper.display(),
            pid,
            path.display(),
            current.display()
        );
        create_command("cmd.exe")
            .args(["/d", "/c", &command_line])
            .spawn()
            .map_err(UpdateError::Installer)?;
        Ok(UpdateInstallStatus::PortableReplacementScheduled)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err(UpdateError::InstallationUnavailable)
    }
}

fn signed_payload(manifest: &SignedUpdateManifest) -> Result<Vec<u8>, UpdateError> {
    #[derive(Serialize)]
    struct Claims<'a> {
        format_version: u32,
        kind: UpdateKind,
        channel: &'a str,
        version: &'a str,
        payload_blake3: &'a str,
    }

    serde_json::to_vec(&Claims {
        format_version: manifest.format_version,
        kind: manifest.kind,
        channel: &manifest.channel,
        version: &manifest.version,
        payload_blake3: &manifest.payload_blake3,
    })
    .map_err(UpdateError::InvalidJson)
}

fn decode_hex(value: &str, expected_bytes: usize) -> Option<Vec<u8>> {
    if value.len() != expected_bytes * 2 {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16)? as u8;
            let low = (pair[1] as char).to_digit(16)? as u8;
            Some((high << 4) | low)
        })
        .collect()
}

fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), UpdateError> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "update path has no parent"))?;
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("update.bin");
    let temporary = parent.join(format!(".{file_name}.{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        replace_atomically(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(UpdateError::Io)
}

fn replace_atomically(source: &Path, target: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING};

        let mut source_wide: Vec<u16> = source.as_os_str().encode_wide().collect();
        let mut target_wide: Vec<u16> = target.as_os_str().encode_wide().collect();
        source_wide.push(0);
        target_wide.push(0);
        unsafe {
            MoveFileExW(
                PCWSTR(source_wide.as_ptr()),
                PCWSTR(target_wide.as_ptr()),
                MOVEFILE_REPLACE_EXISTING,
            )
        }
        .map_err(|error| io::Error::other(error.to_string()))
    }

    #[cfg(not(windows))]
    {
        fs::rename(source, target)
    }
}

#[cfg(windows)]
fn download_https(
    endpoint: &SecureEndpoint,
    max_bytes: usize,
    response_kind: &'static str,
) -> Result<Vec<u8>, UpdateError> {
    use std::ffi::c_void;
    use windows::core::PCWSTR;
    use windows::Win32::Networking::WinHttp::{
        WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
        WinHttpQueryDataAvailable, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
        WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts,
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_DISABLE_AUTHENTICATION,
        WINHTTP_DISABLE_COOKIES, WINHTTP_DISABLE_REDIRECTS, WINHTTP_FLAG_SECURE,
        WINHTTP_OPEN_REQUEST_FLAGS, WINHTTP_OPTION_DISABLE_FEATURE, WINHTTP_QUERY_FLAG_NUMBER,
        WINHTTP_QUERY_STATUS_CODE,
    };

    struct Handle(*mut c_void);

    impl Handle {
        fn open(raw: *mut c_void, operation: &str) -> Result<Self, UpdateError> {
            if raw.is_null() {
                return Err(UpdateError::Network(format!(
                    "{operation} returned a null WinHTTP handle"
                )));
            }
            Ok(Self(raw))
        }
    }

    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn map_error(operation: &str, error: windows::core::Error) -> UpdateError {
        UpdateError::Network(format!("{operation}: {error}"))
    }

    let agent = wide("WinOptimizer/1.0");
    let session = unsafe {
        WinHttpOpen(
            PCWSTR(agent.as_ptr()),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR(std::ptr::null()),
            PCWSTR(std::ptr::null()),
            0,
        )
    };
    let session = Handle::open(session, "WinHttpOpen")?;
    unsafe {
        WinHttpSetTimeouts(
            session.0,
            REQUEST_TIMEOUT_MS,
            REQUEST_TIMEOUT_MS,
            REQUEST_TIMEOUT_MS,
            REQUEST_TIMEOUT_MS,
        )
    }
    .map_err(|error| map_error("WinHttpSetTimeouts", error))?;

    let host = wide(&endpoint.host);
    let connection = unsafe { WinHttpConnect(session.0, PCWSTR(host.as_ptr()), endpoint.port, 0) };
    let connection = Handle::open(connection, "WinHttpConnect")?;

    let path = wide(&endpoint.path);
    let method = wide("GET");
    let request = unsafe {
        WinHttpOpenRequest(
            connection.0,
            PCWSTR(method.as_ptr()),
            PCWSTR(path.as_ptr()),
            PCWSTR(std::ptr::null()),
            PCWSTR(std::ptr::null()),
            std::ptr::null(),
            WINHTTP_OPEN_REQUEST_FLAGS(WINHTTP_FLAG_SECURE.0),
        )
    };
    let request = Handle::open(request, "WinHttpOpenRequest")?;

    let disabled_features =
        (WINHTTP_DISABLE_REDIRECTS | WINHTTP_DISABLE_COOKIES | WINHTTP_DISABLE_AUTHENTICATION)
            .to_ne_bytes();
    unsafe {
        WinHttpSetOption(
            Some(request.0 as *const c_void),
            WINHTTP_OPTION_DISABLE_FEATURE,
            Some(&disabled_features),
        )
    }
    .map_err(|error| map_error("WinHttpSetOption", error))?;

    let headers: Vec<u16> = "Accept: application/json, application/octet-stream\r\n"
        .encode_utf16()
        .collect();
    unsafe { WinHttpSendRequest(request.0, Some(&headers), None, 0, 0, 0) }
        .map_err(|error| map_error("WinHttpSendRequest", error))?;
    unsafe { WinHttpReceiveResponse(request.0, std::ptr::null_mut()) }
        .map_err(|error| map_error("WinHttpReceiveResponse", error))?;

    let mut status = 0u32;
    let mut status_size = std::mem::size_of::<u32>() as u32;
    let mut header_index = 0u32;
    unsafe {
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR(std::ptr::null()),
            Some((&mut status as *mut u32).cast()),
            &mut status_size,
            &mut header_index,
        )
    }
    .map_err(|error| map_error("WinHttpQueryHeaders", error))?;
    if status != 200 {
        return Err(UpdateError::UnexpectedHttpStatus(status as u16));
    }

    let mut body = Vec::new();
    loop {
        let mut available = 0u32;
        unsafe { WinHttpQueryDataAvailable(request.0, &mut available) }
            .map_err(|error| map_error("WinHttpQueryDataAvailable", error))?;
        if available == 0 {
            break;
        }
        if body.len() == max_bytes {
            return Err(UpdateError::ResponseTooLarge {
                kind: response_kind,
                max_bytes,
            });
        }

        let read_size = min(
            min(available as usize, MAX_READ_CHUNK),
            max_bytes - body.len(),
        );
        let start = body.len();
        body.resize(start + read_size, 0);
        let mut read = 0u32;
        let result = unsafe {
            WinHttpReadData(
                request.0,
                body[start..].as_mut_ptr().cast(),
                read_size as u32,
                &mut read,
            )
        };
        if let Err(error) = result {
            body.truncate(start);
            return Err(map_error("WinHttpReadData", error));
        }
        body.truncate(start + read as usize);
        if read == 0 {
            break;
        }
    }

    Ok(body)
}

#[cfg(not(windows))]
fn download_https(
    _endpoint: &SecureEndpoint,
    _max_bytes: usize,
    _response_kind: &'static str,
) -> Result<Vec<u8>, UpdateError> {
    Err(UpdateError::NetworkUnavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use tempfile::tempdir;

    fn encode_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn signed_manifest(kind: UpdateKind) -> (SignedUpdateManifest, UpdateVerifier, Vec<u8>) {
        let seed = *blake3::hash(uuid::Uuid::new_v4().as_bytes()).as_bytes();
        let signing_key = SigningKey::from_bytes(&seed);
        let payload = b"signed guidance payload".to_vec();
        let mut manifest = SignedUpdateManifest {
            format_version: FORMAT_VERSION,
            kind,
            channel: "stable".into(),
            version: "2026.08.10".into(),
            payload_blake3: blake3::hash(&payload).to_hex().to_string(),
            signature: String::new(),
        };
        manifest.signature = encode_hex(
            &signing_key
                .sign(&signed_payload(&manifest).unwrap())
                .to_bytes(),
        );
        let verifier =
            UpdateVerifier::from_public_key_bytes(signing_key.verifying_key().to_bytes()).unwrap();
        (manifest, verifier, payload)
    }

    fn request(kind: UpdateKind) -> UpdateRequest {
        UpdateRequest {
            kind,
            channel: "stable".into(),
            manifest_url: "https://updates.example.test/stable/manifest.json".into(),
            payload_url: "https://updates.example.test/stable/payload.bin".into(),
        }
    }

    #[test]
    fn update_permissions_are_granular_and_off_by_default() {
        let release = request(UpdateKind::Release);
        let guidance = request(UpdateKind::Guidance);
        let mut settings = AppSettings::default();

        assert!(!release.consent_enabled(&settings));
        assert!(!guidance.consent_enabled(&settings));

        settings.update_preferences.check_automatically = true;
        assert!(release.consent_enabled(&settings));
        assert!(!guidance.consent_enabled(&settings));

        settings.recommendation_preferences.update_guidance = true;
        assert!(guidance.consent_enabled(&settings));
    }

    #[test]
    fn disabled_setting_denies_transport_before_endpoint_or_network_work() {
        let (_, verifier, _) = signed_manifest(UpdateKind::Release);
        let request = request(UpdateKind::Release);

        assert!(matches!(
            verifier.fetch_signed_update(&request, &AppSettings::default()),
            Err(UpdateError::NetworkAccessDenied)
        ));
    }

    #[test]
    fn automatic_manifest_check_requires_stored_consent() {
        let (_, verifier, _) = signed_manifest(UpdateKind::Release);
        let request = request(UpdateKind::Release);

        assert!(matches!(
            verifier.fetch_signed_manifest(&request, &AppSettings::default()),
            Err(UpdateError::NetworkAccessDenied)
        ));
    }

    #[test]
    fn explicit_manifest_check_is_consent_without_the_automatic_preference() {
        let (manifest, verifier, _) = signed_manifest(UpdateKind::Release);
        let request = request(UpdateKind::Release);
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();

        // The manual path takes no settings and therefore cannot be gated by the
        // automatic preference; the explicit user action is the consent.
        let fetched = verifier
            .fetch_signed_manifest_explicit_with(&request, |_, _, kind| {
                assert_eq!(kind, "manifest");
                Ok(manifest_bytes.clone())
            })
            .unwrap();

        assert_eq!(fetched, manifest);
    }

    #[test]
    fn endpoints_require_https_and_a_single_origin() {
        let mut request = request(UpdateKind::Guidance);
        request.manifest_url = "http://updates.example.test/manifest.json".into();
        assert!(matches!(
            request.endpoints(),
            Err(UpdateError::InvalidEndpoint)
        ));

        request.manifest_url = "https://updates.example.test/manifest.json".into();
        request.payload_url = "https://cdn.example.test/payload.bin".into();
        assert!(matches!(
            request.endpoints(),
            Err(UpdateError::MismatchedEndpointOrigin)
        ));
    }

    #[test]
    fn consented_transport_verifies_manifest_and_payload_before_returning() {
        let (manifest, verifier, payload) = signed_manifest(UpdateKind::Guidance);
        let request = request(UpdateKind::Guidance);
        let (manifest_endpoint, payload_endpoint) = request.endpoints().unwrap();
        let manifest_bytes = serde_json::to_vec(&manifest).unwrap();

        let update = verifier
            .fetch_signed_update_with(
                &request,
                &manifest_endpoint,
                &payload_endpoint,
                |endpoint, _, kind| {
                    assert_eq!(endpoint.origin, "updates.example.test:443");
                    Ok(if kind == "manifest" {
                        manifest_bytes.clone()
                    } else {
                        payload.clone()
                    })
                },
            )
            .unwrap();

        assert_eq!(update.manifest(), &manifest);
        assert_eq!(update.payload(), payload.as_slice());
    }

    #[test]
    fn failed_guidance_transport_reports_failure_and_keeps_bundled_payload() {
        let (manifest, verifier, _) = signed_manifest(UpdateKind::Guidance);
        let request = request(UpdateKind::Guidance);
        let mut settings = AppSettings::default();
        settings.recommendation_preferences.update_guidance = true;
        let candidate =
            verifier.fetch_signed_update_with_settings(&request, &settings, |_, _, _| {
                Err(UpdateError::Network("DNS lookup failed".into()))
            });

        assert!(matches!(candidate, Err(UpdateError::Network(_))));
        let selection = select_guidance_or_bundled(b"bundled guidance", candidate);
        assert_eq!(selection.source, GuidanceSource::Bundled);
        assert_eq!(selection.payload, b"bundled guidance");
        assert_eq!(
            selection.warning.as_deref(),
            Some("update network request failed: DNS lookup failed")
        );

        let wrong_kind = DownloadedUpdate {
            manifest: SignedUpdateManifest {
                kind: UpdateKind::Release,
                ..manifest
            },
            payload: b"unusable release".to_vec(),
        };
        let selection = select_guidance_or_bundled(b"bundled guidance", Ok(wrong_kind));
        assert_eq!(selection.source, GuidanceSource::Bundled);
        assert_eq!(selection.payload, b"bundled guidance");
        assert_eq!(
            selection.warning.as_deref(),
            Some("downloaded update kind does not match the requested feature")
        );
    }

    #[test]
    fn verifies_and_atomically_installs_signed_payload() {
        let temp = tempdir().unwrap();
        let destination = temp.path().join("guidance.json");
        let (manifest, verifier, payload) = signed_manifest(UpdateKind::Guidance);

        verifier
            .install_verified(&manifest, &payload, &destination)
            .unwrap();

        assert_eq!(fs::read(&destination).unwrap(), payload);
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[test]
    fn rejects_tampered_payload_without_replacing_existing_artifact() {
        let temp = tempdir().unwrap();
        let destination = temp.path().join("guidance.json");
        fs::write(&destination, b"bundled guidance").unwrap();
        let (manifest, verifier, _) = signed_manifest(UpdateKind::Guidance);

        assert!(matches!(
            verifier.install_verified(&manifest, b"tampered", &destination),
            Err(UpdateError::PayloadDigestMismatch)
        ));
        assert_eq!(fs::read(&destination).unwrap(), b"bundled guidance");
    }

    #[test]
    fn rejects_tampered_manifest_signature() {
        let (mut manifest, verifier, payload) = signed_manifest(UpdateKind::Guidance);
        manifest.version = "2026.08.11".into();

        assert!(matches!(
            verifier.verify(&manifest, &payload),
            Err(UpdateError::InvalidSignature)
        ));
    }

    #[test]
    fn rejects_signed_downgrades_and_accepts_newer_versions() {
        let (mut manifest, verifier, _) = signed_manifest(UpdateKind::Release);
        manifest.version = "0.9.0".into();
        assert!(matches!(
            verifier.verify_not_downgrade(&manifest, "1.0.0"),
            Err(UpdateError::Downgrade { .. })
        ));

        manifest.version = "1.1.0".into();
        assert!(verifier.verify_not_downgrade(&manifest, "1.0.0").is_ok());
        assert!(verifier.is_newer_version(&manifest, "1.0.0"));
    }

    #[test]
    fn staging_rejects_a_payload_that_does_not_match_the_selected_distribution() {
        let temp = tempdir().unwrap();
        let (manifest, verifier, payload) = signed_manifest(UpdateKind::Release);
        let update = DownloadedUpdate { manifest, payload };

        assert!(matches!(
            verifier.stage_release(&update, temp.path(), UpdateDistribution::Portable),
            Err(UpdateError::InvalidDistributionPayload { .. })
        ));
        assert!(fs::read_dir(temp.path()).unwrap().next().is_none());
    }

    #[test]
    fn verifies_and_installs_signed_release_payloads() {
        let temp = tempdir().unwrap();
        let destination = temp.path().join("release.bin");
        let (manifest, verifier, payload) = signed_manifest(UpdateKind::Release);

        verifier
            .install_verified(&manifest, &payload, &destination)
            .unwrap();

        assert_eq!(fs::read(&destination).unwrap(), payload);
    }
}
