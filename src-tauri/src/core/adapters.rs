//! Deep seams for host-bound operations.
//!
//! These interfaces deliberately expose values and DTOs rather than Win32
//! handles, COM objects, or `std::process::Command`.

use crate::startup::guidance::GuidanceInfo;
use crate::startup::{programs, scheduler, services};
use crate::system_info::{battery, env_vars, hardware, network, os};
use std::collections::BTreeMap;
use std::process::Output;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterErrorCategory {
    InvalidInput,
    NotFound,
    AccessDenied,
    Native,
    Unsupported,
    Verification,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterError {
    pub category: AdapterErrorCategory,
    pub context: String,
    pub native_code: Option<i32>,
    pub message: String,
}

impl AdapterError {
    fn new(
        category: AdapterErrorCategory,
        context: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            category,
            context: context.into(),
            native_code: None,
            message: message.into(),
        }
    }
    fn from_message(context: impl Into<String>, message: impl Into<String>) -> Self {
        let context = context.into();
        let message = message.into();
        let lower = message.to_ascii_lowercase();
        let category = if lower.contains("not found") || lower.contains("does not exist") {
            AdapterErrorCategory::NotFound
        } else if lower.contains("denied") || lower.contains("access is denied") {
            AdapterErrorCategory::AccessDenied
        } else if (lower.contains("unsupported") && lower.contains("platform"))
            || (lower.contains("only available") && lower.contains("windows"))
        {
            AdapterErrorCategory::Unsupported
        } else if lower.contains("invalid") || lower.contains("unsupported") {
            AdapterErrorCategory::InvalidInput
        } else if lower.contains("verification") || lower.contains("exact") {
            AdapterErrorCategory::Verification
        } else {
            AdapterErrorCategory::Native
        };
        let native_code = message.find("0x").and_then(|start| {
            message
                .get(start + 2..start + 10)
                .and_then(|code| u32::from_str_radix(code, 16).ok())
                .map(|code| code as i32)
        });
        Self {
            category,
            context,
            native_code,
            message,
        }
    }
}
impl std::fmt::Display for AdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.context, self.message)
    }
}
impl std::error::Error for AdapterError {}
impl From<String> for AdapterError {
    fn from(message: String) -> Self {
        Self::from_message("adapter", message)
    }
}
impl From<&str> for AdapterError {
    fn from(message: &str) -> Self {
        Self::from_message("adapter", message)
    }
}
pub type AdapterResult<T> = Result<T, AdapterError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceAction {
    Start,
    Stop,
    Pause,
    Continue,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceStartType {
    Auto,
    Demand,
    Manual,
    Disabled,
}
impl ServiceStartType {
    fn as_native(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Demand | Self::Manual => "demand",
            Self::Disabled => "disabled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceInfo {
    pub name: String,
    pub display_name: String,
    pub status: String,
    pub start_type: String,
    pub description: Option<String>,
    pub binary_path: Option<String>,
    pub delayed_auto_start: Option<bool>,
    pub dependencies: Vec<String>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskDefinitionSnapshot {
    pub task_path: String,
    pub enabled: bool,
    pub current_state: String,
    pub next_run_time: Option<String>,
    pub definition_xml: String,
    pub triggers_xml: String,
    pub actions_xml: String,
    pub principals_xml: String,
    pub run_level: String,
    pub security_descriptor: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledTask {
    pub name: String,
    pub path: String,
    pub status: String,
    pub trigger: String,
    pub classification: String,
    pub can_modify: bool,
    pub recommendation: String,
    pub guidance: GuidanceInfo,
    pub definition: TaskDefinitionSnapshot,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
    pub scope: String,
    pub var_type: String,
}
#[derive(Debug, Clone, Default, PartialEq)]
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
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HardwareInfo {
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub disks: Vec<DiskInfo>,
    pub gpu: Option<GpuInfo>,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CpuInfo {
    pub name: String,
    pub cores: u32,
    pub logical_processors: u32,
    pub architecture: String,
    pub usage_percent: f64,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MemoryInfo {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub in_use_percent: f64,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiskInfo {
    pub name: String,
    pub mount_point: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub file_system: String,
    pub is_ssd: bool,
    pub disk_type: String,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GpuInfo {
    pub name: String,
    pub adapter_ram: u64,
    pub driver_version: String,
}
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BatteryInfo {
    pub present: bool,
    pub charge_percent: f32,
    pub charger_connected: bool,
    pub charging: bool,
    pub time_remaining_minutes: i32,
    pub saving_mode: bool,
    pub is_laptop: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkInfo {
    pub adapters: Vec<NetworkAdapter>,
    pub connections: Vec<TcpConnection>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NetworkAdapter {
    pub name: String,
    pub description: String,
    pub ip_addresses: Vec<String>,
    pub mac_address: String,
    pub gateway: String,
    pub dhcp_server: String,
    pub dns_servers: Vec<String>,
    pub operational: bool,
    pub mtu: u32,
    pub link_speed_mbps: u64,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TcpConnection {
    pub local_addr: String,
    pub local_port: u16,
    pub remote_addr: String,
    pub remote_port: u16,
    pub state: String,
    pub pid: u32,
    pub process_name: String,
}

impl From<services::ServiceInfo> for ServiceInfo {
    fn from(v: services::ServiceInfo) -> Self {
        Self {
            name: v.name,
            display_name: v.display_name,
            status: v.status,
            start_type: v.start_type,
            description: v.description,
            binary_path: v.binary_path,
            delayed_auto_start: v.delayed_auto_start,
            dependencies: v.dependencies,
        }
    }
}
impl From<ServiceInfo> for services::ServiceInfo {
    fn from(v: ServiceInfo) -> Self {
        Self {
            name: v.name,
            display_name: v.display_name,
            status: v.status,
            start_type: v.start_type,
            description: v.description,
            binary_path: v.binary_path,
            delayed_auto_start: v.delayed_auto_start,
            dependencies: v.dependencies,
            guidance: GuidanceInfo::default(),
        }
    }
}
impl From<scheduler::TaskDefinitionSnapshot> for TaskDefinitionSnapshot {
    fn from(v: scheduler::TaskDefinitionSnapshot) -> Self {
        Self {
            task_path: v.task_path,
            enabled: v.enabled,
            current_state: v.current_state,
            next_run_time: v.next_run_time,
            definition_xml: v.definition_xml,
            triggers_xml: v.triggers_xml,
            actions_xml: v.actions_xml,
            principals_xml: v.principals_xml,
            run_level: v.run_level,
            security_descriptor: v.security_descriptor,
        }
    }
}
impl From<&TaskDefinitionSnapshot> for scheduler::TaskDefinitionSnapshot {
    fn from(v: &TaskDefinitionSnapshot) -> Self {
        Self {
            task_path: v.task_path.clone(),
            enabled: v.enabled,
            current_state: v.current_state.clone(),
            next_run_time: v.next_run_time.clone(),
            definition_xml: v.definition_xml.clone(),
            triggers_xml: v.triggers_xml.clone(),
            actions_xml: v.actions_xml.clone(),
            principals_xml: v.principals_xml.clone(),
            run_level: v.run_level.clone(),
            security_descriptor: v.security_descriptor.clone(),
        }
    }
}
impl From<scheduler::ScheduledTask> for ScheduledTask {
    fn from(v: scheduler::ScheduledTask) -> Self {
        Self {
            name: v.name,
            path: v.path,
            status: v.status,
            trigger: v.trigger,
            classification: v.classification,
            can_modify: v.can_modify,
            recommendation: v.recommendation,
            guidance: v.guidance,
            definition: v.definition.into(),
        }
    }
}
impl From<ScheduledTask> for scheduler::ScheduledTask {
    fn from(v: ScheduledTask) -> Self {
        Self {
            name: v.name,
            path: v.path,
            status: v.status,
            trigger: v.trigger,
            classification: v.classification,
            can_modify: v.can_modify,
            recommendation: v.recommendation,
            guidance: v.guidance,
            definition: (&v.definition).into(),
        }
    }
}
impl From<os::OsInfo> for OsInfo {
    fn from(v: os::OsInfo) -> Self {
        Self {
            version: v.version,
            edition: v.edition,
            build: v.build,
            arch: v.arch,
            release_id: v.release_id,
            install_date: v.install_date,
            last_boot: v.last_boot,
            user_name: v.user_name,
            domain: v.domain,
        }
    }
}
impl From<hardware::HardwareInfo> for HardwareInfo {
    fn from(v: hardware::HardwareInfo) -> Self {
        Self {
            cpu: v.cpu.into(),
            memory: v.memory.into(),
            disks: v.disks.into_iter().map(Into::into).collect(),
            gpu: v.gpu.map(Into::into),
        }
    }
}
impl From<hardware::CpuInfo> for CpuInfo {
    fn from(v: hardware::CpuInfo) -> Self {
        Self {
            name: v.name,
            cores: v.cores,
            logical_processors: v.logical_processors,
            architecture: v.architecture,
            usage_percent: v.usage_percent,
        }
    }
}
impl From<hardware::MemoryInfo> for MemoryInfo {
    fn from(v: hardware::MemoryInfo) -> Self {
        Self {
            total_bytes: v.total_bytes,
            available_bytes: v.available_bytes,
            in_use_percent: v.in_use_percent,
        }
    }
}
impl From<hardware::DiskInfo> for DiskInfo {
    fn from(v: hardware::DiskInfo) -> Self {
        Self {
            name: v.name,
            mount_point: v.mount_point,
            total_bytes: v.total_bytes,
            available_bytes: v.available_bytes,
            file_system: v.file_system,
            is_ssd: v.is_ssd,
            disk_type: v.disk_type,
        }
    }
}
impl From<hardware::GpuInfo> for GpuInfo {
    fn from(v: hardware::GpuInfo) -> Self {
        Self {
            name: v.name,
            adapter_ram: v.adapter_ram,
            driver_version: v.driver_version,
        }
    }
}
impl From<battery::BatteryInfo> for BatteryInfo {
    fn from(v: battery::BatteryInfo) -> Self {
        Self {
            present: v.present,
            charge_percent: v.charge_percent,
            charger_connected: v.charger_connected,
            charging: v.charging,
            time_remaining_minutes: v.time_remaining_minutes,
            saving_mode: v.saving_mode,
            is_laptop: v.is_laptop,
        }
    }
}
impl From<network::NetworkInfo> for NetworkInfo {
    fn from(v: network::NetworkInfo) -> Self {
        Self {
            adapters: v.adapters.into_iter().map(Into::into).collect(),
            connections: v.connections.into_iter().map(Into::into).collect(),
        }
    }
}
impl From<network::NetworkAdapter> for NetworkAdapter {
    fn from(v: network::NetworkAdapter) -> Self {
        Self {
            name: v.name,
            description: v.description,
            ip_addresses: v.ip_addresses,
            mac_address: v.mac_address,
            gateway: v.gateway,
            dhcp_server: v.dhcp_server,
            dns_servers: v.dns_servers,
            operational: v.operational,
            mtu: v.mtu,
            link_speed_mbps: v.link_speed_mbps,
        }
    }
}
impl From<network::TcpConnection> for TcpConnection {
    fn from(v: network::TcpConnection) -> Self {
        Self {
            local_addr: v.local_addr,
            local_port: v.local_port,
            remote_addr: v.remote_addr,
            remote_port: v.remote_port,
            state: v.state,
            pid: v.pid,
            process_name: v.process_name,
        }
    }
}
impl From<env_vars::EnvVar> for EnvVar {
    fn from(v: env_vars::EnvVar) -> Self {
        Self {
            name: v.name,
            value: v.value,
            scope: v.scope,
            var_type: v.var_type,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryValue {
    String(String),
    Dword(u32),
    Raw { value_type: u32, bytes: Vec<u8> },
}

pub trait RegistryAdapter: Send + Sync {
    fn list(&self, key: &str) -> AdapterResult<Vec<(String, RegistryValue)>>;
    fn read(&self, key: &str, value: &str) -> AdapterResult<Option<RegistryValue>>;
    fn write(&self, key: &str, value: &str, data: RegistryValue) -> AdapterResult<()>;
    fn delete_value(&self, key: &str, value: &str) -> AdapterResult<()>;
    fn delete_key(&self, key: &str) -> AdapterResult<()>;
}

pub trait StartupProgramAdapter: Send + Sync {
    fn list(&self) -> AdapterResult<Vec<programs::StartupProgram>>;
    fn find_by_id(&self, id: &str) -> AdapterResult<programs::StartupProgram>;
    fn list_with_recovery(
        &self,
        recovery_store: &crate::core::startup_recovery::StartupRecoveryStore,
    ) -> AdapterResult<Vec<programs::StartupProgram>>;
    fn find_by_id_with_recovery(
        &self,
        id: &str,
        recovery_store: &crate::core::startup_recovery::StartupRecoveryStore,
    ) -> AdapterResult<programs::StartupProgram>;
    fn find_recovery_entry_for_program(
        &self,
        program: &programs::StartupProgram,
        recovery_store: &crate::core::startup_recovery::StartupRecoveryStore,
    ) -> AdapterResult<crate::core::undo_log::UndoEntry>;
    fn disable(
        &self,
        program: &programs::StartupProgram,
        recovery_store: &crate::core::startup_recovery::StartupRecoveryStore,
        session_id: &str,
        backup_mgr: &crate::core::backup::BackupManager,
    ) -> AdapterResult<programs::StartupMutationResult>;
}

pub trait ServiceAdapter: Send + Sync {
    fn list(&self) -> AdapterResult<Vec<ServiceInfo>>;
    fn inspect(&self, name: &str) -> AdapterResult<ServiceInfo>;
    fn set_start_type(&self, name: &str, start_type: ServiceStartType) -> AdapterResult<()>;
    fn set_delayed_auto_start(&self, name: &str, delayed: bool) -> AdapterResult<()>;
    fn control(&self, name: &str, action: ServiceAction) -> AdapterResult<()>;
    fn wait_for_status(&self, name: &str, status: &str, timeout_seconds: u64) -> AdapterResult<()>;
}

pub trait TaskAdapter: Send + Sync {
    fn list(&self) -> AdapterResult<Vec<ScheduledTask>>;
    fn find(&self, name: &str) -> AdapterResult<ScheduledTask>;
    fn set_enabled(&self, name: &str, enabled: bool) -> AdapterResult<()>;
    fn capture_definition(&self, name: &str) -> AdapterResult<TaskDefinitionSnapshot>;
    fn restore_definition(&self, definition: &TaskDefinitionSnapshot) -> AdapterResult<()>;
}

pub trait ProcessAdapter: Send + Sync {
    fn snapshot(&self) -> Vec<crate::processes::snapshots::ProcessInfo>;
    fn requires_elevation(&self, pid: u32) -> bool;
    fn kill(
        &self,
        pid: u32,
        expected_start_time: u64,
        expected_name: &str,
        expected_path: Option<&str>,
    ) -> AdapterResult<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub status: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub trait CommandAdapter: Send + Sync {
    fn execute(&self, program: &str, args: &[String]) -> AdapterResult<CommandOutput>;
}

pub trait ElevationAdapter: Send + Sync {
    fn is_elevated(&self) -> bool;
}

pub trait SystemInfoAdapter: Send + Sync {
    fn os(&self) -> OsInfo;
    fn hardware(&self) -> HardwareInfo;
    fn battery(&self) -> BatteryInfo;
    fn network(&self) -> NetworkInfo;
    fn environment(&self) -> AdapterResult<Vec<EnvVar>>;
}

pub struct NativeRegistryAdapter;
impl RegistryAdapter for NativeRegistryAdapter {
    fn list(&self, key: &str) -> AdapterResult<Vec<(String, RegistryValue)>> {
        let scope = if key == crate::system_info::env_vars::SYSTEM_ENVIRONMENT_KEY {
            "system"
        } else {
            "user"
        };
        crate::system_info::env_vars::list()
            .map(|vars| {
                vars.into_iter()
                    .filter(|v| v.scope == scope)
                    .map(|v| {
                        let value_type = if v.var_type == "REG_EXPAND_SZ" { 2 } else { 1 };
                        let mut bytes = v
                            .value
                            .encode_utf16()
                            .flat_map(u16::to_le_bytes)
                            .collect::<Vec<_>>();
                        bytes.extend_from_slice(&[0, 0]);
                        (v.name, RegistryValue::Raw { value_type, bytes })
                    })
                    .collect()
            })
            .map_err(AdapterError::from)
    }
    fn read(&self, key: &str, value: &str) -> AdapterResult<Option<RegistryValue>> {
        #[cfg(windows)]
        {
            let (hive, path) = crate::core::registry_helpers::parse_hive_prefix(key)?;
            let Some(handle) =
                (unsafe { crate::core::registry_helpers::open_key_read(hive, &path) })
            else {
                return Ok(None);
            };
            let result = unsafe { crate::core::registry_helpers::read_reg_raw(handle, value) }.map(
                |(value_type, bytes)| {
                    if value_type == 4 && bytes.len() >= 4 {
                        RegistryValue::Dword(u32::from_ne_bytes(bytes[..4].try_into().unwrap()))
                    } else if value_type == 1 || value_type == 2 {
                        unsafe { crate::core::registry_helpers::read_reg_string(handle, value) }
                            .map(RegistryValue::String)
                            .unwrap_or(RegistryValue::Raw { value_type, bytes })
                    } else {
                        RegistryValue::Raw { value_type, bytes }
                    }
                },
            );
            unsafe {
                crate::core::registry_helpers::close_key(handle);
            }
            Ok(result)
        }
        #[cfg(not(windows))]
        {
            let scope = if key == crate::system_info::env_vars::SYSTEM_ENVIRONMENT_KEY {
                "system"
            } else {
                "user"
            };
            crate::system_info::env_vars::read_scoped(value, scope)
                .map(|v| v.map(|v| RegistryValue::String(v.value)))
                .map_err(AdapterError::from)
        }
    }
    fn write(&self, key: &str, value: &str, data: RegistryValue) -> AdapterResult<()> {
        #[cfg(windows)]
        {
            let (hive, path) = crate::core::registry_helpers::parse_hive_prefix(key)?;
            let handle = unsafe { crate::core::registry_helpers::create_key_write(hive, &path) }
                .ok_or("Unable to open registry key")?;
            let result = unsafe {
                match data {
                    RegistryValue::String(s) => {
                        crate::core::registry_helpers::set_reg_string(handle, value, &s)
                            .map_err(|e| e.to_string())
                    }
                    RegistryValue::Dword(n) => {
                        crate::core::registry_helpers::set_reg_dword(handle, value, n)
                            .map_err(|e| e.to_string())
                    }
                    RegistryValue::Raw { value_type, bytes } => {
                        crate::core::registry_helpers::set_reg_raw(
                            handle, value, value_type, &bytes,
                        )
                        .map_err(|e| e.to_string())
                    }
                }
            };
            unsafe {
                crate::core::registry_helpers::close_key(handle);
            }
            result.map_err(AdapterError::from)
        }
        #[cfg(not(windows))]
        {
            let scope = if key == crate::system_info::env_vars::SYSTEM_ENVIRONMENT_KEY {
                "system"
            } else {
                "user"
            };
            let RegistryValue::Raw { value_type, bytes } = data else {
                return Err("environment adapter requires string data".into());
            };
            let units = bytes
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .take_while(|c| *c != 0)
                .collect::<Vec<_>>();
            let variable = crate::system_info::env_vars::EnvVar {
                name: value.into(),
                value: String::from_utf16_lossy(&units),
                scope: scope.into(),
                var_type: if value_type == 2 {
                    "REG_EXPAND_SZ"
                } else {
                    "REG_SZ"
                }
                .into(),
            };
            crate::system_info::env_vars::restore(&variable).map_err(AdapterError::from)
        }
    }
    fn delete_value(&self, key: &str, value: &str) -> AdapterResult<()> {
        #[cfg(windows)]
        {
            let (hive, path) = crate::core::registry_helpers::parse_hive_prefix(key)?;
            let handle = unsafe { crate::core::registry_helpers::open_key_write(hive, &path) }
                .ok_or("Unable to open registry key")?;
            let result = unsafe { crate::core::registry_helpers::delete_value(handle, value) }
                .map_err(|e| e.to_string());
            unsafe { crate::core::registry_helpers::close_key(handle) };
            result.map_err(AdapterError::from)
        }
        #[cfg(not(windows))]
        {
            let scope = if key == crate::system_info::env_vars::SYSTEM_ENVIRONMENT_KEY {
                "system"
            } else {
                "user"
            };
            crate::system_info::env_vars::delete_without_undo(value, scope)
                .map_err(AdapterError::from)
        }
    }
    fn delete_key(&self, key: &str) -> AdapterResult<()> {
        #[cfg(windows)]
        {
            let (hive, path) = crate::core::registry_helpers::parse_hive_prefix(key)?;
            unsafe { crate::core::registry_helpers::delete_key_recursive(hive, &path) }
                .map_err(|e| AdapterError::from(e.to_string()))
        }
        #[cfg(not(windows))]
        {
            let _ = key;
            Err("Native registry access is only available on Windows".into())
        }
    }
}

pub struct NativeStartupProgramAdapter;
impl StartupProgramAdapter for NativeStartupProgramAdapter {
    fn list(&self) -> AdapterResult<Vec<programs::StartupProgram>> {
        Ok(programs::list())
    }
    fn find_by_id(&self, id: &str) -> AdapterResult<programs::StartupProgram> {
        programs::find_by_id(id).map_err(|e| AdapterError::from(e.to_string()))
    }
    fn list_with_recovery(
        &self,
        store: &crate::core::startup_recovery::StartupRecoveryStore,
    ) -> AdapterResult<Vec<programs::StartupProgram>> {
        programs::list_with_recovery(store).map_err(|e| AdapterError::from(e.to_string()))
    }
    fn find_by_id_with_recovery(
        &self,
        id: &str,
        store: &crate::core::startup_recovery::StartupRecoveryStore,
    ) -> AdapterResult<programs::StartupProgram> {
        programs::find_by_id_with_recovery(id, store).map_err(|e| AdapterError::from(e.to_string()))
    }
    fn find_recovery_entry_for_program(
        &self,
        program: &programs::StartupProgram,
        store: &crate::core::startup_recovery::StartupRecoveryStore,
    ) -> AdapterResult<crate::core::undo_log::UndoEntry> {
        programs::find_recovery_entry_for_program(program, store)
            .map_err(|e| AdapterError::from(e.to_string()))
    }
    fn disable(
        &self,
        program: &programs::StartupProgram,
        store: &crate::core::startup_recovery::StartupRecoveryStore,
        session_id: &str,
        backup_mgr: &crate::core::backup::BackupManager,
    ) -> AdapterResult<programs::StartupMutationResult> {
        programs::disable(program, store, session_id, backup_mgr)
            .map_err(|e| AdapterError::from(e.to_string()))
    }
}

#[derive(Default)]
pub struct FixtureStartupProgramAdapter;
impl StartupProgramAdapter for FixtureStartupProgramAdapter {
    fn list(&self) -> AdapterResult<Vec<programs::StartupProgram>> {
        Ok(Vec::new())
    }
    fn find_by_id(&self, id: &str) -> AdapterResult<programs::StartupProgram> {
        Err(AdapterError::new(
            AdapterErrorCategory::NotFound,
            "startup.find",
            format!("Startup program not found: {id}"),
        ))
    }
    fn list_with_recovery(
        &self,
        _store: &crate::core::startup_recovery::StartupRecoveryStore,
    ) -> AdapterResult<Vec<programs::StartupProgram>> {
        Ok(Vec::new())
    }
    fn find_by_id_with_recovery(
        &self,
        id: &str,
        _store: &crate::core::startup_recovery::StartupRecoveryStore,
    ) -> AdapterResult<programs::StartupProgram> {
        self.find_by_id(id)
    }
    fn find_recovery_entry_for_program(
        &self,
        _program: &programs::StartupProgram,
        _store: &crate::core::startup_recovery::StartupRecoveryStore,
    ) -> AdapterResult<crate::core::undo_log::UndoEntry> {
        Err(AdapterError::new(
            AdapterErrorCategory::NotFound,
            "startup.recovery",
            "Startup recovery record not found",
        ))
    }
    fn disable(
        &self,
        _program: &programs::StartupProgram,
        _store: &crate::core::startup_recovery::StartupRecoveryStore,
        _session_id: &str,
        _backup_mgr: &crate::core::backup::BackupManager,
    ) -> AdapterResult<programs::StartupMutationResult> {
        Err(AdapterError::new(
            AdapterErrorCategory::NotFound,
            "startup.disable",
            "Startup program not found",
        ))
    }
}

pub struct NativeServiceAdapter;
impl ServiceAdapter for NativeServiceAdapter {
    fn list(&self) -> AdapterResult<Vec<ServiceInfo>> {
        services::list_fallible()
            .map(|items| items.into_iter().map(Into::into).collect())
            .map_err(|error| AdapterError::from_message("service.list", error))
    }
    fn inspect(&self, name: &str) -> AdapterResult<ServiceInfo> {
        services::inspect(name)
            .map(Into::into)
            .map_err(AdapterError::from)
    }
    fn set_start_type(&self, name: &str, ty: ServiceStartType) -> AdapterResult<()> {
        services::set_start_type(name, ty.as_native()).map_err(AdapterError::from)
    }
    fn set_delayed_auto_start(&self, name: &str, value: bool) -> AdapterResult<()> {
        services::set_delayed_auto_start(name, value).map_err(AdapterError::from)
    }
    fn control(&self, name: &str, action: ServiceAction) -> AdapterResult<()> {
        let action = match action {
            ServiceAction::Start => "start",
            ServiceAction::Stop => "stop",
            ServiceAction::Pause | ServiceAction::Continue => {
                return Err(AdapterError::new(
                    AdapterErrorCategory::InvalidInput,
                    "service.control",
                    "Unsupported service action: startup services support only start and stop",
                ));
            }
        };
        services::control_service(name, action)
            .map_err(|error| AdapterError::from_message("service.control", error))
    }
    fn wait_for_status(&self, name: &str, status: &str, timeout_seconds: u64) -> AdapterResult<()> {
        services::wait_for_status(name, status, timeout_seconds as usize)
            .map(|_| ())
            .map_err(AdapterError::from)
    }
}

pub struct NativeTaskAdapter;
impl TaskAdapter for NativeTaskAdapter {
    fn list(&self) -> AdapterResult<Vec<ScheduledTask>> {
        scheduler::list_startup_tasks_fallible()
            .map(|items| items.into_iter().map(Into::into).collect())
            .map_err(|error| AdapterError::from_message("task.list", error.to_string()))
    }
    fn find(&self, name: &str) -> AdapterResult<ScheduledTask> {
        scheduler::find_task(name)
            .map(Into::into)
            .map_err(|e| AdapterError::from_message("task.find", e.to_string()))
    }
    fn set_enabled(&self, name: &str, enabled: bool) -> AdapterResult<()> {
        scheduler::set_task_state(name, enabled)
            .map(|_| ())
            .map_err(|e| AdapterError::from_message("task.set_enabled", e.to_string()))
    }
    fn capture_definition(&self, name: &str) -> AdapterResult<TaskDefinitionSnapshot> {
        scheduler::capture_task_definition(name)
            .map(Into::into)
            .map_err(|e| AdapterError::from_message("task.capture", e.to_string()))
    }
    fn restore_definition(&self, definition: &TaskDefinitionSnapshot) -> AdapterResult<()> {
        let native: scheduler::TaskDefinitionSnapshot = definition.into();
        scheduler::restore_task_definition(&native)
            .map(|_| ())
            .map_err(|e| AdapterError::from_message("task.restore", e.to_string()))
    }
}

pub struct NativeProcessAdapter;
impl ProcessAdapter for NativeProcessAdapter {
    fn snapshot(&self) -> Vec<crate::processes::snapshots::ProcessInfo> {
        crate::processes::snapshots::snapshot()
    }
    fn requires_elevation(&self, pid: u32) -> bool {
        crate::processes::kill::requires_elevation(pid)
    }
    fn kill(&self, pid: u32, start: u64, name: &str, path: Option<&str>) -> AdapterResult<()> {
        crate::processes::kill::kill_process(pid, start, name, path)
            .map_err(|e| AdapterError::from(e.to_string()))
    }
}

pub struct NativeCommandAdapter;
impl CommandAdapter for NativeCommandAdapter {
    fn execute(&self, program: &str, args: &[String]) -> AdapterResult<CommandOutput> {
        let output: Output = crate::core::command_helpers::create_command(program)
            .args(args)
            .output()
            .map_err(|e| e.to_string())?;
        Ok(CommandOutput {
            status: output.status.code().unwrap_or(-1),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

pub struct NativeElevationAdapter;
impl ElevationAdapter for NativeElevationAdapter {
    fn is_elevated(&self) -> bool {
        crate::core::elevation::is_elevated()
    }
}

pub struct NativeSystemInfoAdapter;
impl SystemInfoAdapter for NativeSystemInfoAdapter {
    fn os(&self) -> OsInfo {
        crate::system_info::os::gather().into()
    }
    fn hardware(&self) -> HardwareInfo {
        crate::system_info::hardware::gather().into()
    }
    fn battery(&self) -> BatteryInfo {
        crate::system_info::battery::gather().into()
    }
    fn network(&self) -> NetworkInfo {
        crate::system_info::network::gather().into()
    }
    fn environment(&self) -> AdapterResult<Vec<EnvVar>> {
        crate::system_info::env_vars::list()
            .map(|v| v.into_iter().map(Into::into).collect())
            .map_err(AdapterError::from)
    }
}

#[derive(Clone)]
pub struct FixtureRegistryAdapter {
    values: Arc<Mutex<BTreeMap<(String, String), RegistryValue>>>,
}
impl FixtureRegistryAdapter {
    pub fn new() -> Self {
        Self {
            values: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }
}
impl Default for FixtureRegistryAdapter {
    fn default() -> Self {
        Self::new()
    }
}
impl RegistryAdapter for FixtureRegistryAdapter {
    fn list(&self, key: &str) -> AdapterResult<Vec<(String, RegistryValue)>> {
        Ok(self
            .values
            .lock()
            .unwrap()
            .iter()
            .filter_map(|((stored_key, name), value)| {
                stored_key
                    .eq_ignore_ascii_case(key)
                    .then_some((name.clone(), value.clone()))
            })
            .collect())
    }
    fn read(&self, key: &str, value: &str) -> AdapterResult<Option<RegistryValue>> {
        Ok(self
            .values
            .lock()
            .unwrap()
            .iter()
            .find(|((stored_key, stored_value), _)| {
                stored_key.eq_ignore_ascii_case(key) && stored_value.eq_ignore_ascii_case(value)
            })
            .map(|(_, data)| data.clone()))
    }
    fn write(&self, key: &str, value: &str, data: RegistryValue) -> AdapterResult<()> {
        let mut values = self.values.lock().unwrap();
        if let Some(existing) = values
            .keys()
            .find(|(stored_key, stored_value)| {
                stored_key.eq_ignore_ascii_case(key) && stored_value.eq_ignore_ascii_case(value)
            })
            .cloned()
        {
            values.remove(&existing);
            values.insert(existing, data);
        } else {
            values.insert((key.into(), value.into()), data);
        }
        Ok(())
    }
    fn delete_value(&self, key: &str, value: &str) -> AdapterResult<()> {
        let mut values = self.values.lock().unwrap();
        if let Some(existing) = values
            .keys()
            .find(|(stored_key, stored_value)| {
                stored_key.eq_ignore_ascii_case(key) && stored_value.eq_ignore_ascii_case(value)
            })
            .cloned()
        {
            values.remove(&existing);
        }
        Ok(())
    }
    fn delete_key(&self, key: &str) -> AdapterResult<()> {
        self.values.lock().unwrap().retain(|(stored_key, _), _| {
            !(stored_key.eq_ignore_ascii_case(key)
                || (stored_key.len() > key.len()
                    && stored_key.is_char_boundary(key.len())
                    && stored_key[..key.len()].eq_ignore_ascii_case(key)
                    && stored_key.as_bytes().get(key.len()) == Some(&b'\\')))
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_registry_is_isolated_and_deterministic() {
        let first = FixtureRegistryAdapter::new();
        let second = FixtureRegistryAdapter::new();
        first
            .write("HKCU\\Fixture", "Value", RegistryValue::Dword(7))
            .unwrap();
        assert_eq!(
            first.read("HKCU\\Fixture", "Value").unwrap(),
            Some(RegistryValue::Dword(7))
        );
        assert_eq!(second.read("HKCU\\Fixture", "Value").unwrap(), None);
        assert_eq!(
            FixtureCommandAdapter::default()
                .execute("definitely-not-a-process", &[])
                .unwrap()
                .status,
            0
        );
        assert_eq!(FixtureSystemInfoAdapter.os().version, "Fixture Windows");
    }

    #[test]
    fn fixture_registry_mutations_are_case_insensitive_without_duplicates() {
        let adapter = FixtureRegistryAdapter::new();
        adapter
            .write(
                "HKCU\\Environment",
                "Path",
                RegistryValue::String("one".into()),
            )
            .unwrap();
        adapter
            .write(
                "hkcu\\environment",
                "pAtH",
                RegistryValue::String("two".into()),
            )
            .unwrap();

        assert_eq!(
            adapter.read("HKCU\\ENVIRONMENT", "PATH").unwrap(),
            Some(RegistryValue::String("two".into()))
        );
        assert_eq!(adapter.list("hkcu\\environment").unwrap().len(), 1);

        adapter.delete_value("HKCU\\environment", "PATH").unwrap();
        assert!(adapter.list("hkcu\\environment").unwrap().is_empty());

        adapter
            .write("HKCU\\Environment", "One", RegistryValue::Dword(1))
            .unwrap();
        adapter
            .write("HKCU\\Environment\\Child", "Two", RegistryValue::Dword(2))
            .unwrap();
        adapter
            .write("HKCU\\Environmentary", "Keep", RegistryValue::Dword(3))
            .unwrap();
        adapter.delete_key("hkcu\\ENVIRONMENT").unwrap();
        assert!(adapter.list("HKCU\\Environment").unwrap().is_empty());
        assert_eq!(adapter.list("hkcu\\environmentary").unwrap().len(), 1);
    }
}
#[derive(Default)]
pub struct FixtureServiceAdapter {
    services: Mutex<BTreeMap<String, ServiceInfo>>,
}
impl FixtureServiceAdapter {
    pub fn insert(&self, service: ServiceInfo) {
        self.services
            .lock()
            .unwrap()
            .insert(service.name.clone(), service);
    }
}
impl ServiceAdapter for FixtureServiceAdapter {
    fn list(&self) -> AdapterResult<Vec<ServiceInfo>> {
        Ok(self.services.lock().unwrap().values().cloned().collect())
    }
    fn inspect(&self, name: &str) -> AdapterResult<ServiceInfo> {
        self.services
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or_else(|| {
                AdapterError::new(
                    AdapterErrorCategory::NotFound,
                    "service.inspect",
                    format!("Service not found: {name}"),
                )
            })
    }
    fn set_start_type(&self, name: &str, ty: ServiceStartType) -> AdapterResult<()> {
        self.services
            .lock()
            .unwrap()
            .get_mut(name)
            .ok_or_else(|| {
                AdapterError::new(
                    AdapterErrorCategory::NotFound,
                    "service.set_start_type",
                    format!("Service not found: {name}"),
                )
            })?
            .start_type = ty.as_native().into();
        Ok(())
    }
    fn set_delayed_auto_start(&self, name: &str, value: bool) -> AdapterResult<()> {
        self.services
            .lock()
            .unwrap()
            .get_mut(name)
            .ok_or_else(|| {
                AdapterError::new(
                    AdapterErrorCategory::NotFound,
                    "service.set_delayed_auto_start",
                    format!("Service not found: {name}"),
                )
            })?
            .delayed_auto_start = Some(value);
        Ok(())
    }
    fn control(&self, name: &str, action: ServiceAction) -> AdapterResult<()> {
        let status = match action {
            ServiceAction::Start => "Running",
            ServiceAction::Stop => "Stopped",
            ServiceAction::Pause | ServiceAction::Continue => {
                return Err(AdapterError::new(
                    AdapterErrorCategory::InvalidInput,
                    "service.control",
                    "Unsupported service action: startup services support only start and stop",
                ));
            }
        };
        self.services
            .lock()
            .unwrap()
            .get_mut(name)
            .ok_or_else(|| {
                AdapterError::new(
                    AdapterErrorCategory::NotFound,
                    "service.control",
                    format!("Service not found: {name}"),
                )
            })?
            .status = status.into();
        Ok(())
    }
    fn wait_for_status(
        &self,
        name: &str,
        status: &str,
        _timeout_seconds: u64,
    ) -> AdapterResult<()> {
        let current = self.inspect(name)?;
        if current.status.eq_ignore_ascii_case(status) {
            Ok(())
        } else {
            Err(AdapterError::new(
                AdapterErrorCategory::Verification,
                "service.wait",
                format!("Service did not reach {status}"),
            ))
        }
    }
}

#[derive(Default)]
pub struct FixtureTaskAdapter {
    tasks: Mutex<BTreeMap<String, ScheduledTask>>,
}
impl FixtureTaskAdapter {
    pub fn insert(&self, task: ScheduledTask) {
        self.tasks.lock().unwrap().insert(task.name.clone(), task);
    }
}
impl TaskAdapter for FixtureTaskAdapter {
    fn list(&self) -> AdapterResult<Vec<ScheduledTask>> {
        Ok(self.tasks.lock().unwrap().values().cloned().collect())
    }
    fn find(&self, name: &str) -> AdapterResult<ScheduledTask> {
        self.tasks
            .lock()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or_else(|| {
                AdapterError::new(
                    AdapterErrorCategory::NotFound,
                    "task.find",
                    format!("Task not found: {name}"),
                )
            })
    }
    fn set_enabled(&self, name: &str, enabled: bool) -> AdapterResult<()> {
        let mut tasks = self.tasks.lock().unwrap();
        let item = tasks.get_mut(name).ok_or_else(|| {
            AdapterError::new(
                AdapterErrorCategory::NotFound,
                "task.set_enabled",
                format!("Task not found: {name}"),
            )
        })?;
        if !enabled {
            item.status = "Disabled".into();
        } else if item.status.eq_ignore_ascii_case("Disabled") {
            item.status = item.definition.current_state.clone();
        }
        item.definition.enabled = enabled;
        Ok(())
    }
    fn capture_definition(&self, name: &str) -> AdapterResult<TaskDefinitionSnapshot> {
        Ok(self.find(name)?.definition)
    }
    fn restore_definition(&self, definition: &TaskDefinitionSnapshot) -> AdapterResult<()> {
        if definition.task_path.trim().is_empty() {
            return Err(AdapterError::new(
                AdapterErrorCategory::InvalidInput,
                "task.restore",
                "Scheduled task path is missing",
            ));
        }
        if definition.definition_xml.trim().is_empty() {
            return Err(AdapterError::new(
                AdapterErrorCategory::InvalidInput,
                "task.restore",
                "Scheduled task definition XML is missing",
            ));
        }
        if definition
            .security_descriptor
            .as_deref()
            .is_none_or(|v| v.trim().is_empty())
        {
            return Err(AdapterError::new(
                AdapterErrorCategory::Verification,
                "task.restore",
                "Exact scheduled-task restoration requires a captured security descriptor",
            ));
        }
        let mut tasks = self.tasks.lock().unwrap();
        let item = tasks
            .values_mut()
            .find(|item| {
                item.path == definition.task_path
                    || item.definition.task_path == definition.task_path
            })
            .ok_or_else(|| {
                AdapterError::new(
                    AdapterErrorCategory::NotFound,
                    "task.restore",
                    format!("Task not found: {}", definition.task_path),
                )
            })?;
        item.definition = definition.clone();
        item.status = if definition.enabled {
            definition.current_state.clone()
        } else {
            "Disabled".into()
        };
        Ok(())
    }
}

#[derive(Default)]
pub struct FixtureProcessAdapter {
    killed: Mutex<Vec<u32>>,
}
impl FixtureProcessAdapter {
    pub fn killed_pids(&self) -> Vec<u32> {
        self.killed.lock().unwrap().clone()
    }
}
impl ProcessAdapter for FixtureProcessAdapter {
    fn snapshot(&self) -> Vec<crate::processes::snapshots::ProcessInfo> {
        Vec::new()
    }
    fn requires_elevation(&self, _pid: u32) -> bool {
        false
    }
    fn kill(&self, pid: u32, _s: u64, _n: &str, _p: Option<&str>) -> AdapterResult<()> {
        self.killed.lock().unwrap().push(pid);
        Ok(())
    }
}
#[derive(Default)]
pub struct FixtureCommandAdapter {
    last: Mutex<Option<(String, Vec<String>)>>,
}
impl FixtureCommandAdapter {
    pub fn last_invocation(&self) -> Option<(String, Vec<String>)> {
        self.last.lock().unwrap().clone()
    }
}
impl CommandAdapter for FixtureCommandAdapter {
    fn execute(&self, program: &str, args: &[String]) -> AdapterResult<CommandOutput> {
        *self.last.lock().unwrap() = Some((program.into(), args.to_vec()));
        Ok(CommandOutput {
            status: 0,
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }
}
#[derive(Default)]
pub struct FixtureElevationAdapter;
impl ElevationAdapter for FixtureElevationAdapter {
    fn is_elevated(&self) -> bool {
        true
    }
}
#[derive(Default)]
pub struct FixtureSystemInfoAdapter;
impl SystemInfoAdapter for FixtureSystemInfoAdapter {
    fn os(&self) -> OsInfo {
        OsInfo {
            version: "Fixture Windows".into(),
            edition: "Fixture".into(),
            build: "fixture".into(),
            arch: "x64".into(),
            user_name: "fixture-user".into(),
            ..Default::default()
        }
    }
    fn hardware(&self) -> HardwareInfo {
        HardwareInfo {
            cpu: crate::system_info::hardware::CpuInfo {
                name: "Fixture CPU".into(),
                cores: 1,
                logical_processors: 1,
                architecture: "x64".into(),
                usage_percent: 0.0,
            }
            .into(),
            memory: crate::system_info::hardware::MemoryInfo {
                total_bytes: 1024,
                available_bytes: 1024,
                in_use_percent: 0.0,
            }
            .into(),
            disks: vec![],
            gpu: None,
        }
    }
    fn battery(&self) -> BatteryInfo {
        BatteryInfo {
            present: false,
            charge_percent: 0.0,
            charger_connected: true,
            charging: false,
            time_remaining_minutes: -1,
            saving_mode: false,
            is_laptop: false,
        }
    }
    fn network(&self) -> NetworkInfo {
        NetworkInfo {
            adapters: vec![],
            connections: vec![],
        }
    }
    fn environment(&self) -> AdapterResult<Vec<EnvVar>> {
        Ok(vec![])
    }
}

#[derive(Clone)]
pub struct Adapters {
    pub startup_programs: Arc<dyn StartupProgramAdapter>,
    pub registry: Arc<dyn RegistryAdapter>,
    pub services: Arc<dyn ServiceAdapter>,
    pub tasks: Arc<dyn TaskAdapter>,
    pub processes: Arc<dyn ProcessAdapter>,
    pub commands: Arc<dyn CommandAdapter>,
    pub elevation: Arc<dyn ElevationAdapter>,
    pub system_info: Arc<dyn SystemInfoAdapter>,
}

impl Adapters {
    pub fn native() -> Self {
        Self {
            startup_programs: Arc::new(NativeStartupProgramAdapter),
            registry: Arc::new(NativeRegistryAdapter),
            services: Arc::new(NativeServiceAdapter),
            tasks: Arc::new(NativeTaskAdapter),
            processes: Arc::new(NativeProcessAdapter),
            commands: Arc::new(NativeCommandAdapter),
            elevation: Arc::new(NativeElevationAdapter),
            system_info: Arc::new(NativeSystemInfoAdapter),
        }
    }
    pub fn fixture() -> Self {
        Self {
            startup_programs: Arc::new(FixtureStartupProgramAdapter),
            registry: Arc::new(FixtureRegistryAdapter::new()),
            services: Arc::new(FixtureServiceAdapter::default()),
            tasks: Arc::new(FixtureTaskAdapter::default()),
            processes: Arc::new(FixtureProcessAdapter::default()),
            commands: Arc::new(FixtureCommandAdapter::default()),
            elevation: Arc::new(FixtureElevationAdapter),
            system_info: Arc::new(FixtureSystemInfoAdapter),
        }
    }
}

#[cfg(test)]
mod focused_tests {
    use super::*;

    fn service() -> ServiceInfo {
        ServiceInfo {
            name: "svc".into(),
            display_name: "Fixture".into(),
            status: "Stopped".into(),
            start_type: "manual".into(),
            description: None,
            binary_path: None,
            delayed_auto_start: None,
            dependencies: vec![],
        }
    }
    fn task() -> ScheduledTask {
        ScheduledTask {
            name: "Task".into(),
            path: r#"\Task"#.into(),
            status: "Ready".into(),
            trigger: "manual".into(),
            classification: "review".into(),
            can_modify: true,
            recommendation: "review".into(),
            guidance: GuidanceInfo::default(),
            definition: TaskDefinitionSnapshot {
                task_path: r#"\Task"#.into(),
                enabled: true,
                ..Default::default()
            },
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn native_list_errors_are_mapped_instead_of_reported_as_empty_success() {
        assert_eq!(
            NativeServiceAdapter.list().unwrap_err().category,
            AdapterErrorCategory::Unsupported
        );
        assert_eq!(
            NativeTaskAdapter.list().unwrap_err().category,
            AdapterErrorCategory::Unsupported
        );
    }
    #[test]
    fn fixture_service_mutations_and_missing_are_structured() {
        let adapter = FixtureServiceAdapter::default();
        adapter.insert(service());
        adapter.control("svc", ServiceAction::Start).unwrap();
        assert_eq!(adapter.inspect("svc").unwrap().status, "Running");
        adapter
            .set_start_type("svc", ServiceStartType::Disabled)
            .unwrap();
        assert_eq!(adapter.inspect("svc").unwrap().start_type, "disabled");
        let error = adapter.control("missing", ServiceAction::Stop).unwrap_err();
        assert_eq!(error.category, AdapterErrorCategory::NotFound);
    }

    #[test]
    fn service_pause_and_continue_are_invalid_for_fixture_and_native_contracts() {
        let fixture = FixtureServiceAdapter::default();
        fixture.insert(service());
        for action in [ServiceAction::Pause, ServiceAction::Continue] {
            assert_eq!(
                fixture.control("svc", action).unwrap_err().category,
                AdapterErrorCategory::InvalidInput
            );
            assert_eq!(
                NativeServiceAdapter
                    .control("svc", action)
                    .unwrap_err()
                    .category,
                AdapterErrorCategory::InvalidInput
            );
        }
    }

    #[test]
    fn scheduled_task_conversion_preserves_guidance() {
        let mut native = scheduler::ScheduledTask {
            name: "Task".into(),
            path: r#"\Task"#.into(),
            status: "Ready".into(),
            trigger: "manual".into(),
            classification: "review".into(),
            can_modify: true,
            recommendation: "review".into(),
            guidance: GuidanceInfo::default(),
            definition: scheduler::TaskDefinitionSnapshot::default(),
        };
        native.guidance.rationale = "preserve this".into();
        assert_eq!(
            ScheduledTask::from(native).guidance.rationale,
            "preserve this"
        );
    }

    #[test]
    fn native_error_mapping_preserves_evidence_based_categories_and_hresult() {
        let denied = AdapterError::from_message(
            "task.find",
            "Task Scheduler read failed (HRESULT 0x80070005): access denied",
        );
        assert_eq!(denied.category, AdapterErrorCategory::AccessDenied);
        assert_eq!(denied.native_code, Some(0x8007_0005u32 as i32));
        assert_eq!(
            AdapterError::from_message(
                "task.find",
                "Native Task Scheduler access is only available on Windows"
            )
            .category,
            AdapterErrorCategory::Unsupported
        );
    }

    #[test]
    fn fixture_task_enable_preserves_can_modify_and_restore_requires_existing_task() {
        let adapter = FixtureTaskAdapter::default();
        let mut running = task();
        running.status = "Running".into();
        running.definition.current_state = "Running".into();
        adapter.insert(running);
        adapter.set_enabled("Task", false).unwrap();
        let changed = adapter.find("Task").unwrap();
        assert!(!changed.definition.enabled);
        assert!(changed.can_modify);
        assert_eq!(changed.status, "Disabled");
        adapter.set_enabled("Task", true).unwrap();
        assert_eq!(adapter.find("Task").unwrap().status, "Running");
        let missing = TaskDefinitionSnapshot {
            task_path: r#"\missing"#.into(),
            ..Default::default()
        };
        assert_eq!(
            adapter.restore_definition(&missing).unwrap_err().category,
            AdapterErrorCategory::InvalidInput
        );
        let missing_task = TaskDefinitionSnapshot {
            task_path: r#"\missing"#.into(),
            definition_xml: "<Task/>".into(),
            security_descriptor: Some("fixture-sddl".into()),
            ..Default::default()
        };
        assert_eq!(
            adapter
                .restore_definition(&missing_task)
                .unwrap_err()
                .category,
            AdapterErrorCategory::NotFound
        );
        let incomplete = TaskDefinitionSnapshot {
            task_path: r#"\Task"#.into(),
            definition_xml: "<Task/>".into(),
            ..Default::default()
        };
        assert_eq!(
            adapter
                .restore_definition(&incomplete)
                .unwrap_err()
                .category,
            AdapterErrorCategory::Verification
        );
    }
    #[test]
    fn fixture_process_records_kills_and_fixture_elevation_is_deterministic() {
        let process = FixtureProcessAdapter::default();
        process.kill(41, 0, "p", None).unwrap();
        assert_eq!(process.killed_pids(), vec![41]);
        assert!(process.snapshot().is_empty());
        assert!(FixtureElevationAdapter.is_elevated());
    }
    #[test]
    fn fixture_command_records_exact_program_and_args() {
        let command = FixtureCommandAdapter::default();
        let args = vec!["/c".into(), "hello".into()];
        command.execute("cmd.exe", &args).unwrap();
        assert_eq!(command.last_invocation(), Some(("cmd.exe".into(), args)));
    }
    #[test]
    fn fixture_system_info_is_deterministic() {
        let info = FixtureSystemInfoAdapter;
        assert_eq!(info.os(), info.os());
        assert_eq!(info.hardware(), info.hardware());
        assert_eq!(info.os().version, "Fixture Windows");
        assert_eq!(info.environment().unwrap(), Vec::<EnvVar>::new());
    }
}
