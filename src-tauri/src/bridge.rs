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
use waddle_core::tools::{Capabilities, GuiAction, GuiResult, WindowInfo};

use crate::actuate;
use crate::desktop::{self, DesktopWindow};
use crate::overlay::{Geometry, OverlayState};

const OVERLAY: &str = "overlay";
const ARRIVE_TIMEOUT: Duration = Duration::from_secs(6);

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
    approvals: Mutex<HashMap<String, oneshot::Sender<Decision>>>,
    moves: Mutex<HashMap<String, oneshot::Sender<()>>>,
    windows: RwLock<Vec<DesktopWindow>>,
    busy: AtomicBool,
    halt_suppressed_until: AtomicU64,
    next_move: AtomicU64,
    #[cfg(windows)]
    uia: crate::uia::Uia,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl TauriHost {
    pub fn new(app: AppHandle, overlay: Arc<OverlayState>) -> Arc<Self> {
        Arc::new(Self {
            app,
            overlay,
            approvals: Mutex::default(),
            moves: Mutex::default(),
            windows: RwLock::default(),
            busy: AtomicBool::new(false),
            halt_suppressed_until: AtomicU64::new(0),
            next_move: AtomicU64::new(1),
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

    pub fn platforms(&self) -> Vec<Platform> {
        self.to_platforms(&self.windows.read().unwrap())
    }

    pub fn answer_approval(&self, id: &str, approved: bool) {
        if let Some(tx) = self.approvals.lock().unwrap().remove(id) {
            let _ = tx.send(if approved { Decision::Approved } else { Decision::Denied });
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
        EnvInfo { os: os.into(), screen_w: w, screen_h: h, caps: Capabilities { gui: true, accessibility: cfg!(windows), ..Default::default() } }
    }

    async fn request_approval(&self, req: ApprovalRequest) -> Decision {
        let (tx, rx) = oneshot::channel();
        self.approvals.lock().unwrap().insert(req.id.clone(), tx);
        self.emit_overlay("approval", req);
        rx.await.unwrap_or(Decision::Cancelled)
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

    fn set_busy(&self, busy: bool) {
        self.busy.store(busy, Ordering::SeqCst);
        crate::shortcuts::set_halt_hotkey(&self.app, busy);
        if !busy {
            for (_, tx) in self.approvals.lock().unwrap().drain() {
                let _ = tx.send(Decision::Cancelled);
            }
            for (_, tx) in self.moves.lock().unwrap().drain() {
                let _ = tx.send(());
            }
        }
        self.emit_overlay("busy", busy);
    }
}

/// Samples window geometry ten times a second for the duck's platforms.
pub fn spawn_window_sampler(host: Arc<TauriHost>) {
    std::thread::Builder::new()
        .name("waddle-windows".into())
        .spawn(move || loop {
            host.update_windows(desktop::list_windows());
            std::thread::sleep(Duration::from_millis(100));
        })
        .expect("window sampler thread");
}
