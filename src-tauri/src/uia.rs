//! Windows UI Automation fast path for the Eyes: reads the buttons, fields and
//! links of a window as text, so the model can act by element id instead of
//! guessing pixel positions from a screenshot.
//!
//! UIA elements are COM objects tied to the thread that created them, so all
//! UIA work happens on one dedicated worker thread.

use std::sync::mpsc;
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use uiautomation::types::ControlType;
use uiautomation::patterns::UIInvokePattern;
use uiautomation::types::Handle;
use uiautomation::{UIAutomation, UIElement};
use waddle_core::tools::ElementInfo;

const MAX_DEPTH: usize = 12;
const MAX_ELEMENTS: usize = 150;
const TIME_BUDGET: Duration = Duration::from_millis(1500);

pub enum Activation {
    /// The element's own Invoke action ran (no mouse needed).
    Invoked,
    /// Click this physical point instead.
    ClickAt(i32, i32),
}

enum Job {
    Find { hwnd: isize, reply: oneshot::Sender<anyhow::Result<Vec<ElementInfo>>> },
    Activate { id: u32, invoke: bool, reply: oneshot::Sender<anyhow::Result<Activation>> },
}

pub struct Uia {
    tx: mpsc::Sender<Job>,
}

fn role(t: ControlType) -> Option<&'static str> {
    Some(match t {
        ControlType::Button => "button",
        ControlType::SplitButton => "button",
        ControlType::CheckBox => "checkbox",
        ControlType::RadioButton => "radio",
        ControlType::ComboBox => "dropdown",
        ControlType::Edit => "text field",
        ControlType::Document => "document",
        ControlType::Hyperlink => "link",
        ControlType::ListItem => "list item",
        ControlType::MenuItem => "menu item",
        ControlType::TabItem => "tab",
        ControlType::TreeItem => "tree item",
        ControlType::Slider => "slider",
        _ => return None,
    })
}

fn invokable(t: ControlType) -> bool {
    matches!(t, ControlType::Button | ControlType::SplitButton | ControlType::Hyperlink | ControlType::MenuItem)
}

struct Worker {
    automation: UIAutomation,
    last: Vec<(UIElement, ControlType, (i32, i32))>,
}

impl Worker {
    fn find(&mut self, hwnd: isize) -> anyhow::Result<Vec<ElementInfo>> {
        let walker = self.automation.get_control_view_walker()?;
        let root = self.automation.element_from_handle(Handle::from(hwnd))?;
        let start = Instant::now();
        let mut out = vec![];
        self.last.clear();
        let mut stack = vec![(root, 0usize)];
        while let Some((el, depth)) = stack.pop() {
            if start.elapsed() > TIME_BUDGET || out.len() >= MAX_ELEMENTS {
                break;
            }
            if depth > 0 {
                if let Some(info) = self.describe(&el, out.len() as u32 + 1) {
                    out.push(info);
                }
            }
            if depth < MAX_DEPTH {
                let mut children = vec![];
                let mut child = walker.get_first_child(&el).ok();
                while let Some(c) = child {
                    child = walker.get_next_sibling(&c).ok();
                    children.push(c);
                }
                // Reverse so the stack pops in reading order.
                stack.extend(children.into_iter().rev().map(|c| (c, depth + 1)));
            }
        }
        Ok(out)
    }

    fn describe(&mut self, el: &UIElement, id: u32) -> Option<ElementInfo> {
        let t = el.get_control_type().ok()?;
        let role = role(t)?;
        if !el.is_enabled().unwrap_or(false) || el.is_offscreen().unwrap_or(true) {
            return None;
        }
        let r = el.get_bounding_rectangle().ok()?;
        let (w, h) = (r.get_right() - r.get_left(), r.get_bottom() - r.get_top());
        if w <= 2 || h <= 2 {
            return None;
        }
        let mut name = el.get_name().unwrap_or_default().trim().to_string();
        if name.is_empty() {
            if !matches!(t, ControlType::Edit | ControlType::Document) {
                return None;
            }
            name = "(unnamed)".into();
        }
        name = name.chars().take(80).collect();
        let center = (r.get_left() + w / 2, r.get_top() + h / 2);
        self.last.push((el.clone(), t, center));
        Some(ElementInfo { id, role: role.into(), name, x: r.get_left() as f64, y: r.get_top() as f64, w: w as f64, h: h as f64 })
    }

    fn activate(&mut self, id: u32, invoke: bool) -> anyhow::Result<Activation> {
        let (el, t, center) = self
            .last
            .get(id.saturating_sub(1) as usize)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("element {id} is not in the latest find_elements list; call find_elements again"))?;
        if invoke && invokable(t) {
            if let Ok(p) = el.get_pattern::<UIInvokePattern>() {
                if p.invoke().is_ok() {
                    return Ok(Activation::Invoked);
                }
            }
        }
        Ok(Activation::ClickAt(center.0, center.1))
    }
}

impl Uia {
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("waddle-uia".into())
            .spawn(move || {
                let automation = match UIAutomation::new() {
                    Ok(a) => a,
                    Err(e) => {
                        log::error!("UI Automation unavailable: {e}");
                        for job in rx {
                            match job {
                                Job::Find { reply, .. } => drop(reply.send(Err(anyhow::anyhow!("UI Automation unavailable")))),
                                Job::Activate { reply, .. } => drop(reply.send(Err(anyhow::anyhow!("UI Automation unavailable")))),
                            }
                        }
                        return;
                    }
                };
                let mut worker = Worker { automation, last: vec![] };
                for job in rx {
                    match job {
                        Job::Find { hwnd, reply } => drop(reply.send(worker.find(hwnd))),
                        Job::Activate { id, invoke, reply } => drop(reply.send(worker.activate(id, invoke))),
                    }
                }
            })
            .expect("uia thread");
        Self { tx }
    }

    /// Elements of the window with this handle, in physical pixels.
    pub async fn find(&self, hwnd: isize) -> anyhow::Result<Vec<ElementInfo>> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(Job::Find { hwnd, reply })?;
        tokio::time::timeout(Duration::from_secs(4), rx).await.map_err(|_| anyhow::anyhow!("reading the window took too long"))??
    }

    /// Where element `id` (from the latest `find`) is, without activating it.
    pub async fn locate(&self, id: u32) -> anyhow::Result<(i32, i32)> {
        match self.activate_inner(id, false).await? {
            Activation::ClickAt(x, y) => Ok((x, y)),
            Activation::Invoked => unreachable!("locate never invokes"),
        }
    }

    /// Invokes element `id` if it supports it, otherwise reports where to click.
    pub async fn activate(&self, id: u32) -> anyhow::Result<Activation> {
        self.activate_inner(id, true).await
    }

    async fn activate_inner(&self, id: u32, invoke: bool) -> anyhow::Result<Activation> {
        let (reply, rx) = oneshot::channel();
        self.tx.send(Job::Activate { id, invoke, reply })?;
        match tokio::time::timeout(Duration::from_secs(3), rx).await {
            Ok(r) => r?,
            // Invoking something that opens a modal dialog can block; it still happened.
            Err(_) => Ok(Activation::Invoked),
        }
    }
}
