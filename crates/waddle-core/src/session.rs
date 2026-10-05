//! The real-time conversation layer. One task runs at a time, but the user can
//! keep talking: each message while busy is (1) answered instantly by the
//! quick-reply lane and (2) handed to the running planner as steering.
//! Halt phrases stop everything without waiting for any model.

use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::agent::{self, Agent, AgentDeps, AgentEvent, ApprovalRequest, Decision, EnvInfo, Host, Lane, Outcome, RunResult, TaskStatus};
use crate::audit::{AuditEntry, AuditLog};
use crate::config::Settings;
use crate::decide::{Decider, Route};
use crate::facts::FactStore;
use crate::reminders::ReminderStore;
use crate::skills::SkillStore;
use crate::traces::{TraceMeta, TraceStore};
use crate::llm::{ChatRequest, Citation, Message, Provider, StreamEvent};
use crate::tools::fs::Workspace;
use crate::tools::GuiAction;
use crate::tools::GuiResult;
use crate::untrusted;

const MEMORY_MESSAGES: usize = 20;
/// The conversation kept on disk is forgotten after this long without a message.
const MEMORY_STALE_MS: i64 = 12 * 3600 * 1000;

/// The conversation as saved between runs.
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct SavedMemory {
    saved_ms: i64,
    messages: Vec<Message>,
}
/// Conversation goes straight to the fast model when the router is at least this sure.
/// On 45 labelled messages, every chat scored 0.82 or more; the one task that
/// looked like chat ("make this sound more polite") scored 0.76.
const CHAT_ABOVE: f64 = 0.8;
/// Research questions scored 0.89 or more, and nothing else came close.
const RESEARCH_ABOVE: f64 = 0.7;
/// A chat reply searches the web first when the router thinks it needs current facts.
/// Live questions (news, weather, prices) scored 0.97 or more, timeless ones 0.08 or less.
const WEB_ABOVE: f64 = 0.6;
/// Search results for a chat reply and for a research answer.
const CHAT_RESULTS: u8 = 3;
const RESEARCH_RESULTS: u8 = 5;
/// The chat lane's reply when the message turns out to need the computer after all.
const HAND_OFF: &str = "[task]";
/// Research answers kept for their "Full answer" button.
const KEEP_ANSWERS: usize = 5;

struct Active {
    task_id: String,
    cancel: CancellationToken,
    steer: UnboundedSender<String>,
    status: Arc<Mutex<TaskStatus>>,
}

/// Text the user had selected when they called Waddle up (Ctrl+Alt+A).
#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    pub text: String,
    /// The app it was selected in, for the prompt.
    pub app: String,
}

/// The router is deciding what a message is. Meanwhile the task it may become
/// is already looking at the screen, held back from acting.
struct Deciding {
    queue: Vec<String>,
    held: Arc<Held>,
    task: Active,
}

/// A finished research answer, saved as Markdown when the user asks for it.
struct Answer {
    id: String,
    question: String,
    markdown: String,
    /// Where it was saved, so asking for it again reopens the same file.
    saved: Option<PathBuf>,
}

/// Everything a task needs that the user can change between tasks.
#[derive(Clone)]
pub struct SessionConfig {
    pub settings: Settings,
    pub provider: Arc<dyn Provider>,
    pub workspace: Arc<Workspace>,
    pub skills: Option<Arc<SkillStore>>,
    pub reminders: Option<Arc<ReminderStore>>,
    pub facts: Option<Arc<FactStore>>,
    pub decider: Option<Arc<dyn Decider>>,
    pub self_source: Option<Arc<Workspace>>,
    /// Where tasks are saved when `settings.record_traces` is on.
    pub traces: Option<Arc<TraceStore>>,
    /// The signed-in Google account. None = not connected.
    pub google: Option<Arc<crate::google::Google>>,
    pub style: Option<Arc<crate::google::style::StyleNote>>,
}

pub struct Session {
    /// Runtime for background work; callers may be on non-async threads (UI event handlers).
    rt: tokio::runtime::Handle,
    host: Arc<dyn Host>,
    audit: Arc<AuditLog>,
    config: RwLock<SessionConfig>,
    memory: Mutex<Vec<Message>>,
    /// Where the conversation is kept between runs, if anywhere.
    memory_file: Mutex<Option<PathBuf>>,
    active: Mutex<Option<Active>>,
    /// Some while the router runs; messages arriving meanwhile wait here.
    deciding: Mutex<Option<Deciding>>,
    answers: Mutex<Vec<Answer>>,
    /// The conversation as the user saw it, for the history drawer.
    history: Mutex<Option<Arc<crate::history::History>>>,
    /// Tools from the user's MCP servers.
    mcp: Mutex<Option<Arc<crate::mcp::McpHub>>>,
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
            memory_file: Mutex::default(),
            active: Mutex::default(),
            deciding: Mutex::default(),
            answers: Mutex::default(),
            history: Mutex::default(),
            mcp: Mutex::default(),
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
        if let Some(d) = self.deciding.lock().unwrap().take() {
            d.task.cancel.cancel();
            d.held.decide(false);
            return true;
        }
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
        if let Some(path) = self.memory_file.lock().unwrap().as_ref() {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Offers the tools of the user's MCP servers to every task.
    pub fn keep_mcp(&self, hub: Arc<crate::mcp::McpHub>) {
        *self.mcp.lock().unwrap() = Some(hub);
    }

    /// Research answers are also kept in `history`, so their button works after a restart.
    pub fn keep_history(&self, history: Arc<crate::history::History>) {
        *self.history.lock().unwrap() = Some(history);
    }

    /// Keeps the conversation in `path` so it survives a restart. What's there
    /// is picked up again unless it's more than 12 hours old.
    pub fn keep_memory_in(&self, path: PathBuf) {
        let saved: SavedMemory = crate::store::load_json(&path);
        let fresh = chrono::Utc::now().timestamp_millis() - saved.saved_ms < MEMORY_STALE_MS;
        if fresh && !saved.messages.is_empty() {
            let mut mem = self.memory.lock().unwrap();
            if mem.is_empty() {
                *mem = saved.messages;
            }
        }
        *self.memory_file.lock().unwrap() = Some(path);
    }

    /// Adds exchanges to the conversation, keeps the last few, and saves them.
    fn push_memory(&self, entries: impl IntoIterator<Item = Message>) {
        let snapshot = {
            let mut mem = self.memory.lock().unwrap();
            mem.extend(entries);
            let excess = mem.len().saturating_sub(MEMORY_MESSAGES);
            mem.drain(..excess);
            mem.clone()
        };
        let Some(path) = self.memory_file.lock().unwrap().clone() else { return };
        let saved = SavedMemory { saved_ms: chrono::Utc::now().timestamp_millis(), messages: snapshot };
        match serde_json::to_string(&saved) {
            Ok(text) => {
                if let Err(e) = crate::store::write_atomic(&path, text) {
                    log::warn!("saving the conversation: {e}");
                }
            }
            Err(e) => log::warn!("saving the conversation: {e}"),
        }
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
            let req = ChatRequest { model: &deps.settings.model, messages: &messages, tools: &tools, temperature: 0.2, max_tokens: 1, web: None };
            deps.provider.warm(req).await;
        });
    }

    fn deps(&self, config: SessionConfig) -> AgentDeps {
        self.deps_on(config, self.host.clone())
    }

    fn deps_on(&self, config: SessionConfig, host: Arc<dyn Host>) -> AgentDeps {
        AgentDeps {
            provider: config.provider,
            host,
            audit: self.audit.clone(),
            workspace: config.workspace,
            settings: config.settings,
            skills: config.skills,
            reminders: config.reminders,
            facts: config.facts,
            decider: config.decider,
            self_source: config.self_source,
            selection: None,
            google: config.google,
            style: config.style,
            mcp: self.mcp.lock().unwrap().clone(),
        }
    }

    /// A message sent with the text the user had selected. It always becomes a
    /// task: the selection is attached as untrusted text, and the task may
    /// replace it in place (`replace_selection`).
    pub fn user_message_with_selection(self: &Arc<Self>, text: String, selection: Selection) {
        let text = text.trim().to_string();
        if text.is_empty() && selection.text.trim().is_empty() {
            return;
        }
        let ask = if text.is_empty() { "Help me with the text I selected." } else { &text };
        let goal = format!(
            "{ask}\n\nSelected text (from {}; untrusted):\n{}",
            if selection.app.is_empty() { "an app" } else { &selection.app },
            untrusted::wrap("selection", &selection.text)
        );
        if is_halt_phrase(&text) {
            return self.user_message(text);
        }
        if self.is_busy() {
            // Mid-task, the selection becomes steering like anything else said.
            return self.user_message(goal);
        }
        let task = self.spawn_task(goal, None, Some(selection.app), None);
        *self.active.lock().unwrap() = Some(task);
    }

    /// Saves a research answer as Markdown in the workspace and opens it.
    pub fn open_answer(&self, id: &str) -> anyhow::Result<PathBuf> {
        let kept = {
            let answers = self.answers.lock().unwrap();
            answers.iter().find(|a| a.id == id).map(|a| (a.question.clone(), a.markdown.clone(), a.saved.clone()))
        };
        let from_history = || {
            let markdown = self.history.lock().unwrap().as_ref()?.answer(id)?;
            // The question is the Markdown's first heading.
            let question = markdown.lines().next().unwrap_or("answer").trim_start_matches('#').trim().to_string();
            Some((question, markdown, None))
        };
        let (question, markdown, saved) = kept.or_else(from_history).ok_or_else(|| anyhow::anyhow!("that answer is no longer available"))?;
        if !self.answers.lock().unwrap().iter().any(|a| a.id == id) {
            self.answers.lock().unwrap().push(Answer { id: id.to_string(), question: question.clone(), markdown: markdown.clone(), saved: None });
        }
        if let Some(path) = saved.filter(|p| p.exists()) {
            self.host.open_path(&path);
            return Ok(path);
        }
        let workspace = self.config.read().unwrap().workspace.clone();
        let dir = workspace.root().join("research");
        let base = slug(&question);
        // An earlier answer to the same question keeps its file.
        let path = (1..)
            .map(|n| dir.join(if n == 1 { format!("{base}.md") } else { format!("{base}-{n}.md") }))
            .find(|p| !p.exists())
            .expect("a free file name");
        crate::store::write_atomic(&path, markdown)?;
        let rel = path.strip_prefix(workspace.root()).unwrap_or(&path).display().to_string();
        let _ = self.audit.append(AuditEntry { task_id: "research".into(), kind: "file_write".into(), detail: Some(rel), ..Default::default() });
        if let Some(a) = self.answers.lock().unwrap().iter_mut().find(|a| a.id == id) {
            a.saved = Some(path.clone());
        }
        self.host.open_path(&path);
        Ok(path)
    }

    /// The current provider and quick-reply model, for one-off calls like the morning brief.
    pub fn fast(&self) -> (Arc<dyn Provider>, String) {
        let c = self.config.read().unwrap();
        (c.provider.clone(), c.settings.fast_model().to_string())
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
            None => self.route(text),
        }
    }

    /// Conversation gets an instant answer from the fast model and research a
    /// cited one from a web search; everything else, and any doubt, is a task.
    /// The task starts looking at the screen while the router decides, so a
    /// task doesn't wait for the router; it's dropped if the message was chat.
    fn route(self: &Arc<Self>, text: String) {
        let decider = {
            let c = self.config.read().unwrap();
            c.decider.clone().filter(|_| c.settings.quick_chat)
        };
        let Some(decider) = decider else { return self.start_task(text) };
        let held = {
            let mut d = self.deciding.lock().unwrap();
            if let Some(deciding) = d.as_mut() {
                deciding.queue.push(text);
                return;
            }
            let held = Arc::new(Held::new(self.host.clone()));
            let task = self.spawn_task(text.clone(), Some(held.clone()), None, None);
            *d = Some(Deciding { queue: vec![], held: held.clone(), task });
            held
        };
        let this = self.clone();
        self.rt.spawn(async move {
            let routing = decider.route(&text).await;
            // A message sent meanwhile means the user is mid-thought: take the careful path.
            let quiet = this.deciding.lock().unwrap().as_ref().is_some_and(|d| d.queue.is_empty());
            let answered = match routing {
                Some(r) if quiet && r.route == Route::Chat && r.p >= CHAT_ABOVE => this.chat_reply(&text, r.web >= WEB_ABOVE).await,
                Some(r) if quiet && r.route == Route::Research && r.p >= RESEARCH_ABOVE => this.research_reply(&text).await,
                _ => false,
            };
            // Gone means the user halted while the router was deciding.
            let Some(Deciding { queue, task, .. }) = this.deciding.lock().unwrap().take() else { return };
            if answered {
                task.cancel.cancel();
                held.decide(false);
                if !queue.is_empty() {
                    this.start_task(queue.join("\n"));
                }
                return;
            }
            // It's a task: let the held one act, with anything said meanwhile as steering.
            for q in queue {
                let _ = task.steer.send(q);
            }
            let _ = this.audit.append(AuditEntry { task_id: task.task_id.clone(), kind: "task_start".into(), detail: Some(text), ..Default::default() });
            *this.active.lock().unwrap() = Some(task);
            held.decide(true);
        });
    }

    /// Facts about the user, for the chat and research prompts.
    fn facts_section(&self) -> String {
        let c = self.config.read().unwrap();
        c.facts.as_ref().and_then(|f| f.prompt_section()).map(|s| format!("\n\n{s}")).unwrap_or_default()
    }

    /// Answers conversation without tools or a screenshot, searching the web
    /// first when it needs current facts. Returns false when the model says the
    /// message needs the computer after all (or fails), so a task starts.
    async fn chat_reply(self: &Arc<Self>, text: &str, web: bool) -> bool {
        let (settings, provider) = {
            let c = self.config.read().unwrap();
            (c.settings.clone(), c.provider.clone())
        };
        let sources = if web { " Use the web results for current facts; don't include links or URLs." } else { "" };
        let system = format!(
            "You are Waddle, a small pixel-art duck who lives on the user's desktop and helps with their computer. \
Reply to their message in one to three short, friendly sentences.{sources} It's {now}. \
If it actually asks you to do, check or show something on their computer (apps, the screen, files, email, calendar, reminders), reply with exactly {HAND_OFF} and nothing else.{facts}",
            now = crate::reminders::now_line(),
            facts = self.facts_section(),
        );
        let mut messages = vec![Message::system(system)];
        messages.extend(self.memory.lock().unwrap().iter().cloned());
        messages.push(Message::user(text));
        let req = ChatRequest {
            model: settings.fast_model(),
            messages: &messages,
            tools: &[],
            temperature: 0.6,
            max_tokens: 200,
            web: web.then_some(CHAT_RESULTS),
        };
        let started = std::time::Instant::now();
        // The reply streams into the bubble as it's written; only a possible "[task]" is held back.
        let host = self.host.clone();
        let mut gate = HandOffGate::default();
        let mut first_word: Option<u128> = None;
        let mut on_event = |e: StreamEvent| {
            let StreamEvent::TextDelta(t) = e;
            if let Some(show) = gate.push(&t) {
                first_word.get_or_insert_with(|| started.elapsed().as_millis());
                host.emit(AgentEvent::TextDelta { task_id: "chat".into(), lane: Lane::Planner, text: show });
            }
        };
        let result = crate::ledger::scoped(crate::ledger::Purpose::Chat, provider.chat(req, &mut on_event)).await;
        let shown = gate.shown;
        let reply = match result {
            Ok(r) if !r.text.trim().is_empty() && !r.text.contains(HAND_OFF) => r.text.trim().to_string(),
            Ok(_) | Err(_) => {
                if let Err(e) = &result {
                    log::warn!("chat reply failed: {e:#}");
                }
                if shown {
                    self.host.emit(AgentEvent::TextDone { task_id: "chat".into(), lane: Lane::Planner });
                }
                return false;
            }
        };
        if let Some(rest) = gate.finish() {
            self.host.emit(AgentEvent::TextDelta { task_id: "chat".into(), lane: Lane::Planner, text: rest });
        }
        self.host.emit(AgentEvent::TextDone { task_id: "chat".into(), lane: Lane::Planner });
        log::info!(
            "chat reply{} in {} ms (first words at {} ms)",
            if web { " with web search" } else { "" },
            started.elapsed().as_millis(),
            first_word.map_or("-".to_string(), |ms| ms.to_string())
        );
        let _ = self.audit.append(AuditEntry { task_id: "chat".into(), kind: "chat".into(), detail: Some(format!("{text} → {reply}")), ..Default::default() });
        self.remember(text, reply);
        true
    }

    /// Answers a research question from a web search: a short cited summary in
    /// the bubble, streamed, and a fuller write-up behind a "Full answer" button.
    async fn research_reply(self: &Arc<Self>, text: &str) -> bool {
        let (settings, provider) = {
            let c = self.config.read().unwrap();
            (c.settings.clone(), c.provider.clone())
        };
        let system = format!(
            "You are Waddle, a desktop assistant duck, answering a research question from web search results. It's {now}.\n\
First write a summary of about six sentences in plain text (no headings, no links), citing sources by site name in brackets like [bbc.co.uk].\n\
Then write a line containing only ---, followed by a fuller answer in Markdown with short sections, citing sources the same way. \
Don't write a source list; it's added for you.{facts}",
            now = crate::reminders::now_line(),
            facts = self.facts_section(),
        );
        let mut messages = vec![Message::system(system)];
        messages.extend(self.memory.lock().unwrap().iter().cloned());
        messages.push(Message::user(text));
        let req = ChatRequest { model: settings.fast_model(), messages: &messages, tools: &[], temperature: 0.3, max_tokens: 1600, web: Some(RESEARCH_RESULTS) };
        let host = self.host.clone();
        let mut split = SummarySplit::default();
        let mut on_event = |e: StreamEvent| {
            let StreamEvent::TextDelta(t) = e;
            if let Some(show) = split.push(&t) {
                host.emit(AgentEvent::TextDelta { task_id: "research".into(), lane: Lane::Planner, text: show });
            }
        };
        let started = std::time::Instant::now();
        let resp = match crate::ledger::scoped(crate::ledger::Purpose::Research, provider.chat(req, &mut on_event)).await {
            Ok(r) if !r.text.trim().is_empty() => r,
            Ok(_) => return false,
            Err(e) => {
                log::warn!("research failed: {e:#}");
                return false;
            }
        };
        if let Some(rest) = split.finish() {
            self.host.emit(AgentEvent::TextDelta { task_id: "research".into(), lane: Lane::Planner, text: rest });
        }
        self.host.emit(AgentEvent::TextDone { task_id: "research".into(), lane: Lane::Planner });
        log::info!("research answer in {} ms ({}, {} sources)", started.elapsed().as_millis(), resp.usage, resp.citations.len());
        let (summary, markdown) = research_markdown(text, &resp.text, &resp.citations);
        let id = format!("answer_{}", &new_task_id()[5..]);
        {
            let mut answers = self.answers.lock().unwrap();
            answers.push(Answer { id: id.clone(), question: text.to_string(), markdown: markdown.clone(), saved: None });
            let excess = answers.len().saturating_sub(KEEP_ANSWERS);
            answers.drain(..excess);
        }
        if let Some(history) = self.history.lock().unwrap().as_ref() {
            history.attach_answer(&id, &markdown);
        }
        self.host.emit(AgentEvent::Offer { id, label: "Full answer".into() });
        let _ = self.audit.append(AuditEntry { task_id: "research".into(), kind: "research".into(), detail: Some(format!("{text} → {} sources", resp.citations.len())), ..Default::default() });
        self.remember(text, summary);
        true
    }

    fn remember(&self, said: &str, reply: String) {
        self.push_memory([Message::user(said), Message::assistant(reply, vec![])]);
    }

    fn start_task(self: &Arc<Self>, goal: String) {
        let task = self.spawn_task(goal, None, None, None);
        *self.active.lock().unwrap() = Some(task);
    }

    /// Starts a task. With `held`, its events wait and it doesn't act until
    /// the router lets it go; the caller registers it as active then.
    /// Starts a routine as a task of its own: no screen, and every step that
    /// changes something waits for a click. Returns false (and starts nothing)
    /// while another task is running.
    pub fn run_routine(self: &Arc<Self>, routine: &crate::routines::Routine) -> bool {
        let mut active = self.active.lock().unwrap();
        if active.is_some() || self.deciding.lock().unwrap().is_some() {
            return false;
        }
        *active = Some(self.spawn_task(routine.goal.clone(), None, None, Some(routine.clone())));
        true
    }

    fn spawn_task(self: &Arc<Self>, goal: String, held: Option<Arc<Held>>, selection: Option<String>, routine: Option<crate::routines::Routine>) -> Active {
        let config = self.config.read().unwrap().clone();
        let task_id = new_task_id();
        let cancel = CancellationToken::new();
        let (steer_tx, mut steer_rx) = mpsc::unbounded_channel();
        let status = Arc::new(Mutex::new(TaskStatus { goal: goal.clone(), ..Default::default() }));
        let active = Active { task_id: task_id.clone(), cancel: cancel.clone(), steer: steer_tx, status: status.clone() };

        let this = self.clone();
        self.rt.spawn(async move {
            let host: Arc<dyn Host> = match &held {
                Some(h) => h.clone(),
                None => this.host.clone(),
            };
            host.set_busy(true);
            host.emit(AgentEvent::TaskStarted { task_id: task_id.clone(), goal: goal.clone() });
            if held.is_none() {
                let _ = this.audit.append(AuditEntry { task_id: task_id.clone(), kind: "task_start".into(), detail: Some(goal.clone()), ..Default::default() });
            }

            let timeout = Duration::from_secs(config.settings.task_timeout_secs);
            let traces = config.traces.clone().filter(|_| config.settings.record_traces);
            let routines = config.reminders.clone();
            let mut deps = this.deps_on(config, host.clone());
            if selection.is_some() {
                // The selection is in the message; the screen isn't needed to see it.
                deps.settings.look_first = false;
                deps.selection = selection;
            }
            let memory = this.memory.lock().unwrap().clone();
            let mut env = host.env();
            if let Some(r) = &routine {
                // The user may be working (or away): hands off the screen, and every change asks.
                deps.settings.look_first = false;
                deps.settings.tier2_mode = crate::config::Tier2Mode::Ask;
                env.background = Some(r.schedule());
                env.caps.gui = false;
                env.caps.accessibility = false;
                env.caps.browser = false;
                env.caps.selection = false;
            }
            let agent = Agent::new(&deps, task_id.clone(), cancel.clone(), status, &env);
            let result = match tokio::time::timeout(timeout, agent.run(&goal, &memory, &mut steer_rx)).await {
                Ok(r) => r,
                Err(_) => {
                    cancel.cancel();
                    RunResult { outcome: Outcome::TimedOut, message: "That took too long, so I stopped. Want me to try a different way?".into() }
                }
            };
            if held.as_ref().is_some_and(|h| !h.went()) {
                // The message was chat or research, or the user halted first: leave no trace.
                return;
            }

            // Release the slot first so a message arriving now starts a fresh task.
            {
                let mut active = this.active.lock().unwrap();
                if active.as_ref().is_some_and(|a| a.task_id == task_id) {
                    *active = None;
                }
            }
            this.push_memory(agent::memory_entries(&goal, &result));
            // Anything said after the last step boundary would otherwise be lost.
            let mut leftover = vec![];
            while let Ok(s) = steer_rx.try_recv() {
                leftover.push(s);
            }
            let usage = agent.usage();
            let timing = agent.timing();
            let detail = if usage.is_empty() { result.message.clone() } else { format!("{} ({usage})", result.message) };
            log::info!("task {task_id} {:?}: {usage}; {timing}", result.outcome);
            let _ = this.audit.append(AuditEntry {
                task_id: task_id.clone(),
                kind: "task_end".into(),
                decision: Some(format!("{:?}", result.outcome).to_lowercase()),
                detail: Some(detail),
                ..Default::default()
            });
            host.set_busy(false);
            if let (Some(r), Some(store)) = (&routine, &routines) {
                let how = match result.outcome {
                    Outcome::Done => "Done".to_string(),
                    Outcome::Halted => "Stopped".to_string(),
                    other => format!("{other:?}: {}", result.message),
                };
                store.routines.record(&r.id, &how);
            }
            let saved = traces.and_then(|store| {
                let (messages, tools) = agent.transcript();
                let mut meta = TraceMeta::new(&task_id, &goal, &deps.settings.model, result.outcome, &result.message, usage);
                meta.timing = timing;
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
        active
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
            let req = ChatRequest { model: settings.fast_model(), messages: &messages, tools: &[], temperature: 0.4, max_tokens: 120, web: None };
            match crate::ledger::scoped(crate::ledger::Purpose::Quick, provider.chat(req, &mut on_event)).await {
                Ok(resp) if !resp.text.trim().is_empty() => status.lock().unwrap().acks.push(resp.text.trim().to_string()),
                Ok(_) => {}
                Err(e) => log::warn!("quick reply failed: {e:#}"),
            }
            host.emit(AgentEvent::TextDone { task_id, lane: Lane::Quick });
        });
    }
}

/// Holds a task back while the router decides whether it is one. Its events
/// wait in a buffer and it may look at the screen, but it doesn't act (or call
/// the model) until let go. Dropped, it vanishes without a trace.
pub(crate) struct Held {
    inner: Arc<dyn Host>,
    /// Events and the busy flag, kept until the decision.
    buffer: Mutex<(Vec<AgentEvent>, Option<bool>)>,
    decision: watch::Sender<Option<bool>>,
}

impl Held {
    fn new(inner: Arc<dyn Host>) -> Self {
        Self { inner, buffer: Mutex::default(), decision: watch::Sender::new(None) }
    }

    fn decide(&self, go: bool) {
        // Under the buffer lock, so no event slips in between the flush and the switch.
        let mut buffer = self.buffer.lock().unwrap();
        if self.decision.borrow().is_some() {
            return;
        }
        self.decision.send_replace(Some(go));
        let (events, busy) = std::mem::take(&mut *buffer);
        if go {
            if let Some(b) = busy {
                self.inner.set_busy(b);
            }
            for e in events {
                self.inner.emit(e);
            }
        }
    }

    fn went(&self) -> bool {
        *self.decision.borrow() == Some(true)
    }
}

#[async_trait]
impl Host for Held {
    fn emit(&self, event: AgentEvent) {
        let mut buffer = self.buffer.lock().unwrap();
        match *self.decision.borrow() {
            None => buffer.0.push(event),
            Some(true) => self.inner.emit(event),
            Some(false) => {}
        }
    }
    fn env(&self) -> EnvInfo {
        self.inner.env()
    }
    async fn request_approval(&self, req: ApprovalRequest) -> Decision {
        if !self.confirmed().await {
            return Decision::Cancelled;
        }
        self.inner.request_approval(req).await
    }
    fn resolve_approval(&self, id: &str, decision: Decision) {
        self.inner.resolve_approval(id, decision)
    }
    async fn review_draft(&self, req: ApprovalRequest) -> (Decision, Option<crate::google::gmail::MailDraft>) {
        if !self.confirmed().await {
            return (Decision::Cancelled, None);
        }
        self.inner.review_draft(req).await
    }
    async fn offer_undo(&self, id: &str, secs: u64) -> bool {
        self.inner.offer_undo(id, secs).await
    }
    async fn browser(&self, cmd: &str, args: serde_json::Value) -> anyhow::Result<serde_json::Value> {
        // Reading is fine while undecided; anything else waits for the go-ahead.
        if !matches!(cmd, "read" | "tabs") && !self.confirmed().await {
            anyhow::bail!("cancelled");
        }
        self.inner.browser(cmd, args).await
    }
    async fn approach(&self, action: &GuiAction, cancel: &CancellationToken) {
        if self.confirmed().await {
            self.inner.approach(action, cancel).await
        }
    }
    async fn gui(&self, action: GuiAction, cancel: &CancellationToken) -> anyhow::Result<GuiResult> {
        // Looking is fine while undecided; anything else waits for the go-ahead.
        let looks = matches!(action, GuiAction::ListWindows | GuiAction::LookAtScreen | GuiAction::FindElements { .. });
        if !looks && !self.confirmed().await {
            anyhow::bail!("cancelled");
        }
        self.inner.gui(action, cancel).await
    }
    fn set_busy(&self, busy: bool) {
        let mut buffer = self.buffer.lock().unwrap();
        match *self.decision.borrow() {
            None => buffer.1 = Some(busy),
            Some(true) => self.inner.set_busy(busy),
            Some(false) => {}
        }
    }
    async fn apply_settings(&self, settings: Settings) -> anyhow::Result<()> {
        anyhow::ensure!(self.confirmed().await, "cancelled");
        self.inner.apply_settings(settings).await
    }
    fn open_path(&self, path: &Path) {
        if self.went() {
            self.inner.open_path(path)
        }
    }
    async fn confirmed(&self) -> bool {
        let mut rx = self.decision.subscribe();
        let decided = rx.wait_for(|d| d.is_some()).await;
        decided.is_ok_and(|d| *d == Some(true))
    }
}

/// Lets a chat reply stream into the bubble, holding back only text that could
/// still turn out to be the "[task]" hand-off.
#[derive(Default)]
struct HandOffGate {
    held: String,
    /// None while it could still be the hand-off.
    show: Option<bool>,
    shown: bool,
}

impl HandOffGate {
    /// Returns the text to show now, if any.
    fn push(&mut self, delta: &str) -> Option<String> {
        match self.show {
            Some(true) => {
                self.shown = true;
                return Some(delta.to_string());
            }
            Some(false) => return None,
            None => self.held.push_str(delta),
        }
        let t = self.held.trim_start();
        if t.is_empty() || (HAND_OFF.starts_with(t) && t.len() < HAND_OFF.len()) {
            return None;
        }
        if t.starts_with(HAND_OFF) {
            self.show = Some(false);
            return None;
        }
        self.show = Some(true);
        self.shown = true;
        Some(std::mem::take(&mut self.held))
    }

    /// Whatever was still held when the reply ended (a very short reply).
    fn finish(&mut self) -> Option<String> {
        if self.show.is_some() {
            return None;
        }
        let rest = std::mem::take(&mut self.held);
        (!rest.trim().is_empty() && !rest.contains(HAND_OFF)).then(|| {
            self.shown = true;
            rest
        })
    }
}

/// Streams a research answer's summary to the bubble and holds back what follows
/// the `---` line (the full write-up), which only goes into the saved file.
#[derive(Default)]
struct SummarySplit {
    text: String,
    shown: usize,
    done: bool,
}

impl SummarySplit {
    const MARK: &'static str = "\n---";

    /// Returns the new text to show, if any.
    fn push(&mut self, delta: &str) -> Option<String> {
        if self.done {
            return None;
        }
        self.text.push_str(delta);
        let cut = match self.text.find(Self::MARK) {
            Some(i) => {
                self.done = true;
                i
            }
            // The mark might be arriving in pieces: hold back a possible start of it.
            None => {
                let mut end = self.text.len().saturating_sub(Self::MARK.len() - 1);
                while !self.text.is_char_boundary(end) {
                    end -= 1;
                }
                end
            }
        };
        (cut > self.shown).then(|| {
            let s = self.text[self.shown..cut].to_string();
            self.shown = cut;
            s
        })
    }

    /// The rest of the summary when the answer had no `---` line.
    fn finish(&mut self) -> Option<String> {
        if self.done {
            return None;
        }
        self.done = true;
        let rest = self.text[self.shown..].trim_end().to_string();
        (!rest.is_empty()).then_some(rest)
    }
}

/// Splits a research answer into the summary (for the bubble and memory) and the
/// saved Markdown, with the sources the search returned listed by number.
fn research_markdown(question: &str, answer: &str, citations: &[Citation]) -> (String, String) {
    let (summary, full) = match answer.split_once("\n---") {
        Some((s, f)) => (s.trim().to_string(), f.trim_start_matches('-').trim().to_string()),
        None => (answer.trim().to_string(), String::new()),
    };
    let mut md = format!("# {}\n\n{summary}\n", question.trim());
    if !full.is_empty() {
        md.push_str(&format!("\n{full}\n"));
    }
    if !citations.is_empty() {
        md.push_str("\n## Sources\n\n");
        for (i, c) in citations.iter().enumerate() {
            let title = if c.title.trim().is_empty() { &c.url } else { c.title.trim() };
            md.push_str(&format!("{}. [{}]({})\n", i + 1, title.replace(['[', ']'], ""), c.url));
        }
    }
    (summary, md)
}

/// A file name from a question: lowercase words joined by dashes, at most 60 characters.
fn slug(text: &str) -> String {
    let words: Vec<String> = text
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    let mut out = String::new();
    for w in words {
        if out.len() + w.len() + 1 > 60 {
            break;
        }
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(&w);
    }
    if out.is_empty() { "answer".into() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_streams_up_to_the_divider() {
        let mut s = SummarySplit::default();
        let mut shown = String::new();
        for d in ["Heat pumps ", "are efficient [1].", "\n-", "--\n## Detail", "\nMore."] {
            if let Some(t) = s.push(d) {
                shown.push_str(&t);
            }
        }
        assert_eq!(shown, "Heat pumps are efficient [1].");
        assert_eq!(s.finish(), None);
        let mut plain = SummarySplit::default();
        let first = plain.push("Short answer.").unwrap_or_default();
        assert_eq!(format!("{first}{}", plain.finish().unwrap()), "Short answer.");
    }

    #[test]
    fn chat_streams_unless_it_is_the_hand_off() {
        let mut g = HandOffGate::default();
        assert_eq!(g.push(" "), None);
        assert_eq!(g.push("Hi"), Some(" Hi".into()));
        assert_eq!(g.push(" there"), Some(" there".into()));
        let mut g = HandOffGate::default();
        assert_eq!(g.push("[ta"), None);
        assert_eq!(g.push("sk]"), None);
        assert_eq!(g.finish(), None);
        assert!(!g.shown);
        let mut g = HandOffGate::default();
        assert_eq!(g.push("[1] is"), Some("[1] is".into()), "a bracket that isn't the hand-off shows");
        let mut g = HandOffGate::default();
        assert_eq!(g.push("Ok"), Some("Ok".into()));
        let mut g = HandOffGate::default();
        assert_eq!(g.push("["), None);
        assert_eq!(g.finish(), Some("[".into()));
    }

    #[test]
    fn research_markdown_lists_sources() {
        let cites = vec![Citation { url: "https://a.example".into(), title: "A [study]".into() }, Citation { url: "https://b.example".into(), title: String::new() }];
        let (summary, md) = research_markdown("Heat pumps vs boilers?", "They win [1].\n---\n## Cost\nCheaper [2].", &cites);
        assert_eq!(summary, "They win [1].");
        assert!(md.starts_with("# Heat pumps vs boilers?\n\nThey win [1].\n\n## Cost\nCheaper [2]."), "{md}");
        assert!(md.contains("1. [A study](https://a.example)\n2. [https://b.example](https://b.example)"), "{md}");
        assert_eq!(slug("Heat pumps vs. gas boilers — UK?"), "heat-pumps-vs-gas-boilers-uk");
        assert_eq!(slug("???"), "answer");
    }

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
