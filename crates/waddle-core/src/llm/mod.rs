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
    /// What the service wants back with this call on the next request, unchanged: Gemini 3
    /// on Google's own API signs its calls and refuses a conversation that drops the signature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub echo: Option<Value>,
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

/// A free tier said no: a per-minute limit that lifts soon, or a day's allowance used up.
/// `Routed` reads it to send the request to the same model on OpenRouter instead.
#[derive(Debug)]
pub struct QuotaError {
    pub per_day: bool,
    /// How long the service says to wait.
    pub retry: Option<std::time::Duration>,
    /// What to tell the user.
    pub message: String,
}

impl std::fmt::Display for QuotaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for QuotaError {}

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
    /// The second service is Google's Gemini API: any model it names ("gemini-…", no
    /// "google/" in front) goes there, the planner's too, so tasks can run on a free key.
    google: bool,
    /// What the planner's service runs instead when the second service can't take a request.
    fallback: String,
    /// The planner's service is OpenRouter, which has every Gemini model too ("google/…").
    main_is_openrouter: bool,
    /// Until when Google's free key is left alone after one of its limits.
    resting_until: std::sync::Mutex<Option<std::time::Instant>>,
}

/// A model name for Google's own API ("gemini-3.5-flash-lite"), not OpenRouter's ("google/…").
pub fn is_google_name(model: &str) -> bool {
    !model.contains('/') && (model.starts_with("gemini") || model.starts_with("gemma"))
}

impl Routed {
    pub fn new(main: Arc<dyn Provider>, fast: Option<Arc<dyn Provider>>, main_model: &str, fast_model: &str, fast_has_web: bool) -> Self {
        Self {
            main,
            fast,
            main_model: main_model.to_string(),
            fast_model: fast_model.to_string(),
            fast_has_web,
            google: false,
            fallback: main_model.to_string(),
            main_is_openrouter: false,
            resting_until: std::sync::Mutex::new(None),
        }
    }

    /// The second service is Google's Gemini API. A planner model named for it runs
    /// there too; without the Google key it runs as the same Gemini model on
    /// OpenRouter (`main_is_openrouter`), or as the planner's model elsewhere.
    pub fn with_google(mut self, main_is_openrouter: bool) -> Self {
        self.google = true;
        self.main_is_openrouter = main_is_openrouter;
        if main_is_openrouter && is_google_name(&self.main_model) {
            self.fallback = format!("google/{}", self.main_model);
        }
        self
    }

    /// The same model on OpenRouter (paid), for when Google's free key is over a limit.
    fn paid_twin(&self, model: &str) -> Option<String> {
        (self.google && self.main_is_openrouter && is_google_name(model)).then(|| format!("google/{model}"))
    }

    fn google_resting(&self) -> bool {
        self.resting_until.lock().unwrap().is_some_and(|until| std::time::Instant::now() < until)
    }

    /// Leaves the free key alone until Google said to come back (a minute, or an hour for a
    /// day's allowance, when it doesn't say).
    fn rest_google(&self, q: &QuotaError) {
        let wait = q.retry.unwrap_or(std::time::Duration::from_secs(if q.per_day { 3600 } else { 60 }));
        *self.resting_until.lock().unwrap() = Some(std::time::Instant::now() + wait);
    }

    fn goes_to_google(&self, provider: &Arc<dyn Provider>) -> bool {
        self.google && self.fast.as_ref().is_some_and(|f| Arc::ptr_eq(f, provider))
    }

    /// Where `req` goes, and the request to send there.
    fn pick<'a>(&'a self, req: ChatRequest<'a>) -> (&'a Arc<dyn Provider>, ChatRequest<'a>) {
        let to_fast = (req.model == self.fast_model && self.fast_model != self.main_model) || (self.google && is_google_name(req.model));
        if !to_fast {
            return (&self.main, req);
        }
        match &self.fast {
            Some(fast) if req.web.is_none() || self.fast_has_web => (fast, req),
            // No key for the second service yet, or a web search it can't do: the planner's
            // service answers instead.
            _ => (&self.main, ChatRequest { model: &self.fallback, ..req }),
        }
    }
}

#[async_trait]
impl Provider for Routed {
    async fn chat(&self, req: ChatRequest<'_>, on_event: EventSink<'_>) -> anyhow::Result<ChatResponse> {
        let (provider, routed) = self.pick(req);
        let twin = self.paid_twin(req.model).filter(|_| self.goes_to_google(provider));
        let Some(twin) = twin else { return provider.chat(routed, on_event).await };
        // Over one of the free key's limits: the same model on OpenRouter carries on with the
        // same conversation (it accepts Google's calls), and Google gets it back afterwards.
        if self.google_resting() {
            return self.main.chat(ChatRequest { model: &twin, ..req }, on_event).await;
        }
        match provider.chat(routed, on_event).await {
            Err(e) => match e.downcast_ref::<QuotaError>() {
                Some(q) => {
                    log::warn!("{q}; {twin} on OpenRouter answers until it lifts");
                    self.rest_google(q);
                    self.main.chat(ChatRequest { model: &twin, ..req }, on_event).await
                }
                None => Err(e),
            },
            ok => ok,
        }
    }

    async fn warm(&self, req: ChatRequest<'_>) {
        let (provider, req) = self.pick(req);
        provider.warm(req).await
    }
}

/// The planner's service, joined by the quick-reply service when `settings` names one of its
/// own (see `Routed`). `fast` is None while that service has no key yet.
pub fn route(settings: &Settings, main: Arc<dyn Provider>, fast: Option<Arc<dyn Provider>>) -> Arc<dyn Provider> {
    if !settings.has_fast_endpoint() {
        return main;
    }
    let mut routed = Routed::new(main, fast, &settings.model, settings.fast_model(), settings.fast_base_url.contains("openrouter.ai"));
    if openai_compat::is_google(&settings.fast_base_url) {
        // A Gemini planner model then runs on the free Google key too.
        routed = routed.with_google(settings.is_openrouter());
    }
    Arc::new(routed)
}

/// The quick-reply service when `settings` names one of its own (`fast_base_url`).
pub fn build_fast_provider(settings: &Settings, api_key: Option<String>) -> Option<Arc<dyn Provider>> {
    settings.has_fast_endpoint().then(|| {
        let fast = openai_compat::OpenAiCompat::new(settings.fast_base_url.trim().to_string(), api_key)
            .with_stall_timeout(std::time::Duration::from_secs(if settings.fast_is_local() { 300 } else { 60 }));
        // Next to OpenRouter, a Google limit moves the request there at once (see `Routed`)
        // instead of waiting for the limit to lift.
        let fast = if settings.is_openrouter() { fast.without_quota_wait() } else { fast };
        Arc::new(fast) as Arc<dyn Provider>
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

    /// Google's free key, over one of its limits.
    struct OverLimit {
        per_day: bool,
        asked: std::sync::Mutex<usize>,
    }

    #[async_trait]
    impl Provider for OverLimit {
        async fn chat(&self, _req: ChatRequest<'_>, _on_event: EventSink<'_>) -> anyhow::Result<ChatResponse> {
            *self.asked.lock().unwrap() += 1;
            Err(anyhow::Error::new(QuotaError { per_day: self.per_day, retry: Some(std::time::Duration::from_secs(30)), message: "over the limit".into() }).context("Google"))
        }
    }

    #[tokio::test]
    async fn over_a_google_limit_the_same_model_carries_on_on_openrouter() {
        let (main, _) = pair();
        let google = Arc::new(OverLimit { per_day: false, asked: Default::default() });
        let r = Routed::new(main.clone(), Some(google.clone()), "gemini-3.5-flash-lite", "gemini-3.5-flash-lite", false).with_google(true);
        ask(&r, "gemini-3.5-flash-lite", None).await;
        assert_eq!(*main.models.lock().unwrap(), ["google/gemini-3.5-flash-lite"], "answered at once on OpenRouter");
        // Until Google said to come back, the free key isn't asked again.
        ask(&r, "gemini-3.5-flash-lite", None).await;
        assert_eq!(*google.asked.lock().unwrap(), 1);
        assert_eq!(main.models.lock().unwrap().len(), 2);
        // Afterwards it is.
        *r.resting_until.lock().unwrap() = Some(std::time::Instant::now());
        ask(&r, "gemini-3.5-flash-lite", None).await;
        assert_eq!(*google.asked.lock().unwrap(), 2);
        // The planner's own OpenRouter model never touches Google.
        ask(&r, "openai/gpt-6-luna", None).await;
        assert_eq!(main.models.lock().unwrap().last().unwrap(), "openai/gpt-6-luna");
    }

    #[tokio::test]
    async fn with_no_openrouter_to_fall_back_on_the_limit_is_reported() {
        let (main, _) = pair();
        let google = Arc::new(OverLimit { per_day: true, asked: Default::default() });
        // The planner's service isn't OpenRouter (a local server, say): no paid twin to move to.
        let r = Routed::new(main.clone(), Some(google.clone()), "gemini-3.5-flash-lite", "gemini-3.5-flash-lite", false).with_google(false);
        let msgs = [Message::user("hi")];
        let req = ChatRequest { model: "gemini-3.5-flash-lite", messages: &msgs, tools: &[], temperature: 0.2, max_tokens: 10, web: None };
        let err = r.chat(req, &mut |_| {}).await.unwrap_err();
        assert!(err.downcast_ref::<QuotaError>().is_some(), "{err:#}");
        assert!(main.models.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn one_model_name_for_both_stays_on_the_main_service() {
        let (main, fast) = pair();
        let r = Routed::new(main.clone(), Some(fast.clone()), "same", "same", false);
        ask(&r, "same", None).await;
        assert_eq!(main.models.lock().unwrap().len(), 1);
        assert!(fast.models.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn tasks_run_on_a_free_gemini_key_when_the_planner_is_a_gemini_model() {
        // OpenRouter for routing and web searches, Google's free API for the planner and quick replies.
        let (main, fast) = pair();
        let r = Routed::new(main.clone(), Some(fast.clone()), "gemini-3.5-flash-lite", "gemini-3.5-flash-lite", false).with_google(true);
        ask(&r, "gemini-3.5-flash-lite", None).await;
        assert_eq!(*fast.models.lock().unwrap(), ["gemini-3.5-flash-lite"], "the planner went to Google");
        assert!(main.models.lock().unwrap().is_empty());
        // A web search Google's endpoint can't run goes to OpenRouter, under OpenRouter's name for the model.
        ask(&r, "gemini-3.5-flash-lite", Some(5)).await;
        assert_eq!(*main.models.lock().unwrap(), ["google/gemini-3.5-flash-lite"]);
        // Without the Google key, everything still works on OpenRouter.
        let (main, _) = pair();
        let r = Routed::new(main.clone(), None, "gemini-3.5-flash-lite", "gemini-3.5-flash-lite", false).with_google(true);
        ask(&r, "gemini-3.5-flash-lite", None).await;
        assert_eq!(*main.models.lock().unwrap(), ["google/gemini-3.5-flash-lite"]);
        // OpenRouter's own names stay on OpenRouter.
        let (main, fast) = pair();
        let r = Routed::new(main.clone(), Some(fast.clone()), "openai/gpt-6-luna", "gemini-3.5-flash-lite", false).with_google(true);
        ask(&r, "openai/gpt-6-luna", None).await;
        ask(&r, "google/gemini-3.8-flash", None).await;
        assert_eq!(main.models.lock().unwrap().len(), 2);
        assert!(fast.models.lock().unwrap().is_empty());
        assert!(is_google_name("gemini-3.8-flash") && !is_google_name("google/gemini-3.8-flash") && !is_google_name("openai/gpt-6-luna"));
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
