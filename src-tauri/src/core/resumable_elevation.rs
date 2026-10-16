use crate::core::elevation_broker::{plan_digest, BrokerSession};
use crate::core::operation::{OperationError, OperationPlan};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const CONFIRMED_PLAN_SCHEMA_VERSION: u32 = 1;
const CONFIRMED_PLAN_STORAGE_VERSION: u32 = 5;
const PLAN_LIFETIME_MINUTES: i64 = 10;
const TICKET_AUTHENTICATOR_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElevationTicket {
    pub operation_id: String,
    pub plan_operation_id: String,
    pub nonce: String,
    pub plan_digest: String,
    pub authenticator: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ConfirmedPlanPayload {
    operation_id: String,
    plan_operation_id: String,
    schema_version: u32,
    nonce: String,
    plan_digest: String,
    authenticator_secret: String,
    expires_at: String,
    application_version: String,
    plan: OperationPlan,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredConfirmedPlan {
    storage_version: u32,
    protected_payload: Vec<u8>,
}

pub fn store_confirmed_plan(plan: &OperationPlan) -> Result<ElevationTicket, OperationError> {
    let directory = plan_directory()?;
    store_confirmed_plan_at(plan, &directory, Utc::now())
}

pub fn store_confirmed_plan_at_path(
    plan: &OperationPlan,
    directory: impl AsRef<Path>,
) -> Result<ElevationTicket, OperationError> {
    store_confirmed_plan_at(plan, directory.as_ref(), Utc::now())
}

pub fn load_confirmed_plan(ticket: &ElevationTicket) -> Result<OperationPlan, OperationError> {
    let directory = plan_directory()?;
    load_confirmed_plan_at(ticket, &directory, Utc::now())
}

pub fn load_confirmed_plan_at_path(
    ticket: &ElevationTicket,
    directory: impl AsRef<Path>,
) -> Result<OperationPlan, OperationError> {
    load_confirmed_plan_at(ticket, directory.as_ref(), Utc::now())
}

pub fn claim_confirmed_plan(ticket: &ElevationTicket) -> Result<(), OperationError> {
    let directory = plan_directory()?;
    claim_confirmed_plan_at(ticket, &directory, Utc::now())
}

pub fn claim_confirmed_plan_at_path(
    ticket: &ElevationTicket,
    directory: impl AsRef<Path>,
) -> Result<(), OperationError> {
    claim_confirmed_plan_at(ticket, directory.as_ref(), Utc::now())
}

fn plan_directory() -> Result<PathBuf, OperationError> {
    crate::core::paths::AppPaths::installed()
        .map(|paths| paths.confirmed_plans())
        .map_err(|_| validation("LOCALAPPDATA is unavailable; confirmed operations are disabled"))
}

fn prepare_plan_directory(directory: &Path) -> Result<(), OperationError> {
    fs::create_dir_all(directory)
        .map_err(|error| validation(format!("cannot create confirmed-plan directory: {error}")))?;
    #[cfg(windows)]
    windows_directory_security::prepare(directory)?;
    Ok(())
}

fn validate_plan_directory(directory: &Path) -> Result<(), OperationError> {
    #[cfg(windows)]
    windows_directory_security::validate(directory)?;
    #[cfg(not(windows))]
    if !directory.is_dir() {
        return Err(validation("confirmed-plan directory is unavailable"));
    }
    Ok(())
}

#[cfg(windows)]
mod windows_directory_security {
    use super::{validation, OperationError};
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    const EXPECTED_ACE_FLAGS: u8 = 1 | 2;
    const EXPECTED_ACE_COUNT: u32 = 2;
    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;

    struct UserIdentity {
        user_sid: Vec<u8>,
        token_owner_sid: Vec<u8>,
    }

    pub(super) fn prepare(directory: &Path) -> Result<(), OperationError> {
        validate_path(directory)?;
        let identity = current_user_identity()?;
        validate_owner(directory, &identity)?;
        set_restricted_dacl(directory, &identity.user_sid)?;
        validate_security(directory, &identity.user_sid)
    }

    pub(super) fn validate(directory: &Path) -> Result<(), OperationError> {
        validate_path(directory)?;
        let identity = current_user_identity()?;
        validate_owner(directory, &identity)?;
        validate_security(directory, &identity.user_sid)
    }

    fn validate_path(directory: &Path) -> Result<(), OperationError> {
        if !directory.is_absolute() {
            return Err(validation(
                "confirmed-plan directory must be an absolute path",
            ));
        }
        let metadata = std::fs::symlink_metadata(directory).map_err(|error| {
            validation(format!("cannot inspect confirmed-plan directory: {error}"))
        })?;
        if !metadata.is_dir() {
            return Err(validation("confirmed-plan storage path is not a directory"));
        }
        reject_reparse_point(directory)?;
        if let Some(parent) = directory.parent() {
            reject_reparse_point(parent)?;
        }
        Ok(())
    }

    fn reject_reparse_point(path: &Path) -> Result<(), OperationError> {
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{
            GetFileAttributesW, FILE_ATTRIBUTE_REPARSE_POINT, INVALID_FILE_ATTRIBUTES,
        };

        let wide = path_to_wide(path)?;
        let attributes = unsafe { GetFileAttributesW(PCWSTR(wide.as_ptr())) };
        if attributes == INVALID_FILE_ATTRIBUTES {
            return Err(validation(
                "cannot inspect confirmed-plan storage path attributes",
            ));
        }
        if attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
            return Err(validation(
                "confirmed-plan storage path cannot be a reparse point",
            ));
        }
        Ok(())
    }

    fn validate_owner(directory: &Path, identity: &UserIdentity) -> Result<(), OperationError> {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::{LocalFree, HLOCAL};
        use windows::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
        use windows::Win32::Security::{
            EqualSid, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
        };

        let path = path_to_wide(directory)?;
        let mut owner = PSID::default();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let error = unsafe {
            GetNamedSecurityInfoW(
                PCWSTR(path.as_ptr()),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                Some(&mut owner as *mut PSID),
                None,
                None,
                None,
                &mut descriptor as *mut PSECURITY_DESCRIPTOR,
            )
        };
        let owner_matches = error.0 == 0
            && !owner.is_invalid()
            && (unsafe {
                EqualSid(owner, PSID(identity.user_sid.as_ptr() as *mut c_void)).is_ok()
            } || unsafe {
                EqualSid(
                    owner,
                    PSID(identity.token_owner_sid.as_ptr() as *mut c_void),
                )
                .is_ok()
            });
        if !descriptor.is_invalid() {
            unsafe {
                LocalFree(Some(HLOCAL(descriptor.0.cast())));
            }
        }
        if error.0 != 0 {
            return Err(validation(format!(
                "cannot inspect confirmed-plan directory owner (Windows error {})",
                error.0
            )));
        }
        if !owner_matches {
            return Err(validation(
                "confirmed-plan directory is not owned by the current user or its token owner",
            ));
        }
        Ok(())
    }

    fn set_restricted_dacl(directory: &Path, user_sid: &[u8]) -> Result<(), OperationError> {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::{LocalFree, HLOCAL};
        use windows::Win32::Security::Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SetNamedSecurityInfoW,
            SE_FILE_OBJECT,
        };
        use windows::Win32::Security::{
            GetSecurityDescriptorDacl, DACL_SECURITY_INFORMATION,
            PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        };

        let user_sid = sid_to_string(windows::Win32::Security::PSID(
            user_sid.as_ptr() as *mut c_void
        ))?;
        let sddl = format!("D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;{user_sid})")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let path = path_to_wide(directory)?;
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                1,
                &mut descriptor as *mut PSECURITY_DESCRIPTOR,
                None,
            )
            .map_err(|error| {
                validation(format!(
                    "cannot construct confirmed-plan directory ACL: {error}"
                ))
            })?;
        }

        let mut present = windows::core::BOOL(0);
        let mut dacl = std::ptr::null_mut();
        let mut defaulted = windows::core::BOOL(0);
        let dacl_result = unsafe {
            GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted)
        };
        if let Err(error) = dacl_result {
            unsafe {
                LocalFree(Some(HLOCAL(descriptor.0.cast())));
            }
            return Err(validation(format!(
                "cannot read confirmed-plan directory ACL: {error}"
            )));
        }
        if !present.as_bool() || dacl.is_null() {
            unsafe {
                LocalFree(Some(HLOCAL(descriptor.0.cast())));
            }
            return Err(validation(
                "confirmed-plan directory ACL is not explicitly present",
            ));
        }

        let security_information = DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION;
        let error = unsafe {
            SetNamedSecurityInfoW(
                PCWSTR(path.as_ptr()),
                SE_FILE_OBJECT,
                security_information,
                None,
                None,
                Some(dacl as *const windows::Win32::Security::ACL),
                None,
            )
        };
        unsafe {
            LocalFree(Some(HLOCAL(descriptor.0.cast())));
        }
        if error.0 != 0 {
            return Err(validation(format!(
                "cannot secure confirmed-plan directory ACL (Windows error {})",
                error.0
            )));
        }
        Ok(())
    }

    fn validate_security(directory: &Path, user_sid: &[u8]) -> Result<(), OperationError> {
        use windows::core::PCWSTR;
        use windows::Win32::Security::Authorization::GetNamedSecurityInfoW;
        use windows::Win32::Security::Authorization::SE_FILE_OBJECT;
        use windows::Win32::Security::{
            AclSizeInformation, GetAce, GetAclInformation, GetSecurityDescriptorControl,
            GetSecurityDescriptorDacl, ACCESS_ALLOWED_ACE, ACE_HEADER, ACL_SIZE_INFORMATION,
            DACL_SECURITY_INFORMATION, INHERITED_ACE, PSECURITY_DESCRIPTOR, PSID,
            SE_DACL_PROTECTED,
        };
        use windows::Win32::Storage::FileSystem::FILE_ALL_ACCESS;

        let path = path_to_wide(directory)?;
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        let mut dacl = std::ptr::null_mut();
        let error = unsafe {
            GetNamedSecurityInfoW(
                PCWSTR(path.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&mut dacl as *mut *mut windows::Win32::Security::ACL),
                None,
                &mut descriptor as *mut PSECURITY_DESCRIPTOR,
            )
        };
        if error.0 != 0 {
            return Err(validation(format!(
                "cannot inspect confirmed-plan directory ACL (Windows error {})",
                error.0
            )));
        }
        let descriptor_guard = SecurityDescriptorGuard(descriptor);
        let mut control = 0u16;
        let mut revision = 0u32;
        unsafe { GetSecurityDescriptorControl(descriptor_guard.0, &mut control, &mut revision) }
            .map_err(|error| {
                validation(format!(
                    "cannot inspect confirmed-plan ACL control: {error}"
                ))
            })?;
        if control & SE_DACL_PROTECTED.0 == 0 {
            return Err(validation(
                "confirmed-plan directory ACL must not inherit from its parent",
            ));
        }

        let mut present = windows::core::BOOL(0);
        let mut dacl_defaulted = windows::core::BOOL(0);
        unsafe {
            GetSecurityDescriptorDacl(
                descriptor_guard.0,
                &mut present,
                &mut dacl,
                &mut dacl_defaulted,
            )
        }
        .map_err(|error| {
            validation(format!(
                "cannot inspect confirmed-plan directory DACL: {error}"
            ))
        })?;
        if !present.as_bool() || dacl.is_null() {
            return Err(validation(
                "confirmed-plan directory must have a non-null DACL",
            ));
        }

        let mut size = ACL_SIZE_INFORMATION::default();
        unsafe {
            GetAclInformation(
                dacl,
                &mut size as *mut ACL_SIZE_INFORMATION as *mut c_void,
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            )
        }
        .map_err(|error| {
            validation(format!(
                "cannot inspect confirmed-plan ACL entries: {error}"
            ))
        })?;
        if size.AceCount != EXPECTED_ACE_COUNT {
            return Err(validation(
                "confirmed-plan directory ACL contains unexpected principals",
            ));
        }

        let mut system_seen = false;
        let mut user_seen = false;
        let expected_user_sid = sid_to_string(PSID(user_sid.as_ptr() as *mut c_void))?;
        for index in 0..size.AceCount {
            let mut ace = std::ptr::null_mut();
            unsafe { GetAce(dacl, index, &mut ace) }.map_err(|error| {
                validation(format!("cannot inspect confirmed-plan ACL entry: {error}"))
            })?;
            if ace.is_null() {
                return Err(validation("confirmed-plan ACL entry is invalid"));
            }
            let header = unsafe { *(ace as *const ACE_HEADER) };
            if header.AceType != ACCESS_ALLOWED_ACE_TYPE
                || header.AceFlags & INHERITED_ACE.0 as u8 != 0
                || header.AceFlags != EXPECTED_ACE_FLAGS
                || (header.AceSize as usize) < std::mem::size_of::<ACCESS_ALLOWED_ACE>()
            {
                return Err(validation(
                    "confirmed-plan directory ACL contains an unsafe entry",
                ));
            }
            let allowed = unsafe { &*(ace as *const ACCESS_ALLOWED_ACE) };
            if allowed.Mask != FILE_ALL_ACCESS.0 {
                return Err(validation(
                    "confirmed-plan directory ACL grants an unexpected access mask",
                ));
            }
            let sid = PSID((&allowed.SidStart as *const u32).cast_mut().cast());
            let sid_text = sid_to_string(sid)?;
            if sid_text == "S-1-5-18" && !system_seen {
                system_seen = true;
            } else if sid_text == expected_user_sid && !user_seen {
                user_seen = true;
            } else {
                return Err(validation(
                    "confirmed-plan directory ACL is not bound to the current user",
                ));
            }
        }
        if !system_seen || !user_seen {
            return Err(validation(
                "confirmed-plan directory ACL is missing the current-user or SYSTEM entry",
            ));
        }

        drop(descriptor_guard);
        Ok(())
    }

    fn current_user_identity() -> Result<UserIdentity, OperationError> {
        use windows::Win32::Security::{
            TokenOwner, TokenUser, TOKEN_OWNER, TOKEN_QUERY, TOKEN_USER,
        };
        use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        let mut raw_token = windows::Win32::Foundation::HANDLE::default();
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw_token) }
            .map_err(|error| validation(format!("cannot open current user token: {error}")))?;
        let token_guard = HandleGuard(raw_token);

        Ok(UserIdentity {
            user_sid: query_token_sid(token_guard.0, TokenUser, |buffer| unsafe {
                (*(buffer as *const TOKEN_USER)).User.Sid
            })?,
            token_owner_sid: query_token_sid(token_guard.0, TokenOwner, |buffer| unsafe {
                (*(buffer as *const TOKEN_OWNER)).Owner
            })?,
        })
    }

    fn query_token_sid(
        token: windows::Win32::Foundation::HANDLE,
        information_class: windows::Win32::Security::TOKEN_INFORMATION_CLASS,
        sid_at: impl FnOnce(*const u8) -> windows::Win32::Security::PSID,
    ) -> Result<Vec<u8>, OperationError> {
        use windows::Win32::Security::{GetLengthSid, GetTokenInformation};

        let mut required = 0u32;
        let _ = unsafe { GetTokenInformation(token, information_class, None, 0, &mut required) };
        if required == 0 || required > 64 * 1024 {
            return Err(validation("cannot determine the current token SID"));
        }
        let mut buffer = vec![0u8; required as usize];
        unsafe {
            GetTokenInformation(
                token,
                information_class,
                Some(buffer.as_mut_ptr().cast()),
                required,
                &mut required,
            )
        }
        .map_err(|error| validation(format!("cannot read current token SID: {error}")))?;
        let sid = sid_at(buffer.as_ptr());
        if sid.is_invalid() {
            return Err(validation("current token does not contain a valid SID"));
        }
        let sid_length = unsafe { GetLengthSid(sid) } as usize;
        if sid_length == 0 || sid_length > buffer.len() {
            return Err(validation("current token SID is invalid"));
        }
        Ok(unsafe { std::slice::from_raw_parts(sid.0 as *const u8, sid_length).to_vec() })
    }

    fn sid_to_string(sid: windows::Win32::Security::PSID) -> Result<String, OperationError> {
        use windows::core::{PCWSTR, PWSTR};
        use windows::Win32::Foundation::{LocalFree, HLOCAL};
        use windows::Win32::Security::Authorization::ConvertSidToStringSidW;

        let mut string_sid = PWSTR(std::ptr::null_mut());
        unsafe { ConvertSidToStringSidW(sid, &mut string_sid) }.map_err(|error| {
            validation(format!("cannot inspect confirmed-plan ACL SID: {error}"))
        })?;
        let result = if string_sid.0.is_null() {
            Err(validation("confirmed-plan ACL SID is empty"))
        } else {
            let wide = PCWSTR(string_sid.0);
            let mut length = 0usize;
            while length < 1024 && unsafe { *wide.0.add(length) } != 0 {
                length += 1;
            }
            if length == 1024 {
                Err(validation("confirmed-plan ACL SID is malformed"))
            } else {
                String::from_utf16(unsafe { std::slice::from_raw_parts(wide.0, length) })
                    .map_err(|_| validation("confirmed-plan ACL SID is malformed"))
            }
        };
        unsafe {
            LocalFree(Some(HLOCAL(string_sid.0.cast())));
        }
        result
    }

    fn path_to_wide(path: &Path) -> Result<Vec<u16>, OperationError> {
        let mut wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
        if wide.contains(&0) {
            return Err(validation("confirmed-plan directory path contains a NUL"));
        }
        wide.push(0);
        Ok(wide)
    }

    struct HandleGuard(windows::Win32::Foundation::HANDLE);

    impl Drop for HandleGuard {
        fn drop(&mut self) {
            if self.0 != windows::Win32::Foundation::HANDLE::default() {
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(self.0);
                }
            }
        }
    }

    struct SecurityDescriptorGuard(windows::Win32::Security::PSECURITY_DESCRIPTOR);

    impl Drop for SecurityDescriptorGuard {
        fn drop(&mut self) {
            if !self.0.is_invalid() {
                unsafe {
                    windows::Win32::Foundation::LocalFree(Some(
                        windows::Win32::Foundation::HLOCAL(self.0 .0.cast()),
                    ));
                }
            }
        }
    }
}

fn store_confirmed_plan_at(
    plan: &OperationPlan,
    directory: &Path,
    now: DateTime<Utc>,
) -> Result<ElevationTicket, OperationError> {
    if plan.targets.is_empty() {
        return Err(validation("a confirmed elevation plan requires targets"));
    }
    prepare_plan_directory(directory)?;
    let digest = plan_digest(plan).map_err(|error| validation(error.to_string()))?;
    let expires_at = (now + Duration::minutes(PLAN_LIFETIME_MINUTES)).to_rfc3339();
    let application_version = env!("CARGO_PKG_VERSION");
    let authenticator_secret = new_authenticator_secret();
    let ticket = ElevationTicket {
        operation_id: uuid::Uuid::new_v4().to_string(),
        plan_operation_id: plan.id.0.clone(),
        nonce: uuid::Uuid::new_v4().to_string(),
        plan_digest: digest.clone(),
        authenticator: String::new(),
    };
    let ticket = ElevationTicket {
        authenticator: ticket_authenticator(
            &authenticator_secret,
            &ticket.operation_id,
            &ticket.plan_operation_id,
            &ticket.nonce,
            &ticket.plan_digest,
            &expires_at,
            application_version,
        ),
        ..ticket
    };
    let payload = ConfirmedPlanPayload {
        operation_id: ticket.operation_id.clone(),
        plan_operation_id: ticket.plan_operation_id.clone(),
        schema_version: CONFIRMED_PLAN_SCHEMA_VERSION,
        nonce: ticket.nonce.clone(),
        plan_digest: digest,
        authenticator_secret,
        expires_at,
        application_version: application_version.into(),
        plan: plan.clone(),
    };
    let payload_bytes =
        serde_json::to_vec(&payload).map_err(|error| validation(error.to_string()))?;
    let stored = StoredConfirmedPlan {
        storage_version: CONFIRMED_PLAN_STORAGE_VERSION,
        protected_payload: protect_payload(&payload_bytes)?,
    };
    let bytes = serde_json::to_vec(&stored).map_err(|error| validation(error.to_string()))?;
    let path = plan_path(directory, &ticket.operation_id)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| validation(format!("cannot persist confirmed plan: {error}")))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| validation(format!("cannot persist confirmed plan: {error}")))?;
    Ok(ticket)
}

fn claim_confirmed_plan_at(
    ticket: &ElevationTicket,
    directory: &Path,
    now: DateTime<Utc>,
) -> Result<(), OperationError> {
    load_confirmed_plan_at(ticket, directory, now)?;
    let path = claim_path(directory, &ticket.operation_id)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                validation("confirmed elevation ticket has already been consumed")
            } else {
                validation(format!("cannot claim confirmed plan: {error}"))
            }
        })?;
    file.write_all(now.to_rfc3339().as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|error| validation(format!("cannot persist confirmed-plan claim: {error}")))
}

fn load_confirmed_plan_at(
    ticket: &ElevationTicket,
    directory: &Path,
    now: DateTime<Utc>,
) -> Result<OperationPlan, OperationError> {
    validate_plan_directory(directory)?;
    let bytes = std::fs::read(plan_path(directory, &ticket.operation_id)?)
        .map_err(|error| validation(format!("confirmed plan is unavailable: {error}")))?;
    let stored: StoredConfirmedPlan =
        serde_json::from_slice(&bytes).map_err(|_| validation("confirmed plan is invalid"))?;
    if stored.storage_version != CONFIRMED_PLAN_STORAGE_VERSION {
        return Err(validation("confirmed plan storage version is unsupported"));
    }
    if claim_path(directory, &ticket.operation_id)?.exists() {
        return Err(validation(
            "confirmed elevation ticket has already been consumed",
        ));
    }
    let payload_bytes = unprotect_payload(&stored.protected_payload)?;
    let payload: ConfirmedPlanPayload = serde_json::from_slice(&payload_bytes)
        .map_err(|_| validation("confirmed plan is invalid"))?;
    if payload.operation_id != ticket.operation_id || payload.nonce != ticket.nonce {
        return Err(validation("confirmed plan nonce does not match"));
    }
    if payload.plan_operation_id != ticket.plan_operation_id
        || payload.plan_operation_id != payload.plan.id.0
    {
        return Err(validation(
            "confirmed plan operation identifier does not match",
        ));
    }
    let actual_digest =
        plan_digest(&payload.plan).map_err(|error| validation(error.to_string()))?;
    if payload.plan_digest != ticket.plan_digest || payload.plan_digest != actual_digest {
        return Err(validation("confirmed plan digest does not match"));
    }
    let expected_authenticator = ticket_authenticator(
        &payload.authenticator_secret,
        &ticket.operation_id,
        &ticket.plan_operation_id,
        &ticket.nonce,
        &ticket.plan_digest,
        &payload.expires_at,
        &payload.application_version,
    );
    if !constant_time_equal(&ticket.authenticator, &expected_authenticator) {
        return Err(validation("confirmed plan ticket authentication failed"));
    }
    if payload.schema_version != CONFIRMED_PLAN_SCHEMA_VERSION {
        return Err(validation("confirmed plan schema version is unsupported"));
    }
    if payload.application_version != env!("CARGO_PKG_VERSION") {
        return Err(validation(
            "confirmed plan belongs to a different application version",
        ));
    }
    let expires_at = DateTime::parse_from_rfc3339(&payload.expires_at)
        .map_err(|_| validation("confirmed plan expiration is invalid"))?
        .with_timezone(&Utc);
    if now >= expires_at {
        return Err(validation("confirmed plan has expired"));
    }
    Ok(payload.plan)
}

fn plan_path(directory: &Path, operation_id: &str) -> Result<PathBuf, OperationError> {
    uuid::Uuid::parse_str(operation_id)
        .map_err(|_| validation("confirmed plan operation identifier is invalid"))?;
    Ok(directory.join(format!("{operation_id}.json")))
}

fn claim_path(directory: &Path, operation_id: &str) -> Result<PathBuf, OperationError> {
    uuid::Uuid::parse_str(operation_id)
        .map_err(|_| validation("confirmed plan operation identifier is invalid"))?;
    Ok(directory.join(format!("{operation_id}.claimed")))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ElevationHandoff {
    ticket: ElevationTicket,
    endpoint: String,
    session_id: String,
    nonce: String,
    plan_operation_id: String,
    plan_digest: String,
    expires_at: String,
}

fn handoff_path(directory: &Path, operation_id: &str) -> Result<PathBuf, OperationError> {
    uuid::Uuid::parse_str(operation_id)
        .map_err(|_| validation("confirmed plan operation identifier is invalid"))?;
    Ok(directory.join(format!("{operation_id}.handoff.json")))
}

pub fn store_elevation_handoff(
    ticket: &ElevationTicket,
    session: &BrokerSession,
) -> Result<(), OperationError> {
    store_elevation_handoff_at(&plan_directory()?, ticket, session)
}

fn store_elevation_handoff_at(
    directory: &Path,
    ticket: &ElevationTicket,
    session: &BrokerSession,
) -> Result<(), OperationError> {
    prepare_plan_directory(directory)?;
    let payload = ElevationHandoff {
        ticket: ticket.clone(),
        endpoint: session.endpoint.clone(),
        session_id: session.session_id.clone(),
        nonce: session.nonce.clone(),
        plan_operation_id: session.operation_id.clone(),
        plan_digest: session.plan_digest.clone(),
        expires_at: session.expires_at.clone(),
    };
    let bytes = serde_json::to_vec(&payload).map_err(|error| validation(error.to_string()))?;
    let path = handoff_path(directory, &ticket.operation_id)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .map_err(|error| validation(format!("cannot persist elevation handoff: {error}")))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| validation(format!("cannot persist elevation handoff: {error}")))
}

pub fn load_elevation_handoff(
    operation_id: &str,
) -> Result<(ElevationTicket, BrokerSession), OperationError> {
    load_elevation_handoff_at(&plan_directory()?, operation_id)
}

/// The handoff file is plain JSON, so beyond the ticket binding it is checked
/// for internal consistency: the session must belong to the same confirmed
/// plan as the ticket, and the endpoint must be a local WinOptimizer pipe, never
/// a remote or foreign pipe name.
fn validate_handoff_session(payload: &ElevationHandoff) -> Result<(), OperationError> {
    let ticket = &payload.ticket;
    if payload.nonce != ticket.nonce
        || payload.plan_operation_id != ticket.plan_operation_id
        || payload.plan_digest != ticket.plan_digest
    {
        return Err(validation(
            "elevation handoff session does not match its ticket",
        ));
    }
    let endpoint_is_local = payload
        .endpoint
        .strip_prefix(crate::core::elevation_broker::ENDPOINT_PREFIX)
        .is_some_and(|suffix| uuid::Uuid::parse_str(suffix).is_ok());
    if !endpoint_is_local || uuid::Uuid::parse_str(&payload.session_id).is_err() {
        return Err(validation("elevation handoff endpoint is not trusted"));
    }
    Ok(())
}

fn load_elevation_handoff_at(
    directory: &Path,
    operation_id: &str,
) -> Result<(ElevationTicket, BrokerSession), OperationError> {
    validate_plan_directory(directory)?;
    let bytes = std::fs::read(handoff_path(directory, operation_id)?)
        .map_err(|error| validation(format!("elevation handoff is unavailable: {error}")))?;
    let payload: ElevationHandoff =
        serde_json::from_slice(&bytes).map_err(|_| validation("elevation handoff is invalid"))?;
    if payload.ticket.operation_id != operation_id {
        return Err(validation(
            "elevation handoff does not match the resume operation",
        ));
    }
    validate_handoff_session(&payload)?;
    let session = BrokerSession::from_parts(
        payload.endpoint,
        payload.session_id,
        payload.nonce,
        payload.plan_operation_id,
        payload.plan_digest,
        payload.expires_at,
    )
    .map_err(|error| validation(error.to_string()))?;
    Ok((payload.ticket, session))
}

fn new_authenticator_secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn ticket_authenticator(
    secret: &str,
    operation_id: &str,
    plan_operation_id: &str,
    nonce: &str,
    plan_digest: &str,
    expires_at: &str,
    application_version: &str,
) -> String {
    let key = blake3::derive_key(
        "WinOptimizer resumable elevation ticket authenticator v1",
        secret.as_bytes(),
    );
    let mut hasher = blake3::Hasher::new_keyed(&key);
    hasher.update(b"WinOptimizer confirmed-plan ticket\0");
    for field in [
        TICKET_AUTHENTICATOR_VERSION.to_string(),
        operation_id.to_string(),
        plan_operation_id.to_string(),
        nonce.to_string(),
        plan_digest.to_string(),
        expires_at.to_string(),
        application_version.to_string(),
    ] {
        hasher.update(&(field.len() as u64).to_le_bytes());
        hasher.update(field.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn constant_time_equal(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        difference |= usize::from(
            left.get(index).copied().unwrap_or_default()
                ^ right.get(index).copied().unwrap_or_default(),
        );
    }
    difference == 0
}

#[cfg(windows)]
fn protect_payload(payload: &[u8]) -> Result<Vec<u8>, OperationError> {
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: payload.len() as u32,
        pbData: payload.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptProtectData(
            &input,
            windows::core::w!("WinOptimizer confirmed operation"),
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(|error| validation(format!("cannot protect confirmed plan: {error}")))?;
        if output.cbData == 0 || output.pbData.is_null() {
            return Err(validation("Windows returned an empty protected plan"));
        }
        let protected = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(
            output.pbData.cast(),
        )));
        Ok(protected)
    }
}

#[cfg(not(windows))]
fn protect_payload(_payload: &[u8]) -> Result<Vec<u8>, OperationError> {
    Err(validation(
        "confirmed operation protection is only available on Windows",
    ))
}

#[cfg(windows)]
fn unprotect_payload(protected: &[u8]) -> Result<Vec<u8>, OperationError> {
    use windows::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: protected.len() as u32,
        pbData: protected.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(|error| {
            validation(format!(
                "confirmed plan failed Windows protection validation: {error}"
            ))
        })?;
        if output.cbData == 0 || output.pbData.is_null() {
            return Err(validation("Windows returned an empty confirmed plan"));
        }
        let payload = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(
            output.pbData.cast(),
        )));
        Ok(payload)
    }
}

#[cfg(not(windows))]
fn unprotect_payload(_protected: &[u8]) -> Result<Vec<u8>, OperationError> {
    Err(validation(
        "confirmed operation protection is only available on Windows",
    ))
}

fn validation(message: impl Into<String>) -> OperationError {
    OperationError::validation(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::operation::synthetic_operation_contract;

    #[test]
    fn confirmed_plan_round_trip_preserves_exact_targets() {
        let directory = tempfile::tempdir().unwrap();
        let plan = synthetic_operation_contract().unwrap().plan;
        let now = Utc::now();
        let ticket = store_confirmed_plan_at(&plan, directory.path(), now).unwrap();
        let loaded = load_confirmed_plan_at(&ticket, directory.path(), now).unwrap();
        assert_eq!(loaded, plan);
    }

    #[test]
    fn wrong_nonce_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        let plan = synthetic_operation_contract().unwrap().plan;
        let now = Utc::now();
        let mut ticket = store_confirmed_plan_at(&plan, directory.path(), now).unwrap();
        ticket.nonce = uuid::Uuid::new_v4().to_string();
        assert!(load_confirmed_plan_at(&ticket, directory.path(), now).is_err());
    }

    #[test]
    fn forged_ticket_authenticator_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        let plan = synthetic_operation_contract().unwrap().plan;
        let now = Utc::now();
        let mut ticket = store_confirmed_plan_at(&plan, directory.path(), now).unwrap();
        ticket.authenticator = "00".repeat(32);
        assert!(load_confirmed_plan_at(&ticket, directory.path(), now).is_err());
    }

    #[test]
    fn a_confirmed_ticket_can_only_be_claimed_once() {
        let directory = tempfile::tempdir().unwrap();
        let plan = synthetic_operation_contract().unwrap().plan;
        let now = Utc::now();
        let ticket = store_confirmed_plan_at(&plan, directory.path(), now).unwrap();
        claim_confirmed_plan_at(&ticket, directory.path(), now).unwrap();
        assert!(claim_confirmed_plan_at(&ticket, directory.path(), now).is_err());
    }

    #[test]
    fn expired_plan_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        let plan = synthetic_operation_contract().unwrap().plan;
        let now = Utc::now();
        let ticket = store_confirmed_plan_at(&plan, directory.path(), now).unwrap();
        assert!(load_confirmed_plan_at(
            &ticket,
            directory.path(),
            now + Duration::minutes(PLAN_LIFETIME_MINUTES + 1)
        )
        .is_err());
    }

    #[test]
    fn edited_plan_fails_integrity_validation() {
        let directory = tempfile::tempdir().unwrap();
        let plan = synthetic_operation_contract().unwrap().plan;
        let now = Utc::now();
        let ticket = store_confirmed_plan_at(&plan, directory.path(), now).unwrap();
        let path = plan_path(directory.path(), &ticket.operation_id).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let mut stored: StoredConfirmedPlan = serde_json::from_slice(&bytes).unwrap();
        stored.protected_payload[0] ^= 1;
        std::fs::write(path, serde_json::to_vec(&stored).unwrap()).unwrap();
        assert!(load_confirmed_plan_at(&ticket, directory.path(), now).is_err());
    }

    #[test]
    fn unsupported_storage_version_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        let plan = synthetic_operation_contract().unwrap().plan;
        let now = Utc::now();
        let ticket = store_confirmed_plan_at(&plan, directory.path(), now).unwrap();
        let path = plan_path(directory.path(), &ticket.operation_id).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let mut stored: StoredConfirmedPlan = serde_json::from_slice(&bytes).unwrap();
        stored.storage_version = CONFIRMED_PLAN_STORAGE_VERSION + 1;
        std::fs::write(path, serde_json::to_vec(&stored).unwrap()).unwrap();
        assert!(load_confirmed_plan_at(&ticket, directory.path(), now).is_err());
    }

    #[test]
    fn missing_confirmed_plan_directory_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        assert!(validate_plan_directory(&directory.path().join("missing")).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn confirmed_plan_directory_is_bound_to_current_user_and_restricted_acl() {
        let directory = tempfile::tempdir().unwrap();
        prepare_plan_directory(directory.path()).unwrap();
        validate_plan_directory(directory.path()).unwrap();
    }

    fn handoff_fixture(directory: &Path) -> (ElevationTicket, BrokerSession) {
        let plan = synthetic_operation_contract().unwrap().plan;
        let ticket = store_confirmed_plan_at(&plan, directory, Utc::now()).unwrap();
        let session = BrokerSession::create(
            &plan,
            &ticket.plan_operation_id,
            &ticket.nonce,
            &ticket.plan_digest,
        )
        .unwrap();
        (ticket, session)
    }

    fn edit_handoff(
        directory: &Path,
        operation_id: &str,
        edit: impl FnOnce(&mut serde_json::Value),
    ) {
        let path = handoff_path(directory, operation_id).unwrap();
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        edit(&mut value);
        std::fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }

    #[test]
    fn elevation_handoff_round_trip_returns_the_stored_ticket_and_session() {
        let directory = tempfile::tempdir().unwrap();
        let (ticket, session) = handoff_fixture(directory.path());
        store_elevation_handoff_at(directory.path(), &ticket, &session).unwrap();

        let (loaded_ticket, loaded_session) =
            load_elevation_handoff_at(directory.path(), &ticket.operation_id).unwrap();

        assert_eq!(loaded_ticket, ticket);
        assert_eq!(loaded_session.endpoint, session.endpoint);
        assert_eq!(loaded_session.session_id, session.session_id);
        assert_eq!(loaded_session.nonce, session.nonce);
        assert_eq!(loaded_session.operation_id, session.operation_id);
        assert_eq!(loaded_session.plan_digest, session.plan_digest);
    }

    #[test]
    fn elevation_handoff_is_unavailable_when_missing_or_unreadable() {
        let directory = tempfile::tempdir().unwrap();
        let (ticket, session) = handoff_fixture(directory.path());

        assert!(load_elevation_handoff_at(directory.path(), &ticket.operation_id).is_err());

        store_elevation_handoff_at(directory.path(), &ticket, &session).unwrap();
        let path = handoff_path(directory.path(), &ticket.operation_id).unwrap();
        std::fs::write(&path, b"{ not json").unwrap();
        assert!(load_elevation_handoff_at(directory.path(), &ticket.operation_id).is_err());
        std::fs::write(&path, b"").unwrap();
        assert!(load_elevation_handoff_at(directory.path(), &ticket.operation_id).is_err());
    }

    #[test]
    fn elevation_handoff_rejects_an_identifier_that_is_not_a_uuid() {
        let directory = tempfile::tempdir().unwrap();
        prepare_plan_directory(directory.path()).unwrap();
        for id in ["", "..\\..\\evil", "../evil", "not-a-uuid"] {
            assert!(
                load_elevation_handoff_at(directory.path(), id).is_err(),
                "{id:?}"
            );
        }
    }

    #[test]
    fn elevation_handoff_cannot_be_replayed_under_another_operation_id() {
        let directory = tempfile::tempdir().unwrap();
        let (ticket, session) = handoff_fixture(directory.path());
        store_elevation_handoff_at(directory.path(), &ticket, &session).unwrap();

        let other_id = uuid::Uuid::new_v4().to_string();
        std::fs::copy(
            handoff_path(directory.path(), &ticket.operation_id).unwrap(),
            handoff_path(directory.path(), &other_id).unwrap(),
        )
        .unwrap();

        assert!(load_elevation_handoff_at(directory.path(), &other_id).is_err());
    }

    #[test]
    fn tampered_session_fields_that_disagree_with_the_ticket_are_rejected() {
        for field in ["nonce", "plan_operation_id", "plan_digest"] {
            let directory = tempfile::tempdir().unwrap();
            let (ticket, session) = handoff_fixture(directory.path());
            store_elevation_handoff_at(directory.path(), &ticket, &session).unwrap();
            edit_handoff(directory.path(), &ticket.operation_id, |value| {
                value[field] = serde_json::Value::String("tampered".into());
            });

            assert!(
                load_elevation_handoff_at(directory.path(), &ticket.operation_id).is_err(),
                "tampered {field} must be rejected"
            );
        }
    }

    #[test]
    fn untrusted_broker_endpoints_are_rejected() {
        let valid_suffix = uuid::Uuid::new_v4();
        let endpoints = [
            format!(r"\\remote-host\pipe\WinOptimizer-{valid_suffix}"),
            format!(r"\\.\pipe\SomethingElse-{valid_suffix}"),
            r"\\.\pipe\WinOptimizer-not-a-uuid".to_string(),
            format!(r"C:\Users\Public\WinOptimizer-{valid_suffix}"),
            String::new(),
        ];
        for endpoint in endpoints {
            let directory = tempfile::tempdir().unwrap();
            let (ticket, session) = handoff_fixture(directory.path());
            store_elevation_handoff_at(directory.path(), &ticket, &session).unwrap();
            edit_handoff(directory.path(), &ticket.operation_id, |value| {
                value["endpoint"] = serde_json::Value::String(endpoint.clone());
            });

            assert!(
                load_elevation_handoff_at(directory.path(), &ticket.operation_id).is_err(),
                "endpoint {endpoint:?} must be rejected"
            );
        }
    }

    #[test]
    fn a_session_id_that_is_not_a_uuid_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let (ticket, session) = handoff_fixture(directory.path());
        store_elevation_handoff_at(directory.path(), &ticket, &session).unwrap();
        edit_handoff(directory.path(), &ticket.operation_id, |value| {
            value["session_id"] = serde_json::Value::String("session".into());
        });

        assert!(load_elevation_handoff_at(directory.path(), &ticket.operation_id).is_err());
    }
}
