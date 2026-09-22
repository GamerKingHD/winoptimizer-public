//! Hardware inventory via WMI and Win32 API.

use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};
use sysinfo::{DiskKind, Disks, System};
use windows::Win32::System::SystemInformation::*;

/// Persistent sysinfo System for CPU usage deltas across polls.
static CPU_SYSTEM: OnceLock<Mutex<System>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareInfo {
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub disks: Vec<DiskInfo>,
    pub gpu: Option<GpuInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuInfo {
    pub name: String,
    pub cores: u32,
    pub logical_processors: u32,
    pub architecture: String,
    pub usage_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryInfo {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub in_use_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskInfo {
    pub name: String,
    pub mount_point: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub file_system: String,
    pub is_ssd: bool,
    /// Disk type classification reported by Windows or a safe fixed/removable fallback.
    pub disk_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuInfo {
    pub name: String,
    pub adapter_ram: u64,
    pub driver_version: String,
}

/// Gather all hardware information.
pub fn gather() -> HardwareInfo {
    // Parallelize the 4 independent queries with rayon
    let ((cpu, memory), (disks, gpu)) = rayon::join(
        || rayon::join(get_cpu_info, get_memory_info),
        || rayon::join(get_disk_info, get_gpu_info),
    );
    HardwareInfo {
        cpu,
        memory,
        disks,
        gpu,
    }
}

fn get_cpu_info() -> CpuInfo {
    unsafe {
        let mut sys_info = SYSTEM_INFO::default();
        GetNativeSystemInfo(&mut sys_info);

        let cores = sys_info.dwNumberOfProcessors;
        let arch = match sys_info.Anonymous.Anonymous.wProcessorArchitecture.0 {
            0 => "x86",
            5 => "ARM",
            9 => "x64",
            12 => "ARM64",
            _ => "Unknown",
        };

        // Read CPU name from registry
        let cpu_name = get_registry_string(
            r"HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0",
            "ProcessorNameString",
        )
        .unwrap_or_else(|| "Unknown CPU".into());

        // Measure CPU usage via persistent System — delta since last poll.
        // The OnceLock init is a one-time 100ms baseline, then every subsequent
        // call is instant (just refresh + read), giving usage over the poll interval.
        let usage_percent = {
            let system_mutex = CPU_SYSTEM.get_or_init(|| {
                let mut sys = System::new();
                sys.refresh_cpu_all();
                std::thread::sleep(std::time::Duration::from_millis(100));
                sys.refresh_cpu_all();
                Mutex::new(sys)
            });
            let mut system = system_mutex.lock().unwrap();
            system.refresh_cpu_all();
            let cpus = system.cpus();
            if cpus.is_empty() {
                0.0
            } else {
                let sum: f32 = cpus.iter().map(|c| c.cpu_usage()).sum();
                let avg = sum / cpus.len() as f32;
                (avg as f64 * 10.0).round() / 10.0 // round to 1 decimal
            }
        };

        CpuInfo {
            name: cpu_name.trim().to_string(),
            cores,
            logical_processors: cores,
            architecture: arch.to_string(),
            usage_percent,
        }
    }
}

fn get_memory_info() -> MemoryInfo {
    unsafe {
        use windows::Win32::System::SystemInformation::*;
        let mut state = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        if GlobalMemoryStatusEx(&mut state).is_ok() {
            let total = state.ullTotalPhys;
            let available = state.ullAvailPhys;
            let in_use = if total > 0 {
                ((total - available) as f64 / total as f64) * 100.0
            } else {
                0.0
            };
            return MemoryInfo {
                total_bytes: total,
                available_bytes: available,
                in_use_percent: in_use,
            };
        }
    }
    MemoryInfo {
        total_bytes: 0,
        available_bytes: 0,
        in_use_percent: 0.0,
    }
}

fn get_disk_info() -> Vec<DiskInfo> {
    let disks = Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        .map(|disk| {
            let mount_point = disk.mount_point().display().to_string();
            let (is_ssd, disk_type) = match disk.kind() {
                DiskKind::SSD => (true, "SSD".to_string()),
                DiskKind::HDD => (false, "HDD".to_string()),
                DiskKind::Unknown(_) => mount_point
                    .chars()
                    .next()
                    .filter(|letter| letter.is_ascii_alphabetic())
                    .map(detect_disk_type)
                    .filter(|(_, kind)| kind != "Unknown")
                    .unwrap_or_else(|| {
                        if disk.is_removable() {
                            (false, "Removable drive".to_string())
                        } else {
                            (false, "Fixed drive".to_string())
                        }
                    }),
            };
            let name = disk.name().to_string_lossy();
            let name = if name.is_empty() {
                mount_point.clone()
            } else {
                name.into_owned()
            };

            DiskInfo {
                name: format!("{} ({})", name, disk_type),
                mount_point,
                total_bytes: disk.total_space(),
                available_bytes: disk.available_space(),
                file_system: disk.file_system().to_string_lossy().into_owned(),
                is_ssd,
                disk_type,
            }
        })
        .collect()
}

/// Detect whether a drive is SSD or HDD and return its bus type classification.
///
/// Uses DeviceIoControl with IOCTL_STORAGE_QUERY_PROPERTY to query:
/// - `StorageDeviceSeekPenaltyProperty` → if seek penalty is absent → SSD
/// - `StorageAdapterProperty` → bus type (NVMe, SATA, USB, etc.)
///
/// Falls back to heuristic model-name scanning when IOCTL access fails.
fn detect_disk_type(drive_letter: char) -> (bool, String) {
    // Try IOCTL-based detection first
    if let Some(result) = detect_disk_type_ioctl(drive_letter) {
        return result;
    }
    // Fallback: registry-based heuristic
    detect_disk_type_fallback(drive_letter)
}

fn detect_disk_type_ioctl(drive_letter: char) -> Option<(bool, String)> {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::*;
    use windows::Win32::Storage::FileSystem::*;
    use windows::Win32::System::IO::DeviceIoControl;

    // Storage property query constants (winioctl.h)
    const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D1400;
    const STORAGE_PROPERTY_ID_SEEK_PENALTY: i32 = 7; // StorageDeviceSeekPenaltyProperty
    const STORAGE_PROPERTY_ID_ADAPTER: i32 = 1; // StorageAdapterProperty

    // Bus types from winioctl.h
    const BUS_TYPE_SATA: i32 = 0x0B;
    const BUS_TYPE_NVME: i32 = 0x11;
    const BUS_TYPE_USB: i32 = 0x07;
    const BUS_TYPE_SAS: i32 = 0x0A;

    #[repr(C)]
    struct STORAGE_PROPERTY_QUERY {
        property_id: i32,
        query_type: i32,
        additional_params: [u8; 4],
    }

    #[repr(C)]
    struct STORAGE_DEVICE_SEEK_PENALTY_DESCRIPTOR {
        version: u32,
        size: u32,
        incurs_seek_penalty: u8,
    }

    #[repr(C)]
    struct STORAGE_ADAPTER_DESCRIPTOR {
        version: u32,
        size: u32,
        max_transfer_length: u32,
        physical_bytes: u32,
        alignment: u32,
        adapter_flags: u32,
        adapter_uid: u32,
        bus_type: i32,
        // remaining fields ignored
    }

    let vol_path = format!(r"\\.\{}:", drive_letter);
    unsafe {
        let handle = match CreateFileW(
            &HSTRING::from(&vol_path),
            FILE_GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        ) {
            Ok(h) => h,
            Err(_) => return None,
        };

        // Step 1: Query seek penalty (SSD = no seek penalty)
        let query = STORAGE_PROPERTY_QUERY {
            property_id: STORAGE_PROPERTY_ID_SEEK_PENALTY,
            query_type: 0, // PropertyStandardQuery
            additional_params: [0u8; 4],
        };
        let mut seek_desc: STORAGE_DEVICE_SEEK_PENALTY_DESCRIPTOR = std::mem::zeroed();
        let mut bytes_returned: u32 = 0;
        let ioctl_ok = DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(&query as *const _ as *const std::ffi::c_void),
            std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            Some(&mut seek_desc as *mut _ as *mut std::ffi::c_void),
            std::mem::size_of::<STORAGE_DEVICE_SEEK_PENALTY_DESCRIPTOR>() as u32,
            Some(&mut bytes_returned),
            None,
        )
        .is_ok();
        let is_ssd = ioctl_ok && seek_desc.incurs_seek_penalty == 0;

        // Step 2: Query bus type from adapter descriptor
        let query2 = STORAGE_PROPERTY_QUERY {
            property_id: STORAGE_PROPERTY_ID_ADAPTER,
            query_type: 0,
            additional_params: [0u8; 4],
        };
        let mut adapter_desc: STORAGE_ADAPTER_DESCRIPTOR = std::mem::zeroed();
        let mut bytes_returned2: u32 = 0;
        let bus_ok = DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(&query2 as *const _ as *const std::ffi::c_void),
            std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            Some(&mut adapter_desc as *mut _ as *mut std::ffi::c_void),
            std::mem::size_of::<STORAGE_ADAPTER_DESCRIPTOR>() as u32,
            Some(&mut bytes_returned2),
            None,
        )
        .is_ok();

        let _ = CloseHandle(handle);

        // Determine disk type string
        let bus_type = if bus_ok { adapter_desc.bus_type } else { 0 };
        let disk_type = match bus_type {
            BUS_TYPE_NVME => "NVMe".to_string(),
            BUS_TYPE_SATA => {
                if is_ssd {
                    "SATA_SSD".to_string()
                } else {
                    "HDD".to_string()
                }
            }
            BUS_TYPE_SAS => {
                if is_ssd {
                    "SAS_SSD".to_string()
                } else {
                    "SAS_HDD".to_string()
                }
            }
            BUS_TYPE_USB => {
                if is_ssd {
                    "USB_SSD".to_string()
                } else {
                    "USB_HDD".to_string()
                }
            }
            _ => {
                // Bus type unknown or other; use seek penalty as the only signal
                if is_ssd {
                    "SSD".to_string()
                } else {
                    "HDD".to_string()
                }
            }
        };

        Some((is_ssd, disk_type))
    }
}

/// Fallback detection using registry model string heuristics.
fn detect_disk_type_fallback(_drive_letter: char) -> (bool, String) {
    // We try to read the disk model from registry.
    // SCSI/IDE miniport entries contain model strings.
    let port_index = 0;
    for bus in 0..4 {
        for target in 0..16 {
            let key = format!(
                r"HKLM\HARDWARE\DEVICEMAP\Scsi\Scsi Port {}\Scsi Bus {}\Target Id {}",
                port_index, bus, target
            );
            if let Some(model) = get_registry_string(&key, "Identifier") {
                let model_upper = model.to_uppercase();
                if model_upper.contains("NVMe") || model_upper.contains("NVME") {
                    return (true, "NVMe".to_string());
                }
                if model_upper.contains("SSD") || model_upper.contains("SOLID STATE") {
                    if model_upper.contains("USB") {
                        return (true, "USB_SSD".to_string());
                    }
                    return (true, "SATA_SSD".to_string());
                }
                // Not SSD → HDD (or unknown)
                if model_upper.contains("USB") {
                    return (false, "USB_HDD".to_string());
                }
                return (false, "HDD".to_string());
            }
        }
    }
    (false, "Unknown".to_string())
}

fn get_gpu_info() -> Option<GpuInfo> {
    // Read GPU info from registry under the display adapter's driver key.
    // This key stores values set by the GPU driver (NVIDIA, AMD, Intel).
    const GPU_KEY: &str =
        r"HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}\0000";

    let name = get_registry_string(GPU_KEY, "DriverDesc");
    let driver_version = get_registry_string(GPU_KEY, "DriverVersion");

    // Read VRAM from HardwareInformation.qwMemorySize (QWORD, bytes).
    // This is written by NVIDIA/AMD/Intel drivers.
    let adapter_ram = get_registry_qword(GPU_KEY, "HardwareInformation.qwMemorySize");

    name.map(|n| GpuInfo {
        name: n,
        adapter_ram: adapter_ram.unwrap_or(0),
        driver_version: driver_version.unwrap_or_default(),
    })
}

/// Read a QWORD (u64) registry value.
fn get_registry_qword(key: &str, value_name: &str) -> Option<u64> {
    let (hive_str, path) = if let Some(pos) = key.find('\\') {
        (&key[..pos], key[pos + 1..].to_string())
    } else {
        return None;
    };

    use windows::core::*;
    use windows::Win32::System::Registry::*;

    let hive = match hive_str {
        "HKLM" => HKEY_LOCAL_MACHINE,
        "HKCU" => HKEY_CURRENT_USER,
        _ => return None,
    };

    unsafe {
        let mut hkey = std::mem::zeroed();
        if RegOpenKeyExW(hive, &HSTRING::from(&path), Some(0), KEY_READ, &mut hkey).is_err() {
            return None;
        }

        let mut buf = [0u8; 8];
        let mut buf_size = buf.len() as u32;
        let mut value_type = REG_VALUE_TYPE::default();

        let result = RegQueryValueExW(
            hkey,
            &HSTRING::from(value_name),
            None,
            Some(&mut value_type as *mut REG_VALUE_TYPE),
            Some(buf.as_mut_ptr()),
            Some(&mut buf_size),
        );

        let _ = RegCloseKey(hkey);

        if result.is_ok() && value_type == REG_QWORD && buf_size == 8 {
            Some(u64::from_ne_bytes(buf))
        } else {
            None
        }
    }
}

fn get_registry_string(key: &str, value_name: &str) -> Option<String> {
    let (hive_str, path) = if let Some(pos) = key.find('\\') {
        (&key[..pos], key[pos + 1..].to_string())
    } else {
        return None;
    };

    use windows::core::*;
    use windows::Win32::System::Registry::*;

    let hive = match hive_str {
        "HKLM" => HKEY_LOCAL_MACHINE,
        "HKCU" => HKEY_CURRENT_USER,
        _ => return None,
    };

    unsafe {
        let mut hkey = std::mem::zeroed();
        if RegOpenKeyExW(hive, &HSTRING::from(&path), Some(0), KEY_READ, &mut hkey).is_err() {
            return None;
        }

        let mut buf = [0u16; 4096];
        let mut buf_size = (buf.len() * 2) as u32;
        let mut value_type = REG_VALUE_TYPE::default();

        let result = RegQueryValueExW(
            hkey,
            &HSTRING::from(value_name),
            None,
            Some(&mut value_type as *mut REG_VALUE_TYPE),
            Some(buf.as_mut_ptr() as *mut u8),
            Some(&mut buf_size),
        );

        let _ = RegCloseKey(hkey);

        if result.is_ok() && (value_type == REG_SZ || value_type == REG_EXPAND_SZ) {
            let len = (buf_size as usize / 2).saturating_sub(1);
            Some(String::from_utf16_lossy(&buf[..len]))
        } else {
            None
        }
    }
}
