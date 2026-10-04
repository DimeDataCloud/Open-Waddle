//! What Waddle knows about other windows: their geometry (the platforms it
//! walks on), titles (for the model) and the foreground window (to hand
//! keyboard focus back after the user types into the chat box).

#[cfg(not(windows))]
mod fallback;
#[cfg(windows)]
pub(crate) mod windows;

use serde::Serialize;

/// True where the OS reports window bounds in logical points (macOS) rather than pixels.
pub const BOUNDS_ARE_LOGICAL: bool = cfg!(target_os = "macos");

/// A top-level window in screen pixels (logical points on macOS), front to back.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DesktopWindow {
    pub id: u64,
    pub title: String,
    pub app: String,
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub focused: bool,
}

/// Lists visible, non-minimised windows of other processes, front to back.
pub fn list_windows() -> Vec<DesktopWindow> {
    let own = std::process::id();
    #[cfg(windows)]
    let all = windows::list(own);
    #[cfg(not(windows))]
    let all = fallback::list(own);
    all.into_iter().filter(|w| w.w >= 80 && w.h >= 40 && !w.title.trim().is_empty()).collect()
}

/// Remembers the frontmost window of another app so focus can be restored later.
pub fn remember_foreground(windows: &[DesktopWindow]) {
    #[cfg(windows)]
    windows::remember_foreground(windows);
    #[cfg(not(windows))]
    let _ = windows;
}

/// Gives keyboard focus back to the app the user was in before talking to Waddle.
pub fn restore_foreground() {
    #[cfg(windows)]
    windows::restore_foreground();
}

/// Brings a window (by id from `list_windows`) to the front; elsewhere, the last app the user was in.
pub fn focus_window(id: Option<u64>) {
    #[cfg(windows)]
    match id {
        Some(id) => windows::focus(id),
        None => windows::restore_foreground(),
    }
    // X11 has no portable focus call without a library; use xdotool when it's installed.
    #[cfg(all(unix, not(target_os = "macos")))]
    if let Some(id) = id {
        let _ = std::process::Command::new("xdotool").args(["windowactivate", "--sync", &id.to_string()]).status();
    }
    #[cfg(target_os = "macos")]
    let _ = id;
}

/// Opens a file or folder in its default app.
pub fn open_path(path: &std::path::Path) -> anyhow::Result<()> {
    #[cfg(windows)]
    return windows::open_path(path);
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(path).spawn();
    #[cfg(not(windows))]
    r.map(|_| ()).map_err(|e| anyhow::anyhow!("couldn't open {}: {e}", path.display()))
}

/// Opens a web page in the default browser (http and https only).
pub fn open_url(url: &str) -> anyhow::Result<()> {
    anyhow::ensure!(url.starts_with("https://") || url.starts_with("http://"), "not a web address");
    #[cfg(windows)]
    return windows::open_url(url);
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(not(windows))]
    r.map(|_| ()).map_err(|e| anyhow::anyhow!("couldn't open the browser: {e}"))
}

/// Launches an application by name without going through a shell.
pub fn open_app(name: &str) -> anyhow::Result<String> {
    #[cfg(windows)]
    return windows::open_app(name);
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("open").args(["-a", name]).status()?;
        anyhow::ensure!(status.success(), "macOS could not find an app called \"{name}\"");
        Ok(format!("Opened {name}"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use std::process::{Command, Stdio};
        let quiet = |c: &mut Command| c.stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok();
        let id = name.to_lowercase().replace(' ', "-");
        if Command::new("gtk-launch").arg(&id).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
            || quiet(&mut Command::new(&id))
            || quiet(Command::new("xdg-open").arg(name))
        {
            Ok(format!("Opened {name}"))
        } else {
            anyhow::bail!("could not find an app called \"{name}\"")
        }
    }
}
