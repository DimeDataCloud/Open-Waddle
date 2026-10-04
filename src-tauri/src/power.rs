//! Whether the computer is running on battery, so Waddle can do less while it
//! is: slower window sampling, hit testing, ambient decisions and idle frames.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The answer is re-read at most this often.
const RECHECK: Duration = Duration::from_secs(30);

static ON_BATTERY: AtomicBool = AtomicBool::new(false);
static CHECKED_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// True while unplugged (or in Windows' battery saver). Cheap to call often.
pub fn on_battery() -> bool {
    let now = now_ms();
    let checked = CHECKED_MS.load(Ordering::Relaxed);
    if checked == 0 || now.saturating_sub(checked) >= RECHECK.as_millis() as u64 {
        CHECKED_MS.store(now, Ordering::Relaxed);
        ON_BATTERY.store(read(), Ordering::Relaxed);
    }
    ON_BATTERY.load(Ordering::Relaxed)
}

/// `normal` on mains power, `slow` on battery.
pub fn pace(normal: Duration, slow: Duration) -> Duration {
    if on_battery() { slow } else { normal }
}

#[cfg(windows)]
fn read() -> bool {
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    let mut s = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut s) }.is_err() {
        return false;
    }
    // ACLineStatus: 0 offline, 1 online, 255 unknown. SystemStatusFlag 1: battery saver is on.
    s.ACLineStatus == 0 || s.SystemStatusFlag == 1
}

#[cfg(target_os = "linux")]
fn read() -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/class/power_supply") else { return false };
    let mut discharging = false;
    for e in entries.flatten() {
        let p = e.path();
        let get = |f: &str| std::fs::read_to_string(p.join(f)).map(|s| s.trim().to_string()).unwrap_or_default();
        match get("type").as_str() {
            "Mains" if get("online") == "1" => return false,
            "Battery" if get("status") == "Discharging" => discharging = true,
            _ => {}
        }
    }
    discharging
}

#[cfg(not(any(windows, target_os = "linux")))]
fn read() -> bool {
    false
}
