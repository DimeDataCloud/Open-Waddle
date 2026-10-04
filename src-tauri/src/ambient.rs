//! What the duck does between tasks. A quick decision model (Jev) picks from
//! perch / explore / watch / nap / give space / wander, given the front app's
//! name, whether it fills the screen and how long the user has been idle.
//! Window titles never leave the computer. It only asks when something changed
//! (at most every 20 s, at least every 2.5 min), so a day costs a few cents at most.

use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use waddle_core::decide::Situation;

use crate::bridge::TauriHost;
use crate::AppState;

const TICK: Duration = Duration::from_secs(4);
const MIN_GAP: Duration = Duration::from_secs(20);
const MAX_GAP: Duration = Duration::from_secs(150);

/// Coarse idle levels; crossing one is worth a new decision.
fn idle_level(secs: u64) -> u8 {
    match secs {
        0..=59 => 0,
        60..=599 => 1,
        _ => 2,
    }
}

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

pub fn spawn(app: AppHandle, host: Arc<TauriHost>) {
    tauri::async_runtime::spawn(async move {
        let mut last_cursor = (0.0, 0.0);
        let mut last_input = Instant::now();
        let mut app_since = Instant::now();
        let mut front = String::new();
        let mut previous: Option<String> = None;
        let mut asked: Option<(Instant, String, bool, u8)> = None;
        loop {
            tokio::time::sleep(TICK).await;
            let Some(state) = app.try_state::<AppState>() else { continue };
            let (on, decider) = {
                let s = state.settings.read().unwrap();
                (s.ambient_brain && s.wander, state.decider.read().unwrap().clone())
            };
            if let Ok(p) = app.cursor_position() {
                if (p.x, p.y) != last_cursor {
                    last_cursor = (p.x, p.y);
                    last_input = Instant::now();
                }
            }
            let window = host.front_window();
            let app_name = window.as_ref().map(|w| w.0.clone()).unwrap_or_default();
            if app_name != front {
                previous = Some(std::mem::replace(&mut front, app_name.clone())).filter(|p| !p.is_empty());
                app_since = Instant::now();
            }
            let Some(decider) = decider.filter(|_| on && !host.is_busy()) else { continue };
            let idle = os_idle_secs().unwrap_or(u64::MAX).min(last_input.elapsed().as_secs());
            let fullscreen = window.as_ref().is_some_and(|w| w.2);
            let changed = match &asked {
                None => true,
                Some((at, a, f, lvl)) => {
                    let gap = at.elapsed();
                    gap >= MAX_GAP || (gap >= MIN_GAP && (*a != app_name || *f != fullscreen || *lvl != idle_level(idle)))
                }
            };
            if !changed {
                continue;
            }
            asked = Some((Instant::now(), app_name.clone(), fullscreen, idle_level(idle)));
            let situation = Situation {
                app: app_name,
                fullscreen,
                idle_secs: idle,
                app_secs: app_since.elapsed().as_secs(),
                previous_app: previous.clone(),
                local_time: waddle_core::reminders::now_line(),
            };
            let Some((intent, p)) = decider.duck_intent(&situation).await else { continue };
            log::info!("duck intent: {intent:?} ({p:.2})");
            let cursor = host.physical_to_overlay(last_cursor.0, last_cursor.1);
            host.emit_overlay(
                "duck:intent",
                json!({ "intent": intent, "window": window.map(|w| w.1), "cursor": { "x": cursor.0, "y": cursor.1 } }),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_levels() {
        assert_eq!([idle_level(0), idle_level(59), idle_level(60), idle_level(599), idle_level(600)], [0, 0, 1, 1, 2]);
    }
}
