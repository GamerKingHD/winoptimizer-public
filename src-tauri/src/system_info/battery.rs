//! Battery / power status - reads Windows power state via
//! GetSystemPowerStatus including charge %, AC status, and estimated runtime.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatteryInfo {
    pub present: bool,
    pub charge_percent: f32,
    pub charger_connected: bool,
    pub charging: bool,
    pub time_remaining_minutes: i32, // -1 if on AC or unknown
    pub saving_mode: bool,
    pub is_laptop: bool, // true if a battery is detected (even if AC)
}

/// Gather battery / power status.
pub fn gather() -> BatteryInfo {
    use windows::Win32::System::Power::GetSystemPowerStatus;
    use windows::Win32::System::Power::SYSTEM_POWER_STATUS;

    unsafe {
        let mut sps: SYSTEM_POWER_STATUS = std::mem::zeroed();
        if GetSystemPowerStatus(&mut sps).is_ok() {
            let charge = sps.BatteryLifePercent as f32 / 100.0;
            let present = sps.BatteryFlag & 128 == 0; // 128 = no battery

            BatteryInfo {
                present,
                charge_percent: if charge > 1.0 { 0.0 } else { charge * 100.0 },
                charger_connected: sps.ACLineStatus == 1,
                charging: sps.BatteryFlag & 8 != 0,
                time_remaining_minutes: if sps.BatteryLifeTime == u32::MAX || sps.ACLineStatus == 1
                {
                    -1
                } else {
                    (sps.BatteryLifeTime / 60) as i32
                },
                saving_mode: sps.SystemStatusFlag == 1,
                is_laptop: present,
            }
        } else {
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
    }
}
