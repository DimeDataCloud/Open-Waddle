//! Native Win32 window tracking. EnumWindows walks top-level windows in
//! z-order (front first). DWM's extended frame bounds give the visible frame
//! without the invisible resize border, and cloaked windows (hidden UWP shells,
//! other virtual desktops) are skipped.

use std::collections::HashMap;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Mutex;

use windows::core::{BOOL, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetForegroundWindow, GetWindowLongW, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetForegroundWindow, GWL_EXSTYLE, SW_SHOWNORMAL, WS_EX_TOOLWINDOW,
};

use super::DesktopWindow;

const SHELL_CLASSES: &[&str] = &["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

static LAST_FOREGROUND: AtomicIsize = AtomicIsize::new(0);
static EXE_NAMES: Mutex<Option<HashMap<u32, String>>> = Mutex::new(None);

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let out = &mut *(lparam.0 as *mut Vec<HWND>);
    out.push(hwnd);
    BOOL(1)
}

fn wide_to_string(buf: &[u16]) -> String {
    String::from_utf16_lossy(buf).trim_end_matches('\0').to_string()
}

fn exe_name(pid: u32) -> String {
    let mut cache = EXE_NAMES.lock().unwrap();
    let cache = cache.get_or_insert_with(HashMap::new);
    if let Some(n) = cache.get(&pid) {
        return n.clone();
    }
    let name = unsafe {
        match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(handle) => {
                let mut buf = [0u16; 1024];
                let mut len = buf.len() as u32;
                let ok = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
                let _ = CloseHandle(handle);
                if ok {
                    let full = wide_to_string(&buf[..len as usize]);
                    Path::new(&full).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or(full)
                } else {
                    String::new()
                }
            }
            Err(_) => String::new(),
        }
    };
    if cache.len() > 512 {
        cache.clear();
    }
    cache.insert(pid, name.clone());
    name
}

pub fn list(own_pid: u32) -> Vec<DesktopWindow> {
    let mut handles: Vec<HWND> = vec![];
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut handles as *mut _ as isize));
    }
    let foreground = unsafe { GetForegroundWindow() };
    let mut out = vec![];
    for hwnd in handles {
        unsafe {
            if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
                continue;
            }
            if GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
                continue;
            }
            let mut cloaked: u32 = 0;
            if DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut c_void, 4).is_ok() && cloaked != 0 {
                continue;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == own_pid {
                continue;
            }
            let mut class = [0u16; 128];
            let n = GetClassNameW(hwnd, &mut class);
            if SHELL_CLASSES.contains(&wide_to_string(&class[..n.max(0) as usize]).as_str()) {
                continue;
            }
            let len = GetWindowTextLengthW(hwnd);
            if len <= 0 {
                continue;
            }
            let mut title = vec![0u16; len as usize + 1];
            let n = GetWindowTextW(hwnd, &mut title);
            let mut rect = RECT::default();
            if DwmGetWindowAttribute(
                hwnd,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut rect as *mut _ as *mut c_void,
                std::mem::size_of::<RECT>() as u32,
            )
            .is_err()
            {
                continue;
            }
            out.push(DesktopWindow {
                id: hwnd.0 as usize as u64,
                title: wide_to_string(&title[..n.max(0) as usize]),
                app: exe_name(pid),
                x: rect.left,
                y: rect.top,
                w: rect.right - rect.left,
                h: rect.bottom - rect.top,
                focused: hwnd == foreground,
            });
        }
    }
    out
}

pub fn remember_foreground(windows: &[DesktopWindow]) {
    if let Some(w) = windows.iter().find(|w| w.focused) {
        LAST_FOREGROUND.store(w.id as isize, Ordering::Relaxed);
    }
}

pub fn restore_foreground() {
    let raw = LAST_FOREGROUND.load(Ordering::Relaxed);
    if raw != 0 {
        unsafe {
            let _ = SetForegroundWindow(HWND(raw as *mut c_void));
        }
    }
}

pub fn focus(id: u64) {
    unsafe {
        let _ = SetForegroundWindow(HWND(id as isize as *mut c_void));
    }
}

pub fn open_path(path: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(shell_open(&path.to_string_lossy()), "Windows couldn't open {}", path.display());
    Ok(())
}

fn shell_open(target: &str) -> bool {
    let file = HSTRING::from(target);
    let verb = HSTRING::from("open");
    let result = unsafe { ShellExecuteW(None, &verb, &file, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    // Values above 32 mean success.
    result.0 as usize > 32
}

fn start_menu_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![];
    for (var, tail) in [("ProgramData", r"Microsoft\Windows\Start Menu\Programs"), ("APPDATA", r"Microsoft\Windows\Start Menu\Programs")] {
        if let Ok(base) = std::env::var(var) {
            dirs.push(Path::new(&base).join(tail));
        }
    }
    dirs
}

fn find_shortcuts(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() && depth < 4 {
            find_shortcuts(&p, out, depth + 1);
        } else if p.extension().map(|x| x.eq_ignore_ascii_case("lnk")).unwrap_or(false) {
            out.push(p);
        }
    }
}

/// Best Start Menu shortcut for a name: exact stem match, then prefix, then substring.
fn best_shortcut(name: &str) -> Option<PathBuf> {
    let want = name.to_lowercase();
    let mut all = vec![];
    for d in start_menu_dirs() {
        find_shortcuts(&d, &mut all, 0);
    }
    let stem = |p: &PathBuf| p.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    let skip = |s: &str| s.contains("uninstall") || s.contains("readme") || s.contains("help");
    all.iter()
        .find(|p| stem(p) == want)
        .or_else(|| all.iter().find(|p| stem(p).starts_with(&want) && !skip(&stem(p))))
        .or_else(|| all.iter().find(|p| stem(p).contains(&want) && !skip(&stem(p))))
        .cloned()
}

pub fn open_app(name: &str) -> anyhow::Result<String> {
    let name = name.trim();
    let aliases: &[(&str, &str)] = &[
        ("calculator", "calc"),
        ("notepad", "notepad"),
        ("file explorer", "explorer"),
        ("explorer", "explorer"),
        ("settings", "ms-settings:"),
        ("edge", "msedge"),
        ("microsoft edge", "msedge"),
        ("paint", "mspaint"),
        ("terminal", "wt"),
        ("command prompt", "cmd"),
    ];
    let lower = name.to_lowercase();
    let target = aliases.iter().find(|(k, _)| *k == lower).map(|(_, v)| *v).unwrap_or(name);
    if shell_open(target) {
        return Ok(format!("Opened {name}"));
    }
    if let Some(lnk) = best_shortcut(name) {
        if shell_open(&lnk.to_string_lossy()) {
            return Ok(format!("Opened {name} ({})", lnk.file_stem().unwrap_or_default().to_string_lossy()));
        }
    }
    anyhow::bail!("could not find an app called \"{name}\"")
}
