//! A fake desktop host shared by the integration tests: it records what the
//! agent asks for and answers approvals with a policy.

#![allow(dead_code)]

use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio_util::sync::CancellationToken;

use waddle_core::agent::ApprovalRequest;
use waddle_core::llm::ImageData;
use waddle_core::tools::{Capabilities, ElementInfo, GuiAction, GuiResult, WindowInfo};
use waddle_core::{AgentEvent, Decision, EnvInfo, Host, Settings};

pub type Policy = Box<dyn Fn(&ApprovalRequest) -> Option<Decision> + Send + Sync>;

pub struct FakeHost {
    pub events: Mutex<Vec<AgentEvent>>,
    pub approvals: Mutex<Vec<ApprovalRequest>>,
    pub gui_calls: Mutex<Vec<GuiAction>>,
    /// None = never answer (let a countdown or halt resolve it).
    pub policy: Policy,
    pub busy: Mutex<Vec<bool>>,
    pub applied: Mutex<Vec<Settings>>,
    /// What list_windows, find_elements and look_at_screen return (defaults: "ok" and a stub image).
    pub windows: Mutex<Vec<WindowInfo>>,
    pub elements: Mutex<Vec<ElementInfo>>,
    pub screenshot: Mutex<Option<ImageData>>,
    /// What env() reports as the OS.
    pub os: Mutex<String>,
    /// Seconds since creation for each step, tool and finish, for latency reports.
    pub timeline: Mutex<Vec<(f64, String)>>,
    /// Files Waddle opened for the user.
    pub opened: Mutex<Vec<PathBuf>>,
    started: Instant,
}

impl FakeHost {
    pub fn new(policy: Policy) -> Arc<Self> {
        Arc::new(Self {
            events: Mutex::default(),
            approvals: Mutex::default(),
            gui_calls: Mutex::default(),
            policy,
            busy: Mutex::default(),
            applied: Mutex::default(),
            windows: Mutex::default(),
            elements: Mutex::default(),
            screenshot: Mutex::default(),
            os: Mutex::new("TestOS".into()),
            timeline: Mutex::default(),
            opened: Mutex::default(),
            started: Instant::now(),
        })
    }
    pub fn events(&self) -> Vec<AgentEvent> {
        self.events.lock().unwrap().clone()
    }
}

#[async_trait]
impl Host for FakeHost {
    fn emit(&self, event: AgentEvent) {
        let label = match &event {
            AgentEvent::Thinking { .. } => Some("model call".to_string()),
            AgentEvent::ToolStarted { tool, summary, .. } => Some(format!("{tool}: {summary}")),
            AgentEvent::TaskFinished { outcome, .. } => Some(format!("finished ({outcome:?})")),
            _ => None,
        };
        if let Some(label) = label {
            self.timeline.lock().unwrap().push((self.started.elapsed().as_secs_f64(), label));
        }
        self.events.lock().unwrap().push(event);
    }
    fn env(&self) -> EnvInfo {
        // Like Windows: the accessibility fast path exists when there are elements to list.
        let accessibility = !self.elements.lock().unwrap().is_empty();
        EnvInfo { os: self.os.lock().unwrap().clone(), screen_w: 1440.0, screen_h: 960.0, caps: Capabilities { gui: true, accessibility, ..Default::default() } }
    }
    async fn request_approval(&self, req: ApprovalRequest) -> Decision {
        self.approvals.lock().unwrap().push(req.clone());
        match (self.policy)(&req) {
            Some(d) => d,
            None => std::future::pending().await,
        }
    }
    fn resolve_approval(&self, _id: &str, _decision: Decision) {}
    async fn gui(&self, action: GuiAction, _cancel: &CancellationToken) -> anyhow::Result<GuiResult> {
        self.gui_calls.lock().unwrap().push(action.clone());
        Ok(match action {
            GuiAction::LookAtScreen => GuiResult::Screenshot {
                image: self.screenshot.lock().unwrap().clone().unwrap_or(ImageData { mime: "image/png".into(), base64: "AAAA".into() }),
                width: 1440,
                height: 960,
            },
            GuiAction::ListWindows if !self.windows.lock().unwrap().is_empty() => GuiResult::Windows(self.windows.lock().unwrap().clone()),
            GuiAction::FindElements { .. } if !self.elements.lock().unwrap().is_empty() => {
                GuiResult::Elements { window: "Test App".into(), elements: self.elements.lock().unwrap().clone() }
            }
            _ => GuiResult::Done("ok".into()),
        })
    }
    fn set_busy(&self, busy: bool) {
        self.busy.lock().unwrap().push(busy);
    }
    async fn apply_settings(&self, settings: Settings) -> anyhow::Result<()> {
        self.applied.lock().unwrap().push(settings);
        Ok(())
    }
    fn open_path(&self, path: &Path) {
        self.opened.lock().unwrap().push(path.to_path_buf());
    }
}
