//! Training data from real use (opt-in, off by default). Each finished task is
//! saved as a folder holding the whole conversation (`trace.json`) and its
//! screenshots as PNG files. The user can rate a task 👍/👎, and the export
//! turns good traces into chat-format examples for fine-tuning a model on
//! exactly the work Waddle does. Nothing leaves the computer.

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::agent::{Outcome, Timing};
use crate::llm::{Message, Role, ToolSpec, Usage};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceMeta {
    pub task_id: String,
    pub ts_ms: u64,
    pub goal: String,
    pub model: String,
    pub outcome: Outcome,
    pub message: String,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cost: Option<f64>,
    /// The user's verdict: Some(true) = 👍, Some(false) = 👎.
    pub rating: Option<bool>,
    /// Where the time went: the opening look, then each step's model call and tools.
    #[serde(default)]
    pub timing: Timing,
}

pub struct TraceStore {
    dir: PathBuf,
}

/// Task ids become folder names; keep them to safe characters.
fn safe(id: &str) -> anyhow::Result<&str> {
    anyhow::ensure!(!id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.'), "bad task id");
    anyhow::ensure!(!id.starts_with('.'), "bad task id");
    Ok(id)
}

impl TraceStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Saves one finished task. Screenshots are written next to the JSON and
    /// referenced by file name, so the JSON stays small and readable.
    pub fn save(&self, meta: &TraceMeta, messages: &[Message], tools: &[ToolSpec]) -> anyhow::Result<PathBuf> {
        let folder = self.dir.join(safe(&meta.task_id)?);
        std::fs::create_dir_all(&folder)?;
        let mut n = 0;
        let mut out = vec![];
        for m in messages {
            let mut images = vec![];
            for img in &m.images {
                n += 1;
                let name = format!("screen-{n}.png");
                std::fs::write(folder.join(&name), base64::engine::general_purpose::STANDARD.decode(&img.base64)?)?;
                images.push(name);
            }
            out.push(json!({
                "role": m.role,
                "text": m.text,
                "tool_calls": m.tool_calls,
                "tool_call_id": m.tool_call_id,
                "images": images,
            }));
        }
        let trace = json!({ "meta": meta, "tools": tools, "messages": out });
        std::fs::write(folder.join("trace.json"), serde_json::to_string_pretty(&trace)?)?;
        Ok(folder)
    }

    pub fn rate(&self, task_id: &str, good: bool) -> anyhow::Result<()> {
        let path = self.dir.join(safe(task_id)?).join("trace.json");
        let mut trace: Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
        trace["meta"]["rating"] = json!(good);
        std::fs::write(path, serde_json::to_string_pretty(&trace)?)?;
        Ok(())
    }

    pub fn list(&self) -> Vec<TraceMeta> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else { return vec![] };
        let mut out: Vec<TraceMeta> = entries
            .flatten()
            .filter_map(|e| std::fs::read_to_string(e.path().join("trace.json")).ok())
            .filter_map(|t| serde_json::from_str::<Value>(&t).ok())
            .filter_map(|v| serde_json::from_value(v["meta"].clone()).ok())
            .collect();
        out.sort_by_key(|m| std::cmp::Reverse(m.ts_ms));
        out
    }

    /// Writes `train.jsonl`: one fine-tuning example per task the user rated 👍
    /// (or, with `include_unrated`, that also finished as Done). Images are
    /// absolute file paths, the format Unsloth, LLaMA-Factory and ms-swift take
    /// for Qwen-VL models. Returns the file and how many examples it holds.
    pub fn export(&self, out: &Path, include_unrated: bool) -> anyhow::Result<(PathBuf, usize)> {
        let mut lines = vec![];
        for meta in self.list() {
            let keep = match meta.rating {
                Some(r) => r,
                None => include_unrated && meta.outcome == Outcome::Done,
            };
            if !keep {
                continue;
            }
            let folder = self.dir.join(safe(&meta.task_id)?);
            let trace: Value = serde_json::from_str(&std::fs::read_to_string(folder.join("trace.json"))?)?;
            let messages: Vec<Value> = trace["messages"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|m| example_message(m, &folder))
                .collect();
            lines.push(serde_json::to_string(&json!({ "messages": messages, "tools": trace["tools"], "task_id": meta.task_id }))?);
        }
        std::fs::create_dir_all(out)?;
        let file = out.join("train.jsonl");
        std::fs::write(&file, lines.join("\n") + if lines.is_empty() { "" } else { "\n" })?;
        Ok((file, lines.len()))
    }
}

/// One saved message → OpenAI-style chat message with image paths.
fn example_message(m: &Value, folder: &Path) -> Value {
    let role: Role = serde_json::from_value(m["role"].clone()).unwrap_or(Role::User);
    let text = m["text"].as_str().unwrap_or_default();
    let images: Vec<Value> = m["images"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|f| json!({ "type": "image", "image": folder.join(f).display().to_string() }))
        .collect();
    match role {
        Role::Assistant => {
            let calls: Vec<Value> = m["tool_calls"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|c| json!({ "type": "function", "id": c["id"], "function": { "name": c["name"], "arguments": c["arguments"].to_string() } }))
                .collect();
            let mut v = json!({ "role": "assistant", "content": text });
            if !calls.is_empty() {
                v["tool_calls"] = json!(calls);
            }
            v
        }
        Role::Tool => json!({ "role": "tool", "tool_call_id": m["tool_call_id"], "content": text }),
        Role::System => json!({ "role": "system", "content": text }),
        Role::User if images.is_empty() => json!({ "role": "user", "content": text }),
        Role::User => {
            let mut parts = images;
            parts.push(json!({ "type": "text", "text": text }));
            json!({ "role": "user", "content": parts })
        }
    }
}

impl TraceMeta {
    pub fn new(task_id: &str, goal: &str, model: &str, outcome: Outcome, message: &str, usage: Usage) -> Self {
        let ts_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        Self {
            task_id: task_id.into(),
            ts_ms,
            goal: goal.into(),
            model: model.into(),
            outcome,
            message: message.into(),
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            cost: usage.cost,
            rating: None,
            timing: Timing::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{ImageData, ToolCall};

    #[test]
    fn saves_rates_and_exports_good_traces() {
        let dir = tempfile::tempdir().unwrap();
        let store = TraceStore::new(dir.path().join("traces"));
        let png = base64::engine::general_purpose::STANDARD.encode([137u8, 80, 78, 71]);
        let call = ToolCall { id: "c1".into(), name: "click".into(), arguments: json!({"x": 1, "y": 2}) };
        let messages = vec![
            Message::system("sys"),
            Message::user_with_image("play my playlist", ImageData { mime: "image/png".into(), base64: png }),
            Message::assistant("Clicking Play all.", vec![call.clone()]),
            Message::tool_result(&call, "Clicked."),
            Message::assistant("Playing!", vec![]),
        ];
        let usage = Usage { prompt_tokens: 10, completion_tokens: 2, cost: Some(0.001) };
        for (id, outcome) in [("task_a", Outcome::Done), ("task_b", Outcome::Done), ("task_c", Outcome::Failed)] {
            store.save(&TraceMeta::new(id, "play my playlist", "m", outcome, "ok", usage), &messages, &[]).unwrap();
        }
        assert!(dir.path().join("traces/task_a/screen-1.png").exists());
        store.rate("task_a", true).unwrap();
        store.rate("task_b", false).unwrap();
        assert!(store.rate("../evil", true).is_err());

        let (file, n) = store.export(&dir.path().join("out"), false).unwrap();
        assert_eq!(n, 1, "only the 👍 trace");
        let line: Value = serde_json::from_str(std::fs::read_to_string(&file).unwrap().lines().next().unwrap()).unwrap();
        let msgs = line["messages"].as_array().unwrap();
        assert_eq!(msgs[1]["content"][0]["type"], "image");
        assert!(msgs[1]["content"][0]["image"].as_str().unwrap().ends_with("task_a/screen-1.png"));
        assert_eq!(msgs[2]["tool_calls"][0]["function"]["name"], "click");
        assert_eq!(msgs[3]["role"], "tool");

        // Unrated Done traces count when asked; 👎 and failed ones never do.
        store.save(&TraceMeta::new("task_d", "g", "m", Outcome::Done, "ok", usage), &messages, &[]).unwrap();
        assert_eq!(store.export(&dir.path().join("out"), true).unwrap().1, 2);
    }
}
