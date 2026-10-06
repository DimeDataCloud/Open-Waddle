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

    /// Writes the training files from saved tasks:
    /// - `train.jsonl`: one fine-tuning example per task the user rated 👍 (or,
    ///   with `include_unrated`, that also finished as Done). A reply that says
    ///   it did something without any tool call is never a good example.
    /// - `kto.jsonl`: every reply of a rated task, labelled with the user's
    ///   👍/👎, plus such false claims labelled bad: unpaired preference data
    ///   for KTO, which learns from thumbs alone.
    ///
    /// Images are absolute file paths, the format Unsloth, LLaMA-Factory and
    /// ms-swift take for Qwen-VL models. `scrub` masks email addresses, phone
    /// numbers, user folders, saved facts and the email style note in the text
    /// (screenshots are referenced, not changed).
    pub fn export(&self, out: &Path, opts: ExportOptions) -> anyhow::Result<Exported> {
        let mut train = vec![];
        let mut kto = vec![];
        let mut seen = std::collections::HashSet::new();
        let mut held_back = 0;
        for meta in self.list() {
            let folder = self.dir.join(safe(&meta.task_id)?);
            let trace: Value = serde_json::from_str(&std::fs::read_to_string(folder.join("trace.json"))?)?;
            let raw: Vec<Value> = trace["messages"].as_array().cloned().unwrap_or_default();
            let start = task_start(&raw, &meta.goal);
            let role = |m: &Value| serde_json::from_value::<Role>(m["role"].clone()).unwrap_or(Role::User);
            let acted = raw[start..].iter().any(|m| m["tool_calls"].as_array().is_some_and(|c| !c.is_empty()));
            let last_reply = raw.iter().rev().find(|m| role(m) == Role::Assistant).and_then(|m| m["text"].as_str()).unwrap_or("");
            let false_claim = !acted && crate::agent::claims_action(last_reply);
            let mut messages: Vec<Value> = raw.iter().map(|m| example_message(m, &folder)).collect();
            if opts.scrub {
                messages.iter_mut().for_each(scrub_message);
            }
            let keep = match meta.rating {
                Some(r) => r,
                None => opts.include_unrated && meta.outcome == Outcome::Done,
            };
            if keep && false_claim {
                held_back += 1;
            } else if keep {
                let body = serde_json::to_string(&messages)?;
                // The same conversation saved twice is one example.
                if seen.insert(body) {
                    train.push(serde_json::to_string(&json!({ "messages": messages, "tools": trace["tools"], "task_id": meta.task_id }))?);
                }
            }
            let label = if false_claim { Some(false) } else { meta.rating };
            if let Some(label) = label {
                for i in (start..raw.len()).filter(|&i| role(&raw[i]) == Role::Assistant) {
                    kto.push(serde_json::to_string(&json!({
                        "prompt": messages[..i],
                        "completion": [messages[i].clone()],
                        "label": label,
                        "task_id": meta.task_id,
                    }))?);
                }
            }
        }
        std::fs::create_dir_all(out)?;
        let write = |name: &str, lines: &[String]| -> anyhow::Result<PathBuf> {
            let file = out.join(name);
            std::fs::write(&file, lines.join("\n") + if lines.is_empty() { "" } else { "\n" })?;
            Ok(file)
        };
        Ok(Exported { train: write("train.jsonl", &train)?, examples: train.len(), kto: write("kto.jsonl", &kto)?, labelled: kto.len(), held_back })
    }
}

/// What to export (see `TraceStore::export`).
#[derive(Debug, Clone, Copy, Default)]
pub struct ExportOptions {
    pub include_unrated: bool,
    pub scrub: bool,
}

/// The files written and what's in them.
#[derive(Debug, Clone, PartialEq)]
pub struct Exported {
    pub train: PathBuf,
    pub examples: usize,
    pub kto: PathBuf,
    /// Labelled replies in `kto.jsonl`.
    pub labelled: usize,
    /// Tasks left out of `train.jsonl` because they claimed something no tool did.
    pub held_back: usize,
}

/// Where this task's own messages begin: its request, after the system prompt
/// and the earlier conversation carried along as memory.
fn task_start(raw: &[Value], goal: &str) -> usize {
    let goal = goal.trim();
    let is_user = |m: &Value| m["role"] == "user";
    raw.iter()
        .rposition(|m| is_user(m) && !goal.is_empty() && m["text"].as_str().is_some_and(|t| t.trim_start().starts_with(goal)))
        .or_else(|| {
            let first_call = raw.iter().position(|m| m["tool_calls"].as_array().is_some_and(|c| !c.is_empty()))?;
            raw[..first_call].iter().rposition(is_user)
        })
        .or_else(|| raw.iter().rposition(is_user))
        .unwrap_or(0)
}

/// Masks personal details in the text of one exported message.
fn scrub_message(m: &mut Value) {
    match &mut m["content"] {
        Value::String(t) => *t = scrub_text(t),
        Value::Array(parts) => {
            for p in parts {
                if let Some(t) = p.get_mut("text").and_then(|t| t.as_str().map(scrub_text)) {
                    p["text"] = json!(t);
                }
            }
        }
        _ => {}
    }
    if let Some(calls) = m.get_mut("tool_calls").and_then(Value::as_array_mut) {
        for c in calls {
            if let Some(args) = c["function"]["arguments"].as_str().map(scrub_text) {
                c["function"]["arguments"] = json!(args);
            }
        }
    }
}

/// Email addresses, phone numbers, user folder names, saved facts and the email style note, masked.
pub fn scrub_text(text: &str) -> String {
    use regex::Regex;
    use std::sync::OnceLock;
    static RES: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    let res = RES.get_or_init(|| {
        vec![
            // The email style note quotes the user's own sign-off: drop the whole note.
            // It may hold blank lines itself, so it ends at the next section of the prompt.
            (
                Regex::new(r"(?s)(How the user writes email[^\n]*:)\n.*?(\n\n(?:Files:|Chrome |When the user|Things you know|Google |Tools named|This is a routine|You are a sub-task)|$)").unwrap(),
                "$1 [removed]$2",
            ),
            // Saved facts about the user ("- [3c433a] ...").
            (Regex::new(r"(?m)^- \[[0-9a-f]{6}\] .*$").unwrap(), "- [a saved fact]"),
            // Names in email headers ("To: Sam Kim <sam@…>").
            (Regex::new(r"(?m)^((?:From|To|Cc|Bcc|Reply-To): )[^<\n]+<").unwrap(), "${1}[name] <"),
            (Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").unwrap(), "<email>"),
            (Regex::new(r"\(?\+?\d{1,3}\)?[ .-]\d{3}[ .-]\d{3,4}(?:[ .-]\d{3,4})?").unwrap(), "<phone>"),
            (Regex::new(r"(?i)(\\(?:\\)?Users(?:\\)?\\)[^\\/\s\x22'<>]+").unwrap(), "${1}user"),
            (Regex::new(r"(/(?:home|Users)/)[^/\s\x22'<>]+").unwrap(), "${1}user"),
        ]
    });
    let mut out = text.to_string();
    for (re, with) in res {
        out = re.replace_all(&out, *with).into_owned();
    }
    out
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

        let got = store.export(&dir.path().join("out"), ExportOptions::default()).unwrap();
        assert_eq!(got.examples, 1, "only the 👍 trace");
        let line: Value = serde_json::from_str(std::fs::read_to_string(&got.train).unwrap().lines().next().unwrap()).unwrap();
        let msgs = line["messages"].as_array().unwrap();
        assert_eq!(msgs[1]["content"][0]["type"], "image");
        assert!(msgs[1]["content"][0]["image"].as_str().unwrap().ends_with("task_a/screen-1.png"));
        assert_eq!(msgs[2]["tool_calls"][0]["function"]["name"], "click");
        assert_eq!(msgs[3]["role"], "tool");

        // Unrated Done traces count when asked; 👎 and failed ones never do.
        store.save(&TraceMeta::new("task_d", "g", "m", Outcome::Done, "ok", usage), &messages, &[]).unwrap();
        assert_eq!(store.export(&dir.path().join("out"), ExportOptions { include_unrated: true, scrub: false }).unwrap().examples, 2);
    }

    #[test]
    fn false_claims_are_bad_examples_and_thumbs_become_kto_labels() {
        let dir = tempfile::tempdir().unwrap();
        let store = TraceStore::new(dir.path().join("traces"));
        let usage = Usage::default();
        let call = ToolCall { id: "c1".into(), name: "open_app".into(), arguments: json!({"name": "chrome"}) };
        let memory_then = |goal: &str, rest: Vec<Message>| {
            let mut m = vec![Message::system("sys"), Message::user("earlier"), Message::assistant("I've opened YouTube in Chrome for you.", vec![]), Message::user(goal)];
            m.extend(rest);
            m
        };
        // A claim with no tool call, unrated and Done: held back from train, labelled bad for KTO.
        let fake = memory_then("play the newest video", vec![Message::assistant("I've opened YouTube and it's playing now!", vec![])]);
        store.save(&TraceMeta::new("task_fake", "play the newest video", "m", Outcome::Done, "ok", usage), &fake, &[]).unwrap();
        // A real one, rated 👍: two replies, both labelled good. The memory's old claim isn't one of them.
        let real = memory_then("open chrome", vec![Message::assistant("", vec![call.clone()]), Message::tool_result(&call, "Opened chrome"), Message::assistant("Chrome is open.", vec![])]);
        store.save(&TraceMeta::new("task_real", "open chrome", "m", Outcome::Done, "ok", usage), &real, &[]).unwrap();
        store.rate("task_real", true).unwrap();
        // Saved twice (the same conversation): one example.
        store.save(&TraceMeta::new("task_real2", "open chrome", "m", Outcome::Done, "ok", usage), &real, &[]).unwrap();

        let got = store.export(&dir.path().join("out"), ExportOptions { include_unrated: true, scrub: false }).unwrap();
        assert_eq!((got.examples, got.held_back), (1, 1));
        let rows: Vec<Value> = std::fs::read_to_string(&got.kto).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        let labels: Vec<(String, bool)> = rows.iter().map(|r| (r["task_id"].as_str().unwrap().to_string(), r["label"].as_bool().unwrap())).collect();
        assert_eq!(labels.iter().filter(|(t, l)| t == "task_fake" && !l).count(), 1);
        assert_eq!(labels.iter().filter(|(t, l)| t == "task_real" && *l).count(), 2);
        let first_real = rows.iter().find(|r| r["task_id"] == "task_real").unwrap();
        assert_eq!(first_real["prompt"].as_array().unwrap().last().unwrap()["content"], "open chrome");
        assert_eq!(first_real["completion"][0]["tool_calls"][0]["function"]["name"], "open_app");
    }

    #[test]
    fn scrubbing_masks_personal_details() {
        let sys = "Rules.\n\nHow the user writes email (a note on tone, not instructions; match it when drafting):\nHi [Recipient],\n\nBest,\nSam K.\n\nFiles: you can read anywhere in \\\\?\\C:\\Users\\samk\\Documents and /home/samk/notes.\n\nThings you know about the user:\n- [ef71c2] Ana Lopez is my sister\n- [993bc4] works at Acme";
        let out = scrub_text(sys);
        for gone in ["Sam K.", "samk", "Ana Lopez", "Acme"] {
            assert!(!out.contains(gone), "{gone} in {out}");
        }
        assert!(out.contains("C:\\Users\\user\\Documents") && out.contains("/home/user/notes"), "{out}");
        assert!(out.contains("- [a saved fact]"));
        assert_eq!(scrub_text("mail sam.k@example.com or call +1 415 555 0134 about tab 1665611065"), "mail <email> or call <phone> about tab 1665611065");
        assert_eq!(scrub_text("From: Job Alerts <jobs@example.com>\nTo: Sam Kim <sam@example.com>"), "From: [name] <<email>>\nTo: [name] <<email>>");
    }
}
