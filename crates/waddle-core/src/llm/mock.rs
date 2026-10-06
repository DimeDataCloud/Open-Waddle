//! Scripted provider. `scripted` replays fixed responses (tests); `demo` walks
//! through a short tour of Waddle's abilities without any network or API key.

use async_trait::async_trait;
use serde_json::json;
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use super::{ChatRequest, ChatResponse, EventSink, Message, Provider, Role, StreamEvent, ToolCall};

enum Mode {
    Scripted(Mutex<VecDeque<ChatResponse>>),
    Demo,
}

pub struct MockProvider {
    mode: Mode,
    delay: Duration,
    /// Every request's messages, for assertions in tests.
    pub requests: Mutex<Vec<Vec<Message>>>,
    /// Every warm-up's messages; warm-ups don't use up scripted replies.
    pub warmups: Mutex<Vec<Vec<Message>>>,
    /// Whether each request asked for a web search (and how many results).
    pub webs: Mutex<Vec<Option<u8>>>,
    /// The tool names offered with each request.
    pub tools: Mutex<Vec<Vec<String>>>,
    /// The model each request asked for.
    pub models: Mutex<Vec<String>>,
}

impl MockProvider {
    pub fn scripted(responses: Vec<ChatResponse>) -> Self {
        Self { mode: Mode::Scripted(Mutex::new(responses.into())), delay: Duration::ZERO, requests: Mutex::default(), warmups: Mutex::default(), webs: Mutex::default(), tools: Mutex::default(), models: Mutex::default() }
    }

    pub fn demo() -> Self {
        Self { mode: Mode::Demo, delay: Duration::from_millis(35), requests: Mutex::default(), warmups: Mutex::default(), webs: Mutex::default(), tools: Mutex::default(), models: Mutex::default() }
    }

    /// Adds a pause before each response, to exercise cancellation.
    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    pub fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

pub fn call(name: &str, args: serde_json::Value) -> ToolCall {
    ToolCall { id: super::new_call_id(), name: name.into(), arguments: args }
}

pub fn reply(text: &str, calls: Vec<ToolCall>) -> ChatResponse {
    ChatResponse { text: text.into(), tool_calls: calls, ..Default::default() }
}

fn demo_step(req: &ChatRequest<'_>) -> ChatResponse {
    if req.tools.is_empty() {
        return reply("Got it! I'll fold that in as soon as this step finishes. (demo mode)", vec![]);
    }
    // Count tool results since the task's opening user message to know where we are.
    let step = req.messages.iter().filter(|m| m.role == Role::Tool).count();
    let cleanup = if cfg!(windows) { "Remove-Item waddle-demo.txt" } else { "rm waddle-demo.txt" };
    match step {
        0 => reply("Hi! This is demo mode. First, a peek at what's open.", vec![call("list_windows", json!({}))]),
        1 => reply(
            "I'll leave a note in your workspace.",
            vec![call("write_file", json!({ "path": "waddle-demo.txt", "content": "Hello from Project Waddle!\n" }))],
        ),
        2 => reply("Now I'll waddle over and tap the screen.", vec![call("click", json!({ "x": 420, "y": 260 }))]),
        3 => reply("Cleaning up deletes a file, so I need your OK.", vec![call("run_command", json!({ "command": cleanup }))]),
        _ => reply(
            "Demo done! I say what I'm about to do, walk there, and ask before anything risky. Add an API key in Settings to make me useful.",
            vec![],
        ),
    }
}

#[async_trait]
impl Provider for MockProvider {
    async fn chat(&self, req: ChatRequest<'_>, on_event: EventSink<'_>) -> anyhow::Result<ChatResponse> {
        self.requests.lock().unwrap().push(req.messages.to_vec());
        self.webs.lock().unwrap().push(req.web);
        self.models.lock().unwrap().push(req.model.to_string());
        self.tools.lock().unwrap().push(req.tools.iter().map(|t| t.name.clone()).collect());
        let resp = match &self.mode {
            Mode::Scripted(q) => q
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| reply("(script exhausted)", vec![])),
            Mode::Demo => demo_step(&req),
        };
        if !self.delay.is_zero() && matches!(self.mode, Mode::Scripted(_)) {
            tokio::time::sleep(self.delay).await;
        }
        // Stream word by word so the UI path is exercised.
        for word in resp.text.split_inclusive(' ') {
            if matches!(self.mode, Mode::Demo) {
                tokio::time::sleep(self.delay).await;
            }
            on_event(StreamEvent::TextDelta(word.to_string()));
        }
        Ok(resp)
    }

    async fn warm(&self, req: ChatRequest<'_>) {
        self.warmups.lock().unwrap().push(req.messages.to_vec());
    }
}
