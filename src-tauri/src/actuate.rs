//! The Hands that touch the GUI: synthetic mouse/keyboard input (enigo) and
//! screenshots (xcap). All coordinates here are physical desktop pixels.

use anyhow::{anyhow, Context};
use base64::Engine;
use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use std::time::Duration;
use waddle_core::llm::ImageData;
use waddle_core::tools::MouseButton;

fn enigo() -> anyhow::Result<Enigo> {
    Enigo::new(&Settings::default()).map_err(|e| anyhow!("input simulation unavailable: {e}"))
}

/// Self-test: input simulation can start and read the cursor (physical pixels).
pub fn probe_input() -> anyhow::Result<(i32, i32)> {
    enigo()?.location().map_err(|e| anyhow!("can't read the cursor: {e}"))
}

pub fn click(x: i32, y: i32, button: MouseButton, double: bool) -> anyhow::Result<()> {
    let mut e = enigo()?;
    let home = e.location().ok();
    e.move_mouse(x, y, Coordinate::Abs)?;
    std::thread::sleep(Duration::from_millis(40));
    let b = match button {
        MouseButton::Left => Button::Left,
        MouseButton::Right => Button::Right,
    };
    e.button(b, Direction::Click)?;
    if double {
        std::thread::sleep(Duration::from_millis(60));
        e.button(b, Direction::Click)?;
    }
    // Hand the cursor back where the user left it.
    if let Some((hx, hy)) = home {
        std::thread::sleep(Duration::from_millis(40));
        let _ = e.move_mouse(hx, hy, Coordinate::Abs);
    }
    Ok(())
}

/// Mouse-wheel notches at a point (or where the cursor is); positive = down / right.
pub fn scroll(at: Option<(i32, i32)>, dx: i32, dy: i32) -> anyhow::Result<()> {
    let mut e = enigo()?;
    let home = e.location().ok();
    if let Some((x, y)) = at {
        e.move_mouse(x, y, Coordinate::Abs)?;
        std::thread::sleep(Duration::from_millis(40));
    }
    // A notch at a time reads as real scrolling to apps that smooth it.
    for (n, axis) in [(dy, Axis::Vertical), (dx, Axis::Horizontal)] {
        for _ in 0..n.unsigned_abs() {
            e.scroll(n.signum(), axis)?;
            std::thread::sleep(Duration::from_millis(15));
        }
    }
    if let (Some(_), Some((hx, hy))) = (at, home) {
        let _ = e.move_mouse(hx, hy, Coordinate::Abs);
    }
    Ok(())
}

/// Press, glide to the target in small steps (so apps register a drag), release.
pub fn drag(from: (i32, i32), to: (i32, i32)) -> anyhow::Result<()> {
    let mut e = enigo()?;
    let home = e.location().ok();
    e.move_mouse(from.0, from.1, Coordinate::Abs)?;
    std::thread::sleep(Duration::from_millis(60));
    e.button(Button::Left, Direction::Press)?;
    let steps = 20;
    let mut result = Ok(());
    for i in 1..=steps {
        let x = from.0 + (to.0 - from.0) * i / steps;
        let y = from.1 + (to.1 - from.1) * i / steps;
        if let Err(err) = e.move_mouse(x, y, Coordinate::Abs) {
            result = Err(err);
            break;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    std::thread::sleep(Duration::from_millis(80));
    let released = e.button(Button::Left, Direction::Release);
    result?;
    released?;
    if let Some((hx, hy)) = home {
        std::thread::sleep(Duration::from_millis(40));
        let _ = e.move_mouse(hx, hy, Coordinate::Abs);
    }
    Ok(())
}

pub fn read_clipboard() -> anyhow::Result<String> {
    let mut c = arboard::Clipboard::new().map_err(|e| anyhow!("clipboard unavailable: {e}"))?;
    match c.get_text() {
        Ok(t) => Ok(t),
        Err(arboard::Error::ContentNotAvailable) => Ok(String::new()),
        Err(e) => Err(anyhow!("couldn't read the clipboard: {e}")),
    }
}

pub fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    let mut c = arboard::Clipboard::new().map_err(|e| anyhow!("clipboard unavailable: {e}"))?;
    c.set_text(text.to_string()).map_err(|e| anyhow!("couldn't set the clipboard: {e}"))?;
    // On Linux the clipboard lives in the process that set it; give managers a moment to take it.
    #[cfg(target_os = "linux")]
    std::thread::sleep(Duration::from_millis(100));
    Ok(())
}

pub fn type_text(text: &str) -> anyhow::Result<()> {
    let mut e = enigo()?;
    e.text(text).context("typing failed")?;
    Ok(())
}

pub fn parse_key(name: &str) -> anyhow::Result<Key> {
    let n = name.trim().to_lowercase();
    Ok(match n.as_str() {
        "ctrl" | "control" => Key::Control,
        "shift" => Key::Shift,
        "alt" | "option" | "opt" => Key::Alt,
        "win" | "windows" | "meta" | "cmd" | "command" | "super" => Key::Meta,
        "enter" | "return" => Key::Return,
        "tab" => Key::Tab,
        "esc" | "escape" => Key::Escape,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "space" | "spacebar" => Key::Space,
        "up" | "arrowup" => Key::UpArrow,
        "down" | "arrowdown" => Key::DownArrow,
        "left" | "arrowleft" => Key::LeftArrow,
        "right" | "arrowright" => Key::RightArrow,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "pgup" => Key::PageUp,
        "pagedown" | "pgdn" => Key::PageDown,
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        "plus" => Key::Unicode('+'),
        other => {
            let mut chars = other.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Key::Unicode(c),
                _ => return Err(anyhow!("unknown key `{name}`")),
            }
        }
    })
}

/// Splits "ctrl+shift+s" into keys; a trailing "+" means the plus key.
pub fn parse_combo(combo: &str) -> anyhow::Result<Vec<Key>> {
    let combo = combo.trim();
    let mut parts: Vec<&str> = combo.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    if combo.ends_with("++") || combo == "+" {
        parts.push("plus");
    }
    anyhow::ensure!(!parts.is_empty(), "no keys given");
    parts.into_iter().map(parse_key).collect()
}

pub fn press_combo(keys: &[Key]) -> anyhow::Result<()> {
    let mut e = enigo()?;
    let (mods, last) = keys.split_at(keys.len() - 1);
    for k in mods {
        e.key(*k, Direction::Press)?;
    }
    let result = e.key(last[0], Direction::Click);
    for k in mods.iter().rev() {
        let _ = e.key(*k, Direction::Release);
    }
    result?;
    Ok(())
}

/// Captures the primary monitor and scales it to logical size, so screenshot
/// pixels line up with the coordinates the model is told about.
pub fn screenshot(logical_w: u32, logical_h: u32) -> anyhow::Result<(ImageData, u32, u32)> {
    let monitors = xcap::Monitor::all().context("listing monitors")?;
    let monitor = monitors
        .iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .or_else(|| monitors.first())
        .ok_or_else(|| anyhow!("no monitor found"))?;
    let img = monitor.capture_image().context("screen capture failed (on macOS, grant Screen Recording permission)")?;
    let img = if img.width() != logical_w || img.height() != logical_h {
        image::imageops::resize(&img, logical_w.max(1), logical_h.max(1), image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img).to_rgb8().write_to(&mut png, image::ImageFormat::Png)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
    Ok((ImageData { mime: "image/png".into(), base64: b64 }, logical_w, logical_h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_combos() {
        assert_eq!(parse_combo("ctrl+s").unwrap(), vec![Key::Control, Key::Unicode('s')]);
        assert_eq!(parse_combo("Ctrl + Shift + N").unwrap(), vec![Key::Control, Key::Shift, Key::Unicode('n')]);
        assert_eq!(parse_combo("enter").unwrap(), vec![Key::Return]);
        assert_eq!(parse_combo("ctrl++").unwrap(), vec![Key::Control, Key::Unicode('+')]);
        assert!(parse_combo("ctrl+banana").is_err());
        assert!(parse_combo("").is_err());
    }
}
