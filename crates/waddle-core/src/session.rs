//! The real-time conversation layer. One task runs at a time, but the user can
//! keep talking: each message while busy is (1) answered instantly by the
//! quick-reply lane and (2) handed to the running planner as steering.
//! Halt phrases stop everything without waiting for any model.

use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio_util::sync::CancellationToken;

use crate::agent::{self, Agent, AgentDeps, AgentEvent, Host, Lane, Outcome, RunResult, TaskStatus};
use crate::audit::{AuditEntry, AuditLog};
use crate::config::Settings;
use crate::skills::SkillStore;
use crate::traces::{TraceMeta, TraceStore};
use crate::llm::{ChatRequest, Message, Provider, StreamEvent};
use crate::tools::fs::Workspace;

const MEMORY_MESSAGES: usize = 20;

struct Active {
    cancel: CancellationToken,
    steer: UnboundedSender<String>,
    status: Arc<Mutex<TaskStatus>>,
}

/// Everything a task needs that the user can change between tasks.
#[derive(Clone)]
pub struct SessionConfig {
    pub settings: Settings,
    pub provider: Arc<dyn Provider>,
    pub workspace: Arc<Workspace>,
    pub skills: Option<Arc<SkillStore>>,
    pub self_source: Option<Arc<Workspace>>,
    /// Where tasks are saved when `settings.record_traces` is on.
    pub traces: Option<Arc<TraceStore>>,
}

pub struct Session {
    /// Runtime for background work; callers may be on non-async threads (UI event handlers).
    rt: tokio::runtime::Handle,
    host: Arc<dyn Host>,
    audit: Arc<AuditLog>,
    config: RwLock<SessionConfig>,
    memory: Mutex<Vec<Message>>,
    active: Mutex<Option<Active>>,
}

/// True when the whole utterance is a request to stop ("stop", "wait!", "please cancel", "stop stop").
pub fn is_halt_phrase(text: &str) -> bool {
    const FILLER: &[&str] = &["please", "now", "ok", "okay", "hey", "waddle", "right", "it", "that", "everything", "the", "task"];
    const HALT: &[&str] = &["stop", "halt", "cancel", "abort", "wait", "pause", "freeze", "quit", "nevermind", "enough"];
    let norm: String = text
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c.is_whitespace() { c } else { ' ' })
        .collect();
    let words: Vec<&str> = norm.split_whitespace().filter(|w| !FILLER.contains(w)).collect();
    match words.as_slice() {
        [] => false,
        ["hold", "on"] | ["never", "mind"] => true,
        ws => ws.len() <= 2 && ws.iter().all(|w| HALT.contains(w)),
    }
}

fn new_task_id() -> String {
    let mut bytes = [0u8; 4];
    let _ = getrandom::fill(&mut bytes);
    format!("task_{}", hex::encode(bytes))
}

impl Session {
    pub fn new(rt: tokio::runtime::Handle, host: Arc<dyn Host>, audit: Arc<AuditLog>, config: SessionConfig) -> Arc<Self> {
        Arc::new(Self {
            rt,
            host,
            audit,
            config: RwLock::new(config),
            memory: Mutex::default(),
            active: Mutex::default(),
        })
    }

    /// Applies new settings. A running task keeps the configuration it started with.
    pub fn configure(&self, config: SessionConfig) {
        *self.config.write().unwrap() = config;
    }

    pub fn is_busy(&self) -> bool {
        self.active.lock().unwrap().is_some()
    }

    /// Stops the running task. Returns false when nothing was running.
    pub fn halt(&self) -> bool {
        match self.active.lock().unwrap().as_ref() {
            Some(a) => {
                a.cancel.cancel();
                true
            }
            None => false,
        }
    }

    pub fn clear_memory(&self) {
        self.memory.lock().unwrap().clear();
    }

    /// Called when the user starts talking. A local model loads and reads the
    /// system prompt, tools and conversation so far while they type, so the
    /// first step after Enter only has to read their message. Hosted APIs would
    /// bill for this, so they are left alone.
    pub fn warm(self: &Arc<Self>) {
        if self.is_busy() {
            return;
        }
        let config = self.config.read().unwrap().clone();
        if !config.settings.is_local() {
            return;
        }
        let this = self.clone();
        self.rt.spawn(async move {
            let deps = this.deps(config);
            let memory = this.memory.lock().unwrap().clone();
            let env = this.host.env();
            let agent = Agent::new(&deps, String::new(), CancellationToken::new(), Arc::default(), &env);
            let (messages, tools) = agent.opening(&memory);
            let req = ChatRequest { model: &deps.settings.model, messages: &messages, tools: &tools, temperature: 0.2, max_tokens: 1 };
            deps.provider.warm(req).await;
        });
    }

    fn deps(&self, config: SessionConfig) -> AgentDeps {
        AgentDeps {
            provider: config.provider,
            host: self.host.clone(),
            audit: self.audit.clone(),
            workspace: config.workspace,
            settings: config.settings,
            skills: config.skills,
            self_source: config.self_source,
        }
    }

    /// Entry point for everything the user says or types.
    pub fn user_message(self: &Arc<Self>, text: String) {
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        if is_halt_phrase(&text) {
            let notice = if self.halt() { "Stopping!" } else { "I'm not doing anything right now." };
            self.host.emit(AgentEvent::Notice { text: notice.into() });
            return;
        }
        let steered = {
            let active = self.active.lock().unwrap();
            match active.as_ref() {
                Some(a) if a.steer.send(text.clone()).is_ok() => Some(a.status.clone()),
                _ => None,
            }
        };
        match steered {
            Some(status) => self.spawn_quick_reply(text, status),
            None => self.start_task(text),
        }
    }

    fn start_task(self: &Arc<Self>, goal: String) {
        let config = self.config.read().unwrap().clone();
        let task_id = new_task_id();
        let cancel = CancellationToken::new();
        let (steer_tx, mut steer_rx) = mpsc::unbounded_channel();
        let status = Arc::new(Mutex::new(TaskStatus { goal: goal.clone(), ..Default::default() }));
        *self.active.lock().unwrap() = Some(Active { cancel: cancel.clone(), steer: steer_tx, status: status.clone() });

        let this = self.clone();
        self.rt.spawn(async move {
            let host = this.host.clone();
            host.set_busy(true);
            host.emit(AgentEvent::TaskStarted { task_id: task_id.clone(), goal: goal.clone() });
            let _ = this.audit.append(AuditEntry { task_id: task_id.clone(), kind: "task_start".into(), detail: Some(goal.clone()), ..Default::default() });

            let timeout = Duration::from_secs(config.settings.task_timeout_secs);
            let traces = config.traces.clone().filter(|_| config.settings.record_traces);
            let deps = this.deps(config);
            let memory = this.memory.lock().unwrap().clone();
            let env = host.env();
            let agent = Agent::new(&deps, task_id.clone(), cancel.clone(), status, &env);
            let result = match tokio::time::timeout(timeout, agent.run(&goal, &memory, &mut steer_rx)).await {
                Ok(r) => r,
                Err(_) => {
                    cancel.cancel();
                    RunResult { outcome: Outcome::TimedOut, message: "That took too long, so I stopped. Want me to try a different way?".into() }
                }
            };

            // Release the slot first so a message arriving now starts a fresh task.
            *this.active.lock().unwrap() = None;
            {
                let mut mem = this.memory.lock().unwrap();
                mem.extend(agent::memory_entries(&goal, &result));
                let excess = mem.len().saturating_sub(MEMORY_MESSAGES);
                mem.drain(..excess);
            }
            // Anything said after the last step boundary would otherwise be lost.
            let mut leftover = vec![];
            while let Ok(s) = steer_rx.try_recv() {
                leftover.push(s);
            }
            let usage = agent.usage();
            let detail = if usage.is_empty() { result.message.clone() } else { format!("{} ({usage})", result.message) };
            log::info!("task {task_id} {:?}: {usage}", result.outcome);
            let _ = this.audit.append(AuditEntry {
                task_id: task_id.clone(),
                kind: "task_end".into(),
                decision: Some(format!("{:?}", result.outcome).to_lowercase()),
                detail: Some(detail),
                ..Default::default()
            });
            host.set_busy(false);
            let saved = traces.and_then(|store| {
                let (messages, tools) = agent.transcript();
                let meta = TraceMeta::new(&task_id, &goal, &deps.settings.model, result.outcome, &result.message, usage);
                store.save(&meta, &messages, &tools).map_err(|e| log::warn!("saving the training trace failed: {e:#}")).ok()
            });
            host.emit(AgentEvent::TaskFinished { task_id: task_id.clone(), outcome: result.outcome, message: result.message });
            if saved.is_some() {
                host.emit(AgentEvent::TraceSaved { task_id });
            }
            if !leftover.is_empty() && result.outcome != Outcome::Halted {
                this.start_task(leftover.join("\n"));
            }
        });
    }

    /// System 1 lane: a fast, tool-free reply so the user is never left waiting.
    fn spawn_quick_reply(self: &Arc<Self>, text: String, status: Arc<Mutex<TaskStatus>>) {
        let (settings, provider) = {
            let c = self.config.read().unwrap();
            (c.settings.clone(), c.provider.clone())
        };
        let host = self.host.clone();
        self.rt.spawn(async move {
            let snapshot = status.lock().unwrap().clone();
            let system = format!(
                "You are Waddle, a friendly desktop duck. You are in the middle of a task and the user just spoke to you. \
Reply in one short sentence (two at most) that acknowledges what they said. Do not claim to have done anything new: \
a separate planner will act on their message at the next step. If they asked a question about your progress, answer from the status below.\n\n\
Current task: {goal}\nStep: {step}\nLast thing you said: {narration}\nCurrent action: {action}",
                goal = snapshot.goal,
                step = snapshot.step,
                narration = if snapshot.narration.is_empty() { "(nothing yet)" } else { &snapshot.narration },
                action = snapshot.current_action.as_deref().unwrap_or("thinking"),
            );
            let messages = [Message::system(system), Message::user(text)];
            let task_id = "quick".to_string();
            let mut on_event = |e: StreamEvent| {
                let StreamEvent::TextDelta(t) = e;
                host.emit(AgentEvent::TextDelta { task_id: task_id.clone(), lane: Lane::Quick, text: t });
            };
            let req = ChatRequest { model: settings.fast_model(), messages: &messages, tools: &[], temperature: 0.4, max_tokens: 120 };
            match provider.chat(req, &mut on_event).await {
                Ok(resp) if !resp.text.trim().is_empty() => status.lock().unwrap().acks.push(resp.text.trim().to_string()),
                Ok(_) => {}
                Err(e) => log::warn!("quick reply failed: {e:#}"),
            }
            host.emit(AgentEvent::TextDone { task_id, lane: Lane::Quick });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halt_phrases() {
        for yes in ["stop", "Stop!", "STOP STOP", "wait", "please stop", "hold on", "cancel that", "Waddle, stop now", "never mind", "abort"] {
            assert!(is_halt_phrase(yes), "{yes}");
        }
        for no in ["stop using notepad and use word", "wait, use the other file", "open the stopwatch", "hello", "", "please"] {
            assert!(!is_halt_phrase(no), "{no}");
        }
    }
}
