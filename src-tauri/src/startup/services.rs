//! Windows Service manager - list, inspect, and guarded service changes.
//!
//! Windows builds use the Service Control Manager APIs directly. The public
//! `ServiceInfo` shape remains compatible with the existing Startup Manager.

use crate::startup::guidance::GuidanceInfo;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceInfo {
    pub name: String,
    pub display_name: String,
    pub status: String,     // "Running", "Stopped", "Paused"
    pub start_type: String, // "Auto", "Manual", "Disabled"
    pub description: Option<String>,
    /// The configured executable command captured directly from SCM.
    #[serde(default)]
    pub binary_path: Option<String>,
    /// `None` means SCM did not expose the delayed-auto flag for this read.
    #[serde(default)]
    pub delayed_auto_start: Option<bool>,
    /// Ordered dependency names (including SCM dependency-group markers).
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub guidance: GuidanceInfo,
}

#[derive(Debug, Clone)]
struct NativeServiceState {
    name: String,
    display_name: String,
    status: String,
    start_type: String,
    description: Option<String>,
    binary_path: Option<String>,
    delayed_auto_start: Option<bool>,
    dependencies: Vec<String>,
}

impl NativeServiceState {
    fn into_service_info(self) -> ServiceInfo {
        let name = self.name;
        ServiceInfo {
            display_name: self.display_name,
            status: self.status,
            start_type: self.start_type,
            description: self.description,
            binary_path: self.binary_path,
            delayed_auto_start: self.delayed_auto_start,
            dependencies: self.dependencies,
            guidance: crate::startup::guidance::service_guidance(&name),
            name,
        }
    }
}

/// List all services through the Service Control Manager.
pub fn list() -> Vec<ServiceInfo> {
    list_fallible().unwrap_or_default()
}

/// Fallible native service enumeration for infrastructure callers that must
/// preserve SCM failures instead of treating them as an empty inventory.
pub fn list_fallible() -> Result<Vec<ServiceInfo>, String> {
    native::list().map(|services| {
        services
            .into_iter()
            .map(NativeServiceState::into_service_info)
            .collect()
    })
}

/// Set a service's start type through ChangeServiceConfigW.
pub fn set_start_type(
    service_name: &str,
    start_type: &str, // "auto", "demand", "disabled"
) -> Result<(), String> {
    validate_service_name(service_name)?;
    ensure_not_protected(service_name)?;
    validate_start_type(start_type)?;
    ensure_elevated()?;
    let before = inspect(service_name)?;

    if !before.exact_configuration_available() {
        return Err(format!(
            "Exact Service Control Manager configuration is unavailable for {service_name}"
        ));
    }

    native::set_start_type(service_name, start_type)?;
    let after = inspect(service_name)?;
    if !after.start_type.eq_ignore_ascii_case(start_type) {
        return Err(format!(
            "Service Control Manager did not report {start_type} startup for {service_name}"
        ));
    }
    Ok(())
}

/// Restore the SCM delayed-auto-start flag captured for a service.
pub fn set_delayed_auto_start(service_name: &str, delayed: bool) -> Result<(), String> {
    validate_service_name(service_name)?;
    ensure_not_protected(service_name)?;
    ensure_elevated()?;
    let before = inspect(service_name)?;
    if !before.exact_configuration_available() {
        return Err(format!(
            "Exact Service Control Manager configuration is unavailable for {service_name}"
        ));
    }
    native::set_delayed_auto_start(service_name, delayed)?;
    let after = inspect(service_name)?;
    if after.delayed_auto_start != Some(delayed) {
        return Err(format!(
            "Service Control Manager did not report delayed-auto-start={} for {service_name}",
            delayed
        ));
    }
    Ok(())
}

/// Start or stop a service through the Service Control Manager.
pub fn control_service(service_name: &str, action: &str) -> Result<(), String> {
    validate_service_name(service_name)?;
    ensure_not_protected(service_name)?;
    validate_action(action)?;
    ensure_elevated()?;
    let before = inspect(service_name)?;

    if !before.exact_configuration_available() {
        return Err(format!(
            "Exact Service Control Manager configuration is unavailable for {service_name}"
        ));
    }

    native::control_service(service_name, action)
}

/// Control only the runtime state for a governed cleanup coordination guard.
///
/// The surrounding operation journal owns the cleanup mutation record; this
/// adapter deliberately does not create a second legacy undo entry.
pub(crate) fn control_runtime_state_for_cleanup(
    service_name: &str,
    action: &str,
) -> Result<(), String> {
    validate_service_name(service_name)?;
    ensure_not_protected(service_name)?;
    validate_action(action)?;
    ensure_elevated()?;
    native::control_service(service_name, action)
}

fn ensure_elevated() -> Result<(), String> {
    if crate::core::elevation::is_elevated() {
        Ok(())
    } else {
        Err(crate::core::error::WinOptimizerError::ElevationRequired.to_string())
    }
}

fn validate_action(action: &str) -> Result<(), String> {
    match action {
        "start" | "stop" => Ok(()),
        _ => Err(format!("Unsupported service action: {}", action)),
    }
}

fn validate_start_type(start_type: &str) -> Result<(), String> {
    match start_type {
        "auto" | "demand" | "disabled" => Ok(()),
        _ => Err(format!("Unsupported service start type: {}", start_type)),
    }
}

pub(crate) fn validate_service_name(service_name: &str) -> Result<(), String> {
    if service_name.is_empty() || service_name.len() > 256 {
        return Err("Invalid service name".to_string());
    }
    if service_name
        .chars()
        .any(|c| c.is_control() || matches!(c, '\\' | '/'))
    {
        Err("Service name contains unsupported characters".to_string())
    } else {
        Ok(())
    }
}

pub fn inspect(service_name: &str) -> Result<ServiceInfo, String> {
    validate_service_name(service_name)?;
    Ok(native::inspect(service_name)?.into_service_info())
}

/// Re-read only one known service after a mutation.
pub fn inspect_state(service_name: &str, known: &ServiceInfo) -> Result<ServiceInfo, String> {
    validate_service_name(service_name)?;
    let mut refreshed = native::inspect(service_name)?.into_service_info();
    refreshed.guidance = known.guidance.clone();
    Ok(refreshed)
}

impl ServiceInfo {
    /// Compare SCM configuration that these workflows do not intend to change.
    pub(crate) fn configuration_matches(&self, other: &Self) -> bool {
        self.name == other.name
            && self.display_name == other.display_name
            && self.description == other.description
            && self.binary_path == other.binary_path
            && self.delayed_auto_start == other.delayed_auto_start
            && self.dependencies == other.dependencies
    }

    /// Mutations require a complete native SCM configuration snapshot.
    pub(crate) fn exact_configuration_available(&self) -> bool {
        !self.start_type.eq_ignore_ascii_case("unknown") && self.delayed_auto_start.is_some()
    }

    /// Compare the mutable SCM state without coupling legacy plans to the
    /// versioned guidance catalog.
    pub fn state_matches(&self, other: &Self) -> bool {
        self.configuration_matches(other)
            && self.status == other.status
            && self.start_type == other.start_type
    }
}

pub fn wait_for_status(
    service_name: &str,
    expected: &str,
    attempts: usize,
) -> Result<ServiceInfo, String> {
    let attempts = attempts.max(1);
    for attempt in 0..attempts {
        let current = inspect(service_name)?;
        if current.status.eq_ignore_ascii_case(expected) {
            return Ok(current);
        }
        if attempt + 1 < attempts {
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    }
    Err(format!(
        "Service {service_name} did not reach {expected} before timeout"
    ))
}

pub fn is_protected(service_name: &str) -> bool {
    const PROTECTED: &[&str] = &[
        "BFE",
        "CryptSvc",
        "DcomLaunch",
        "EventLog",
        "LSM",
        "PlugPlay",
        "Power",
        "ProfSvc",
        "RpcEptMapper",
        "RpcSs",
        "SamSs",
        "Schedule",
        "SecurityHealthService",
        "Sense",
        "TrustedInstaller",
        "UserManager",
        "WdNisSvc",
        "WinDefend",
        "Winmgmt",
        "MdCoreSvc",
        "mpssvc",
        "wscsvc",
    ];
    PROTECTED
        .iter()
        .any(|protected| protected.eq_ignore_ascii_case(service_name))
}

fn ensure_not_protected(service_name: &str) -> Result<(), String> {
    if is_protected(service_name) {
        Err(format!(
            "Protected Windows service cannot be changed: {service_name}"
        ))
    } else {
        Ok(())
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn status_label(state: u32) -> &'static str {
    match state {
        1 => "Stopped",
        4 => "Running",
        7 => "Paused",
        _ => "Unknown",
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn start_type_label(start_type: u32) -> &'static str {
    match start_type {
        2 => "auto",
        3 => "demand",
        4 => "disabled",
        _ => "unknown",
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn map_service_error(operation: &str, code: u32) -> String {
    let reason = match code {
        5 => "access denied",
        6 => "invalid service handle",
        1051 => "dependent services are running",
        1053 => "the service did not respond to the request in time",
        1056 => "the service is already running",
        1058 => "the service is disabled",
        1060 => "the service does not exist",
        1062 => "the service is not active",
        122 => "the SCM response buffer was too small",
        234 => "the SCM response contains more data",
        _ => "the Service Control Manager rejected the request",
    };
    format!("{operation} failed: {reason} (Windows error {code})")
}

#[cfg(windows)]
mod native {
    use super::{map_service_error, start_type_label, status_label, NativeServiceState};
    use std::mem::size_of;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_MORE_DATA};
    use windows::Win32::System::Services::{
        ChangeServiceConfig2W, ChangeServiceConfigW, CloseServiceHandle, ControlService,
        EnumServicesStatusExW, OpenSCManagerW, OpenServiceW, QueryServiceConfig2W,
        QueryServiceConfigW, QueryServiceStatusEx, StartServiceW, ENUM_SERVICE_STATUS_PROCESSW,
        ENUM_SERVICE_TYPE, SC_ENUM_PROCESS_INFO, SC_HANDLE, SC_MANAGER_CONNECT,
        SC_MANAGER_ENUMERATE_SERVICE, SC_STATUS_PROCESS_INFO, SERVICE_CHANGE_CONFIG,
        SERVICE_CONFIG_DELAYED_AUTO_START_INFO, SERVICE_CONTROL_STOP,
        SERVICE_DELAYED_AUTO_START_INFO, SERVICE_ERROR, SERVICE_NO_CHANGE, SERVICE_QUERY_CONFIG,
        SERVICE_QUERY_STATUS, SERVICE_START, SERVICE_START_TYPE, SERVICE_STATE_ALL, SERVICE_STATUS,
        SERVICE_STATUS_PROCESS, SERVICE_STOP, SERVICE_WIN32,
    };

    const READ_ACCESS: u32 = SERVICE_QUERY_STATUS | SERVICE_QUERY_CONFIG;
    const MUTATE_ACCESS: u32 =
        SERVICE_QUERY_STATUS | SERVICE_QUERY_CONFIG | SERVICE_START | SERVICE_STOP;
    const CONFIG_ACCESS: u32 = SERVICE_QUERY_STATUS | SERVICE_QUERY_CONFIG | SERVICE_CHANGE_CONFIG;

    struct ServiceHandle(SC_HANDLE);

    impl Drop for ServiceHandle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseServiceHandle(self.0);
            }
        }
    }

    fn win32_code(error: &windows::core::Error) -> u32 {
        error.code().0 as u32 & 0xffff
    }

    fn native_error(operation: &str, error: windows::core::Error) -> String {
        let code = win32_code(&error);
        let mapped = map_service_error(operation, code);
        let detail = error.message();
        if detail.trim().is_empty() {
            mapped
        } else {
            format!("{mapped}: {}", detail.trim())
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn open_manager(access: u32) -> Result<ServiceHandle, String> {
        unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), access) }
            .map(ServiceHandle)
            .map_err(|error| native_error("OpenSCManager", error))
    }

    fn open_service(
        manager: &ServiceHandle,
        service_name: &str,
        access: u32,
    ) -> Result<ServiceHandle, String> {
        let service_name = wide(service_name);
        unsafe { OpenServiceW(manager.0, PCWSTR(service_name.as_ptr()), access) }
            .map(ServiceHandle)
            .map_err(|error| native_error("OpenService", error))
    }

    fn decode(value: windows::core::PWSTR) -> String {
        if value.is_null() {
            String::new()
        } else {
            unsafe { value.to_string().unwrap_or_default() }
        }
    }

    fn decode_multi_string(value: windows::core::PWSTR) -> Vec<String> {
        const MAX_DEPENDENCY_CHARS: usize = 32 * 1024;
        if value.is_null() {
            return Vec::new();
        }

        let mut result = Vec::new();
        let mut cursor = value.0;
        let mut consumed = 0;
        while consumed < MAX_DEPENDENCY_CHARS {
            let mut length = 0;
            while consumed + length < MAX_DEPENDENCY_CHARS && unsafe { *cursor.add(length) } != 0 {
                length += 1;
            }
            if length == 0 || consumed + length >= MAX_DEPENDENCY_CHARS {
                break;
            }
            let units = unsafe { std::slice::from_raw_parts(cursor, length) };
            result.push(String::from_utf16_lossy(units));
            consumed += length + 1;
            cursor = unsafe { cursor.add(length + 1) };
        }
        result
    }

    fn query_status(service: &ServiceHandle) -> Result<String, String> {
        let mut status = SERVICE_STATUS_PROCESS::default();
        let mut bytes_needed = 0;
        let buffer = unsafe {
            std::slice::from_raw_parts_mut(
                (&mut status as *mut SERVICE_STATUS_PROCESS).cast::<u8>(),
                size_of::<SERVICE_STATUS_PROCESS>(),
            )
        };
        unsafe {
            QueryServiceStatusEx(
                service.0,
                SC_STATUS_PROCESS_INFO,
                Some(buffer),
                &mut bytes_needed,
            )
        }
        .map_err(|error| native_error("QueryServiceStatusEx", error))?;
        Ok(status_label(status.dwCurrentState.0).to_string())
    }

    struct NativeServiceConfig {
        display_name: String,
        start_type: String,
        description: Option<String>,
        binary_path: Option<String>,
        dependencies: Vec<String>,
        delayed_auto_start: Option<bool>,
    }

    fn query_delayed_auto_start(service: &ServiceHandle) -> Result<Option<bool>, String> {
        let mut info = SERVICE_DELAYED_AUTO_START_INFO::default();
        let mut bytes_needed = 0;
        let buffer = unsafe {
            std::slice::from_raw_parts_mut(
                (&mut info as *mut SERVICE_DELAYED_AUTO_START_INFO).cast::<u8>(),
                size_of::<SERVICE_DELAYED_AUTO_START_INFO>(),
            )
        };
        unsafe {
            QueryServiceConfig2W(
                service.0,
                SERVICE_CONFIG_DELAYED_AUTO_START_INFO,
                Some(buffer),
                &mut bytes_needed,
            )
        }
        .map_err(|error| native_error("QueryServiceConfig2W(delayed auto-start)", error))?;
        Ok(Some(info.fDelayedAutostart.0 != 0))
    }

    fn query_description(service: &ServiceHandle) -> Result<Option<String>, String> {
        let mut bytes_needed = 0;
        let initial = unsafe {
            QueryServiceConfig2W(
                service.0,
                windows::Win32::System::Services::SERVICE_CONFIG_DESCRIPTION,
                None,
                &mut bytes_needed,
            )
        };
        if let Err(error) = initial {
            let code = win32_code(&error);
            if code != ERROR_INSUFFICIENT_BUFFER.0 {
                return Err(native_error("QueryServiceConfig2W(description)", error));
            }
        }
        if bytes_needed == 0 {
            return Ok(None);
        }

        let word_count = (bytes_needed as usize).div_ceil(size_of::<usize>());
        let mut storage = vec![0usize; word_count];
        let buffer = unsafe {
            std::slice::from_raw_parts_mut(
                storage.as_mut_ptr().cast::<u8>(),
                storage.len() * size_of::<usize>(),
            )
        };
        unsafe {
            QueryServiceConfig2W(
                service.0,
                windows::Win32::System::Services::SERVICE_CONFIG_DESCRIPTION,
                Some(buffer),
                &mut bytes_needed,
            )
        }
        .map_err(|error| native_error("QueryServiceConfig2W(description)", error))?;
        let info = unsafe {
            std::ptr::read(
                storage
                    .as_ptr()
                    .cast::<windows::Win32::System::Services::SERVICE_DESCRIPTIONW>(),
            )
        };
        let description = decode(info.lpDescription);
        Ok((!description.is_empty()).then_some(description))
    }

    fn query_config(service: &ServiceHandle) -> Result<NativeServiceConfig, String> {
        let mut bytes_needed = 0;
        let initial = unsafe { QueryServiceConfigW(service.0, None, 0, &mut bytes_needed) };
        if let Err(error) = initial {
            let code = win32_code(&error);
            if code != ERROR_INSUFFICIENT_BUFFER.0 {
                return Err(native_error("QueryServiceConfigW", error));
            }
        }

        let struct_size = size_of::<windows::Win32::System::Services::QUERY_SERVICE_CONFIGW>();
        let count = (bytes_needed as usize)
            .max(struct_size)
            .div_ceil(struct_size);
        let mut buffer =
            vec![windows::Win32::System::Services::QUERY_SERVICE_CONFIGW::default(); count];
        let mut bytes_needed = 0;
        unsafe {
            QueryServiceConfigW(
                service.0,
                Some(buffer.as_mut_ptr()),
                (buffer.len() * struct_size) as u32,
                &mut bytes_needed,
            )
        }
        .map_err(|error| native_error("QueryServiceConfigW", error))?;
        let config = buffer[0];
        let display_name = decode(config.lpDisplayName);
        Ok(NativeServiceConfig {
            display_name,
            start_type: start_type_label(config.dwStartType.0).to_string(),
            description: query_description(service)?,
            binary_path: match decode(config.lpBinaryPathName).as_str() {
                "" => None,
                value => Some(value.to_string()),
            },
            dependencies: decode_multi_string(config.lpDependencies),
            delayed_auto_start: query_delayed_auto_start(service)?,
        })
    }

    fn inspect_open_service(
        service_name: &str,
        service: &ServiceHandle,
    ) -> Result<NativeServiceState, String> {
        let status = query_status(service)?;
        let config = query_config(service)?;
        Ok(NativeServiceState {
            name: service_name.to_string(),
            display_name: if config.display_name.is_empty() {
                service_name.to_string()
            } else {
                config.display_name
            },
            status,
            start_type: config.start_type,
            description: config.description,
            binary_path: config.binary_path,
            delayed_auto_start: config.delayed_auto_start,
            dependencies: config.dependencies,
        })
    }

    pub(super) fn inspect(service_name: &str) -> Result<NativeServiceState, String> {
        let manager = open_manager(SC_MANAGER_CONNECT)?;
        let service = open_service(&manager, service_name, READ_ACCESS)?;
        inspect_open_service(service_name, &service)
    }

    pub(super) fn list() -> Result<Vec<NativeServiceState>, String> {
        let manager = open_manager(SC_MANAGER_CONNECT | SC_MANAGER_ENUMERATE_SERVICE)?;
        let entry_size = size_of::<ENUM_SERVICE_STATUS_PROCESSW>();
        let mut buffer = vec![ENUM_SERVICE_STATUS_PROCESSW::default(); 256];
        let mut resume = 0;
        let mut result = Vec::new();

        loop {
            let mut bytes_needed = 0;
            let mut returned = 0;
            let call = unsafe {
                let bytes = std::slice::from_raw_parts_mut(
                    buffer.as_mut_ptr().cast::<u8>(),
                    buffer.len() * entry_size,
                );
                EnumServicesStatusExW(
                    manager.0,
                    SC_ENUM_PROCESS_INFO,
                    SERVICE_WIN32,
                    SERVICE_STATE_ALL,
                    Some(bytes),
                    &mut bytes_needed,
                    &mut returned,
                    Some(&mut resume),
                    PCWSTR::null(),
                )
            };

            let available = returned as usize;
            for entry in buffer.iter().take(available) {
                let name = decode(entry.lpServiceName);
                let listed_display_name = decode(entry.lpDisplayName);
                let config = match open_service(&manager, &name, READ_ACCESS)
                    .and_then(|service| query_config(&service))
                {
                    Ok(config) => {
                        let display_name = if config.display_name.is_empty() {
                            listed_display_name.clone()
                        } else {
                            config.display_name.clone()
                        };
                        (display_name, config)
                    }
                    Err(_) => (
                        listed_display_name,
                        NativeServiceConfig {
                            display_name: String::new(),
                            start_type: "unknown".into(),
                            description: None,
                            binary_path: None,
                            dependencies: Vec::new(),
                            delayed_auto_start: None,
                        },
                    ),
                };
                result.push(NativeServiceState {
                    name,
                    display_name: config.0,
                    status: status_label(entry.ServiceStatusProcess.dwCurrentState.0).into(),
                    start_type: config.1.start_type,
                    description: config.1.description,
                    binary_path: config.1.binary_path,
                    delayed_auto_start: config.1.delayed_auto_start,
                    dependencies: config.1.dependencies,
                });
            }

            match call {
                Ok(()) => break,
                Err(error) => {
                    let code = win32_code(&error);
                    if code != ERROR_MORE_DATA.0 && code != ERROR_INSUFFICIENT_BUFFER.0 {
                        return Err(native_error("EnumServicesStatusExW", error));
                    }
                    let required = (bytes_needed as usize).max(entry_size);
                    let count = required.div_ceil(entry_size);
                    buffer.resize(
                        count.max(buffer.len() * 2),
                        ENUM_SERVICE_STATUS_PROCESSW::default(),
                    );
                }
            }
        }

        Ok(result)
    }

    pub(super) fn set_start_type(service_name: &str, start_type: &str) -> Result<(), String> {
        let start_type = match start_type {
            "auto" => 2,
            "demand" => 3,
            "disabled" => 4,
            _ => return Err(format!("Unsupported service start type: {start_type}")),
        };
        let manager = open_manager(SC_MANAGER_CONNECT)?;
        let service = open_service(&manager, service_name, CONFIG_ACCESS)?;
        unsafe {
            ChangeServiceConfigW(
                service.0,
                ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE),
                SERVICE_START_TYPE(start_type),
                SERVICE_ERROR(SERVICE_NO_CHANGE),
                PCWSTR::null(),
                PCWSTR::null(),
                None,
                PCWSTR::null(),
                PCWSTR::null(),
                PCWSTR::null(),
                PCWSTR::null(),
            )
        }
        .map_err(|error| native_error("ChangeServiceConfigW", error))
    }

    pub(super) fn set_delayed_auto_start(service_name: &str, delayed: bool) -> Result<(), String> {
        let manager = open_manager(SC_MANAGER_CONNECT)?;
        let service = open_service(&manager, service_name, CONFIG_ACCESS)?;
        let info = SERVICE_DELAYED_AUTO_START_INFO {
            fDelayedAutostart: delayed.into(),
        };
        unsafe {
            ChangeServiceConfig2W(
                service.0,
                SERVICE_CONFIG_DELAYED_AUTO_START_INFO,
                Some((&info as *const SERVICE_DELAYED_AUTO_START_INFO).cast()),
            )
        }
        .map_err(|error| native_error("ChangeServiceConfig2W(delayed auto-start)", error))
    }

    pub(super) fn control_service(service_name: &str, action: &str) -> Result<(), String> {
        let manager = open_manager(SC_MANAGER_CONNECT)?;
        let service = open_service(&manager, service_name, MUTATE_ACCESS)?;
        match action {
            "start" => unsafe { StartServiceW(service.0, None) }
                .map_err(|error| native_error("StartServiceW", error)),
            "stop" => {
                let mut status = SERVICE_STATUS::default();
                unsafe { ControlService(service.0, SERVICE_CONTROL_STOP, &mut status) }
                    .map_err(|error| native_error("ControlService(STOP)", error))
            }
            _ => Err(format!("Unsupported service action: {action}")),
        }
    }
}

#[cfg(not(windows))]
mod native {
    use super::NativeServiceState;

    pub(super) fn list() -> Result<Vec<NativeServiceState>, String> {
        Err("Native Service Control Manager access is only available on Windows".into())
    }

    pub(super) fn inspect(_service_name: &str) -> Result<NativeServiceState, String> {
        Err("Windows Service Control Manager is unavailable on this platform".into())
    }

    pub(super) fn set_start_type(_service_name: &str, _start_type: &str) -> Result<(), String> {
        Err("Windows Service Control Manager is unavailable on this platform".into())
    }

    pub(super) fn set_delayed_auto_start(
        _service_name: &str,
        _delayed: bool,
    ) -> Result<(), String> {
        Err("Windows Service Control Manager is unavailable on this platform".into())
    }

    pub(super) fn control_service(_service_name: &str, _action: &str) -> Result<(), String> {
        Err("Windows Service Control Manager is unavailable on this platform".into())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ensure_not_protected, map_service_error, start_type_label, status_label,
        validate_service_name, ServiceInfo,
    };

    #[test]
    fn protected_services_are_rejected_before_scm_access() {
        let error = ensure_not_protected("rpcss").unwrap_err();
        assert!(error.contains("Protected Windows service"));
    }

    #[test]
    fn defender_services_are_protected_before_scm_access() {
        for service in ["Sense", "WdNisSvc", "MdCoreSvc", "wscsvc"] {
            let error = ensure_not_protected(service).unwrap_err();
            assert!(error.contains("Protected Windows service"), "{service}");
        }
    }

    #[test]
    fn accepts_valid_scm_names_with_spaces() {
        assert!(validate_service_name("Steam Client Service").is_ok());
        assert!(validate_service_name("Vendor.Service-2").is_ok());
    }

    #[test]
    fn rejects_control_characters_and_path_separators() {
        assert!(validate_service_name("Steam\nClient Service").is_err());
        assert!(validate_service_name("Vendor\\Service").is_err());
        assert!(validate_service_name("Vendor/Service").is_err());
    }

    #[test]
    fn native_state_labels_preserve_public_values() {
        assert_eq!(status_label(1), "Stopped");
        assert_eq!(status_label(4), "Running");
        assert_eq!(status_label(7), "Paused");
        assert_eq!(status_label(3), "Unknown");
        assert_eq!(start_type_label(2), "auto");
        assert_eq!(start_type_label(3), "demand");
        assert_eq!(start_type_label(4), "disabled");
        assert_eq!(start_type_label(1), "unknown");
    }

    #[test]
    fn native_error_mapping_is_actionable() {
        assert!(map_service_error("StartServiceW", 5).contains("access denied"));
        assert!(map_service_error("OpenService", 1060).contains("does not exist"));
        assert!(map_service_error("ControlService", 1051).contains("dependent services"));
        assert!(map_service_error("ChangeServiceConfigW", 9999).contains("rejected"));
    }

    #[test]
    fn native_configuration_is_part_of_stale_state_revalidation() {
        let base = ServiceInfo {
            name: "FixtureSvc".into(),
            display_name: "Fixture Service".into(),
            status: "Stopped".into(),
            start_type: "auto".into(),
            description: None,
            binary_path: Some(r#"C:\Fixture\service.exe"#.into()),
            delayed_auto_start: Some(true),
            dependencies: vec!["RpcSs".into(), "+NetworkProvider".into()],
            guidance: Default::default(),
        };

        let mut changed_binary = base.clone();
        changed_binary.binary_path = Some(r#"C:\Fixture\other.exe"#.into());
        assert!(!base.state_matches(&changed_binary));

        let mut changed_delay = base.clone();
        changed_delay.delayed_auto_start = Some(false);
        assert!(!base.state_matches(&changed_delay));

        let mut changed_dependencies = base.clone();
        changed_dependencies.dependencies.push("Dnscache".into());
        assert!(!base.state_matches(&changed_dependencies));
    }

    #[test]
    fn incomplete_native_configuration_is_not_mutation_safe() {
        let mut incomplete = ServiceInfo {
            name: "FixtureSvc".into(),
            display_name: "Fixture Service".into(),
            status: "Stopped".into(),
            start_type: "auto".into(),
            description: None,
            binary_path: None,
            delayed_auto_start: None,
            dependencies: Vec::new(),
            guidance: Default::default(),
        };
        assert!(!incomplete.exact_configuration_available());

        incomplete.start_type = "unknown".into();
        incomplete.delayed_auto_start = Some(false);
        assert!(!incomplete.exact_configuration_available());
    }
}
