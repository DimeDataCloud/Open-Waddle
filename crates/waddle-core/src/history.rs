//! The conversation as the user saw it: their messages, Waddle's replies,
//! nudges and reminders. It's kept on disk for the history drawer and isn't
//! sent to any model (the model's memory is the session's, and shorter).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::agent::{AgentEvent, Lane, Outcome};

/// Entries kept.
pub const KEEP: usize = 50;
/// A longer entry is cut here (a pasted document, a runaway reply).
const MAX_CHARS: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Who {
    You,
    Waddle,
    Nudge,
    Reminder,
    /// A task that failed or was stopped, in its own words.
    Problem,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Increases with each entry, so the drawer can tell what's new.
    pub n: u64,
    pub at_ms: i64,
    pub who: Who,
    pub text: String,
    /// A research answer's id, for its "Full answer" button.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    /// The full research answer (Markdown), so the button still works after a restart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    entries: Vec<Entry>,
}

#[derive(Default)]
struct Inner {
    entries: Vec<Entry>,
    /// Text streaming in, by task and lane, until its line is done.
    pending: HashMap<(String, bool), String>,
}

pub struct History {
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
}

fn cut(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(MAX_CHARS) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text.to_string(),
    }
}

impl History {
    /// Loads what's in `path` (None: kept in memory only, for tests).
    pub fn new(path: Option<PathBuf>) -> Self {
        let saved: Saved = path.as_deref().map(crate::store::load_json).unwrap_or_default();
        Self { path, inner: Mutex::new(Inner { entries: saved.entries, pending: HashMap::new() }) }
    }

    fn save(&self, entries: &[Entry]) {
        let Some(path) = &self.path else { return };
        let saved = Saved { entries: entries.to_vec() };
        match serde_json::to_string(&saved) {
            Ok(json) => {
                if let Err(e) = crate::store::write_atomic(path, json) {
                    log::warn!("couldn't save the conversation history: {e}");
                }
            }
            Err(e) => log::warn!("couldn't save the conversation history: {e}"),
        }
    }

    fn push(&self, inner: &mut Inner, who: Who, text: &str) -> bool {
        let text = cut(text);
        if text.is_empty() {
            return false;
        }
        let n = inner.entries.last().map_or(1, |e| e.n + 1);
        inner.entries.push(Entry { n, at_ms: chrono::Utc::now().timestamp_millis(), who, text, answer: None, full: None });
        let excess = inner.entries.len().saturating_sub(KEEP);
        inner.entries.drain(..excess);
        self.save(&inner.entries);
        true
    }

    /// Adds an entry. Returns false when there was nothing to add.
    pub fn add(&self, who: Who, text: &str) -> bool {
        let mut inner = self.inner.lock().unwrap();
        self.push(&mut inner, who, text)
    }

    /// Follows the agent's events: each finished line of a reply becomes an
    /// entry, and so does a task that ends badly. Returns true when something was added.
    pub fn observe(&self, event: &AgentEvent) -> bool {
        let mut inner = self.inner.lock().unwrap();
        match event {
            AgentEvent::TextDelta { task_id, lane, text } => {
                inner.pending.entry((task_id.clone(), *lane == Lane::Quick)).or_default().push_str(text);
                false
            }
            AgentEvent::TextDone { task_id, lane } => match inner.pending.remove(&(task_id.clone(), *lane == Lane::Quick)) {
                Some(text) => self.push(&mut inner, Who::Waddle, &text),
                None => false,
            },
            AgentEvent::TaskFinished { task_id, outcome, message } => {
                // A line cut off by the end of the task still counts.
                let open: Vec<_> = inner.pending.keys().filter(|(t, _)| t == task_id).cloned().collect();
                let mut added = false;
                for key in open {
                    if let Some(text) = inner.pending.remove(&key) {
                        added |= self.push(&mut inner, Who::Waddle, &text);
                    }
                }
                if *outcome != Outcome::Done {
                    added |= self.push(&mut inner, Who::Problem, message);
                }
                added
            }
            _ => false,
        }
    }

    /// Puts a research answer behind the latest reply.
    pub fn attach_answer(&self, id: &str, markdown: &str) -> bool {
        let mut inner = self.inner.lock().unwrap();
        let Some(last) = inner.entries.iter_mut().rev().find(|e| e.who == Who::Waddle) else { return false };
        last.answer = Some(id.to_string());
        last.full = Some(markdown.to_string());
        let entries = inner.entries.clone();
        self.save(&entries);
        true
    }

    /// The full research answer with this id, if it's still in the history.
    pub fn answer(&self, id: &str) -> Option<String> {
        self.inner.lock().unwrap().entries.iter().rev().find(|e| e.answer.as_deref() == Some(id)).and_then(|e| e.full.clone())
    }

    /// Everything kept, oldest first, without the full research answers.
    pub fn list(&self) -> Vec<Entry> {
        self.inner.lock().unwrap().entries.iter().map(|e| Entry { full: None, ..e.clone() }).collect()
    }

    /// What the user said, oldest first (for ↑ in the chat box).
    pub fn said(&self) -> Vec<String> {
        self.inner.lock().unwrap().entries.iter().filter(|e| e.who == Who::You).map(|e| e.text.clone()).collect()
    }

    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.entries.clear();
        inner.pending.clear();
        if let Some(path) = &self.path {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delta(task: &str, lane: Lane, text: &str) -> AgentEvent {
        AgentEvent::TextDelta { task_id: task.into(), lane, text: text.into() }
    }

    #[test]
    fn replies_are_kept_line_by_line_and_survive_a_restart() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("history.json");
        let h = History::new(Some(path.clone()));
        assert!(h.add(Who::You, "  what's the weather?  "));
        assert!(!h.add(Who::You, "   "));
        assert!(!h.observe(&delta("t1", Lane::Planner, "Sunny, ")));
        // A quick reply in between doesn't mix into the planner's line.
        h.observe(&delta("t1", Lane::Quick, "On it!"));
        h.observe(&delta("t1", Lane::Planner, "about 20°C."));
        assert!(h.observe(&AgentEvent::TextDone { task_id: "t1".into(), lane: Lane::Quick }));
        assert!(h.observe(&AgentEvent::TextDone { task_id: "t1".into(), lane: Lane::Planner }));
        assert!(!h.observe(&AgentEvent::TaskFinished { task_id: "t1".into(), outcome: Outcome::Done, message: "done".into() }));

        let again = History::new(Some(path));
        let list = again.list();
        let texts: Vec<_> = list.iter().map(|e| (e.who, e.text.as_str())).collect();
        assert_eq!(texts, vec![(Who::You, "what's the weather?"), (Who::Waddle, "On it!"), (Who::Waddle, "Sunny, about 20°C.")]);
        assert_eq!(list.iter().map(|e| e.n).collect::<Vec<_>>(), vec![1, 2, 3]);
        assert_eq!(again.said(), vec!["what's the weather?".to_string()]);
    }

    #[test]
    fn a_task_that_ends_badly_is_kept_with_its_unfinished_line() {
        let h = History::new(None);
        h.observe(&delta("t2", Lane::Planner, "Opening Chrome"));
        assert!(h.observe(&AgentEvent::TaskFinished { task_id: "t2".into(), outcome: Outcome::Halted, message: "Stopped.".into() }));
        let texts: Vec<_> = h.list().into_iter().map(|e| (e.who, e.text)).collect();
        assert_eq!(texts, vec![(Who::Waddle, "Opening Chrome".to_string()), (Who::Problem, "Stopped.".to_string())]);
    }

    #[test]
    fn keeps_the_last_fifty_and_research_answers_ride_along() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("history.json");
        let h = History::new(Some(path.clone()));
        for i in 0..60 {
            h.add(Who::You, &format!("message {i}"));
        }
        let list = h.list();
        assert_eq!(list.len(), KEEP);
        assert_eq!(list[0].text, "message 10");
        assert_eq!(list.last().unwrap().n, 60);

        assert!(!History::new(None).attach_answer("a1", "# nothing to attach to"));
        h.add(Who::Waddle, "Short summary [bbc.co.uk].");
        assert!(h.attach_answer("answer_1", "# Full\n\nLonger."));
        let again = History::new(Some(path.clone()));
        assert_eq!(again.answer("answer_1").as_deref(), Some("# Full\n\nLonger."));
        // The list leaves the long text out; the id stays for the button.
        let last = again.list().pop().unwrap();
        assert_eq!((last.answer.as_deref(), last.full), (Some("answer_1"), None));

        again.clear();
        assert!(again.list().is_empty() && !path.exists());
        assert!(History::new(Some(path)).list().is_empty());
    }

    #[test]
    fn very_long_text_is_cut() {
        let h = History::new(None);
        h.add(Who::You, &"é".repeat(MAX_CHARS + 10));
        let text = &h.list()[0].text;
        assert_eq!(text.chars().count(), MAX_CHARS + 1);
        assert!(text.ends_with('…'));
    }
}
