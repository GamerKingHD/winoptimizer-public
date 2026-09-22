//! Scheduled tasks manager - list tasks and enable/disable them.
//!
//! Windows access is performed through the native Task Scheduler COM API.
//! The non-Windows implementation remains explicitly unavailable so tests and
//! other platforms cannot silently fall back to a localized command-line tool.

use crate::startup::guidance::GuidanceInfo;
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub name: String,
    pub path: String,
    pub status: String, // "Ready", "Disabled", "Running"
    pub trigger: String,
    pub classification: String, // "recommended_disable", "review", "protected"
    pub can_modify: bool,
    pub recommendation: String,
    #[serde(default)]
    pub guidance: GuidanceInfo,
    /// Exact native state captured with the task identity and definition.
    #[serde(default)]
    pub definition: TaskDefinitionSnapshot,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDefinitionSnapshot {
    /// Canonical Task Scheduler path returned by the native API.
    pub task_path: String,
    pub enabled: bool,
    pub current_state: String,
    #[serde(default)]
    pub next_run_time: Option<String>,
    /// Full XML returned by IRegisteredTask::get_Xml.
    pub definition_xml: String,
    #[serde(default)]
    pub triggers_xml: String,
    #[serde(default)]
    pub actions_xml: String,
    #[serde(default)]
    pub principals_xml: String,
    #[serde(default)]
    pub run_level: String,
    /// Optional because some tasks do not expose their descriptor to a
    /// standard-user inventory. Exact restoration requires this value.
    #[serde(default)]
    pub security_descriptor: Option<String>,
}

impl TaskDefinitionSnapshot {
    /// Mutation and exact restoration require both the full XML and the
    /// native security descriptor captured for the same task.
    pub(crate) fn exact_capture_available(&self) -> bool {
        !self.task_path.is_empty()
            && !self.definition_xml.trim().is_empty()
            && self
                .security_descriptor
                .as_deref()
                .is_some_and(|descriptor| !descriptor.trim().is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchedulerError {
    InvalidTaskName(String),
    ProtectedTarget(String),
    NotFound(String),
    AccessDenied {
        operation: String,
        hresult: i32,
    },
    Native {
        operation: String,
        hresult: i32,
        message: String,
    },
    StaleState(String),
    VerificationFailed(String),
    InvalidDefinition(String),
    RestorationUnavailable(String),
    UnsupportedPlatform,
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTaskName(message)
            | Self::ProtectedTarget(message)
            | Self::NotFound(message)
            | Self::StaleState(message)
            | Self::VerificationFailed(message)
            | Self::InvalidDefinition(message)
            | Self::RestorationUnavailable(message) => formatter.write_str(message),
            Self::AccessDenied { operation, hresult } => write!(
                formatter,
                "Task Scheduler {operation} was denied (HRESULT {})",
                format_hresult(*hresult)
            ),
            Self::Native {
                operation,
                hresult,
                message,
            } => write!(
                formatter,
                "Task Scheduler {operation} failed (HRESULT {}): {message}",
                format_hresult(*hresult)
            ),
            Self::UnsupportedPlatform => {
                formatter.write_str("Native Task Scheduler access is only available on Windows")
            }
        }
    }
}

impl std::error::Error for SchedulerError {}

const HRESULT_ACCESS_DENIED: i32 = 0x8007_0005u32 as i32;
const HRESULT_FILE_NOT_FOUND: i32 = 0x8007_0002u32 as i32;
const HRESULT_INVALID_ARGUMENT: i32 = 0x8007_0057u32 as i32;
const HRESULT_TASK_NOT_FOUND: i32 = 0x8004_130Fu32 as i32;

fn format_hresult(hresult: i32) -> String {
    format!("0x{:08X}", hresult as u32)
}

fn map_hresult_code(
    operation: impl Into<String>,
    hresult: i32,
    message: impl Into<String>,
) -> SchedulerError {
    let operation = operation.into();
    match hresult {
        HRESULT_ACCESS_DENIED => SchedulerError::AccessDenied { operation, hresult },
        HRESULT_FILE_NOT_FOUND | HRESULT_TASK_NOT_FOUND => SchedulerError::NotFound(message.into()),
        HRESULT_INVALID_ARGUMENT => SchedulerError::InvalidDefinition(message.into()),
        _ => SchedulerError::Native {
            operation,
            hresult,
            message: message.into(),
        },
    }
}

#[cfg(windows)]
fn map_windows_error(operation: &str, error: windows::core::Error) -> SchedulerError {
    map_hresult_code(operation, error.code().0, error.to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct ScheduledTaskScan {
    pub tasks: Vec<ScheduledTask>,
    pub collected_at: String,
    pub classification: String,
    pub mutation_available: bool,
    pub error: Option<String>,
}

/// List scheduled tasks through the native adapter.
pub fn list_startup_tasks() -> Vec<ScheduledTask> {
    list_startup_tasks_fallible().unwrap_or_default()
}

/// Fallible native task enumeration for infrastructure callers that must not
/// turn Task Scheduler failures into an empty successful inventory.
pub fn list_startup_tasks_fallible() -> Result<Vec<ScheduledTask>, SchedulerError> {
    scan_tasks()
}

pub fn scan() -> ScheduledTaskScan {
    match scan_tasks() {
        Ok(tasks) => ScheduledTaskScan {
            tasks,
            collected_at: chrono::Utc::now().to_rfc3339(),
            classification: "supporting".into(),
            mutation_available: cfg!(windows),
            error: None,
        },
        Err(error) => ScheduledTaskScan {
            tasks: Vec::new(),
            collected_at: chrono::Utc::now().to_rfc3339(),
            classification: "supporting".into(),
            mutation_available: cfg!(windows),
            error: Some(error.to_string()),
        },
    }
}

fn scan_tasks() -> Result<Vec<ScheduledTask>, SchedulerError> {
    #[cfg(windows)]
    {
        native::list_tasks()
    }
    #[cfg(not(windows))]
    {
        Err(SchedulerError::UnsupportedPlatform)
    }
}

/// Read one task by its canonical Task Scheduler path.
pub fn find_task(task_name: &str) -> Result<ScheduledTask, SchedulerError> {
    validate_task_name(task_name)?;
    #[cfg(windows)]
    {
        native::find_task(task_name)
    }
    #[cfg(not(windows))]
    {
        let _ = task_name;
        Err(SchedulerError::UnsupportedPlatform)
    }
}

/// Enable or disable one task after the caller has reviewed the exact task name.
pub fn set_task_state(task_name: &str, enable: bool) -> Result<ScheduledTask, SchedulerError> {
    validate_task_name(task_name)?;
    if is_protected(task_name) {
        return Err(SchedulerError::ProtectedTarget(format!(
            "Important Windows scheduled task cannot be changed: {task_name}"
        )));
    }
    let current = find_task(task_name)?;
    set_task_state_checked(&current, enable)
}

/// Apply a state change only if the native task still matches the reviewed
/// identity, state, and definition snapshot.
pub fn set_task_state_checked(
    expected: &ScheduledTask,
    enable: bool,
) -> Result<ScheduledTask, SchedulerError> {
    validate_task_name(&expected.name)?;
    if is_protected(&expected.name) || !expected.can_modify {
        return Err(SchedulerError::ProtectedTarget(format!(
            "Important Windows scheduled task cannot be changed: {}",
            expected.name
        )));
    }

    let current = find_task(&expected.name)?;
    if !expected.definition.exact_capture_available() {
        return Err(SchedulerError::RestorationUnavailable(
            "Exact Task Scheduler definition/security-descriptor capture is unavailable".into(),
        ));
    }
    if !current.definition.exact_capture_available() {
        return Err(SchedulerError::RestorationUnavailable(
            "Task Scheduler no longer exposes an exact definition/security descriptor".into(),
        ));
    }
    if !current.state_matches(expected) {
        return Err(SchedulerError::StaleState(format!(
            "Scheduled task changed after confirmation; review {} again.",
            expected.name
        )));
    }
    if is_enabled(&current) == enable {
        return Ok(current);
    }

    #[cfg(windows)]
    native::set_enabled(&expected.name, enable)?;
    #[cfg(not(windows))]
    return Err(SchedulerError::UnsupportedPlatform);

    let updated = find_task(&expected.name)?;
    let mut differences = Vec::new();
    if updated.name != expected.name || updated.path != expected.path {
        differences.push("task_identity");
    }
    if is_enabled(&updated) != enable {
        differences.push("enabled_state");
    }
    differences.extend(definition_mismatch_fields(
        &expected.definition,
        &updated.definition,
    ));
    if !differences.is_empty() {
        return Err(SchedulerError::VerificationFailed(format!(
            "Task Scheduler did not verify {} as {} without changing its captured definition (differences: {})",
            expected.name,
            if enable { "enabled" } else { "disabled" },
            differences.join(", ")
        )));
    }
    Ok(updated)
}

pub fn capture_task_definition(task_name: &str) -> Result<TaskDefinitionSnapshot, SchedulerError> {
    Ok(find_task(task_name)?.definition)
}

/// Restore a captured definition only when the snapshot contains the security
/// descriptor needed to verify exact restoration. This is intentionally not an
/// implicit undo path; callers must expose it through a governed operation.
pub fn restore_task_definition(
    snapshot: &TaskDefinitionSnapshot,
) -> Result<ScheduledTask, SchedulerError> {
    validate_definition_snapshot(snapshot)?;
    if snapshot
        .security_descriptor
        .as_deref()
        .is_none_or(|descriptor| descriptor.trim().is_empty())
    {
        return Err(SchedulerError::RestorationUnavailable(
            "Exact scheduled-task restoration requires a captured security descriptor.".into(),
        ));
    }
    if is_protected(&snapshot.task_path) {
        return Err(SchedulerError::ProtectedTarget(format!(
            "Important Windows scheduled task cannot be restored: {}",
            snapshot.task_path
        )));
    }
    if snapshot.security_descriptor.is_none() {
        return Err(SchedulerError::RestorationUnavailable(
            "Exact scheduled-task restoration requires a captured security descriptor.".into(),
        ));
    }

    #[cfg(windows)]
    {
        native::restore_task_definition(snapshot)
    }
    #[cfg(not(windows))]
    {
        Err(SchedulerError::UnsupportedPlatform)
    }
}

fn validate_task_name(task_name: &str) -> Result<(), SchedulerError> {
    if task_name.is_empty()
        || task_name.len() > 512
        || !task_name.starts_with('\\')
        || task_name == "\\"
        || task_name.ends_with('\\')
    {
        return Err(SchedulerError::InvalidTaskName(
            "Invalid scheduled task name".into(),
        ));
    }
    if task_name
        .chars()
        .any(|character| matches!(character, '\r' | '\n' | '|' | '&' | '<' | '>' | ';' | '"'))
    {
        return Err(SchedulerError::InvalidTaskName(
            "Scheduled task name contains unsupported characters".into(),
        ));
    }
    Ok(())
}

fn task_path(task_name: &str) -> String {
    task_name
        .rfind('\\')
        .map(|index| {
            if index == 0 {
                "\\".into()
            } else {
                task_name[..index].to_string()
            }
        })
        .unwrap_or_else(|| "\\".into())
}

fn task_leaf(task_name: &str) -> &str {
    task_name
        .rsplit_once('\\')
        .map(|(_, leaf)| leaf)
        .unwrap_or(task_name)
}

fn is_enabled(task: &ScheduledTask) -> bool {
    task.definition.enabled && !task.status.eq_ignore_ascii_case("disabled")
}

fn validate_definition_snapshot(snapshot: &TaskDefinitionSnapshot) -> Result<(), SchedulerError> {
    validate_task_name(&snapshot.task_path)?;
    if snapshot.definition_xml.trim().is_empty() {
        return Err(SchedulerError::InvalidDefinition(
            "Scheduled task definition XML is missing".into(),
        ));
    }
    if !snapshot
        .definition_xml
        .to_ascii_lowercase()
        .contains("<task")
    {
        return Err(SchedulerError::InvalidDefinition(
            "Scheduled task definition XML is invalid".into(),
        ));
    }
    Ok(())
}

fn normalize_xml(xml: &str) -> String {
    xml.trim().replace("\r\n", "\n")
}

fn normalize_definition_xml_except_enabled(xml: &str) -> String {
    let mut normalized = normalize_xml(xml);
    let Some(open_start) = normalized.find("<Enabled>") else {
        return normalized;
    };
    let value_start = open_start + "<Enabled>".len();
    let Some(value_end_offset) = normalized[value_start..].find("</Enabled>") else {
        return normalized;
    };
    let element_end = value_start + value_end_offset + "</Enabled>".len();
    let line_start = normalized[..open_start]
        .rfind('\n')
        .map(|offset| offset + 1)
        .filter(|offset| normalized[*offset..open_start].trim().is_empty())
        .unwrap_or(open_start);
    let line_end = if normalized[element_end..].starts_with('\n') {
        element_end + 1
    } else {
        element_end
    };
    normalized.replace_range(line_start..line_end, "");
    normalized
}

fn definition_matches_exact(
    expected: &TaskDefinitionSnapshot,
    actual: &TaskDefinitionSnapshot,
) -> bool {
    expected.exact_capture_available()
        && actual.exact_capture_available()
        && expected.task_path.eq_ignore_ascii_case(&actual.task_path)
        && expected.enabled == actual.enabled
        && normalize_xml(&expected.definition_xml) == normalize_xml(&actual.definition_xml)
        && expected.triggers_xml == actual.triggers_xml
        && expected.actions_xml == actual.actions_xml
        && expected.principals_xml == actual.principals_xml
        && expected.run_level == actual.run_level
        && expected.security_descriptor == actual.security_descriptor
}

pub(crate) fn definition_matches_except_runtime_state(
    expected: &TaskDefinitionSnapshot,
    actual: &TaskDefinitionSnapshot,
) -> bool {
    definition_mismatch_fields(expected, actual).is_empty()
}

fn definition_mismatch_fields(
    expected: &TaskDefinitionSnapshot,
    actual: &TaskDefinitionSnapshot,
) -> Vec<&'static str> {
    let mut differences = Vec::new();
    if !expected.exact_capture_available() || !actual.exact_capture_available() {
        differences.push("exact_capture");
    }
    if !expected.task_path.eq_ignore_ascii_case(&actual.task_path) {
        differences.push("task_path");
    }
    if normalize_definition_xml_except_enabled(&expected.definition_xml)
        != normalize_definition_xml_except_enabled(&actual.definition_xml)
    {
        differences.push("definition_xml");
    }
    if expected.triggers_xml != actual.triggers_xml {
        differences.push("triggers_xml");
    }
    if expected.actions_xml != actual.actions_xml {
        differences.push("actions_xml");
    }
    if expected.principals_xml != actual.principals_xml {
        differences.push("principals_xml");
    }
    if expected.run_level != actual.run_level {
        differences.push("run_level");
    }
    if expected.security_descriptor != actual.security_descriptor {
        differences.push("security_descriptor");
    }
    differences
}

fn xml_section(xml: &str, tag: &str) -> String {
    let open_start = format!("<{tag}");
    let close = format!("</{tag}>");
    let Some(start) = xml.find(&open_start) else {
        return String::new();
    };
    let Some(open_end_offset) = xml[start..].find('>') else {
        return String::new();
    };
    let content_start = start + open_end_offset + 1;
    let Some(close_offset) = xml[content_start..].find(&close) else {
        return String::new();
    };
    xml[start..content_start + close_offset + close.len()].to_string()
}

fn xml_value(xml: &str, tag: &str) -> String {
    let section = xml_section(xml, tag);
    section
        .split_once('>')
        .and_then(|(_, value)| value.rsplit_once('<').map(|(value, _)| value.trim().into()))
        .unwrap_or_default()
}

#[cfg(windows)]
mod native {
    use super::*;
    use windows::core::{BSTR, GUID};
    use windows::Win32::Foundation::{VARIANT_FALSE, VARIANT_TRUE};
    use windows::Win32::Security::{
        DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED,
    };
    use windows::Win32::System::TaskScheduler::{
        IRegisteredTask, ITaskFolder, ITaskService, TASK_CREATE_OR_UPDATE, TASK_LOGON_NONE,
        TASK_STATE, TASK_STATE_DISABLED, TASK_STATE_QUEUED, TASK_STATE_READY, TASK_STATE_RUNNING,
        TASK_STATE_UNKNOWN,
    };
    use windows::Win32::System::Variant::{VARIANT, VARIANT_0_0, VT_I4};

    // CLSID_TaskScheduler is not generated by windows-rs 0.61, although the
    // ITaskService interface is. Keep the documented CLSID local to this
    // adapter so the rest of the application does not depend on COM details.
    const CLSID_TASK_SCHEDULER: GUID = GUID::from_u128(0x0f87369f_a4e5_4cfc_bd3e_73e6154572dd);
    const SECURITY_INFORMATION: i32 = (OWNER_SECURITY_INFORMATION.0
        | GROUP_SECURITY_INFORMATION.0
        | DACL_SECURITY_INFORMATION.0) as i32;

    struct ComGuard;

    impl Drop for ComGuard {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    fn with_service<T>(
        work: impl FnOnce(&ITaskService) -> Result<T, SchedulerError>,
    ) -> Result<T, SchedulerError> {
        let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if initialized.is_err() {
            return Err(map_hresult_code(
                "COM initialization",
                initialized.0,
                "COM could not be initialized for Task Scheduler",
            ));
        }
        let _com_guard = ComGuard;
        let service: ITaskService =
            unsafe { CoCreateInstance(&CLSID_TASK_SCHEDULER, None, CLSCTX_INPROC_SERVER) }
                .map_err(|error| map_windows_error("Task Scheduler COM activation", error))?;
        let empty = VARIANT::default();
        unsafe {
            service
                .Connect(&empty, &empty, &empty, &empty)
                .map_err(|error| map_windows_error("Task Scheduler connect", error))?;
        }
        work(&service)
    }

    pub fn list_tasks() -> Result<Vec<ScheduledTask>, SchedulerError> {
        with_service(|service| {
            let root = get_folder(service, "\\")?;
            let mut tasks = Vec::new();
            collect_folder(&root, &mut tasks, 0)?;
            tasks.sort_by(|left, right| {
                left.name
                    .to_ascii_lowercase()
                    .cmp(&right.name.to_ascii_lowercase())
            });
            Ok(tasks)
        })
    }

    pub fn find_task(task_name: &str) -> Result<ScheduledTask, SchedulerError> {
        with_service(|service| {
            let registered = get_registered_task(service, task_name)?;
            capture_registered_task(&registered)
        })
    }

    pub fn set_enabled(task_name: &str, enabled: bool) -> Result<(), SchedulerError> {
        with_service(|service| {
            let registered = get_registered_task(service, task_name)?;
            let value = if enabled { VARIANT_TRUE } else { VARIANT_FALSE };
            unsafe {
                registered
                    .SetEnabled(value)
                    .map_err(|error| map_windows_error("set enabled state", error))?;
            }
            Ok(())
        })
    }

    pub fn restore_task_definition(
        snapshot: &TaskDefinitionSnapshot,
    ) -> Result<ScheduledTask, SchedulerError> {
        with_service(|service| {
            let folder = get_folder(service, &task_path(&snapshot.task_path))?;
            let leaf = BSTR::from(task_leaf(&snapshot.task_path));
            let xml = BSTR::from(snapshot.definition_xml.as_str());
            let empty = VARIANT::default();
            let registered = unsafe {
                folder
                    .RegisterTask(
                        &leaf,
                        &xml,
                        TASK_CREATE_OR_UPDATE.0,
                        &empty,
                        &empty,
                        TASK_LOGON_NONE,
                        &empty,
                    )
                    .map_err(|error| map_windows_error("restore definition", error))?
            };
            let enabled = if snapshot.enabled {
                VARIANT_TRUE
            } else {
                VARIANT_FALSE
            };
            unsafe {
                registered
                    .SetEnabled(enabled)
                    .map_err(|error| map_windows_error("restore enabled state", error))?;
            }
            let descriptor = snapshot.security_descriptor.as_ref().ok_or_else(|| {
                SchedulerError::RestorationUnavailable(
                    "Exact scheduled-task restoration requires a captured security descriptor."
                        .into(),
                )
            })?;
            let descriptor = BSTR::from(descriptor.as_str());
            unsafe {
                registered
                    .SetSecurityDescriptor(&descriptor, 0)
                    .map_err(|error| map_windows_error("restore security descriptor", error))?;
            }
            let after = capture_registered_task(&registered)?;
            if !definition_matches_exact(snapshot, &after.definition) {
                return Err(SchedulerError::VerificationFailed(format!(
                    "Task Scheduler did not restore the exact definition for {}",
                    snapshot.task_path
                )));
            }
            Ok(after)
        })
    }

    fn get_folder(service: &ITaskService, path: &str) -> Result<ITaskFolder, SchedulerError> {
        let path = BSTR::from(path);
        unsafe {
            service
                .GetFolder(&path)
                .map_err(|error| map_windows_error("open task folder", error))
        }
    }

    fn get_registered_task(
        service: &ITaskService,
        task_name: &str,
    ) -> Result<IRegisteredTask, SchedulerError> {
        let folder = get_folder(service, &task_path(task_name))?;
        let leaf = BSTR::from(task_leaf(task_name));
        unsafe {
            folder
                .GetTask(&leaf)
                .map_err(|error| map_windows_error("open task", error))
        }
    }

    fn collect_folder(
        folder: &ITaskFolder,
        tasks: &mut Vec<ScheduledTask>,
        depth: usize,
    ) -> Result<(), SchedulerError> {
        if depth > 64 {
            return Err(SchedulerError::Native {
                operation: "enumerate task folders".into(),
                hresult: HRESULT_INVALID_ARGUMENT,
                message: "Task Scheduler folder depth exceeded the safety limit".into(),
            });
        }
        let registered = unsafe {
            folder
                .GetTasks(0)
                .map_err(|error| map_windows_error("enumerate tasks", error))?
        };
        let task_count = unsafe {
            registered
                .Count()
                .map_err(|error| map_windows_error("count tasks", error))?
        };
        for index in 1..=task_count {
            let item = variant_i4(index);
            let task = unsafe {
                registered
                    .get_Item(&item)
                    .map_err(|error| map_windows_error("read task", error))?
            };
            tasks.push(capture_registered_task(&task)?);
        }

        let folders = unsafe {
            folder
                .GetFolders(0)
                .map_err(|error| map_windows_error("enumerate task folders", error))?
        };
        let folder_count = unsafe {
            folders
                .Count()
                .map_err(|error| map_windows_error("count task folders", error))?
        };
        for index in 1..=folder_count {
            let item = variant_i4(index);
            let child = unsafe {
                folders
                    .get_Item(&item)
                    .map_err(|error| map_windows_error("read task folder", error))?
            };
            collect_folder(&child, tasks, depth + 1)?;
        }
        Ok(())
    }

    fn capture_registered_task(task: &IRegisteredTask) -> Result<ScheduledTask, SchedulerError> {
        let name = unsafe {
            task.Path()
                .map_err(|error| map_windows_error("read task path", error))?
                .to_string()
        };
        let enabled = unsafe {
            task.Enabled()
                .map_err(|error| map_windows_error("read enabled state", error))?
                .0
                != 0
        };
        let state = unsafe {
            task.State()
                .map_err(|error| map_windows_error("read task state", error))?
        };
        let current_state: String = state_label(state).into();
        let definition_xml = unsafe {
            task.Xml()
                .map_err(|error| map_windows_error("read task definition", error))?
                .to_string()
        };
        let next_run_time = unsafe {
            task.NextRunTime()
                .ok()
                .filter(|value| value.is_finite() && *value != 0.0)
                .map(|value| format!("{value:.17}"))
        };
        let security_descriptor = unsafe {
            task.GetSecurityDescriptor(SECURITY_INFORMATION)
                .ok()
                .map(|descriptor| descriptor.to_string())
                .filter(|descriptor| !descriptor.trim().is_empty())
        };
        let definition = TaskDefinitionSnapshot {
            task_path: name.clone(),
            enabled,
            current_state: current_state.clone(),
            next_run_time,
            triggers_xml: xml_section(&definition_xml, "Triggers"),
            actions_xml: xml_section(&definition_xml, "Actions"),
            principals_xml: xml_section(&definition_xml, "Principals"),
            run_level: xml_value(&definition_xml, "RunLevel"),
            definition_xml,
            security_descriptor,
        };
        let classification = classify_task(&name);
        let guidance = crate::startup::guidance::scheduled_task_guidance(&name);
        Ok(ScheduledTask {
            path: task_path(&name),
            name,
            status: if enabled {
                current_state.clone()
            } else {
                "Disabled".into()
            },
            trigger: definition.triggers_xml.clone(),
            classification: classification.as_str().into(),
            can_modify: !guidance.protected,
            recommendation: guidance.rationale.clone(),
            guidance,
            definition,
        })
    }

    fn variant_i4(value: i32) -> VARIANT {
        let mut variant = VARIANT::default();
        let mut inner = VARIANT_0_0 {
            vt: VT_I4,
            ..Default::default()
        };
        unsafe {
            inner.Anonymous.lVal = value;
            std::ptr::addr_of_mut!(variant.Anonymous.Anonymous)
                .write(std::mem::ManuallyDrop::new(inner));
        }
        variant
    }

    fn state_label(state: TASK_STATE) -> &'static str {
        match state {
            TASK_STATE_DISABLED => "Disabled",
            TASK_STATE_QUEUED => "Queued",
            TASK_STATE_READY => "Ready",
            TASK_STATE_RUNNING => "Running",
            TASK_STATE_UNKNOWN => "Unknown",
            _ => "Unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskClassification {
    RecommendedDisable,
    Review,
    Protected,
}

impl TaskClassification {
    fn as_str(self) -> &'static str {
        match self {
            Self::RecommendedDisable => "recommended_disable",
            Self::Review => "review",
            Self::Protected => "protected",
        }
    }
}

fn classify_task(task_name: &str) -> TaskClassification {
    let normalized = task_name.trim_start_matches('\\').to_ascii_lowercase();

    if [
        "microsoft\\windows\\application experience\\microsoft compatibility appraiser",
        "microsoft\\windows\\application experience\\programdataupdater",
        "microsoft\\windows\\customer experience improvement program\\consolidator",
        "microsoft\\windows\\customer experience improvement program\\kernelceiptask",
        "microsoft\\windows\\customer experience improvement program\\usbceiptask",
        "microsoft\\windows\\feedback\\siuf\\dmclient",
        "microsoft\\windows\\feedback\\siuf\\dmclientonscenariodownload",
        "microsoft\\windows\\maps\\mapstoasttask",
        "microsoft\\windows\\maps\\mapsupdatetask",
        "microsoft\\windows\\windows error reporting\\queuereporting",
    ]
    .iter()
    .any(|known| normalized == *known)
    {
        return TaskClassification::RecommendedDisable;
    }

    if [
        "microsoft\\windows\\windowsupdate\\",
        "microsoft\\windows\\updateorchestrator\\",
        "microsoft\\windows\\servicing\\",
        "microsoft\\windows\\defender\\",
        "microsoft\\windows\\windows defender\\",
        "microsoft\\windows\\bitlocker\\",
        "microsoft\\windows\\chkdsk\\",
        "microsoft\\windows\\recoveryenvironment\\",
        "microsoft\\windows\\systemrestore\\",
        "microsoft\\windows\\maintenance\\",
        "microsoft\\windows\\taskscheduler\\",
        "microsoft\\windows\\time synchronization\\",
        "microsoft\\windows\\certificateservicesclient\\",
        "microsoft\\windows\\workplace join\\",
        "microsoft\\windows\\pushtoinstall\\",
    ]
    .iter()
    .any(|known| normalized.starts_with(known))
    {
        return TaskClassification::Protected;
    }

    TaskClassification::Review
}

pub fn is_protected(task_name: &str) -> bool {
    classify_task(task_name) == TaskClassification::Protected
}

impl ScheduledTask {
    /// Compare mutable Task Scheduler state without coupling legacy plans to
    /// the versioned guidance catalog.
    pub fn state_matches(&self, other: &Self) -> bool {
        self.definition.exact_capture_available()
            && other.definition.exact_capture_available()
            && self.name == other.name
            && self.path == other.path
            && self.status == other.status
            && self.trigger == other.trigger
            && self.classification == other.classification
            && self.can_modify == other.can_modify
            && self.recommendation == other.recommendation
            && self.definition == other.definition
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arbitrary_task_mutation_fails_closed() {
        assert!(
            set_task_state(r"\Microsoft\Windows\UpdateOrchestrator\Fixture", false)
                .unwrap_err()
                .to_string()
                .contains("Important Windows")
        );
    }

    #[test]
    fn hresult_mapping_preserves_stable_error_categories() {
        assert!(matches!(
            map_hresult_code("read", HRESULT_ACCESS_DENIED, "denied"),
            SchedulerError::AccessDenied { .. }
        ));
        assert!(matches!(
            map_hresult_code("read", HRESULT_TASK_NOT_FOUND, "missing"),
            SchedulerError::NotFound(message) if message == "missing"
        ));
        assert!(matches!(
            map_hresult_code("read", -1, "native failure"),
            SchedulerError::Native { operation, .. } if operation == "read"
        ));
    }

    #[test]
    fn known_optional_tasks_are_actionable_but_important_tasks_are_protected() {
        assert_eq!(
            classify_task(r"\Microsoft\Windows\Feedback\Siuf\DmClient"),
            TaskClassification::RecommendedDisable
        );
        assert!(is_protected(
            r"\Microsoft\Windows\WindowsUpdate\Scheduled Start"
        ));
        assert!(!is_protected(r"\Vendor\Task"));
    }

    #[test]
    fn task_paths_are_derived_without_truncating_task_names() {
        assert_eq!(task_path(r"\Vendor\Folder\Task"), r"\Vendor\Folder");
        assert_eq!(task_path(r"\Task"), r"\");
        assert_eq!(task_leaf(r"\Vendor\Folder\Task"), "Task");
    }

    #[test]
    fn definition_capture_and_restore_contract_requires_exact_snapshot_data() {
        let snapshot = TaskDefinitionSnapshot {
            task_path: r"\Vendor\Task".into(),
            enabled: true,
            current_state: "Ready".into(),
            next_run_time: None,
            definition_xml:
                "<Task><Settings><Enabled>true</Enabled></Settings><Triggers/><Actions/><Principals/></Task>"
                    .into(),
            triggers_xml: "<Triggers/>".into(),
            actions_xml: "<Actions/>".into(),
            principals_xml: "<Principals/>".into(),
            run_level: "LeastPrivilege".into(),
            security_descriptor: Some("D:P(A;;FA;;;SY)".into()),
        };
        assert!(validate_definition_snapshot(&snapshot).is_ok());
        assert!(definition_matches_exact(&snapshot, &snapshot));

        let mut changed_runtime = snapshot.clone();
        changed_runtime.current_state = "Running".into();
        changed_runtime.next_run_time = Some("2026-08-10T12:00:00".into());
        assert!(definition_matches_exact(&snapshot, &changed_runtime));

        changed_runtime.enabled = false;
        changed_runtime.definition_xml = snapshot
            .definition_xml
            .replace("<Enabled>true</Enabled>", "<Enabled>false</Enabled>");
        assert!(!definition_matches_exact(&snapshot, &changed_runtime));
        assert!(definition_matches_except_runtime_state(
            &snapshot,
            &changed_runtime
        ));

        let mut scheduler_omits_enabled = snapshot.clone();
        scheduler_omits_enabled.definition_xml = snapshot
            .definition_xml
            .replace("<Enabled>true</Enabled>", "");
        assert!(definition_matches_except_runtime_state(
            &snapshot,
            &scheduler_omits_enabled
        ));

        let missing_descriptor = TaskDefinitionSnapshot {
            security_descriptor: None,
            ..snapshot
        };
        assert!(matches!(
            restore_task_definition(&missing_descriptor),
            Err(SchedulerError::RestorationUnavailable(_))
                | Err(SchedulerError::UnsupportedPlatform)
        ));
    }

    #[test]
    fn blank_security_descriptor_is_rejected_before_restore_mutation() {
        let snapshot = TaskDefinitionSnapshot {
            task_path: r"\Vendor\Task".into(),
            definition_xml: "<Task/>".into(),
            security_descriptor: Some(" \t\r\n ".into()),
            ..Default::default()
        };

        assert!(matches!(
            restore_task_definition(&snapshot),
            Err(SchedulerError::RestorationUnavailable(message))
                if message.contains("security descriptor")
        ));
    }
    #[test]
    fn missing_security_descriptor_is_not_an_exact_task_snapshot() {
        let snapshot = TaskDefinitionSnapshot {
            task_path: r"\Vendor\Task".into(),
            definition_xml: "<Task/>".into(),
            security_descriptor: None,
            ..Default::default()
        };

        assert!(!snapshot.exact_capture_available());
        assert!(!definition_matches_exact(&snapshot, &snapshot));
        assert!(!definition_matches_except_runtime_state(
            &snapshot, &snapshot
        ));
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_adapter_is_explicitly_unavailable() {
        let scan_result = scan();
        assert!(!scan_result.mutation_available);
        assert!(matches!(
            find_task(r"\Vendor\Task"),
            Err(SchedulerError::UnsupportedPlatform)
        ));
    }
}
