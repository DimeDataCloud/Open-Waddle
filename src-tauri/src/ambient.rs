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
use crate::presence::Presence;
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

pub fn spawn(app: AppHandle, host: Arc<TauriHost>) {
    tauri::async_runtime::spawn(async move {
        let mut presence = Presence::new();
        let mut app_since = Instant::now();
        let mut front = String::new();
        let mut previous: Option<String> = None;
        let mut asked: Option<(Instant, String, bool, u8)> = None;
        let mut battery = None;
        loop {
            // Half as often on battery.
            tokio::time::sleep(crate::power::pace(TICK, TICK * 2)).await;
            let Some(state) = app.try_state::<AppState>() else { continue };
            // The overlay slows its idle frames on battery too.
            let now_battery = crate::power::on_battery();
            if battery != Some(now_battery) {
                battery = Some(now_battery);
                host.emit_overlay("power:battery", now_battery);
            }
            let (on, decider) = {
                let s = state.settings.read().unwrap();
                (s.ambient_brain && s.wander, state.decider.read().unwrap().clone())
            };
            let now = presence.sample(&app, &host);
            if now.app != front {
                previous = Some(std::mem::replace(&mut front, now.app.clone())).filter(|p| !p.is_empty());
                app_since = Instant::now();
            }
            let Some(decider) = decider.filter(|_| on && !host.is_busy()) else { continue };
            let (idle, fullscreen, app_name) = (now.idle_secs, now.fullscreen, now.app.clone());
            let changed = match &asked {
                None => true,
                Some((at, a, f, lvl)) => {
                    let gap = at.elapsed();
                    gap >= MAX_GAP || (gap >= crate::power::pace(MIN_GAP, MIN_GAP * 2) && (*a != app_name || *f != fullscreen || *lvl != idle_level(idle)))
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
            let cursor = host.physical_to_overlay(now.cursor.0, now.cursor.1);
            host.emit_overlay(
                "duck:intent",
                json!({ "intent": intent, "window": now.window, "cursor": { "x": cursor.0, "y": cursor.1 } }),
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
