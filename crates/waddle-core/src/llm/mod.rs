//! Provider-neutral chat types and the streaming `Provider` trait.

pub mod mock;
pub mod ollama;
pub mod openai_compat;
mod textcalls;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

use crate::config::{ProviderKind, Settings};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageData {
    pub mime: String,
    pub base64: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImageData>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

impl Message {
    fn new(role: Role, text: impl Into<String>) -> Self {
        Self { role, text: text.into(), images: vec![], tool_calls: vec![], tool_call_id: None, tool_name: None }
    }
    pub fn system(text: impl Into<String>) -> Self {
        Self::new(Role::System, text)
    }
    pub fn user(text: impl Into<String>) -> Self {
        Self::new(Role::User, text)
    }
    pub fn user_with_image(text: impl Into<String>, image: ImageData) -> Self {
        Self { images: vec![image], ..Self::new(Role::User, text) }
    }
    pub fn assistant(text: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self { tool_calls, ..Self::new(Role::Assistant, text) }
    }
    pub fn tool_result(call: &ToolCall, text: impl Into<String>) -> Self {
        Self {
            tool_call_id: Some(call.id.clone()),
            tool_name: Some(call.name.clone()),
            ..Self::new(Role::Tool, text)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Clone, Copy)]
pub struct ChatRequest<'a> {
    pub model: &'a str,
    pub messages: &'a [Message],
    pub tools: &'a [ToolSpec],
    pub temperature: f32,
    pub max_tokens: u32,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChatResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
}

/// What a call (or a whole task) used, as reported by the server.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// In US dollars, when the server says (OpenRouter does).
    pub cost: Option<f64>,
}

impl Usage {
    pub fn add(&mut self, other: Usage) {
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
        if let Some(c) = other.cost {
            self.cost = Some(self.cost.unwrap_or(0.0) + c);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.prompt_tokens == 0 && self.completion_tokens == 0 && self.cost.is_none()
    }
}

impl std::fmt::Display for Usage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} tokens in, {} out", self.prompt_tokens, self.completion_tokens)?;
        match self.cost {
            Some(c) => write!(f, ", ${c:.5}"),
            None => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    TextDelta(String),
}

pub type EventSink<'a> = &'a mut (dyn FnMut(StreamEvent) + Send);

#[async_trait]
pub trait Provider: Send + Sync {
    /// Streams a chat completion. Text deltas are reported through `on_event`
    /// as they arrive; the full response (including tool calls) is returned at the end.
    async fn chat(&self, req: ChatRequest<'_>, on_event: EventSink<'_>) -> anyhow::Result<ChatResponse>;

    /// Sends the start of a conversation ahead of time and discards the answer,
    /// so a local server has the model loaded and the prompt cached when the
    /// real request arrives. Only worth calling on local servers: a hosted API
    /// would bill for it.
    async fn warm(&self, req: ChatRequest<'_>) {
        if let Err(e) = self.chat(ChatRequest { max_tokens: 1, ..req }, &mut |_| {}).await {
            log::debug!("warm-up failed: {e:#}");
        }
    }
}

/// Builds the provider described by `settings`.
pub fn build_provider(settings: &Settings, api_key: Option<String>) -> Arc<dyn Provider> {
    match settings.provider {
        ProviderKind::OpenaiCompat => {
            Arc::new(openai_compat::OpenAiCompat::new(settings.base_url.clone(), api_key))
        }
        ProviderKind::Ollama => Arc::new(ollama::Ollama::new(settings.base_url.clone(), settings.ollama.clone())),
        ProviderKind::Mock => Arc::new(mock::MockProvider::demo()),
    }
}

/// Parses tool-call arguments that arrive as a JSON string. Models occasionally
/// send nothing or invalid JSON; keep the raw text so the tool can report the problem.
pub(crate) fn parse_arguments(raw: &str) -> Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Value::Object(Default::default());
    }
    serde_json::from_str(trimmed).unwrap_or_else(|_| serde_json::json!({ "_raw": trimmed }))
}

pub(crate) fn new_call_id() -> String {
    let mut bytes = [0u8; 6];
    let _ = getrandom::fill(&mut bytes);
    format!("call_{}", hex::encode(bytes))
}
