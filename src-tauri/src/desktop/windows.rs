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
    EnumWindows, GetClassNameW, GetForegroundWindow, GetWindowLongW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, IsZoomed, SetForegroundWindow, SetWindowPos, ShowWindow,
    GWL_EXSTYLE, SWP_NOACTIVATE, SWP_NOZORDER, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, SW_SHOWNORMAL, WS_EX_TOOLWINDOW,
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
    collect_windows(own_pid, false)
}

/// Minimized windows, which `list` leaves out (they have no place on screen).
pub fn list_minimized(own_pid: u32) -> Vec<DesktopWindow> {
    collect_windows(own_pid, true)
}

fn collect_windows(own_pid: u32, minimized: bool) -> Vec<DesktopWindow> {
    let mut handles: Vec<HWND> = vec![];
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut handles as *mut _ as isize));
    }
    let foreground = unsafe { GetForegroundWindow() };
    let mut out = vec![];
    for hwnd in handles {
        unsafe {
            if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() != minimized {
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

/// The invisible resize borders around a window's visible frame: left, top, right, bottom.
fn frame_margins(hwnd: HWND) -> (i32, i32, i32, i32) {
    let (mut outer, mut visible) = (RECT::default(), RECT::default());
    // SAFETY: plain queries on a window handle with out-pointers we own.
    let ok = unsafe {
        GetWindowRect(hwnd, &mut outer).is_ok()
            && DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut visible as *mut _ as *mut c_void, std::mem::size_of::<RECT>() as u32).is_ok()
    };
    if !ok {
        return (0, 0, 0, 0);
    }
    let m = |v: i32| v.clamp(0, 32);
    (m(visible.left - outer.left), m(visible.top - outer.top), m(outer.right - visible.right), m(outer.bottom - visible.bottom))
}

/// Places a window so its visible frame fills `rect`, shows it as asked and brings it forward.
pub fn arrange(id: u64, rect: Option<(i32, i32, i32, i32)>, state: super::WindowState) -> anyhow::Result<()> {
    use super::WindowState;
    let hwnd = HWND(id as isize as *mut c_void);
    // SAFETY: Win32 window calls on a handle from EnumWindows; a stale handle fails harmlessly.
    unsafe {
        anyhow::ensure!(IsWindow(Some(hwnd)).as_bool(), "that window is gone; check list_windows");
        if state == WindowState::Minimize {
            let _ = ShowWindow(hwnd, SW_MINIMIZE);
            return Ok(());
        }
        if let Some((x, y, w, h)) = rect {
            // A maximized or minimized window ignores a new position until it's restored.
            if IsIconic(hwnd).as_bool() || IsZoomed(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let (l, t, r, b) = frame_margins(hwnd);
            SetWindowPos(hwnd, None, x - l, y - t, w + l + r, h + t + b, SWP_NOZORDER | SWP_NOACTIVATE)?;
        }
        match state {
            WindowState::Maximize => {
                let _ = ShowWindow(hwnd, SW_MAXIMIZE);
            }
            WindowState::Restore => {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            _ if IsIconic(hwnd).as_bool() => {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            _ => {}
        }
        let _ = SetForegroundWindow(hwnd);
    }
    Ok(())
}

pub fn open_path(path: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(shell_open(&path.to_string_lossy()), "Windows couldn't open {}", path.display());
    Ok(())
}

/// Sets the default value of a key under HKEY_CURRENT_USER, creating it if needed.
pub fn set_user_registry_default(subkey: &str, value: &str) -> anyhow::Result<()> {
    use windows::Win32::System::Registry::{RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ};
    let mut key = HKEY::default();
    let sub = HSTRING::from(subkey);
    // SAFETY: plain registry calls with valid, NUL-terminated strings and an out-pointer we own.
    unsafe {
        RegCreateKeyExW(HKEY_CURRENT_USER, &sub, None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut key, None).ok()?;
        let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2);
        let r = RegSetValueExW(key, PCWSTR::null(), None, REG_SZ, Some(bytes));
        let _ = RegCloseKey(key);
        r.ok()?;
    }
    Ok(())
}

pub fn open_url(url: &str) -> anyhow::Result<()> {
    anyhow::ensure!(shell_open(url), "Windows couldn't open the browser");
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
    // Speech writes names as they sound ("Clawed" for Claude): try the installed apps by sound and spelling.
    let apps = installed_apps();
    let names: Vec<String> = apps.iter().map(|(n, _)| n.clone()).collect();
    match waddle_core::heard::match_app(name, &names) {
        waddle_core::heard::AppMatch::Clear(found) => {
            let target = apps.iter().find(|(n, _)| *n == found).map(|(_, t)| t.clone()).unwrap_or_default();
            anyhow::ensure!(shell_open(&target), "couldn't start {found}");
            Ok(format!("Opened {found} (the closest installed app to \"{name}\")"))
        }
        waddle_core::heard::AppMatch::Unsure(close) if !close.is_empty() => anyhow::bail!(
            "could not find an app called \"{name}\". Installed apps with similar names: {}. If one is what the user meant, open it by that name; otherwise ask them.",
            close.join(", ")
        ),
        _ => anyhow::bail!("could not find an app called \"{name}\"; ask the user what it's called"),
    }
}

/// Installed apps by name, with what to open for each: Start menu shortcuts,
/// plus Store apps (Get-StartApps), which have no shortcut file. Cached for 10 minutes.
fn installed_apps() -> Vec<(String, String)> {
    static CACHE: Mutex<Option<(std::time::Instant, Vec<(String, String)>)>> = Mutex::new(None);
    if let Some((at, apps)) = CACHE.lock().unwrap().as_ref() {
        if at.elapsed() < std::time::Duration::from_secs(600) {
            return apps.clone();
        }
    }
    let mut all = vec![];
    for d in start_menu_dirs() {
        find_shortcuts(&d, &mut all, 0);
    }
    let skip = |s: &str| s.contains("uninstall") || s.contains("readme") || s.contains("help");
    let mut apps: Vec<(String, String)> = all
        .iter()
        .filter_map(|p| Some((p.file_stem()?.to_string_lossy().to_string(), p.to_string_lossy().to_string())))
        .filter(|(n, _)| !skip(&n.to_lowercase()))
        .collect();
    for (name, id) in start_apps() {
        if !apps.iter().any(|(n, _)| n.eq_ignore_ascii_case(&name)) {
            apps.push((name, format!("shell:AppsFolder\\{id}")));
        }
    }
    *CACHE.lock().unwrap() = Some((std::time::Instant::now(), apps.clone()));
    apps
}

/// Name and AppUserModelID of every app in the Start menu, from PowerShell (at most 5 s).
fn start_apps() -> Vec<(String, String)> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    let child = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", "Get-StartApps | Select-Object Name,AppID | ConvertTo-Json -Compress"])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return vec![] };
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < std::time::Duration::from_secs(5) => std::thread::sleep(std::time::Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                return vec![];
            }
        }
    }
    let Ok(out) = child.wait_with_output() else { return vec![] };
    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_default();
    // One app comes back as an object, several as an array.
    let list = match parsed {
        serde_json::Value::Array(a) => a,
        v @ serde_json::Value::Object(_) => vec![v],
        _ => vec![],
    };
    list.iter()
        .filter_map(|a| Some((a["Name"].as_str()?.to_string(), a["AppID"].as_str()?.to_string())))
        .collect()
}
