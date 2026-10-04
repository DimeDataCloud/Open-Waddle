//! Is the user here, and what are they doing? Idle time and the front app, shared
//! by the duck's ambient behaviour and the nudge watcher.

use std::time::Instant;
use tauri::AppHandle;

use crate::bridge::{Platform, TauriHost};

/// Seconds since the last keyboard or mouse input, where the OS tells us.
#[cfg(windows)]
fn os_idle_secs() -> Option<u64> {
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    // SAFETY: plain Win32 calls on a correctly sized struct.
    unsafe {
        if !GetLastInputInfo(&mut info).as_bool() {
            return None;
        }
        Some(u64::from(GetTickCount().wrapping_sub(info.dwTime)) / 1000)
    }
}

#[cfg(not(windows))]
fn os_idle_secs() -> Option<u64> {
    None
}

pub struct Sample {
    /// The front app's name (never its window title).
    pub app: String,
    pub window: Option<Platform>,
    pub fullscreen: bool,
    pub idle_secs: u64,
    /// Mouse position in physical desktop pixels.
    pub cursor: (f64, f64),
}

pub struct Presence {
    last_cursor: (f64, f64),
    last_input: Instant,
}

impl Presence {
    pub fn new() -> Self {
        Self { last_cursor: (0.0, 0.0), last_input: Instant::now() }
    }

    pub fn sample(&mut self, app: &AppHandle, host: &TauriHost) -> Sample {
        if let Ok(p) = app.cursor_position() {
            if (p.x, p.y) != self.last_cursor {
                self.last_cursor = (p.x, p.y);
                self.last_input = Instant::now();
            }
        }
        let front = host.front_window();
        // Where the OS can't say, mouse movement is the best sign of life.
        let idle_secs = os_idle_secs().unwrap_or(u64::MAX).min(self.last_input.elapsed().as_secs());
        Sample {
            app: front.as_ref().map(|w| w.0.clone()).unwrap_or_default(),
            fullscreen: front.as_ref().is_some_and(|w| w.2),
            window: front.map(|w| w.1),
            idle_secs,
            cursor: self.last_cursor,
        }
    }
}
