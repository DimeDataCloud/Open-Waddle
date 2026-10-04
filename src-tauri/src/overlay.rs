//! Click-through for the full-screen transparent overlay.
//!
//! The OS can only make a whole window click-through or not. So the frontend
//! reports the rectangles it draws (the duck, its bubbles, the chat box), and
//! a light thread polls the cursor: the overlay accepts clicks only while the
//! cursor is over one of them. During synthetic input (Waddle clicking for the
//! user) it is forced click-through so Waddle never clicks on itself.

use serde::Deserialize;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewWindow};

#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    fn contains(&self, x: f64, y: f64, margin: f64) -> bool {
        x >= self.x - margin && x <= self.x + self.w + margin && y >= self.y - margin && y <= self.y + self.h + margin
    }
}

/// Where the overlay sits, in physical pixels, and the display scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    pub origin_x: i32,
    pub origin_y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    /// Full primary-monitor size in physical pixels (the overlay covers its work area).
    pub screen_w: u32,
    pub screen_h: u32,
    pub screen_x: i32,
    pub screen_y: i32,
}

impl Geometry {
    /// Logical screen coordinates → logical overlay-local coordinates.
    pub fn screen_to_overlay(&self, x: f64, y: f64) -> (f64, f64) {
        (x - (self.origin_x - self.screen_x) as f64 / self.scale, y - (self.origin_y - self.screen_y) as f64 / self.scale)
    }
    /// Logical screen coordinates → physical desktop pixels (for input injection).
    pub fn screen_to_physical(&self, x: f64, y: f64) -> (i32, i32) {
        ((x * self.scale).round() as i32 + self.screen_x, (y * self.scale).round() as i32 + self.screen_y)
    }
    pub fn screen_logical(&self) -> (f64, f64) {
        (self.screen_w as f64 / self.scale, self.screen_h as f64 / self.scale)
    }
}

pub struct OverlayState {
    rects: RwLock<Vec<Rect>>,
    capture: AtomicBool,
    forced: AtomicU32,
    ignoring: AtomicBool,
    pub geometry: RwLock<Geometry>,
}

impl OverlayState {
    pub fn new(geometry: Geometry) -> Arc<Self> {
        Arc::new(Self {
            rects: RwLock::default(),
            capture: AtomicBool::new(false),
            forced: AtomicU32::new(0),
            ignoring: AtomicBool::new(false),
            geometry: RwLock::new(geometry),
        })
    }

    pub fn set_rects(&self, rects: Vec<Rect>) {
        *self.rects.write().unwrap() = rects;
    }

    /// Keeps the overlay interactive regardless of cursor position (dragging, typing in chat).
    pub fn set_capture(&self, on: bool) {
        self.capture.store(on, Ordering::Relaxed);
    }

    pub fn geometry(&self) -> Geometry {
        *self.geometry.read().unwrap()
    }

    /// Forces click-through until the guard drops.
    pub fn force_passthrough(self: &Arc<Self>, window: &WebviewWindow) -> PassthroughGuard {
        self.forced.fetch_add(1, Ordering::SeqCst);
        if !self.ignoring.swap(true, Ordering::SeqCst) {
            let _ = window.set_ignore_cursor_events(true);
        }
        PassthroughGuard { state: self.clone() }
    }

    fn wants_ignore(&self, cursor_x: f64, cursor_y: f64) -> bool {
        if self.forced.load(Ordering::SeqCst) > 0 {
            return true;
        }
        if self.capture.load(Ordering::Relaxed) {
            return false;
        }
        let g = self.geometry();
        let lx = (cursor_x - g.origin_x as f64) / g.scale;
        let ly = (cursor_y - g.origin_y as f64) / g.scale;
        !self.rects.read().unwrap().iter().any(|r| r.contains(lx, ly, 2.0))
    }
}

pub struct PassthroughGuard {
    state: Arc<OverlayState>,
}

impl Drop for PassthroughGuard {
    fn drop(&mut self) {
        self.state.forced.fetch_sub(1, Ordering::SeqCst);
    }
}

pub fn spawn_hit_test(app: AppHandle, state: Arc<OverlayState>) {
    std::thread::Builder::new()
        .name("waddle-hit-test".into())
        .spawn(move || {
            let Some(window) = app.get_webview_window("overlay") else { return };
            let _ = window.set_ignore_cursor_events(true);
            state.ignoring.store(true, Ordering::SeqCst);
            loop {
                std::thread::sleep(crate::power::pace(Duration::from_millis(25), Duration::from_millis(50)));
                let Ok(pos) = app.cursor_position() else { continue };
                let want = state.wants_ignore(pos.x, pos.y);
                if state.ignoring.load(Ordering::SeqCst) != want && window.set_ignore_cursor_events(want).is_ok() {
                    state.ignoring.store(want, Ordering::SeqCst);
                }
            }
        })
        .expect("hit-test thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geo() -> Geometry {
        Geometry { origin_x: 0, origin_y: 0, width: 2880, height: 1800, scale: 2.0, screen_w: 2880, screen_h: 1920, screen_x: 0, screen_y: 0 }
    }

    #[test]
    fn hit_testing_uses_logical_overlay_coordinates() {
        let s = OverlayState::new(geo());
        s.set_rects(vec![Rect { x: 100.0, y: 100.0, w: 64.0, h: 56.0 }]);
        assert!(!s.wants_ignore(250.0, 250.0), "physical (250,250) is logical (125,125), inside the duck");
        assert!(s.wants_ignore(100.0, 100.0), "logical (50,50) is outside");
        s.set_capture(true);
        assert!(!s.wants_ignore(0.0, 0.0));
    }

    #[test]
    fn coordinate_conversions_account_for_offset_work_area() {
        let g = Geometry { origin_y: 96, ..geo() };
        assert_eq!(g.screen_to_overlay(100.0, 100.0), (100.0, 52.0));
        assert_eq!(g.screen_to_physical(100.0, 100.0), (200, 200));
        assert_eq!(g.screen_logical(), (1440.0, 960.0));
    }
}
