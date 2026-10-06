//! `arrange_window`: snapping, maximizing and moving windows between monitors
//! through the OS instead of key presses, clicks and screenshots to check.
//! The geometry is worked out here (pure, so it's tested); the host applies it.

use serde::{Deserialize, Serialize};

/// What to do with the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Arrange {
    /// Bring it to the front (restoring it if it was minimized).
    Front,
    LeftHalf,
    RightHalf,
    Maximize,
    Minimize,
    Restore,
}

impl Arrange {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_lowercase().replace([' ', '-'], "_").as_str() {
            "front" | "focus" | "show" | "bring_to_front" | "activate" => Arrange::Front,
            "left_half" | "left" | "snap_left" => Arrange::LeftHalf,
            "right_half" | "right" | "snap_right" => Arrange::RightHalf,
            "maximize" | "maximise" | "fill" | "full" | "fullscreen" => Arrange::Maximize,
            "minimize" | "minimise" | "hide" => Arrange::Minimize,
            "restore" | "unmaximize" => Arrange::Restore,
            _ => return None,
        })
    }

    fn words(self) -> &'static str {
        match self {
            Arrange::Front => "brought to the front",
            Arrange::LeftHalf => "snapped to the left half",
            Arrange::RightHalf => "snapped to the right half",
            Arrange::Maximize => "maximized",
            Arrange::Minimize => "minimized",
            Arrange::Restore => "restored",
        }
    }
}

/// Which monitor to put the window on first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MonitorPick {
    Left,
    Right,
    Primary,
    /// The next monitor after the one the window is on.
    Other,
}

impl MonitorPick {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_lowercase().as_str() {
            "left" | "leftmost" | "west" => MonitorPick::Left,
            "right" | "rightmost" | "east" => MonitorPick::Right,
            "primary" | "main" => MonitorPick::Primary,
            "other" | "next" | "second" | "secondary" => MonitorPick::Other,
            _ => return None,
        })
    }
}

/// A rectangle in physical screen pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    fn center(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }
}

/// A monitor's usable area (without the taskbar).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Monitor {
    pub work: Rect,
    pub primary: bool,
}

/// The monitor a window is on: the one holding its centre, else the nearest.
pub fn monitor_of(monitors: &[Monitor], window: Rect) -> Option<usize> {
    let (cx, cy) = window.center();
    monitors.iter().position(|m| m.work.contains(cx, cy)).or_else(|| {
        monitors
            .iter()
            .enumerate()
            .min_by_key(|(_, m)| {
                let (mx, my) = m.work.center();
                (i64::from(mx - cx)).pow(2) + (i64::from(my - cy)).pow(2)
            })
            .map(|(i, _)| i)
    })
}

/// The monitor to use: `pick` if given, else the one the window is on.
pub fn pick_monitor(monitors: &[Monitor], current: Option<usize>, pick: Option<MonitorPick>) -> Option<usize> {
    if monitors.is_empty() {
        return None;
    }
    let by_x = |rev: bool| {
        let mut idx: Vec<usize> = (0..monitors.len()).collect();
        idx.sort_by_key(|&i| (monitors[i].work.x, monitors[i].work.y));
        if rev {
            idx.last().copied()
        } else {
            idx.first().copied()
        }
    };
    match pick {
        None => current.or_else(|| monitors.iter().position(|m| m.primary)).or(Some(0)),
        Some(MonitorPick::Left) => by_x(false),
        Some(MonitorPick::Right) => by_x(true),
        Some(MonitorPick::Primary) => monitors.iter().position(|m| m.primary).or(Some(0)),
        Some(MonitorPick::Other) => {
            let mut idx: Vec<usize> = (0..monitors.len()).collect();
            idx.sort_by_key(|&i| (monitors[i].work.x, monitors[i].work.y));
            let at = current.and_then(|c| idx.iter().position(|&i| i == c)).unwrap_or(0);
            Some(idx[(at + 1) % idx.len()])
        }
    }
}

/// Where the window goes on that monitor. None: only its state changes
/// (front, minimize, restore) and it stays where it is.
pub fn place(work: Rect, how: Arrange, moving: bool) -> Option<Rect> {
    let half = work.w / 2;
    match how {
        Arrange::LeftHalf => Some(Rect { x: work.x, y: work.y, w: half, h: work.h }),
        Arrange::RightHalf => Some(Rect { x: work.x + half, y: work.y, w: work.w - half, h: work.h }),
        Arrange::Maximize => Some(work),
        // Moving to another monitor without a layout: a comfortable centred window.
        Arrange::Front | Arrange::Restore if moving => {
            let (w, h) = (work.w * 4 / 5, work.h * 4 / 5);
            Some(Rect { x: work.x + (work.w - w) / 2, y: work.y + (work.h - h) / 2, w, h })
        }
        _ => None,
    }
}

/// What the model is told once it's done.
pub fn describe(title: &str, how: Arrange, monitor: Option<(usize, usize)>) -> String {
    let screen = match monitor {
        Some((i, n)) if n > 1 => format!(" on screen {} of {n}", i + 1),
        _ => String::new(),
    };
    format!("\"{title}\" {}{screen}.", how.words())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two() -> Vec<Monitor> {
        // A laptop on the left (primary) and a bigger screen to its right, both with a taskbar.
        vec![
            Monitor { work: Rect { x: 0, y: 0, w: 1920, h: 1032 }, primary: true },
            Monitor { work: Rect { x: 1920, y: 0, w: 2560, h: 1392 }, primary: false },
        ]
    }

    #[test]
    fn halves_split_the_work_area_without_gaps() {
        let work = Rect { x: 1920, y: 0, w: 2561, h: 1392 };
        let l = place(work, Arrange::LeftHalf, false).unwrap();
        let r = place(work, Arrange::RightHalf, false).unwrap();
        assert_eq!(l, Rect { x: 1920, y: 0, w: 1280, h: 1392 });
        assert_eq!(r.x, l.x + l.w);
        assert_eq!(l.w + r.w, work.w);
        assert_eq!(place(work, Arrange::Maximize, false), Some(work));
        assert_eq!(place(work, Arrange::Minimize, false), None);
        assert_eq!(place(work, Arrange::Front, false), None);
        let moved = place(work, Arrange::Front, true).unwrap();
        assert!(work.contains(moved.x, moved.y) && moved.w < work.w);
    }

    #[test]
    fn monitors_are_picked_by_side_primary_or_next() {
        let m = two();
        let on_left = monitor_of(&m, Rect { x: 100, y: 100, w: 800, h: 600 });
        assert_eq!(on_left, Some(0));
        assert_eq!(monitor_of(&m, Rect { x: 3000, y: 200, w: 800, h: 600 }), Some(1));
        // Mostly off every screen: the nearest one.
        assert_eq!(monitor_of(&m, Rect { x: 5000, y: 100, w: 400, h: 300 }), Some(1));
        assert_eq!(pick_monitor(&m, on_left, None), Some(0));
        assert_eq!(pick_monitor(&m, on_left, Some(MonitorPick::Right)), Some(1));
        assert_eq!(pick_monitor(&m, Some(1), Some(MonitorPick::Left)), Some(0));
        assert_eq!(pick_monitor(&m, Some(1), Some(MonitorPick::Primary)), Some(0));
        assert_eq!(pick_monitor(&m, Some(0), Some(MonitorPick::Other)), Some(1));
        assert_eq!(pick_monitor(&m, Some(1), Some(MonitorPick::Other)), Some(0));
        assert_eq!(pick_monitor(&[], None, Some(MonitorPick::Right)), None);
    }

    #[test]
    fn spoken_words_map_to_actions() {
        assert_eq!(Arrange::parse("left half"), Some(Arrange::LeftHalf));
        assert_eq!(Arrange::parse("snap-right"), Some(Arrange::RightHalf));
        assert_eq!(Arrange::parse("Maximise"), Some(Arrange::Maximize));
        assert_eq!(Arrange::parse("focus"), Some(Arrange::Front));
        assert_eq!(Arrange::parse("explode"), None);
        assert_eq!(MonitorPick::parse("Right"), Some(MonitorPick::Right));
        assert_eq!(MonitorPick::parse("west"), Some(MonitorPick::Left));
        assert_eq!(describe("GitHub", Arrange::RightHalf, Some((1, 2))), "\"GitHub\" snapped to the right half on screen 2 of 2.");
        assert_eq!(describe("Notes", Arrange::Maximize, Some((0, 1))), "\"Notes\" maximized.");
    }
}
