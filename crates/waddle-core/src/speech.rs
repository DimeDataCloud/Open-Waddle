//! What Waddle says out loud, and when it keeps quiet. The app does the
//! speaking (per OS); this decides which words, from the same events the
//! bubble shows: answers are read, step-by-step narration isn't.

use std::collections::HashMap;

use crate::agent::{AgentEvent, Lane, Outcome};
use crate::config::VoiceOutSettings;

/// Longer text is cut at the end of a sentence near here; the rest stays in the bubble and history.
const MAX_SPOKEN: usize = 480;

/// Turns a reply into something a voice can read: no Markdown, emoji, web
/// addresses or bracketed citations, and not too long.
pub fn speakable(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_code = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code || t.is_empty() {
            continue;
        }
        // Headings and list markers go; each line ends as a sentence.
        let t = t.trim_start_matches('#').trim_start();
        let t = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ ")).unwrap_or(t);
        let t = match t.split_once(". ") {
            Some((n, rest)) if !n.is_empty() && n.len() <= 3 && n.chars().all(|c| c.is_ascii_digit()) => rest,
            _ => t,
        };
        if !out.is_empty() {
            out.push_str(if out.ends_with(['.', '!', '?', ':', ';']) { " " } else { ". " });
        }
        out.push_str(t);
    }
    let mut s = String::with_capacity(out.len());
    let chars: Vec<char> = out.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // [label](url) → label; [bbc.co.uk] citations → nothing.
        if c == '[' {
            if let Some(close) = chars[i..].iter().position(|&x| x == ']').map(|p| i + p) {
                let label: String = chars[i + 1..close].iter().collect();
                if chars.get(close + 1) == Some(&'(') {
                    if let Some(end) = chars[close..].iter().position(|&x| x == ')').map(|p| close + p) {
                        s.push_str(&label);
                        i = end + 1;
                        continue;
                    }
                }
                if label.len() <= 40 && !label.contains(' ') {
                    // Drop the space before a citation too.
                    while s.ends_with(' ') {
                        s.pop();
                    }
                    i = close + 1;
                    continue;
                }
            }
        }
        if c == '_' {
            s.push(' ');
            i += 1;
            continue;
        }
        if matches!(c, '*' | '`' | '~' | '|' | '>') || is_emoji(c) {
            i += 1;
            continue;
        }
        s.push(c);
        i += 1;
    }
    // Web addresses are read as "a link".
    let words: Vec<String> = s
        .split_whitespace()
        .map(|w| if w.starts_with("http://") || w.starts_with("https://") || w.starts_with("www.") { "a link".to_string() } else { w.to_string() })
        .collect();
    let s = words.join(" ");
    cut_at_sentence(&s, MAX_SPOKEN)
}

fn is_emoji(c: char) -> bool {
    matches!(c as u32,
        0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2B00..=0x2BFF | 0xFE0F | 0x200D | 0x2300..=0x23FF)
}

fn cut_at_sentence(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    match head.rfind(['.', '!', '?']) {
        Some(i) if i > max / 3 => head[..=i].to_string(),
        _ => match head.rfind(' ') {
            Some(i) => format!("{}…", &head[..i]),
            None => head,
        },
    }
}

/// Why Waddle is keeping quiet right now, if it is.
pub fn quiet_reason(fullscreen: bool, in_meeting: bool) -> Option<&'static str> {
    if in_meeting {
        Some("in a meeting")
    } else if fullscreen {
        Some("full screen")
    } else {
        None
    }
}

/// Picks the words to say from the agent's events.
#[derive(Default)]
pub struct Picker {
    /// Text streaming in, by task and lane.
    pending: HashMap<(String, bool), String>,
    /// A task's latest finished line: read out only if it turns out to be the last.
    last_line: HashMap<String, String>,
}

impl Picker {
    /// Follows one event. Returns what to read out now, if anything.
    pub fn observe(&mut self, event: &AgentEvent, want: &VoiceOutSettings) -> Option<String> {
        let say = |text: &str| -> Option<String> {
            if !(want.enabled && want.replies) {
                return None;
            }
            Some(speakable(text)).filter(|s| !s.is_empty())
        };
        match event {
            AgentEvent::TaskStarted { task_id, .. } => {
                self.last_line.remove(task_id);
                None
            }
            AgentEvent::TextDelta { task_id, lane, text } => {
                self.pending.entry((task_id.clone(), *lane == Lane::Quick)).or_default().push_str(text);
                None
            }
            AgentEvent::TextDone { task_id, lane } => {
                let text = self.pending.remove(&(task_id.clone(), *lane == Lane::Quick))?;
                // Chat and research answers, and quick replies mid-task, are complete answers.
                if *lane == Lane::Quick || task_id == "chat" || task_id == "research" {
                    return say(&text);
                }
                // A task's narration: only its last line is read, when it finishes.
                self.last_line.insert(task_id.clone(), text);
                None
            }
            AgentEvent::TaskFinished { task_id, outcome, message } => {
                let open: Vec<_> = self.pending.keys().filter(|(t, _)| t == task_id).cloned().collect();
                let mut last = self.last_line.remove(task_id);
                for key in open {
                    if let Some(text) = self.pending.remove(&key) {
                        last = Some(text);
                    }
                }
                match outcome {
                    Outcome::Done => last.and_then(|t| say(&t)),
                    // The user stopped it: they know.
                    Outcome::Halted => None,
                    _ => say(message),
                }
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> VoiceOutSettings {
        VoiceOutSettings { enabled: true, ..Default::default() }
    }

    fn delta(task: &str, lane: Lane, text: &str) -> AgentEvent {
        AgentEvent::TextDelta { task_id: task.into(), lane, text: text.into() }
    }

    fn done(task: &str, lane: Lane) -> AgentEvent {
        AgentEvent::TextDone { task_id: task.into(), lane }
    }

    #[test]
    fn replies_lose_their_markup() {
        assert_eq!(speakable("**Done!** I saved `notes.md` 🎉"), "Done! I saved notes.md");
        assert_eq!(
            speakable("Heat pumps cost less [bbc.co.uk]. See [the guide](https://x.example) or https://y.example/z"),
            "Heat pumps cost less. See the guide or a link"
        );
        assert_eq!(speakable("## Tips\n- Short\n- Clear\n\n1. First\n```\ncode\n```\nThat's it."), "Tips. Short. Clear. First. That's it.");
        assert_eq!(speakable("Keep [some words here] as said."), "Keep [some words here] as said.");
        assert_eq!(speakable("snake_case and 2 * 3"), "snake case and 2 3");
    }

    #[test]
    fn long_replies_stop_at_a_sentence() {
        let long = "This is a sentence of a fair length. ".repeat(30);
        let s = speakable(&long);
        assert!(s.len() <= MAX_SPOKEN && s.ends_with('.'), "{s}");
        let one = "word ".repeat(200);
        assert!(speakable(&one).ends_with('…'));
    }

    #[test]
    fn answers_are_read_and_narration_only_at_the_end() {
        let mut p = Picker::default();
        let want = on();
        p.observe(&delta("chat", Lane::Planner, "Hello **there**!"), &want);
        assert_eq!(p.observe(&done("chat", Lane::Planner), &want).as_deref(), Some("Hello there!"));

        p.observe(&AgentEvent::TaskStarted { task_id: "t1".into(), goal: "g".into() }, &want);
        p.observe(&delta("t1", Lane::Planner, "Opening Notepad."), &want);
        assert_eq!(p.observe(&done("t1", Lane::Planner), &want), None, "narration waits");
        // A quick reply mid-task is an answer.
        p.observe(&delta("t1", Lane::Quick, "Sure, after this step."), &want);
        assert_eq!(p.observe(&done("t1", Lane::Quick), &want).as_deref(), Some("Sure, after this step."));
        p.observe(&delta("t1", Lane::Planner, "Typed it."), &want);
        p.observe(&done("t1", Lane::Planner), &want);
        let end = AgentEvent::TaskFinished { task_id: "t1".into(), outcome: Outcome::Done, message: "done".into() };
        assert_eq!(p.observe(&end, &want).as_deref(), Some("Typed it."), "only the last line");
    }

    #[test]
    fn failures_are_read_but_not_a_stop_and_nothing_when_off() {
        let mut p = Picker::default();
        let want = on();
        let failed = AgentEvent::TaskFinished { task_id: "t".into(), outcome: Outcome::Failed, message: "I couldn't open it.".into() };
        assert_eq!(p.observe(&failed, &want).as_deref(), Some("I couldn't open it."));
        p.observe(&delta("t2", Lane::Planner, "Working"), &want);
        let halted = AgentEvent::TaskFinished { task_id: "t2".into(), outcome: Outcome::Halted, message: "Stopped.".into() };
        assert_eq!(p.observe(&halted, &want), None);

        for want in [VoiceOutSettings::default(), VoiceOutSettings { enabled: true, replies: false, ..Default::default() }] {
            let mut p = Picker::default();
            p.observe(&delta("chat", Lane::Planner, "Hi"), &want);
            assert_eq!(p.observe(&done("chat", Lane::Planner), &want), None);
        }
    }

    #[test]
    fn keeps_quiet_in_meetings_and_full_screen() {
        assert_eq!(quiet_reason(false, false), None);
        assert_eq!(quiet_reason(true, false), Some("full screen"));
        assert_eq!(quiet_reason(true, true), Some("in a meeting"));
    }
}
