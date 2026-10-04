//! Native Ollama chat API (NDJSON streaming). Used for local mode because it
//! exposes thread count and keep-alive, which keep the machine usable while a
//! model runs on a CPU-only laptop.

use anyhow::{anyhow, Context};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};

use super::{textcalls, ChatRequest, ChatResponse, EventSink, Message, Provider, Role, StreamEvent, ToolCall};
use crate::config::OllamaSettings;

pub struct Ollama {
    base_url: String,
    opts: OllamaSettings,
    http: reqwest::Client,
}

impl Ollama {
    pub fn new(base_url: String, opts: OllamaSettings) -> Self {
        let base = base_url.trim_end_matches('/').trim_end_matches("/v1").to_string();
        Self { base_url: base, opts, http: reqwest::Client::new() }
    }

    /// A third of the logical cores (at least 2), so inference never starves the user's other work.
    pub fn default_threads() -> u32 {
        let cores = std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(4);
        (cores / 3).max(2)
    }
}

fn message_to_json(m: &Message) -> Value {
    let role = match m.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };
    let mut v = json!({ "role": role, "content": m.text });
    if !m.images.is_empty() {
        v["images"] = m.images.iter().map(|i| Value::String(i.base64.clone())).collect();
    }
    if !m.tool_calls.is_empty() {
        v["tool_calls"] = m
            .tool_calls
            .iter()
            .map(|c| json!({ "function": { "name": c.name, "arguments": c.arguments } }))
            .collect();
    }
    if let Some(name) = &m.tool_name {
        v["tool_name"] = Value::String(name.clone());
    }
    v
}

pub(crate) fn request_body(req: &ChatRequest<'_>, opts: &OllamaSettings) -> Value {
    let mut body = json!({
        "model": req.model,
        "messages": req.messages.iter().map(message_to_json).collect::<Vec<_>>(),
        "stream": true,
        // Hidden reasoning costs hundreds of tokens per step, which is tens of
        // seconds on a laptop CPU. Waddle narrates instead.
        "think": false,
        "keep_alive": opts.keep_alive,
        "options": {
            "temperature": req.temperature,
            "num_predict": req.max_tokens,
            "num_ctx": opts.num_ctx,
            "num_thread": opts.num_thread.unwrap_or_else(Ollama::default_threads),
        }
    });
    if !req.tools.is_empty() {
        body["tools"] = req
            .tools
            .iter()
            .map(|t| json!({ "type": "function", "function": { "name": t.name, "description": t.description, "parameters": t.parameters } }))
            .collect();
    }
    body
}

/// Applies one NDJSON line; returns true when the stream reports `done`.
pub(crate) fn apply_line(
    line: &str,
    text: &mut String,
    calls: &mut Vec<ToolCall>,
    on_event: &mut (dyn FnMut(StreamEvent) + Send),
) -> anyhow::Result<bool> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(false);
    }
    let v: Value = serde_json::from_str(line).with_context(|| format!("bad stream chunk: {line}"))?;
    if let Some(err) = v.get("error").and_then(Value::as_str) {
        return Err(anyhow!("ollama error: {err}"));
    }
    if let Some(t) = v.pointer("/message/content").and_then(Value::as_str) {
        if !t.is_empty() {
            text.push_str(t);
            on_event(StreamEvent::TextDelta(t.to_string()));
        }
    }
    if let Some(tc) = v.pointer("/message/tool_calls").and_then(Value::as_array) {
        for c in tc {
            let Some(name) = c.pointer("/function/name").and_then(Value::as_str) else { continue };
            let arguments = match c.pointer("/function/arguments") {
                Some(Value::String(s)) => super::parse_arguments(s),
                Some(other) => other.clone(),
                None => json!({}),
            };
            calls.push(ToolCall { id: super::new_call_id(), name: name.to_string(), arguments });
        }
    }
    Ok(v.get("done").and_then(Value::as_bool).unwrap_or(false))
}

#[async_trait]
impl Provider for Ollama {
    async fn chat(&self, req: ChatRequest<'_>, on_event: EventSink<'_>) -> anyhow::Result<ChatResponse> {
        let url = format!("{}/api/chat", self.base_url);
        let resp = self
            .http
            .post(&url)
            .json(&request_body(&req, &self.opts))
            .send()
            .await
            .with_context(|| format!("could not reach Ollama at {url}. Is it running?"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("{status} from Ollama: {}", body.chars().take(400).collect::<String>()));
        }
        let mut text = String::new();
        let mut calls = vec![];
        let mut buffer = String::new();
        let mut stream = resp.bytes_stream();
        'outer: while let Some(chunk) = stream.next().await {
            buffer.push_str(&String::from_utf8_lossy(&chunk.context("stream interrupted")?));
            while let Some(pos) = buffer.find('\n') {
                let line: String = buffer.drain(..=pos).collect();
                if apply_line(&line, &mut text, &mut calls, on_event)? {
                    break 'outer;
                }
            }
        }
        if !buffer.trim().is_empty() {
            apply_line(&buffer, &mut text, &mut calls, on_event)?;
        }
        if calls.is_empty() {
            let (cleaned, extracted) = textcalls::extract(&text);
            if !extracted.is_empty() {
                return Ok(ChatResponse { text: cleaned, tool_calls: extracted });
            }
        }
        Ok(ChatResponse { text, tool_calls: calls })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ndjson_stream_with_tool_call() {
        let lines = [
            r#"{"message":{"role":"assistant","content":"Writing "},"done":false}"#,
            r#"{"message":{"role":"assistant","content":"it.","tool_calls":[{"function":{"name":"write_file","arguments":{"path":"a.txt","content":"hi"}}}]},"done":false}"#,
            r#"{"message":{"role":"assistant","content":""},"done":true}"#,
        ];
        let (mut text, mut calls, mut deltas) = (String::new(), vec![], vec![]);
        let mut sink = |e: StreamEvent| {
            let StreamEvent::TextDelta(t) = e;
            deltas.push(t);
        };
        let mut done = false;
        for l in lines {
            done = apply_line(l, &mut text, &mut calls, &mut sink).unwrap();
        }
        assert!(done);
        assert_eq!(text, "Writing it.");
        assert_eq!(deltas.len(), 2);
        assert_eq!(calls[0].name, "write_file");
        assert_eq!(calls[0].arguments["path"], "a.txt");
    }

    #[test]
    fn body_carries_resource_caps() {
        let opts = OllamaSettings { num_thread: Some(3), keep_alive: "30s".into(), num_ctx: 4096 };
        let msgs = vec![Message::user("hi")];
        let body = request_body(&ChatRequest { model: "qwen3.5:4b", messages: &msgs, tools: &[], temperature: 0.2, max_tokens: 64 }, &opts);
        assert_eq!(body["think"], false);
        assert_eq!(body["options"]["num_thread"], 3);
        assert_eq!(body["options"]["num_ctx"], 4096);
        assert_eq!(body["keep_alive"], "30s");
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn default_threads_leaves_headroom() {
        let t = Ollama::default_threads();
        let cores = std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(4);
        assert!(t >= 2 && (t <= cores || cores < 2));
    }
}
