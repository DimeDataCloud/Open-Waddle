//! End-to-end behaviour of the planner loop and the real-time session, using a
//! scripted provider and a fake host.

mod common;

use async_trait::async_trait;
use common::FakeHost;
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use waddle_core::agent::{promises_action, prune_images, writes_tool_call, Agent, AgentDeps, TaskStatus};
use waddle_core::audit::AuditLog;
use waddle_core::config::ProviderKind;
use waddle_core::llm::mock::{call, reply, MockProvider};
use waddle_core::llm::{ChatRequest, ChatResponse, EventSink, ImageData, Message, Provider, Role, StreamEvent};
use waddle_core::tools::fs::Workspace;
use waddle_core::tools::{ElementInfo, GuiAction, WindowInfo};
use waddle_core::skills::SkillStore;
use waddle_core::{AgentEvent, Decision, Host, Lane, Outcome, Session, SessionConfig, Settings};

fn settings() -> Settings {
    // Most tests script exact tool sequences; the opening look has its own test.
    Settings { model: "test-model".into(), tier2_countdown_ms: 20, look_first: false, ..Settings::default() }
}

struct Fixture {
    _dir: tempfile::TempDir,
    workspace: Arc<Workspace>,
    audit: Arc<AuditLog>,
}

fn session_config(f: &Fixture, provider: Arc<dyn Provider>) -> SessionConfig {
    SessionConfig { settings: settings(), provider, workspace: f.workspace.clone(), skills: None, reminders: None, self_source: None, traces: None }
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let workspace = Arc::new(Workspace::new(dir.path().join("ws")).unwrap());
    Fixture { _dir: dir, workspace, audit: Arc::new(AuditLog::open_in_memory().unwrap()) }
}

async fn run_agent(f: &Fixture, host: Arc<FakeHost>, provider: Arc<MockProvider>, cancel: CancellationToken) -> (Outcome, String) {
    let deps = AgentDeps { provider, host: host.clone(), audit: f.audit.clone(), workspace: f.workspace.clone(), settings: settings(), skills: None, reminders: None, self_source: None };
    let env = host.env();
    let agent = Agent::new(&deps, "t1".into(), cancel, Arc::new(Mutex::new(TaskStatus::default())), &env);
    let (_tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let r = agent.run("do the thing", &[], &mut rx).await;
    (r.outcome, r.message)
}

#[tokio::test]
async fn tiers_gate_actions_and_denials_reach_the_model() {
    let f = fixture();
    let provider = Arc::new(MockProvider::scripted(vec![
        reply("Writing a note.", vec![call("write_file", json!({"path":"note.txt","content":"hi"}))]),
        reply("Deleting it.", vec![call("run_command", json!({"command":"rm note.txt"}))]),
        reply("Okay, I'll leave it.", vec![]),
    ]));
    // Tier 2 countdown elapses on its own; tier 3 is denied by the user.
    let host = FakeHost::new(Box::new(|r| if r.tier == 3 { Some(Decision::Denied) } else { None }));
    let (outcome, message) = run_agent(&f, host.clone(), provider.clone(), CancellationToken::new()).await;

    assert_eq!(outcome, Outcome::Done);
    assert_eq!(message, "Okay, I'll leave it.");
    assert_eq!(std::fs::read_to_string(f.workspace.root().join("note.txt")).unwrap(), "hi");

    let approvals = host.approvals.lock().unwrap().clone();
    assert_eq!(approvals.len(), 2);
    assert_eq!((approvals[0].tier, approvals[0].countdown_ms), (2, Some(20)));
    assert_eq!((approvals[1].tier, approvals[1].countdown_ms), (3, None));

    let last_request = provider.requests.lock().unwrap().last().unwrap().clone();
    let denial = last_request.iter().rev().find(|m| m.role == Role::Tool).unwrap();
    assert!(denial.text.starts_with("The user denied this action (tier 3"), "{}", denial.text);

    let report = f.audit.verify().unwrap();
    // write_file (gate + result) and the denied command.
    assert!(report.ok && report.entries == 3, "{report:?}");
    let decisions: Vec<_> = f.audit.recent(50).unwrap().into_iter().filter_map(|r| r.decision).collect();
    assert!(decisions.contains(&"denied".to_string()) && decisions.contains(&"approved".to_string()));

    let events = host.events();
    assert!(events.iter().any(|e| matches!(e, AgentEvent::TextDelta { lane: Lane::Planner, .. })));
    assert!(events.iter().any(|e| matches!(e, AgentEvent::ToolFinished { tool, ok: true, .. } if tool == "write_file")));
}

#[tokio::test]
async fn halting_mid_command_kills_it_and_stops_the_loop() {
    let f = fixture();
    let sleeper = if cfg!(windows) { "Start-Sleep 30" } else { "sleep 30" };
    let provider = Arc::new(MockProvider::scripted(vec![
        reply("Waiting.", vec![call("run_command", json!({"command": sleeper}))]),
        reply("should never be asked", vec![]),
    ]));
    let host = FakeHost::new(Box::new(|_| Some(Decision::Approved)));
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        c2.cancel();
    });
    let start = std::time::Instant::now();
    let (outcome, _) = run_agent(&f, host, provider.clone(), cancel).await;
    assert_eq!(outcome, Outcome::Halted);
    assert!(start.elapsed() < Duration::from_secs(5));
    assert_eq!(provider.request_count(), 1, "no model call after halt");
    assert!(f.audit.recent(20).unwrap().iter().any(|r| r.kind == "halt"));
}

#[tokio::test]
async fn denial_skips_remaining_calls_in_the_same_turn() {
    let f = fixture();
    let provider = Arc::new(MockProvider::scripted(vec![
        reply(
            "Two things.",
            vec![call("run_command", json!({"command":"rm a"})), call("write_file", json!({"path":"b.txt","content":"b"}))],
        ),
        reply("Fine.", vec![]),
    ]));
    let host = FakeHost::new(Box::new(|_| Some(Decision::Denied)));
    run_agent(&f, host.clone(), provider.clone(), CancellationToken::new()).await;
    assert_eq!(host.approvals.lock().unwrap().len(), 1);
    assert!(!f.workspace.root().join("b.txt").exists());
    let req = provider.requests.lock().unwrap().last().unwrap().clone();
    let results: Vec<_> = req.iter().filter(|m| m.role == Role::Tool).collect();
    assert_eq!(results.len(), 2, "every tool call is answered");
    assert!(results[1].text.starts_with("Skipped"));
}

#[tokio::test]
async fn gui_actions_convert_coordinates_and_screenshots_ride_in_user_messages() {
    let f = fixture();
    let provider = Arc::new(MockProvider::scripted(vec![
        reply("Looking.", vec![call("look_at_screen", json!({}))]),
        reply("Clicking.", vec![call("click", json!({"x": 500, "y": 250}))]),
        reply("Done.", vec![]),
    ]));
    let host = FakeHost::new(Box::new(|_| Some(Decision::Approved)));
    let deps = AgentDeps {
        provider: provider.clone(),
        host: host.clone(),
        audit: f.audit.clone(),
        workspace: f.workspace.clone(),
        settings: Settings { model: "qwen/qwen3-vl-8b-instruct".into(), ..settings() },
        skills: None,
        reminders: None,
        self_source: None,
    };
    let env = host.env();
    let agent = Agent::new(&deps, "t".into(), CancellationToken::new(), Arc::default(), &env);
    let (_tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    agent.run("click the middle top", &[], &mut rx).await;

    let gui = host.gui_calls.lock().unwrap().clone();
    assert!(matches!(gui[1], GuiAction::Click { x, y, .. } if x == 720.0 && y == 240.0), "{:?}", gui[1]);
    let second = provider.requests.lock().unwrap()[1].clone();
    assert!(second.iter().any(|m| m.role == Role::User && !m.images.is_empty()));
}

#[tokio::test]
async fn the_first_request_shows_the_screen() {
    let f = fixture();
    let provider = Arc::new(MockProvider::scripted(vec![reply("Clicking Play.", vec![call("click", json!({"x": 100, "y": 100}))]), reply("Done.", vec![])]));
    let host = FakeHost::new(Box::new(|_| Some(Decision::Approved)));
    *host.windows.lock().unwrap() = vec![WindowInfo { title: "DuckTube".into(), app: "chrome".into(), x: 0.0, y: 0.0, w: 1440.0, h: 960.0, focused: true }];
    let deps = AgentDeps { settings: Settings { look_first: true, ..settings() }, ..deps_with(&f, host.clone(), provider.clone(), None, None) };
    let env = host.env();
    let agent = Agent::new(&deps, "t".into(), CancellationToken::new(), Arc::default(), &env);
    let (_tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    agent.run("play my playlist", &[], &mut rx).await;

    let first = provider.requests.lock().unwrap()[0].clone();
    let goal = first.last().unwrap();
    assert_eq!(goal.role, Role::User);
    assert!(goal.text.starts_with("play my playlist"), "{}", goal.text);
    assert!(goal.text.contains("DuckTube") && goal.text.contains("<untrusted"), "window titles arrive wrapped: {}", goal.text);
    assert_eq!(goal.images.len(), 1);
    // Having seen the screen, the click is allowed straight away.
    let gui = host.gui_calls.lock().unwrap().clone();
    assert_eq!(gui[..2], [GuiAction::ListWindows, GuiAction::LookAtScreen]);
    assert!(matches!(gui[2], GuiAction::Click { .. }), "{gui:?}");
}

#[tokio::test]
async fn a_third_identical_click_is_refused() {
    let f = fixture();
    let click = || call("click", json!({"x": 10, "y": 10}));
    let provider = Arc::new(MockProvider::scripted(vec![
        reply("Looking.", vec![call("list_windows", json!({}))]),
        reply("Clicking.", vec![click()]),
        reply("Again.", vec![click()]),
        reply("Again.", vec![click()]),
        reply("Trying elsewhere.", vec![call("click", json!({"x": 20, "y": 10}))]),
        reply("Done.", vec![]),
    ]));
    let host = FakeHost::new(Box::new(|_| Some(Decision::Approved)));
    run_agent(&f, host.clone(), provider.clone(), CancellationToken::new()).await;

    let clicks = host.gui_calls.lock().unwrap().iter().filter(|a| matches!(a, GuiAction::Click { .. })).count();
    assert_eq!(clicks, 3, "the third identical click is refused, a different one goes through");
    let fifth = provider.requests.lock().unwrap()[4].clone();
    assert!(fifth.last().unwrap().text.contains("already done exactly this twice"), "{:?}", fifth.last());
}

#[tokio::test]
async fn blind_input_is_refused_until_waddle_has_looked() {
    let f = fixture();
    let provider = Arc::new(MockProvider::scripted(vec![
        reply("Saving.", vec![call("press_keys", json!({"keys": "ctrl+s"}))]),
        reply("Let me look first.", vec![call("list_windows", json!({}))]),
        reply("Saving.", vec![call("press_keys", json!({"keys": "ctrl+s"}))]),
        reply("Saved.", vec![]),
    ]));
    let host = FakeHost::new(Box::new(|_| Some(Decision::Approved)));
    run_agent(&f, host.clone(), provider.clone(), CancellationToken::new()).await;

    let gui = host.gui_calls.lock().unwrap().clone();
    assert_eq!(gui, vec![GuiAction::ListWindows, GuiAction::PressKeys { keys: "ctrl+s".into() }]);
    let second = provider.requests.lock().unwrap()[1].clone();
    let refusal = second.iter().rev().find(|m| m.role == Role::Tool).unwrap();
    assert!(refusal.text.contains("haven't looked"), "{}", refusal.text);
    let key_approvals = host.approvals.lock().unwrap().iter().filter(|a| a.tool == "press_keys").count();
    assert_eq!(key_approvals, 1, "the refused attempt never reaches the approval gate");
}

#[tokio::test]
async fn an_announced_action_without_a_tool_call_gets_one_nudge() {
    let f = fixture();
    let provider = Arc::new(MockProvider::scripted(vec![
        reply("I see the red button. I'll click it for you.", vec![]),
        reply("Clicking it.", vec![call("list_windows", json!({}))]),
        reply("Let me take another look.", vec![]),
    ]));
    let host = FakeHost::new(Box::new(|_| Some(Decision::Approved)));
    let (outcome, message) = run_agent(&f, host.clone(), provider.clone(), CancellationToken::new()).await;

    assert_eq!(host.gui_calls.lock().unwrap().clone(), vec![GuiAction::ListWindows]);
    let second = provider.requests.lock().unwrap()[1].clone();
    assert!(second.last().unwrap().text.contains("didn't call a tool"), "{:?}", second.last());
    // Only one nudge per task, so a model that keeps talking still finishes.
    assert_eq!((outcome, message.as_str(), provider.request_count()), (Outcome::Done, "Let me take another look.", 3));
}

#[test]
fn only_replies_that_announce_an_action_count_as_promises() {
    for t in ["I'll click it for you.", "Let me take a screenshot first.", "Now I\u{2019}m going to open Notepad", "Okay! I will go ahead and type it."] {
        assert!(promises_action(t), "{t}");
    }
    for t in ["Okay, I'll leave it.", "Done! Let me know if you need anything else.", "I clicked the red button.", "I'll be here if you need me."] {
        assert!(!promises_action(t), "{t}");
    }
}

#[test]
fn screenshots_stay_put_until_the_budget_is_exceeded() {
    let img = || ImageData { mime: "image/png".into(), base64: "X".into() };
    let mut msgs = vec![Message::user_with_image("a", img()), Message::user("b"), Message::user_with_image("c", img())];
    // Within budget the history is untouched (append-only keeps a local prompt cache valid).
    let before = msgs.clone();
    prune_images(&mut msgs, 2);
    assert_eq!(msgs, before);
    // Over budget, only the newest screenshot survives.
    prune_images(&mut msgs, 1);
    assert!(msgs[0].images.is_empty() && msgs[0].text.contains("removed"));
    assert_eq!(msgs[2].images.len(), 1);
}

/// Planner with a delay per step; instant canned answer when called without tools (quick lane).
struct LaneProvider {
    planner: MockProvider,
    quick_calls: Mutex<u32>,
}

#[async_trait]
impl Provider for LaneProvider {
    async fn chat(&self, req: ChatRequest<'_>, on_event: EventSink<'_>) -> anyhow::Result<ChatResponse> {
        if req.tools.is_empty() {
            *self.quick_calls.lock().unwrap() += 1;
            on_event(StreamEvent::TextDelta("On it!".into()));
            return Ok(reply("On it!", vec![]));
        }
        self.planner.chat(req, on_event).await
    }
}

async fn wait_until(mut cond: impl FnMut() -> bool) {
    for _ in 0..200 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition not met in time");
}

#[tokio::test]
async fn messages_while_busy_get_a_quick_reply_and_steer_the_planner() {
    let f = fixture();
    let provider = Arc::new(LaneProvider {
        planner: MockProvider::scripted(vec![
            reply("Listing.", vec![call("list_dir", json!({}))]),
            reply("Adjusted!", vec![]),
        ])
        .with_delay(Duration::from_millis(150)),
        quick_calls: Mutex::new(0),
    });
    let host = FakeHost::new(Box::new(|_| Some(Decision::Approved)));
    let session = Session::new(tokio::runtime::Handle::current(), host.clone(), f.audit.clone(), session_config(&f, provider.clone()));

    session.user_message("look at my files".into());
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(session.is_busy());
    session.user_message("actually only text files".into());

    wait_until(|| !session.is_busy()).await;
    assert_eq!(*provider.quick_calls.lock().unwrap(), 1);
    let events = host.events();
    assert!(events.iter().any(|e| matches!(e, AgentEvent::TextDelta { lane: Lane::Quick, text, .. } if text == "On it!")));
    let second = provider.planner.requests.lock().unwrap()[1].clone();
    let steer = second.iter().find(|m| m.text.contains("(New message from the user")).expect("steering message");
    assert!(steer.text.contains("only text files"));
    assert!(steer.text.contains("You already replied"), "{}", steer.text);
    assert!(matches!(events.last(), Some(AgentEvent::TaskFinished { outcome: Outcome::Done, .. })));
    assert_eq!(*host.busy.lock().unwrap(), vec![true, false]);
}

#[tokio::test]
async fn saying_stop_halts_without_a_model_call() {
    let f = fixture();
    let provider = Arc::new(LaneProvider {
        planner: MockProvider::scripted(vec![reply("Thinking hard.", vec![call("list_dir", json!({}))])]).with_delay(Duration::from_secs(5)),
        quick_calls: Mutex::new(0),
    });
    let host = FakeHost::new(Box::new(|_| Some(Decision::Approved)));
    let session = Session::new(tokio::runtime::Handle::current(), host.clone(), f.audit.clone(), session_config(&f, provider.clone()));
    session.user_message("do something slow".into());
    tokio::time::sleep(Duration::from_millis(50)).await;
    session.user_message("Stop!".into());
    wait_until(|| !session.is_busy()).await;
    assert_eq!(*provider.quick_calls.lock().unwrap(), 0);
    assert!(host.events().iter().any(|e| matches!(e, AgentEvent::TaskFinished { outcome: Outcome::Halted, .. })));
}

#[tokio::test]
async fn follow_up_tasks_see_conversation_memory() {
    let f = fixture();
    let provider = Arc::new(MockProvider::scripted(vec![reply("Hi there!", vec![]), reply("Sure.", vec![])]));
    let host = FakeHost::new(Box::new(|_| Some(Decision::Approved)));
    let session = Session::new(tokio::runtime::Handle::current(), host.clone(), f.audit.clone(), session_config(&f, provider.clone()));
    session.user_message("hello".into());
    wait_until(|| host.events().iter().any(|e| matches!(e, AgentEvent::TaskFinished { .. }))).await;
    wait_until(|| !session.is_busy()).await;
    session.user_message("and again".into());
    wait_until(|| provider.request_count() == 2).await;
    let second = provider.requests.lock().unwrap()[1].clone();
    assert!(second.iter().any(|m| m.role == Role::Assistant && m.text == "Hi there!"));
}

#[tokio::test]
async fn warming_sends_the_next_tasks_opening_to_local_models_only() {
    let f = fixture();
    let provider = Arc::new(MockProvider::scripted(vec![reply("Hi there!", vec![]), reply("Sure.", vec![])]));
    let host = FakeHost::new(Box::new(|_| Some(Decision::Approved)));
    let session = Session::new(tokio::runtime::Handle::current(), host.clone(), f.audit.clone(), session_config(&f, provider.clone()));
    session.warm();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(provider.warmups.lock().unwrap().is_empty(), "a hosted API was warmed (and would bill for it)");

    let mut local = session_config(&f, provider.clone());
    local.settings.provider = ProviderKind::Ollama;
    session.configure(local);
    session.user_message("hello".into());
    wait_until(|| host.events().iter().any(|e| matches!(e, AgentEvent::TaskFinished { .. }))).await;
    wait_until(|| !session.is_busy()).await;
    session.warm();
    wait_until(|| provider.warmups.lock().unwrap().len() == 1).await;
    session.user_message("and again".into());
    wait_until(|| provider.request_count() == 2).await;

    // The warm-up is exactly the next request minus the user's new message,
    // so a local server can reuse everything it read while the user typed.
    let warm = provider.warmups.lock().unwrap()[0].clone();
    let next = provider.requests.lock().unwrap()[1].clone();
    assert_eq!(warm.as_slice(), &next[..next.len() - 1]);
    assert!(warm.iter().any(|m| m.text == "Hi there!"), "memory missing from the warm-up");
}

fn deps_with(f: &Fixture, host: Arc<FakeHost>, provider: Arc<MockProvider>, skills: Option<Arc<SkillStore>>, self_source: Option<Arc<Workspace>>) -> AgentDeps {
    AgentDeps { provider, host, audit: f.audit.clone(), workspace: f.workspace.clone(), settings: settings(), skills, reminders: None, self_source }
}

async fn run_with(deps: &AgentDeps) -> waddle_core::agent::RunResult {
    let env = deps.host.env();
    let agent = Agent::new(deps, "t".into(), CancellationToken::new(), Arc::default(), &env);
    let (_tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    agent.run("goal", &[], &mut rx).await
}

#[tokio::test]
async fn delegate_runs_a_nested_agent_and_returns_its_result() {
    let f = fixture();
    let provider = Arc::new(MockProvider::scripted(vec![
        reply("Splitting this up.", vec![call("delegate", json!({"goal": "write the note", "context": "say hi"}))]),
        reply("Writing.", vec![call("write_file", json!({"path": "sub.txt", "content": "hi"}))]),
        reply("Note written.", vec![]),
        reply("All done.", vec![]),
    ]));
    let host = FakeHost::new(Box::new(|_| Some(Decision::Approved)));
    let deps = deps_with(&f, host.clone(), provider.clone(), None, None);
    let r = run_with(&deps).await;
    assert_eq!((r.outcome, r.message.as_str()), (Outcome::Done, "All done."));
    assert!(f.workspace.root().join("sub.txt").exists());
    let requests = provider.requests.lock().unwrap().clone();
    assert!(requests[1][0].text.contains("You are a sub-task (depth 1)"));
    assert!(requests[1].iter().any(|m| m.text.contains("Context from the parent task:\nsay hi")));
    let parent_view = requests[3].iter().rev().find(|m| m.role == Role::Tool).unwrap();
    assert_eq!(parent_view.text, "Sub-task done: Note written.");
}

#[tokio::test]
async fn delegation_stops_at_the_depth_limit() {
    let f = fixture();
    let d = |g: &str| reply("Delegating.", vec![call("delegate", json!({"goal": g}))]);
    let provider = Arc::new(MockProvider::scripted(vec![d("a"), d("b"), d("c"), reply("leaf", vec![]), reply("mid", vec![]), reply("top", vec![])]));
    let host = FakeHost::new(Box::new(|_| Some(Decision::Approved)));
    let deps = deps_with(&f, host, provider.clone(), None, None);
    run_with(&deps).await;
    let requests = provider.requests.lock().unwrap().clone();
    // Depth-2 agent is not offered `delegate`; its attempt is refused.
    let refused = requests.iter().flatten().any(|m| m.text.contains("delegation depth limit (2) reached"));
    assert!(refused);
}

#[tokio::test]
async fn skills_need_approval_and_load_into_later_prompts() {
    let f = fixture();
    let store = Arc::new(SkillStore::new(f.workspace.root().parent().unwrap().join("skills")).unwrap());
    let provider = Arc::new(MockProvider::scripted(vec![
        reply("Remembering that.", vec![call("save_skill", json!({"name": "Notes", "instructions": "Notes go in notes.txt"}))]),
        reply("Saved.", vec![]),
        reply("Hi again.", vec![]),
    ]));
    let host = FakeHost::new(Box::new(|_| Some(Decision::Approved)));
    let deps = deps_with(&f, host.clone(), provider.clone(), Some(store.clone()), None);
    run_with(&deps).await;
    assert_eq!(host.approvals.lock().unwrap()[0].tier, 3);
    run_with(&deps).await;
    let third = provider.requests.lock().unwrap()[2].clone();
    assert!(third[0].text.contains("### notes\nNotes go in notes.txt"));
}

#[tokio::test]
async fn self_settings_changes_are_whitelisted_and_applied_through_the_host() {
    let f = fixture();
    let provider = Arc::new(MockProvider::scripted(vec![
        reply("Switching.", vec![call("update_settings", json!({"changes": {"base_url": "https://evil.example"}}))]),
        reply("Okay, model then.", vec![call("update_settings", json!({"changes": {"model": "better/model"}}))]),
        reply("Done.", vec![]),
    ]));
    let host = FakeHost::new(Box::new(|_| Some(Decision::Approved)));
    let deps = deps_with(&f, host.clone(), provider, Some(Arc::new(SkillStore::new(f.workspace.root().join("..").join("sk")).unwrap())), None);
    run_with(&deps).await;
    let applied = host.applied.lock().unwrap().clone();
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0].model, "better/model");
    assert_eq!(applied[0].base_url, Settings::default().base_url);
}

#[tokio::test]
async fn self_source_is_reachable_with_approval_only() {
    let f = fixture();
    let src = Arc::new(Workspace::new(f.workspace.root().parent().unwrap().join("src")).unwrap());
    let provider = Arc::new(MockProvider::scripted(vec![
        reply("Patching myself.", vec![call("write_file", json!({"path": "self/notes.md", "content": "v2"}))]),
        reply("Reading back.", vec![call("read_file", json!({"path": "self/notes.md"}))]),
        reply("Done.", vec![]),
    ]));
    let host = FakeHost::new(Box::new(|r| Some(if r.tier == 3 { Decision::Approved } else { Decision::Denied })));
    let deps = deps_with(&f, host.clone(), provider.clone(), None, Some(src.clone()));
    run_with(&deps).await;
    assert_eq!(std::fs::read_to_string(src.root().join("notes.md")).unwrap(), "v2");
    assert_eq!(host.approvals.lock().unwrap()[0].tier, 3);
    let last = provider.requests.lock().unwrap().last().unwrap().clone();
    assert!(last.iter().any(|m| m.role == Role::Tool && m.text.contains("v2")));
}

#[tokio::test]
async fn recorded_tasks_are_saved_for_training_and_offered_for_rating() {
    let f = fixture();
    let store = Arc::new(waddle_core::traces::TraceStore::new(f._dir.path().join("traces")));
    let provider = Arc::new(MockProvider::scripted(vec![reply("All done.", vec![]), reply("Done again.", vec![])]));
    let host = FakeHost::new(Box::new(|_| Some(Decision::Approved)));
    let mut config = SessionConfig { traces: Some(store.clone()), ..session_config(&f, provider.clone()) };
    // Off by default: nothing is saved.
    let session = Session::new(tokio::runtime::Handle::current(), host.clone(), f.audit.clone(), config.clone());
    session.user_message("say hi".into());
    wait_until(|| !session.is_busy() && host.events().iter().any(|e| matches!(e, AgentEvent::TaskFinished { .. }))).await;
    assert!(store.list().is_empty());

    config.settings.record_traces = true;
    session.configure(config);
    session.user_message("say hi again".into());
    wait_until(|| host.events().iter().any(|e| matches!(e, AgentEvent::TraceSaved { .. }))).await;
    let saved = store.list();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].goal, "say hi again");
    assert_eq!(saved[0].outcome, Outcome::Done);
}

#[tokio::test]
async fn click_element_trusts_the_name_when_the_id_is_off() {
    let f = fixture();
    let provider = Arc::new(MockProvider::scripted(vec![
        reply("Reading the dialog.", vec![call("find_elements", json!({}))]),
        reply("Clicking Don't save.", vec![call("click_element", json!({"id": 1, "name": "Don't save"}))]),
        reply("Clicking Cancel.", vec![call("click_element", json!({"id": 3, "name": "Cancel"}))]),
        reply("Trying a name that isn't listed.", vec![call("click_element", json!({"id": 1, "name": "Close"}))]),
        reply("Done.", vec![]),
    ]));
    let host = FakeHost::new(Box::new(|_| Some(Decision::Approved)));
    let el = |id, name: &str| ElementInfo { id, role: "button".into(), name: name.into(), x: 0.0, y: 0.0, w: 10.0, h: 10.0 };
    *host.elements.lock().unwrap() = vec![el(1, "Save"), el(2, "Don't save"), el(3, "Cancel")];
    run_agent(&f, host.clone(), provider.clone(), CancellationToken::new()).await;

    let clicked: Vec<u32> = host.gui_calls.lock().unwrap().iter().filter_map(|a| if let GuiAction::ClickElement { id } = a { Some(*id) } else { None }).collect();
    assert_eq!(clicked, vec![2, 3, 1], "wrong id + listed name → the named element; matching or unknown names → the id as given");
}

#[tokio::test]
async fn pointing_and_reminders_need_no_approval() {
    let f = fixture();
    let host = FakeHost::new(Box::new(|_| Some(Decision::Denied)));
    let store = Arc::new(waddle_core::reminders::ReminderStore::new(f._dir.path().join("reminders.json")));
    let provider = Arc::new(MockProvider::scripted(vec![
        reply("Here it is.", vec![call("point_at", json!({"x": 100, "y": 200, "label": "Night light"}))]),
        reply("I'll remind you.", vec![call("reminder", json!({"action": "add", "text": "stretch", "in_minutes": 20}))]),
        reply("Done: it's circled, and I'll remind you at the time.", vec![]),
    ]));
    let deps = AgentDeps { reminders: Some(store.clone()), ..deps_with(&f, host.clone(), provider, None, None) };
    let r = run_with(&deps).await;
    assert_eq!(r.outcome, Outcome::Done, "{}", r.message);
    assert!(host.approvals.lock().unwrap().is_empty(), "tier 0/1 tools never ask");
    assert!(matches!(host.gui_calls.lock().unwrap()[0], GuiAction::PointAt { .. }), "pointing needs no look first: it touches nothing");
    assert_eq!(store.list().iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["stretch"]);
}

#[tokio::test]
async fn scrolling_counts_as_input_that_needs_a_look_first() {
    let f = fixture();
    let provider = Arc::new(MockProvider::scripted(vec![
        reply("Scrolling down.", vec![call("scroll", json!({"direction": "down"}))]),
        reply("Looking first.", vec![call("look_at_screen", json!({}))]),
        reply("Scrolling down.", vec![call("scroll", json!({"direction": "down", "amount": 10}))]),
        reply("There's more below now.", vec![]),
    ]));
    let host = FakeHost::new(Box::new(|_| Some(Decision::Approved)));
    run_agent(&f, host.clone(), provider, CancellationToken::new()).await;
    let calls = host.gui_calls.lock().unwrap().clone();
    assert_eq!(calls, vec![GuiAction::LookAtScreen, GuiAction::Scroll { dx: 0, dy: 10, at: None }]);
}

#[test]
fn spots_tool_calls_written_as_text() {
    let caps = waddle_core::tools::Capabilities { gui: true, ..Default::default() };
    let coords = waddle_core::tools::Coords { mode: waddle_core::config::CoordMode::Norm1000, screen_w: 1440.0, screen_h: 960.0 };
    let tools = waddle_core::tools::specs(caps, &coords);
    assert!(writes_tool_call("I'll point you to it.  point_at(x=920, y=240, label=\"toggle\")", &tools));
    assert!(writes_tool_call("Here.  point_at {\"x\": 920, \"y\": 242} </tool_call>", &tools));
    assert!(writes_tool_call("click (500, 300)", &tools));
    assert!(!writes_tool_call("I clicked Compose (the blue button) for you.", &tools));
    assert!(!writes_tool_call("Double-click {the file} to open it.", &tools), "click inside another word doesn't count");
    assert!(promises_action("I'll show you where it is."));
}
