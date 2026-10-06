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
    /// Search the web first and give the model this many results (OpenRouter's web plugin).
    pub web: Option<u8>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChatResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    /// Pages a web search found, as the server cited them.
    pub citations: Vec<Citation>,
    /// The reply stopped at `max_tokens`: its text or tool-call arguments may be cut off.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Citation {
    pub url: String,
    pub title: String,
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

/// Two services behind one `Provider`: the planner's, and an optional second one for the
/// quick-reply model (say OpenRouter for the planner and Google's free Gemini API for chat).
/// A request goes by its model name, so no caller has to know there are two.
pub struct Routed {
    main: Arc<dyn Provider>,
    fast: Option<Arc<dyn Provider>>,
    main_model: String,
    fast_model: String,
    /// The second service can search the web for a request (only OpenRouter's plugin can).
    fast_has_web: bool,
}

impl Routed {
    pub fn new(main: Arc<dyn Provider>, fast: Option<Arc<dyn Provider>>, main_model: &str, fast_model: &str, fast_has_web: bool) -> Self {
        Self { main, fast, main_model: main_model.to_string(), fast_model: fast_model.to_string(), fast_has_web }
    }

    /// Where `req` goes, and the request to send there.
    fn pick<'a>(&'a self, req: ChatRequest<'a>) -> (&'a Arc<dyn Provider>, ChatRequest<'a>) {
        if req.model != self.fast_model || self.fast_model == self.main_model {
            return (&self.main, req);
        }
        match &self.fast {
            Some(fast) if req.web.is_none() || self.fast_has_web => (fast, req),
            // No key for the second service yet, or a web search it can't do: the planner's
            // service and model answer instead.
            _ => (&self.main, ChatRequest { model: &self.main_model, ..req }),
        }
    }
}

#[async_trait]
impl Provider for Routed {
    async fn chat(&self, req: ChatRequest<'_>, on_event: EventSink<'_>) -> anyhow::Result<ChatResponse> {
        let (provider, req) = self.pick(req);
        provider.chat(req, on_event).await
    }

    async fn warm(&self, req: ChatRequest<'_>) {
        let (provider, req) = self.pick(req);
        provider.warm(req).await
    }
}

/// The quick-reply service when `settings` names one of its own (`fast_base_url`).
pub fn build_fast_provider(settings: &Settings, api_key: Option<String>) -> Option<Arc<dyn Provider>> {
    settings.has_fast_endpoint().then(|| {
        Arc::new(
            openai_compat::OpenAiCompat::new(settings.fast_base_url.trim().to_string(), api_key)
                .with_stall_timeout(std::time::Duration::from_secs(if settings.fast_is_local() { 300 } else { 60 })),
        ) as Arc<dyn Provider>
    })
}

/// Builds the provider described by `settings`.
pub fn build_provider(settings: &Settings, api_key: Option<String>) -> Arc<dyn Provider> {
    match settings.provider {
        ProviderKind::OpenaiCompat => Arc::new(
            openai_compat::OpenAiCompat::new(settings.base_url.clone(), api_key)
                .with_reasoning(settings.reasoning.effort())
                .with_no_training(settings.no_training && settings.is_openrouter())
                // A local server can read a long prompt for minutes before the first token.
                .with_stall_timeout(std::time::Duration::from_secs(if settings.is_local() { 300 } else { 60 })),
        ),
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

#[cfg(test)]
mod routed_tests {
    use super::mock::{reply, MockProvider};
    use super::*;

    fn pair() -> (Arc<MockProvider>, Arc<MockProvider>) {
        let script = || (0..4).map(|_| reply("ok", vec![])).collect::<Vec<_>>();
        (Arc::new(MockProvider::scripted(script())), Arc::new(MockProvider::scripted(script())))
    }

    async fn ask(p: &Routed, model: &str, web: Option<u8>) {
        let msgs = [Message::user("hi")];
        let req = ChatRequest { model, messages: &msgs, tools: &[], temperature: 0.2, max_tokens: 10, web };
        p.chat(req, &mut |_| {}).await.unwrap();
    }

    #[tokio::test]
    async fn the_quick_reply_model_goes_to_its_own_service_and_the_planner_to_the_main_one() {
        let (main, fast) = pair();
        let r = Routed::new(main.clone(), Some(fast.clone()), "openai/gpt-6-luna", "gemini-3.5-flash-lite", false);
        ask(&r, "openai/gpt-6-luna", None).await;
        ask(&r, "gemini-3.5-flash-lite", None).await;
        assert_eq!(*main.models.lock().unwrap(), ["openai/gpt-6-luna"]);
        assert_eq!(*fast.models.lock().unwrap(), ["gemini-3.5-flash-lite"]);
    }

    #[tokio::test]
    async fn a_web_search_the_second_service_cant_do_goes_to_the_planners_service() {
        let (main, fast) = pair();
        let r = Routed::new(main.clone(), Some(fast.clone()), "openai/gpt-6-luna", "gemini-3.5-flash-lite", false);
        ask(&r, "gemini-3.5-flash-lite", Some(5)).await;
        assert_eq!(*main.models.lock().unwrap(), ["openai/gpt-6-luna"], "the planner's model answers there");
        assert_eq!(*main.webs.lock().unwrap(), [Some(5)]);
        assert!(fast.models.lock().unwrap().is_empty());
        // A second service that can search (another OpenRouter, say) keeps its own searches.
        let (main, fast) = pair();
        let r = Routed::new(main.clone(), Some(fast.clone()), "a", "b", true);
        ask(&r, "b", Some(5)).await;
        assert_eq!(*fast.webs.lock().unwrap(), [Some(5)]);
    }

    #[tokio::test]
    async fn without_a_key_for_the_second_service_the_planner_covers_quick_replies() {
        let (main, _) = pair();
        let r = Routed::new(main.clone(), None, "openai/gpt-6-luna", "gemini-3.5-flash-lite", false);
        ask(&r, "gemini-3.5-flash-lite", None).await;
        assert_eq!(*main.models.lock().unwrap(), ["openai/gpt-6-luna"]);
    }

    #[tokio::test]
    async fn one_model_name_for_both_stays_on_the_main_service() {
        let (main, fast) = pair();
        let r = Routed::new(main.clone(), Some(fast.clone()), "same", "same", false);
        ask(&r, "same", None).await;
        assert_eq!(main.models.lock().unwrap().len(), 1);
        assert!(fast.models.lock().unwrap().is_empty());
    }

    #[test]
    fn a_fast_service_is_built_only_when_one_is_named() {
        let s = Settings::default();
        assert!(build_fast_provider(&s, Some("k".into())).is_none());
        let s = Settings { fast_base_url: "https://generativelanguage.googleapis.com/v1beta/openai/".into(), ..Settings::default() };
        assert!(build_fast_provider(&s, Some("k".into())).is_some());
        let ollama = Settings { provider: ProviderKind::Ollama, fast_base_url: "http://x".into(), ..Settings::default() };
        assert!(!ollama.has_fast_endpoint(), "a second service is for OpenAI-compatible endpoints");
    }
}
