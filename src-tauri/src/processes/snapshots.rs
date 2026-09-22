//! Process snapshot - list all running processes with CPU/memory/disk I/O.

use serde::{Deserialize, Serialize};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub start_time: u64,
    pub name: String,
    pub cpu_usage: f32,
    pub memory_bytes: u64,
    pub disk_read_bytes: u64,
    pub disk_write_bytes: u64,
    pub status: String,
    pub path: Option<String>,
}

/// Take a snapshot of all running processes.
/// CPU usage is normalized to 0-100% (sysinfo returns 0-100 x num_cpus).
pub fn snapshot() -> Vec<ProcessInfo> {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .with_cpu()
            .with_memory()
            .with_disk_usage()
            .with_exe(UpdateKind::OnlyIfNotSet),
    );

    // sysinfo::Process::cpu_usage() returns a value in the range 0-100 x num_cpus.
    // We normalize to 0-100% by dividing by the number of logical CPUs.
    let num_cpus = system.cpus().len().max(1) as f32;

    let mut processes: Vec<ProcessInfo> = system
        .processes()
        .iter()
        .map(|(&pid, process)| ProcessInfo {
            pid: pid.as_u32(),
            start_time: process.start_time(),
            name: process.name().to_string_lossy().to_string(),
            cpu_usage: process.cpu_usage() / num_cpus,
            memory_bytes: process.memory(),
            disk_read_bytes: process.disk_usage().read_bytes,
            disk_write_bytes: process.disk_usage().written_bytes,
            status: format!("{:?}", process.status()),
            path: process.exe().map(|p| p.to_string_lossy().to_string()),
        })
        .collect();

    processes.sort_by(|a, b| {
        b.cpu_usage
            .partial_cmp(&a.cpu_usage)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    processes
}
