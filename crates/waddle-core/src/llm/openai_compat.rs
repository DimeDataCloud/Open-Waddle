//! OpenAI-compatible chat completions with SSE streaming. Covers OpenRouter,
//! OpenAI, LM Studio, llama.cpp server and Microsoft Foundry Local.

use anyhow::{anyhow, bail, Context};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::collections::BTreeMap;

use super::{textcalls, ChatRequest, ChatResponse, EventSink, Message, Provider, Role, StreamEvent, ToolCall, Usage};

pub struct OpenAiCompat {
    base_url: String,
    api_key: Option<String>,
    http: reqwest::Client,
}

impl OpenAiCompat {
    pub fn new(base_url: String, api_key: Option<String>) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("http client");
        Self { base_url: base_url.trim_end_matches('/').to_string(), api_key, http }
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

pub(crate) fn request_body(req: &ChatRequest<'_>) -> Value {
    let mut body = json!({
        "model": req.model,
        "messages": req.messages.iter().map(message_to_json).collect::<Vec<_>>(),
        "stream": true,
        "temperature": req.temperature,
        "max_tokens": req.max_tokens,
    });
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
            let msg = err.get("message").and_then(Value::as_str).unwrap_or("unknown error");
            bail!("provider error: {msg}");
        }
        // OpenRouter sends token counts and the price with the last chunk.
        if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
            let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
            self.usage = Usage { prompt_tokens: n("prompt_tokens"), completion_tokens: n("completion_tokens"), cost: u.get("cost").and_then(Value::as_f64) };
        }
        let Some(delta) = v.pointer("/choices/0/delta") else { return Ok(()) };
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
        ChatResponse { text, tool_calls, usage: self.usage }
    }
}

#[async_trait]
impl Provider for OpenAiCompat {
    async fn chat(&self, req: ChatRequest<'_>, on_event: EventSink<'_>) -> anyhow::Result<ChatResponse> {
        let url = format!("{}/chat/completions", self.base_url);
        let mut builder = self
            .http
            .post(&url)
            .header("HTTP-Referer", "https://github.com/DimeDataCloud/Waddle")
            .header("X-Title", "Project Waddle")
            .json(&request_body(&req));
        if let Some(key) = self.api_key.as_deref().filter(|k| !k.is_empty()) {
            builder = builder.bearer_auth(key);
        }
        let resp = builder.send().await.with_context(|| format!("could not reach {url}"))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let detail = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v.pointer("/error/message").and_then(Value::as_str).map(str::to_string))
                .unwrap_or(body);
            return Err(anyhow!("{} from {}: {}", status, url, detail.chars().take(400).collect::<String>()));
        }
        let mut acc = SseAccumulator::default();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("stream interrupted")?;
            acc.push(&String::from_utf8_lossy(&chunk), on_event)?;
            if acc.done {
                break;
            }
        }
        Ok(acc.finish())
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
        let body = request_body(&ChatRequest { model: "m", messages: &msgs, tools: &tools, temperature: 0.2, max_tokens: 10 });
        assert_eq!(body["messages"][1]["tool_calls"][0]["function"]["arguments"], "{}");
        assert_eq!(body["messages"][2]["tool_call_id"], "c9");
        assert_eq!(body["messages"][3]["content"][1]["image_url"]["url"], "data:image/png;base64,AAA");
        assert_eq!(body["tools"][0]["function"]["name"], "x");
        assert_eq!(body["stream"], true);
    }
}
