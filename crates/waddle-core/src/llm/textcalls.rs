//! Fallback for models that print tool calls as text instead of using the
//! structured field, e.g. `<tool_call>{"name": "...", "arguments": {...}}</tool_call>`.

use serde_json::Value;

use super::ToolCall;

pub(crate) fn extract(text: &str) -> (String, Vec<ToolCall>) {
    const OPEN: &str = "<tool_call>";
    const CLOSE: &str = "</tool_call>";
    let mut calls = vec![];
    let mut cleaned = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        let after = &rest[start + OPEN.len()..];
        let Some(end) = after.find(CLOSE) else { break };
        cleaned.push_str(&rest[..start]);
        if let Some(call) = parse_one(after[..end].trim()) {
            calls.push(call);
        }
        rest = &after[end + CLOSE.len()..];
    }
    cleaned.push_str(rest);
    (cleaned.trim().to_string(), calls)
}

fn parse_one(body: &str) -> Option<ToolCall> {
    let v: Value = serde_json::from_str(body).ok()?;
    let name = v.get("name")?.as_str()?.to_string();
    let arguments = match v.get("arguments").or_else(|| v.get("parameters")) {
        Some(Value::String(s)) => super::parse_arguments(s),
        Some(other) => other.clone(),
        None => Value::Object(Default::default()),
    };
    Some(ToolCall { id: super::new_call_id(), name, arguments, echo: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_qwen_style_calls() {
        let (text, calls) =
            extract("Let me look.\n<tool_call>\n{\"name\": \"look_at_screen\", \"arguments\": {}}\n</tool_call>");
        assert_eq!(text, "Let me look.");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "look_at_screen");
    }

    #[test]
    fn leaves_plain_text_alone() {
        let (text, calls) = extract("All done!");
        assert_eq!(text, "All done!");
        assert!(calls.is_empty());
    }
}
