//! OpenAI-compatible chat completions with SSE streaming. Covers OpenRouter,
//! OpenAI, LM Studio, llama.cpp server and Microsoft Foundry Local.

use anyhow::{anyhow, Context};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::Duration;

use super::{textcalls, ChatRequest, ChatResponse, Citation, EventSink, Message, Provider, Role, StreamEvent, ToolCall, Usage};

/// Waits before the first and second retry, unless the server asks for something else.
const BACKOFF: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(3)];
/// The longest `Retry-After` Waddle honours; past that, the user is told instead.
const MAX_WAIT: Duration = Duration::from_secs(8);

pub struct OpenAiCompat {
    base_url: String,
    api_key: Option<String>,
    reasoning: Option<&'static str>,
    no_training: bool,
    /// A stream that sends nothing (not even keep-alive comments) for this long is treated as dropped.
    stall: Duration,
    http: reqwest::Client,
}

/// One failed attempt, with whether trying the same request again could help.
struct Failure {
    error: anyhow::Error,
    retryable: bool,
    retry_after: Option<Duration>,
}

impl Failure {
    fn transient(error: anyhow::Error) -> Self {
        Self { error, retryable: true, retry_after: None }
    }
}

/// An error object the server sent in the middle of a stream.
#[derive(Debug)]
pub(crate) struct StreamError {
    pub code: Option<u64>,
    pub message: String,
}

impl std::fmt::Display for StreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "provider error: {}", self.message)
    }
}

impl std::error::Error for StreamError {}

/// Rate limits, timeouts and server trouble pass; bad keys, bad requests and missing credit don't.
fn retryable_status(status: u16) -> bool {
    matches!(status, 408 | 429 | 500 | 502 | 503 | 504 | 520..=529)
}

/// "OpenRouter" for openrouter.ai, else the host name, for error messages.
fn service_name(base_url: &str) -> String {
    let host = base_url.split("://").nth(1).unwrap_or(base_url).split(['/', ':']).next().unwrap_or(base_url);
    if host.ends_with("openrouter.ai") { "OpenRouter".into() } else { host.to_string() }
}

/// What a failed request means for the user, in plain words.
pub(crate) fn describe_status(status: u16, detail: &str, service: &str) -> String {
    let detail = detail.trim();
    let tail = if detail.is_empty() { String::new() } else { format!(" ({})", detail.chars().take(300).collect::<String>()) };
    match status {
        401 => format!("{service} didn't accept the API key. Check it in Settings{tail}"),
        402 => format!("the {service} account is out of credit. Add some at openrouter.ai/credits, then try again{tail}"),
        403 => format!("{service} refused this request{tail}"),
        404 => format!("{service} doesn't know that model. Check the model name in Settings{tail}"),
        408 | 504 => format!("the model took too long to answer. Try again in a moment{tail}"),
        413 => format!("the request was too big for the model. Try a shorter task{tail}"),
        429 => format!("{service} is rate-limiting this model right now. Try again in a minute{tail}"),
        500..=599 => format!("the model's provider is having trouble ({status}). Try again in a moment{tail}"),
        _ => format!("{status} from {service}{tail}"),
    }
}

fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let secs: f64 = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?.trim().parse().ok()?;
    (secs.is_finite() && secs >= 0.0).then(|| Duration::from_secs_f64(secs))
}

impl OpenAiCompat {
    pub fn new(base_url: String, api_key: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("http client");
        Self { base_url: base_url.trim_end_matches('/').to_string(), api_key, reasoning: None, no_training: false, stall: Duration::from_secs(60), http }
    }

    /// How long a stream may go silent before it counts as dropped.
    pub fn with_stall_timeout(mut self, stall: Duration) -> Self {
        self.stall = stall;
        self
    }

    /// Sends OpenRouter's `reasoning.effort` with every request ("none", "low", ...).
    pub fn with_reasoning(mut self, effort: Option<&'static str>) -> Self {
        self.reasoning = effort;
        self
    }

    /// Asks OpenRouter to route only to providers that don't keep or train on prompts.
    pub fn with_no_training(mut self, on: bool) -> Self {
        self.no_training = on;
        self
    }

    fn body(&self, req: &ChatRequest<'_>) -> Value {
        let mut body = request_body(req, self.reasoning);
        if self.no_training {
            body["provider"] = json!({ "data_collection": "deny" });
        }
        body
    }
}

pub(crate) fn message_to_json(m: &Message) -> Value {
    match m.role {
        Role::System => json!({ "role": "system", "content": m.text }),
        Role::User => {
            if m.images.is_empty() {
                json!({ "role": "user", "content": m.text })
            } else {
                let mut parts = vec![json!({ "type": "text", "text": m.text })];
                for img in &m.images {
                    parts.push(json!({
                        "type": "image_url",
                        "image_url": { "url": format!("data:{};base64,{}", img.mime, img.base64) }
                    }));
                }
                json!({ "role": "user", "content": parts })
            }
        }
        Role::Assistant => {
            let mut v = json!({ "role": "assistant", "content": m.text });
            if !m.tool_calls.is_empty() {
                v["tool_calls"] = m
                    .tool_calls
                    .iter()
                    .map(|c| {
                        json!({
                            "id": c.id,
                            "type": "function",
                            "function": { "name": c.name, "arguments": c.arguments.to_string() }
                        })
                    })
                    .collect();
            }
            v
        }
        Role::Tool => json!({
            "role": "tool",
            "tool_call_id": m.tool_call_id.clone().unwrap_or_default(),
            "content": m.text
        }),
    }
}

pub(crate) fn request_body(req: &ChatRequest<'_>, reasoning: Option<&str>) -> Value {
    let mut body = json!({
        "model": req.model,
        "messages": req.messages.iter().map(message_to_json).collect::<Vec<_>>(),
        "stream": true,
        "temperature": req.temperature,
        "max_tokens": req.max_tokens,
    });
    if let Some(effort) = reasoning {
        body["reasoning"] = json!({ "effort": effort });
    }
    if let Some(n) = req.web {
        // Exa costs a flat ~$0.007 a search; "native" would bill the model's own (dearer) search.
        body["plugins"] = json!([{ "id": "web", "engine": "exa", "max_results": n }]);
    }
    if !req.tools.is_empty() {
        body["tools"] = req
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": { "name": t.name, "description": t.description, "parameters": t.parameters }
                })
            })
            .collect();
    }
    body
}

#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

/// Incremental parser for the `data:` lines of an SSE chat stream.
#[derive(Default)]
pub(crate) struct SseAccumulator {
    buffer: String,
    text: String,
    calls: BTreeMap<u64, PartialCall>,
    usage: Usage,
    citations: Vec<Citation>,
    truncated: bool,
    done: bool,
}

impl SseAccumulator {
    /// Feeds raw bytes; emits text deltas as complete lines arrive.
    pub fn push(&mut self, chunk: &str, on_event: &mut (dyn FnMut(StreamEvent) + Send)) -> anyhow::Result<()> {
        self.buffer.push_str(chunk);
        while let Some(pos) = self.buffer.find('\n') {
            let line: String = self.buffer.drain(..=pos).collect();
            self.line(line.trim_end_matches(['\r', '\n']), on_event)?;
        }
        Ok(())
    }

    fn line(&mut self, line: &str, on_event: &mut (dyn FnMut(StreamEvent) + Send)) -> anyhow::Result<()> {
        // Blank lines separate events; lines starting with ':' are keep-alive comments.
        let Some(data) = line.strip_prefix("data:") else { return Ok(()) };
        let data = data.trim();
        if data == "[DONE]" {
            self.done = true;
            return Ok(());
        }
        let v: Value = serde_json::from_str(data).with_context(|| format!("bad stream chunk: {data}"))?;
        if let Some(err) = v.get("error") {
            let message = err.get("message").and_then(Value::as_str).unwrap_or("unknown error").to_string();
            let code = err.get("code").and_then(|c| c.as_u64().or_else(|| c.as_str().and_then(|s| s.parse().ok())));
            return Err(StreamError { code, message }.into());
        }
        if v.pointer("/choices/0/finish_reason").and_then(Value::as_str) == Some("length") {
            self.truncated = true;
        }
        // OpenRouter sends token counts and the price with the last chunk.
        if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
            let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
            self.usage = Usage { prompt_tokens: n("prompt_tokens"), completion_tokens: n("completion_tokens"), cost: u.get("cost").and_then(Value::as_f64) };
        }
        let Some(delta) = v.pointer("/choices/0/delta") else { return Ok(()) };
        for a in delta.get("annotations").and_then(Value::as_array).into_iter().flatten() {
            let Some(c) = a.get("url_citation") else { continue };
            let url = c.get("url").and_then(Value::as_str).unwrap_or("").to_string();
            if !url.is_empty() && !self.citations.iter().any(|x| x.url == url) {
                let title = c.get("title").and_then(Value::as_str).unwrap_or("").to_string();
                self.citations.push(Citation { url, title });
            }
        }
        if let Some(t) = delta.get("content").and_then(Value::as_str) {
            if !t.is_empty() {
                self.text.push_str(t);
                on_event(StreamEvent::TextDelta(t.to_string()));
            }
        }
        if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
            for (pos, c) in calls.iter().enumerate() {
                let index = c.get("index").and_then(Value::as_u64).unwrap_or(pos as u64);
                let entry = self.calls.entry(index).or_default();
                if let Some(id) = c.get("id").and_then(Value::as_str) {
                    entry.id = id.to_string();
                }
                if let Some(name) = c.pointer("/function/name").and_then(Value::as_str) {
                    entry.name.push_str(name);
                }
                if let Some(args) = c.pointer("/function/arguments") {
                    match args {
                        Value::String(s) => entry.arguments.push_str(s),
                        other => entry.arguments.push_str(&other.to_string()),
                    }
                }
            }
        }
        Ok(())
    }

    pub fn finish(mut self) -> ChatResponse {
        if !self.buffer.trim().is_empty() {
            let rest = std::mem::take(&mut self.buffer);
            let _ = self.line(rest.trim(), &mut |_| {});
        }
        let mut tool_calls: Vec<ToolCall> = self
            .calls
            .into_values()
            .filter(|c| !c.name.is_empty())
            .map(|c| ToolCall {
                id: if c.id.is_empty() { super::new_call_id() } else { c.id },
                name: c.name,
                arguments: super::parse_arguments(&c.arguments),
            })
            .collect();
        let mut text = self.text;
        if tool_calls.is_empty() {
            // Some local servers leave Qwen-style <tool_call> blocks in the text.
            let (cleaned, calls) = textcalls::extract(&text);
            if !calls.is_empty() {
                text = cleaned;
                tool_calls = calls;
            }
        }
        ChatResponse { text, tool_calls, usage: self.usage, citations: self.citations, truncated: self.truncated }
    }
}

impl OpenAiCompat {
    /// One request, streamed to the end.
    async fn attempt(&self, req: &ChatRequest<'_>, on_event: EventSink<'_>) -> Result<ChatResponse, Failure> {
        let url = format!("{}/chat/completions", self.base_url);
        let service = service_name(&self.base_url);
        let mut builder = self
            .http
            .post(&url)
            .header("HTTP-Referer", "https://github.com/DimeDataCloud/Waddle")
            .header("X-Title", "Project Waddle")
            .json(&self.body(req));
        if let Some(key) = self.api_key.as_deref().filter(|k| !k.is_empty()) {
            builder = builder.bearer_auth(key);
        }
        let resp = match builder.send().await {
            Ok(r) => r,
            Err(e) => {
                let error = anyhow::Error::msg(format!("{e:#}")).context(format!("couldn't reach {service}. Check the internet connection"));
                return Err(Failure::transient(error));
            }
        };
        let status = resp.status();
        if !status.is_success() {
            let wait = retry_after(resp.headers());
            let body = resp.text().await.unwrap_or_default();
            let detail = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_string))
                .unwrap_or(body);
            log::warn!("{status} from {url}: {}", detail.chars().take(400).collect::<String>());
            let error = anyhow!(describe_status(status.as_u16(), &detail, &service));
            return Err(Failure { error, retryable: retryable_status(status.as_u16()), retry_after: wait });
        }
        let mut acc = SseAccumulator::default();
        let mut stream = resp.bytes_stream();
        loop {
            let chunk = match tokio::time::timeout(self.stall, stream.next()).await {
                Err(_) => return Err(Failure::transient(anyhow!("the model stopped responding mid-answer"))),
                Ok(None) => break,
                Ok(Some(Err(e))) => {
                    return Err(Failure::transient(anyhow::Error::msg(format!("{e:#}")).context(format!("the connection to {service} dropped mid-answer"))));
                }
                Ok(Some(Ok(c))) => c,
            };
            if let Err(e) = acc.push(&String::from_utf8_lossy(&chunk), on_event) {
                let retryable = e.downcast_ref::<StreamError>().and_then(|s| s.code).is_some_and(|c| retryable_status(c as u16));
                return Err(Failure { error: e, retryable, retry_after: None });
            }
            if acc.done {
                break;
            }
        }
        Ok(acc.finish())
    }
}

#[async_trait]
impl Provider for OpenAiCompat {
    /// Retries rate limits, server errors and dropped connections up to twice on
    /// the same model, but only while nothing has reached the user yet: a retry
    /// after words were shown would show them twice.
    async fn chat(&self, req: ChatRequest<'_>, on_event: EventSink<'_>) -> anyhow::Result<ChatResponse> {
        let mut retries = 0;
        loop {
            let mut shown = false;
            let result = {
                let mut sink = |e: StreamEvent| {
                    shown = true;
                    on_event(e)
                };
                self.attempt(&req, &mut sink).await
            };
            match result {
                Ok(r) => return Ok(r),
                Err(f) if f.retryable && !shown && retries < BACKOFF.len() => {
                    let wait = f.retry_after.unwrap_or(BACKOFF[retries]);
                    if wait > MAX_WAIT {
                        return Err(f.error);
                    }
                    log::warn!("model call failed ({:#}); retrying in {:.1} s", f.error, wait.as_secs_f64());
                    tokio::time::sleep(wait).await;
                    retries += 1;
                }
                Err(f) => return Err(f.error),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{ImageData, ToolSpec};

    fn feed(chunks: &[&str]) -> (ChatResponse, Vec<String>) {
        let mut acc = SseAccumulator::default();
        let mut deltas = vec![];
        let mut sink = |e: StreamEvent| {
            let StreamEvent::TextDelta(t) = e;
            deltas.push(t);
        };
        for c in chunks {
            acc.push(c, &mut sink).unwrap();
        }
        (acc.finish(), deltas)
    }

    #[test]
    fn streams_text_and_assembles_split_tool_call() {
        let chunks = [
            ": OPENROUTER PROCESSING\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"Opening \"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"Notepad.\"}}]}\n",
            "\ndata: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"open_app\",\"arguments\":\"{\\\"na\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"me\\\":\\\"notepad\\\"}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":1800,\"completion_tokens\":30,\"cost\":0.00023}}\n\n",
            "data: [DONE]\n\n",
        ];
        let (resp, deltas) = feed(&chunks);
        assert_eq!(deltas, vec!["Opening ", "Notepad."]);
        assert_eq!(resp.text, "Opening Notepad.");
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].id, "c1");
        assert_eq!(resp.tool_calls[0].name, "open_app");
        assert_eq!(resp.tool_calls[0].arguments, json!({"name": "notepad"}));
        assert_eq!(resp.usage, Usage { prompt_tokens: 1800, completion_tokens: 30, cost: Some(0.00023) });
    }

    #[test]
    fn line_split_across_chunks_is_reassembled() {
        let (resp, _) = feed(&["data: {\"choices\":[{\"delta\":{\"con", "tent\":\"hi\"}}]}\n\n"]);
        assert_eq!(resp.text, "hi");
    }

    #[test]
    fn stream_error_object_is_reported() {
        let mut acc = SseAccumulator::default();
        let err = acc.push("data: {\"error\":{\"message\":\"rate limited\"}}\n", &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("rate limited"));
    }

    #[test]
    fn empty_arguments_become_empty_object() {
        let (resp, _) = feed(&[
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"a\",\"function\":{\"name\":\"list_windows\",\"arguments\":\"\"}}]}}]}\n",
        ]);
        assert_eq!(resp.tool_calls[0].arguments, json!({}));
    }

    #[test]
    fn serialises_images_and_tool_messages() {
        let call = ToolCall { id: "c9".into(), name: "look_at_screen".into(), arguments: json!({}) };
        let msgs = vec![
            Message::system("sys"),
            Message::assistant("", vec![call.clone()]),
            Message::tool_result(&call, "ok"),
            Message::user_with_image("screen", ImageData { mime: "image/png".into(), base64: "AAA".into() }),
        ];
        let tools = vec![ToolSpec { name: "x".into(), description: "d".into(), parameters: json!({"type":"object"}) }];
        let req = ChatRequest { model: "m", messages: &msgs, tools: &tools, temperature: 0.2, max_tokens: 10, web: None };
        let body = request_body(&req, None);
        assert_eq!(body["messages"][1]["tool_calls"][0]["function"]["arguments"], "{}");
        assert_eq!(body["messages"][2]["tool_call_id"], "c9");
        assert_eq!(body["messages"][3]["content"][1]["image_url"]["url"], "data:image/png;base64,AAA");
        assert_eq!(body["tools"][0]["function"]["name"], "x");
        assert_eq!(body["stream"], true);
        assert!(body.get("reasoning").is_none());
        assert!(body.get("plugins").is_none() && body.get("provider").is_none());
        let body = request_body(&req, Some("none"));
        assert_eq!(body["reasoning"]["effort"], "none");
    }

    #[test]
    fn web_search_and_no_training_reach_the_request() {
        let msgs = vec![Message::user("who won?")];
        let req = ChatRequest { model: "m", messages: &msgs, tools: &[], temperature: 0.2, max_tokens: 10, web: Some(3) };
        let private = OpenAiCompat::new("https://openrouter.ai/api/v1".into(), None).with_no_training(true);
        let body = private.body(&req);
        assert_eq!(body["provider"]["data_collection"], "deny");
        assert_eq!(body["plugins"][0]["id"], "web");
        assert_eq!(body["plugins"][0]["max_results"], 3);
        let open = OpenAiCompat::new("https://openrouter.ai/api/v1".into(), None);
        assert!(open.body(&req).get("provider").is_none());
    }

    /// What the scripted server does with one connection.
    enum Reply {
        Raw(&'static str),
        /// Sends the headers and part of the stream, then waits silently.
        Stall(&'static str),
    }

    /// Serves one scripted reply per connection, in order; returns the base URL and the connection count.
    async fn scripted(replies: Vec<Reply>) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = count.clone();
        tokio::spawn(async move {
            for reply in replies {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                // Read the whole request so closing the socket doesn't reset it.
                let mut req = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    req.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&req).to_string();
                    if let Some(end) = text.find("\r\n\r\n") {
                        let len = text.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                        if req.len() >= end + 4 + len {
                            break;
                        }
                    }
                    if n == 0 {
                        break;
                    }
                }
                match reply {
                    Reply::Raw(r) => {
                        let _ = sock.write_all(r.as_bytes()).await;
                    }
                    Reply::Stall(r) => {
                        let _ = sock.write_all(r.as_bytes()).await;
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        });
        (format!("http://{addr}/v1"), count)
    }

    const SSE_OK: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"Hi!\"}}]}\n\ndata: [DONE]\n\n";
    const RATE_LIMITED: &str = "HTTP/1.1 429 Too Many Requests\r\nretry-after: 0\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: 41\r\n\r\n{\"error\":{\"message\":\"slow down please\"}}";
    const BAD_KEY: &str = "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: 36\r\n\r\n{\"error\":{\"message\":\"No auth found\"}}";

    async fn ask(p: &OpenAiCompat) -> (anyhow::Result<ChatResponse>, String) {
        let msgs = vec![Message::user("hi")];
        let req = ChatRequest { model: "m", messages: &msgs, tools: &[], temperature: 0.2, max_tokens: 10, web: None };
        let mut shown = String::new();
        let r = p.chat(req, &mut |e: StreamEvent| {
            let StreamEvent::TextDelta(t) = e;
            shown.push_str(&t);
        }).await;
        (r, shown)
    }

    #[tokio::test]
    async fn rate_limits_and_stalls_are_retried_on_the_same_model() {
        let (url, count) = scripted(vec![
            Reply::Raw(RATE_LIMITED),
            Reply::Stall("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n"),
            Reply::Raw(SSE_OK),
        ])
        .await;
        let p = OpenAiCompat::new(url, None).with_stall_timeout(Duration::from_millis(300));
        let (r, shown) = ask(&p).await;
        assert_eq!(r.unwrap().text, "Hi!");
        assert_eq!(shown, "Hi!");
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_bad_key_fails_at_once_in_plain_words() {
        let (url, count) = scripted(vec![Reply::Raw(BAD_KEY), Reply::Raw(SSE_OK)]).await;
        let p = OpenAiCompat::new(url, None);
        let err = ask(&p).await.0.unwrap_err().to_string();
        assert!(err.contains("didn't accept the API key") && err.contains("No auth found"), "{err}");
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn no_retry_once_words_were_shown() {
        let partial = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n";
        let (url, count) = scripted(vec![Reply::Stall(partial), Reply::Raw(SSE_OK)]).await;
        let p = OpenAiCompat::new(url, None).with_stall_timeout(Duration::from_millis(300));
        let (r, shown) = ask(&p).await;
        assert!(r.unwrap_err().to_string().contains("stopped responding"));
        assert_eq!(shown, "Hel");
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn cut_off_replies_are_flagged() {
        let (resp, _) = feed(&["data: {\"choices\":[{\"delta\":{\"content\":\"long\"},\"finish_reason\":\"length\"}]}\n"]);
        assert!(resp.truncated);
        let (resp, _) = feed(&["data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n"]);
        assert!(!resp.truncated);
    }

    #[test]
    fn statuses_read_as_next_steps() {
        assert!(describe_status(402, "", "OpenRouter").contains("openrouter.ai/credits"));
        assert!(describe_status(429, "", "OpenRouter").starts_with("OpenRouter is rate-limiting"));
        assert_eq!(service_name("https://openrouter.ai/api/v1"), "OpenRouter");
        assert_eq!(service_name("http://localhost:1234/v1"), "localhost");
        assert!(retryable_status(503) && retryable_status(429) && !retryable_status(400) && !retryable_status(402));
    }

    #[test]
    fn collects_web_citations_once_each() {
        let ann = r#"{"type":"url_citation","url_citation":{"url":"https://a.example/x","title":"A","content":"…"}}"#;
        let line = format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"Hi\",\"annotations\":[{ann},{ann}]}}}}]}}\n");
        let (resp, _) = feed(&[&line, &line]);
        assert_eq!(resp.citations, vec![Citation { url: "https://a.example/x".into(), title: "A".into() }]);
    }
}
