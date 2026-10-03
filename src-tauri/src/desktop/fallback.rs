//! macOS and Linux window listing through x-win. On macOS the order is
//! front to back (CGWindowList). Wayland hides other windows' positions, so
//! Waddle only walks along the screen bottom there.

use super::DesktopWindow;

pub fn list(own_pid: u32) -> Vec<DesktopWindow> {
    let active = x_win::get_active_window().ok().map(|w| w.id);
    let windows = match x_win::get_open_windows() {
        Ok(w) => w,
        Err(e) => {
            log::debug!("x-win: {e}");
            return vec![];
        }
    };
    windows
        .into_iter()
        .filter(|w| w.info.process_id != own_pid)
        .map(|w| DesktopWindow {
            id: w.id as u64,
            focused: Some(w.id) == active,
            title: w.title,
            app: if w.info.name.is_empty() { w.info.exec_name } else { w.info.name },
            x: w.position.x,
            y: w.position.y,
            w: w.position.width,
            h: w.position.height,
        })
        .collect()
}
