//! `TauriHost`: the core's view of the desktop. It turns agent events into UI
//! events, waits for the user's approvals, walks the duck to targets (and waits
//! for it to arrive) before acting, and performs GUI actions.

use async_trait::async_trait;
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use waddle_core::agent::{AgentEvent, ApprovalRequest, Decision, EnvInfo, Host};
use waddle_core::session::Selection;
use waddle_core::google::gmail::MailDraft;

/// The user's answer to an approval card, with the email as they left it on a send card.
type Answer = (Decision, Option<MailDraft>);
use waddle_core::tools::{Capabilities, GuiAction, GuiResult, WindowInfo};

use crate::actuate;
use crate::desktop::{self, DesktopWindow};
use crate::overlay::{Geometry, OverlayState};

const OVERLAY: &str = "overlay";
const ARRIVE_TIMEOUT: Duration = Duration::from_secs(6);
/// Longest selection sent along with a message.
const MAX_SELECTION_CHARS: usize = 8000;

/// A window as the overlay sees it: logical pixels relative to the overlay.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Platform {
    pub id: u64,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

pub struct TauriHost {
    app: AppHandle,
    overlay: Arc<OverlayState>,
    /// Waddle's Chrome extension, when it's connected.
    pub browser: Arc<crate::browser::BrowserLink>,
    approvals: Mutex<HashMap<String, oneshot::Sender<Answer>>>,
    /// Undo buttons showing after Send, by approval id.
    undos: Mutex<HashMap<String, oneshot::Sender<()>>>,
    moves: Mutex<HashMap<String, oneshot::Sender<()>>>,
    windows: RwLock<Vec<DesktopWindow>>,
    busy: AtomicBool,
    halt_suppressed_until: AtomicU64,
    next_move: AtomicU64,
    /// Text grabbed by Ctrl+Alt+A, waiting for the message it goes with, and the window it came from.
    selection: Mutex<Option<(Selection, Option<u64>)>>,
    /// The window `replace_selection` pastes into.
    selection_window: Mutex<Option<u64>>,
    #[cfg(windows)]
    uia: crate::uia::Uia,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl TauriHost {
    pub fn new(app: AppHandle, overlay: Arc<OverlayState>, browser: Arc<crate::browser::BrowserLink>) -> Arc<Self> {
        Arc::new(Self {
            app,
            overlay,
            browser,
            approvals: Mutex::default(),
            undos: Mutex::default(),
            moves: Mutex::default(),
            windows: RwLock::default(),
            busy: AtomicBool::new(false),
            halt_suppressed_until: AtomicU64::new(0),
            next_move: AtomicU64::new(1),
            selection: Mutex::default(),
            selection_window: Mutex::default(),
            #[cfg(windows)]
            uia: crate::uia::Uia::spawn(),
        })
    }

    fn geometry(&self) -> Geometry {
        self.overlay.geometry()
    }

    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    /// Reads the text selected in the front app (Ctrl+Alt+A) and keeps it for the
    /// next message. Returns what the chat box shows: length, a preview and the app.
    pub async fn capture_selection(&self) -> Option<serde_json::Value> {
        let front = desktop::list_windows().into_iter().find(|w| w.focused);
        #[cfg(windows)]
        let via_uia = self.uia.selection().await.ok().filter(|t| !t.trim().is_empty());
        #[cfg(not(windows))]
        let via_uia: Option<String> = None;
        let text = match via_uia {
            Some(t) => t,
            None => Self::blocking(actuate::copy_selection).await.ok()?,
        };
        let text: String = text.chars().take(MAX_SELECTION_CHARS).collect();
        if text.trim().is_empty() {
            *self.selection.lock().unwrap() = None;
            return None;
        }
        let app = front.as_ref().map(|w| w.app.clone()).unwrap_or_default();
        let preview: String = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(60).collect();
        let view = json!({ "chars": text.chars().count(), "preview": preview, "app": app });
        *self.selection.lock().unwrap() = Some((Selection { text, app }, front.map(|w| w.id)));
        Some(view)
    }

    /// The captured selection, for the message being sent; `replace_selection` will paste into its window.
    pub fn take_selection(&self) -> Option<Selection> {
        let (sel, window) = self.selection.lock().unwrap().take()?;
        *self.selection_window.lock().unwrap() = window;
        Some(sel)
    }

    /// The Escape hotkey should halt, unless Waddle itself just pressed Escape.
    pub fn halt_hotkey_allowed(&self) -> bool {
        self.is_busy() && now_ms() > self.halt_suppressed_until.load(Ordering::SeqCst)
    }

    pub fn emit_overlay<S: Serialize + Clone>(&self, event: &str, payload: S) {
        let _ = self.app.emit_to(OVERLAY, event, payload);
    }

    /// Window bounds → logical screen coordinates (relative to the primary monitor).
    fn to_logical(&self, w: &DesktopWindow) -> (f64, f64, f64, f64) {
        let g = self.geometry();
        if desktop::BOUNDS_ARE_LOGICAL {
            let (sx, sy) = (g.screen_x as f64 / g.scale, g.screen_y as f64 / g.scale);
            (w.x as f64 - sx, w.y as f64 - sy, w.w as f64, w.h as f64)
        } else {
            (
                (w.x - g.screen_x) as f64 / g.scale,
                (w.y - g.screen_y) as f64 / g.scale,
                w.w as f64 / g.scale,
                w.h as f64 / g.scale,
            )
        }
    }

    /// Called by the sampler thread with each fresh window list.
    pub fn update_windows(&self, list: Vec<DesktopWindow>) {
        desktop::remember_foreground(&list);
        let changed = *self.windows.read().unwrap() != list;
        if !changed {
            return;
        }
        let platforms = self.to_platforms(&list);
        *self.windows.write().unwrap() = list;
        self.emit_overlay("desktop:windows", platforms);
    }

    /// Sends the platforms again after the display changed (same windows, new conversion).
    pub fn refresh_platforms(&self) {
        let platforms = self.platforms();
        self.emit_overlay("desktop:windows", platforms);
    }

    fn to_platforms(&self, list: &[DesktopWindow]) -> Vec<Platform> {
        let g = self.geometry();
        list.iter()
            .map(|w| {
                let (x, y, ww, hh) = self.to_logical(w);
                let (ox, oy) = g.screen_to_overlay(x, y);
                Platform { id: w.id, x: ox, y: oy, w: ww, h: hh }
            })
            .collect()
    }

    /// The front window of another app: its app name, where it is on the overlay, and whether it fills the screen.
    pub fn front_window(&self) -> Option<(String, Platform, bool)> {
        let list = self.windows.read().unwrap();
        let w = list.iter().find(|w| w.focused)?;
        let (sw, sh) = self.geometry().screen_logical();
        let (_, _, lw, lh) = self.to_logical(w);
        let fullscreen = lw >= sw * 0.98 && lh >= sh * 0.98;
        let p = self.to_platforms(std::slice::from_ref(w)).pop()?;
        Some((w.app.clone(), p, fullscreen))
    }

    /// Logical screen point → overlay coordinates.
    pub fn to_overlay(&self, x: f64, y: f64) -> (f64, f64) {
        self.geometry().screen_to_overlay(x, y)
    }

    /// Physical desktop pixels → overlay coordinates.
    pub fn physical_to_overlay(&self, px: f64, py: f64) -> (f64, f64) {
        let g = self.geometry();
        g.screen_to_overlay((px - g.screen_x as f64) / g.scale, (py - g.screen_y as f64) / g.scale)
    }

    pub fn platforms(&self) -> Vec<Platform> {
        self.to_platforms(&self.windows.read().unwrap())
    }

    /// The user's click on an approval card; a send card also returns the message as they edited it.
    pub fn answer_approval(&self, id: &str, approved: bool, draft: Option<MailDraft>) {
        if let Some(tx) = self.approvals.lock().unwrap().remove(id) {
            let _ = tx.send((if approved { Decision::Approved } else { Decision::Denied }, draft));
        }
    }

    pub fn undo_send(&self, id: &str) {
        if let Some(tx) = self.undos.lock().unwrap().remove(id) {
            let _ = tx.send(());
        }
    }

    pub fn duck_arrived(&self, id: &str) {
        if let Some(tx) = self.moves.lock().unwrap().remove(id) {
            let _ = tx.send(());
        }
    }

    /// Walks the duck to a point (logical screen pixels) and waits until it gets there.
    async fn move_to(&self, x: f64, y: f64, purpose: &str, cancel: &CancellationToken) {
        let id = format!("mv{}", self.next_move.fetch_add(1, Ordering::SeqCst));
        let (tx, rx) = oneshot::channel();
        self.moves.lock().unwrap().insert(id.clone(), tx);
        let (ox, oy) = self.geometry().screen_to_overlay(x, y);
        self.emit_overlay("duck:move", json!({ "id": id, "x": ox, "y": oy, "purpose": purpose }));
        tokio::select! {
            _ = rx => {}
            _ = tokio::time::sleep(ARRIVE_TIMEOUT) => { self.moves.lock().unwrap().remove(&id); }
            _ = cancel.cancelled() => { self.moves.lock().unwrap().remove(&id); }
        }
    }

    fn act(&self, kind: &str) {
        self.emit_overlay("duck:act", json!({ "kind": kind }));
    }

    async fn blocking<T: Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> anyhow::Result<T> {
        tokio::task::spawn_blocking(f).await.map_err(|e| anyhow::anyhow!("worker failed: {e}"))?
    }

    async fn click_physical(&self, px: i32, py: i32, button: waddle_core::tools::MouseButton, double: bool) -> anyhow::Result<()> {
        let window = self.app.get_webview_window(OVERLAY);
        let _guard = window.as_ref().map(|w| self.overlay.force_passthrough(w));
        Self::blocking(move || actuate::click(px, py, button, double)).await
    }

    #[cfg(windows)]
    fn physical_to_logical(&self, px: f64, py: f64) -> (f64, f64) {
        let g = self.geometry();
        ((px - g.screen_x as f64) / g.scale, (py - g.screen_y as f64) / g.scale)
    }

    /// Self-test: the app in front and how many accessible controls it exposes.
    #[cfg(windows)]
    pub async fn probe_accessibility(&self) -> anyhow::Result<(String, usize)> {
        let target = self.target_window(None).ok_or_else(|| anyhow::anyhow!("no other window is open to inspect"))?;
        let elements = self.uia.find(target.id as isize).await?;
        Ok((target.app, elements.len()))
    }

    #[cfg(windows)]
    fn target_window(&self, filter: Option<&str>) -> Option<DesktopWindow> {
        let list = self.windows.read().unwrap().clone();
        match filter {
            Some(f) => {
                let f = f.to_lowercase();
                list.into_iter().find(|w| w.title.to_lowercase().contains(&f))
            }
            None => list.iter().find(|w| w.focused).cloned().or_else(|| list.first().cloned()),
        }
    }
}

#[async_trait]
impl Host for TauriHost {
    fn emit(&self, event: AgentEvent) {
        self.emit_overlay("agent", event);
    }

    fn env(&self) -> EnvInfo {
        let (w, h) = self.geometry().screen_logical();
        let os = if cfg!(windows) {
            "Windows"
        } else if cfg!(target_os = "macos") {
            "macOS"
        } else {
            "Linux"
        };
        let caps = Capabilities { gui: true, accessibility: cfg!(windows), browser: self.browser.connected(), ..Default::default() };
        EnvInfo { os: os.into(), screen_w: w, screen_h: h, caps }
    }

    async fn browser(&self, cmd: &str, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        let mut r = self.browser.request(cmd, args).await?;
        if cmd == "locate" {
            anyhow::ensure!(!r.is_null(), "that element isn't on the page any more");
            // Page coordinates → logical screen pixels, for a real click.
            if let Some((x, y)) = waddle_core::tools::browser::screen_point(&r, self.geometry().scale) {
                r["screen_x"] = json!(x);
                r["screen_y"] = json!(y);
            }
        }
        Ok(r)
    }

    async fn request_approval(&self, req: ApprovalRequest) -> Decision {
        self.review_draft(req).await.0
    }

    async fn review_draft(&self, req: ApprovalRequest) -> (Decision, Option<MailDraft>) {
        let (tx, rx) = oneshot::channel();
        self.approvals.lock().unwrap().insert(req.id.clone(), tx);
        self.emit_overlay("approval", req);
        rx.await.unwrap_or((Decision::Cancelled, None))
    }

    async fn offer_undo(&self, id: &str, secs: u64) -> bool {
        if secs == 0 {
            return false;
        }
        let (tx, rx) = oneshot::channel();
        self.undos.lock().unwrap().insert(id.to_string(), tx);
        self.emit_overlay("undo", json!({ "id": id, "secs": secs }));
        let undone = tokio::select! {
            r = rx => r.is_ok(),
            _ = tokio::time::sleep(Duration::from_secs(secs)) => false,
        };
        self.undos.lock().unwrap().remove(id);
        self.emit_overlay("undo:done", json!({ "id": id, "undone": undone }));
        undone
    }

    fn resolve_approval(&self, id: &str, decision: Decision) {
        self.approvals.lock().unwrap().remove(id);
        self.emit(AgentEvent::ApprovalResolved { id: id.to_string(), decision });
    }

    async fn approach(&self, action: &GuiAction, cancel: &CancellationToken) {
        if let Some((x, y)) = action.target() {
            self.move_to(x, y, "approach", cancel).await;
            return;
        }
        #[cfg(windows)]
        if let GuiAction::ClickElement { id } = action {
            if let Ok((px, py)) = self.uia.locate(*id).await {
                let (x, y) = self.physical_to_logical(px as f64, py as f64);
                self.move_to(x, y, "approach", cancel).await;
            }
        }
    }

    async fn gui(&self, action: GuiAction, cancel: &CancellationToken) -> anyhow::Result<GuiResult> {
        let g = self.geometry();
        match action {
            GuiAction::ListWindows => {
                let list = Self::blocking(|| Ok(desktop::list_windows())).await?;
                let infos = list
                    .iter()
                    .map(|w| {
                        let (x, y, ww, hh) = self.to_logical(w);
                        WindowInfo { title: w.title.clone(), app: w.app.clone(), x, y, w: ww, h: hh, focused: w.focused }
                    })
                    .collect();
                Ok(GuiResult::Windows(infos))
            }
            GuiAction::LookAtScreen => {
                self.act("look");
                let (w, h) = g.screen_logical();
                let (image, width, height) = Self::blocking(move || actuate::screenshot(w.round() as u32, h.round() as u32)).await?;
                Ok(GuiResult::Screenshot { image, width, height })
            }
            GuiAction::FindElements { window } => {
                #[cfg(windows)]
                {
                    self.act("look");
                    let target = self.target_window(window.as_deref()).ok_or_else(|| anyhow::anyhow!("no matching window"))?;
                    let elements = self
                        .uia
                        .find(target.id as isize)
                        .await?
                        .into_iter()
                        .map(|mut e| {
                            let (x, y) = self.physical_to_logical(e.x, e.y);
                            (e.x, e.y, e.w, e.h) = (x, y, e.w / g.scale, e.h / g.scale);
                            e
                        })
                        .collect();
                    Ok(GuiResult::Elements { window: target.title, elements })
                }
                #[cfg(not(windows))]
                {
                    let _ = window;
                    anyhow::bail!("find_elements is only available on Windows for now; use look_at_screen")
                }
            }
            GuiAction::OpenApp { name } => {
                self.act("type");
                let msg = Self::blocking(move || desktop::open_app(&name)).await?;
                // Give the window a moment to appear before the next look.
                tokio::time::sleep(Duration::from_millis(1200)).await;
                Ok(GuiResult::Done(msg))
            }
            GuiAction::Click { x, y, button, double } => {
                self.move_to(x, y, "act", cancel).await;
                self.act("peck");
                let (px, py) = g.screen_to_physical(x, y);
                self.click_physical(px, py, button, double).await?;
                Ok(GuiResult::Done(format!("Clicked{}.", if double { " twice" } else { "" })))
            }
            GuiAction::ClickElement { id } => {
                #[cfg(windows)]
                {
                    let (px, py) = self.uia.locate(id).await?;
                    let (x, y) = self.physical_to_logical(px as f64, py as f64);
                    self.move_to(x, y, "act", cancel).await;
                    self.act("peck");
                    match self.uia.activate(id).await? {
                        crate::uia::Activation::Invoked => Ok(GuiResult::Done(format!("Activated element {id}."))),
                        crate::uia::Activation::ClickAt(cx, cy) => {
                            self.click_physical(cx, cy, waddle_core::tools::MouseButton::Left, false).await?;
                            Ok(GuiResult::Done(format!("Clicked element {id}.")))
                        }
                    }
                }
                #[cfg(not(windows))]
                {
                    let _ = (id, cancel);
                    anyhow::bail!("click_element is only available on Windows for now")
                }
            }
            GuiAction::TypeText { text, at } => {
                if let Some((x, y)) = at {
                    self.move_to(x, y, "act", cancel).await;
                    self.act("peck");
                    let (px, py) = g.screen_to_physical(x, y);
                    self.click_physical(px, py, waddle_core::tools::MouseButton::Left, false).await?;
                    tokio::time::sleep(Duration::from_millis(120)).await;
                } else {
                    desktop::restore_foreground();
                    tokio::time::sleep(Duration::from_millis(150)).await;
                }
                self.act("type");
                let n = text.chars().count();
                Self::blocking(move || actuate::type_text(&text)).await?;
                Ok(GuiResult::Done(format!("Typed {n} characters.")))
            }
            GuiAction::Scroll { dx, dy, at } => {
                let at = match at {
                    Some((x, y)) => {
                        self.move_to(x, y, "act", cancel).await;
                        Some(g.screen_to_physical(x, y))
                    }
                    None => {
                        desktop::restore_foreground();
                        None
                    }
                };
                self.act("peck");
                let window = self.app.get_webview_window(OVERLAY);
                let _guard = window.as_ref().map(|w| self.overlay.force_passthrough(w));
                Self::blocking(move || actuate::scroll(at, dx, dy)).await?;
                let dir = match (dx.signum(), dy.signum()) {
                    (_, 1) => "down",
                    (_, -1) => "up",
                    (1, _) => "right",
                    _ => "left",
                };
                Ok(GuiResult::Done(format!("Scrolled {dir} {} notches.", dx.abs().max(dy.abs()))))
            }
            GuiAction::Drag { from, to } => {
                self.move_to(from.0, from.1, "act", cancel).await;
                self.act("peck");
                let (a, b) = (g.screen_to_physical(from.0, from.1), g.screen_to_physical(to.0, to.1));
                let window = self.app.get_webview_window(OVERLAY);
                let _guard = window.as_ref().map(|w| self.overlay.force_passthrough(w));
                Self::blocking(move || actuate::drag(a, b)).await?;
                self.move_to(to.0, to.1, "act", cancel).await;
                Ok(GuiResult::Done("Dragged.".into()))
            }
            GuiAction::PointAt { x, y, label } => {
                self.move_to(x, y, "approach", cancel).await;
                let (ox, oy) = g.screen_to_overlay(x, y);
                self.emit_overlay("duck:point", json!({ "x": ox, "y": oy, "label": label }));
                Ok(GuiResult::Done("Pointing at it for the user. Tell them what it is or what to do there.".into()))
            }
            GuiAction::ReadClipboard => Ok(GuiResult::Clipboard(Self::blocking(actuate::read_clipboard).await?)),
            GuiAction::WriteClipboard { text } => {
                let n = text.chars().count();
                Self::blocking(move || actuate::copy_to_clipboard(&text)).await?;
                Ok(GuiResult::Done(format!("Copied {n} characters to the clipboard.")))
            }
            GuiAction::ReplaceSelection { text } => {
                let window = *self.selection_window.lock().unwrap();
                desktop::focus_window(window);
                tokio::time::sleep(Duration::from_millis(200)).await;
                self.act("type");
                let n = text.chars().count();
                Self::blocking(move || actuate::paste_text(&text)).await?;
                Ok(GuiResult::Done(format!("Replaced the selection with {n} characters.")))
            }
            GuiAction::PressKeys { keys } => {
                let combo = actuate::parse_combo(&keys)?;
                if combo.contains(&enigo::Key::Escape) {
                    self.halt_suppressed_until.store(now_ms() + 600, Ordering::SeqCst);
                }
                desktop::restore_foreground();
                tokio::time::sleep(Duration::from_millis(100)).await;
                self.act("type");
                Self::blocking(move || actuate::press_combo(&combo)).await?;
                Ok(GuiResult::Done(format!("Pressed {keys}.")))
            }
        }
    }

    async fn apply_settings(&self, settings: waddle_core::Settings) -> anyhow::Result<()> {
        let state = self.app.try_state::<crate::AppState>().ok_or_else(|| anyhow::anyhow!("app not ready"))?;
        state.apply_settings(settings)?;
        let s = state.settings.read().unwrap().clone();
        self.emit_overlay("settings", json!({ "color": s.character.color, "wander": s.wander, "demo": state.is_demo() }));
        Ok(())
    }

    fn open_path(&self, path: &std::path::Path) {
        if let Err(e) = desktop::open_path(path) {
            log::warn!("{e:#}");
        }
    }

    fn set_busy(&self, busy: bool) {
        self.busy.store(busy, Ordering::SeqCst);
        crate::shortcuts::set_halt_hotkey(&self.app, busy);
        if !busy {
            for (_, tx) in self.approvals.lock().unwrap().drain() {
                let _ = tx.send((Decision::Cancelled, None));
            }
            for (_, tx) in self.moves.lock().unwrap().drain() {
                let _ = tx.send(());
            }
        }
        self.emit_overlay("busy", busy);
    }
}

/// Pops up reminders when they're due, including ones that came due while Waddle was closed.
pub fn spawn_reminder_clock(host: Arc<TauriHost>, store: Arc<waddle_core::reminders::ReminderStore>) {
    std::thread::Builder::new()
        .name("waddle-reminders".into())
        .spawn(move || {
            // Let the overlay load before the first pop-up.
            std::thread::sleep(Duration::from_secs(3));
            loop {
                let now = now_ms() as i64;
                for r in store.take_due(now) {
                    let late = now - r.due_ms > 120_000;
                    host.emit_overlay("reminder", json!({ "text": r.text, "late": late }));
                }
                std::thread::sleep(Duration::from_secs(5));
            }
        })
        .expect("reminder thread");
}

/// Samples window geometry ten times a second for the duck's platforms.
pub fn spawn_window_sampler(host: Arc<TauriHost>) {
    std::thread::Builder::new()
        .name("waddle-windows".into())
        .spawn(move || loop {
            host.update_windows(desktop::list_windows());
            // Ten times a second on mains power, four on battery.
            std::thread::sleep(crate::power::pace(Duration::from_millis(100), Duration::from_millis(250)));
        })
        .expect("window sampler thread");
}
