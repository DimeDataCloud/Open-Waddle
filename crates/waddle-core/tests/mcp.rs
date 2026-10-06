//! MCP tools against a small fake server (tests/fixtures/fake_mcp.py): the
//! handshake, paged tool lists, calls, errors, crashes, halts, and how the
//! planner offers and gates the tools.

mod common;
use common::FakeHost;
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use waddle_core::agent::{Agent, AgentDeps, TaskStatus};
use waddle_core::audit::AuditLog;
use waddle_core::llm::mock::{call, reply, MockProvider};
use waddle_core::mcp::{self, Launch, McpHub};
use waddle_core::tools::fs::Workspace;
use waddle_core::{Decision, Host, Outcome, Settings};

fn python() -> Option<&'static str> {
    ["python3", "python"].into_iter().find(|p| std::process::Command::new(p).arg("--version").output().is_ok_and(|o| o.status.success()))
}

fn launch(dir: &std::path::Path, trusted: bool) -> Option<Launch> {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_mcp.py");
    Some(Launch {
        name: "fake".into(),
        command: python()?.into(),
        args: vec![fixture.display().to_string()],
        env: vec![("NOTES".into(), dir.join("notes.txt").display().to_string())],
        trusted,
    })
}

#[tokio::test]
async fn lists_calls_and_survives_a_crash() {
    let dir = tempfile::tempdir().unwrap();
    let Some(l) = launch(dir.path(), false) else { return eprintln!("no python; skipped") };
    let cache = dir.path().join("mcp_tools.json");
    let hub = McpHub::new(Some(cache.clone()));
    hub.configure(vec![l.clone()]).await;
    assert!(hub.specs().is_empty(), "nothing is offered before the tools are listed");
    assert_eq!(hub.unlisted(), vec!["fake".to_string()]);

    let tools = hub.refresh("fake", &CancellationToken::new()).await.unwrap();
    assert_eq!(tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), vec!["echo", "write_note", "fails", "crash", "slow", "ask"], "both pages");
    let specs = hub.specs();
    assert_eq!(specs.len(), 6);
    let echo = specs.iter().find(|s| s.name == "mcp_fake_echo").unwrap();
    assert_eq!(echo.description, "Repeats the text. (from the fake MCP server)");
    assert!(echo.parameters.get("$schema").is_none());
    assert!(hub.unlisted().is_empty());

    let cancel = CancellationToken::new();
    let out = hub.call("mcp_fake_echo", &json!({ "text": "hi" }), &cancel).await.unwrap();
    assert_eq!((out.text.as_str(), out.untrusted_source), ("From fake:\necho: hi", Some("mcp")));
    let out = hub.call("mcp_fake_fails", &json!({}), &cancel).await.unwrap();
    assert_eq!(out.text, "fake reported an error: no such thing");
    let out = hub.call("mcp_fake_ask", &json!({}), &cancel).await.unwrap();
    assert_eq!(out.text, "From fake:\npong ok", "the server's ping is answered");

    // A crash fails that call; the next one starts the server again.
    let err = hub.call("mcp_fake_crash", &json!({}), &cancel).await.unwrap_err();
    assert!(format!("{err:#}").contains("stopped"), "{err:#}");
    let out = hub.call("mcp_fake_echo", &json!({ "text": "again" }), &cancel).await.unwrap();
    assert_eq!(out.text, "From fake:\necho: again");

    // A fresh start offers the cached tools without starting anything.
    let again = McpHub::new(Some(cache));
    again.configure(vec![l.clone()]).await;
    assert_eq!(again.specs().len(), 6);
    assert_eq!(again.running_count().await, 0);
    // Changing how the server starts makes its list stale.
    let changed = Launch { args: vec![l.args[0].clone(), "--flag".into()], ..l };
    again.configure(vec![changed]).await;
    assert!(again.specs().is_empty());

    hub.stop_idle(Duration::ZERO).await;
    assert_eq!(hub.running_count().await, 0, "idle servers stop");
}

#[tokio::test]
async fn a_halt_stops_a_slow_tool_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let Some(l) = launch(dir.path(), false) else { return };
    let hub = McpHub::new(None);
    hub.configure(vec![l]).await;
    hub.refresh("fake", &CancellationToken::new()).await.unwrap();
    let cancel = CancellationToken::new();
    let c = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        c.cancel();
    });
    let started = std::time::Instant::now();
    let err = hub.call("mcp_fake_slow", &json!({}), &cancel).await.unwrap_err();
    assert!(format!("{err:#}").contains("stopped"), "{err:#}");
    assert!(started.elapsed() < Duration::from_secs(3));
    hub.stop_all().await;
}

#[tokio::test]
async fn probe_lists_and_bad_commands_say_so() {
    let dir = tempfile::tempdir().unwrap();
    let Some(l) = launch(dir.path(), false) else { return };
    assert_eq!(mcp::probe(&l).await.unwrap().len(), 6);
    let bad = Launch { command: "no-such-program-waddle".into(), args: vec![], ..l };
    let err = mcp::probe(&bad).await.unwrap_err();
    assert!(format!("{err:#}").contains("couldn't start `no-such-program-waddle`"), "{err:#}");
}

async fn run_with(hub: Arc<McpHub>, host: Arc<FakeHost>, provider: Arc<MockProvider>) -> Outcome {
    let dir = tempfile::tempdir().unwrap();
    let workspace = Arc::new(Workspace::new(dir.path().join("ws")).unwrap());
    let settings = Settings { model: "test-model".into(), tier2_countdown_ms: 20, look_first: false, ..Settings::default() };
    let deps = AgentDeps {
        provider,
        host: host.clone(),
        audit: Arc::new(AuditLog::open_in_memory().unwrap()),
        workspace,
        settings,
        skills: None,
        reminders: None,
        facts: None,
        decider: None,
        self_source: None,
        selection: None,
        google: None,
        style: None,
        mcp: Some(hub),
    };
    let env = host.env();
    let agent = Agent::new(&deps, "t1".into(), CancellationToken::new(), Arc::new(Mutex::new(TaskStatus::default())), &env);
    let (_tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    agent.run("note it", &[], &mut rx).await.outcome
}

#[tokio::test]
async fn the_planner_gets_the_tools_and_tiers_follow_trust() {
    let dir = tempfile::tempdir().unwrap();
    let Some(untrusted) = launch(dir.path(), false) else { return };
    let script = || {
        Arc::new(MockProvider::scripted(vec![
            reply("", vec![call("mcp_fake_echo", json!({ "text": "hello" }))]),
            reply("", vec![call("mcp_fake_write_note", json!({ "note": "buy milk" }))]),
            reply("Saved.", vec![]),
        ]))
    };
    for (trusted, expected) in [(false, vec![("mcp_fake_echo", 2), ("mcp_fake_write_note", 3)]), (true, vec![("mcp_fake_write_note", 2)])] {
        let hub = McpHub::new(None);
        hub.configure(vec![Launch { trusted, ..untrusted.clone() }]).await;
        hub.refresh("fake", &CancellationToken::new()).await.unwrap();
        let host = FakeHost::new(Box::new(|_| Some(Decision::Approved)));
        let provider = script();
        assert_eq!(run_with(hub.clone(), host.clone(), provider.clone()).await, Outcome::Done);
        let asked: Vec<(String, u8)> = host.approvals.lock().unwrap().iter().map(|a| (a.tool.clone(), a.tier)).collect();
        let expected: Vec<(String, u8)> = expected.into_iter().map(|(t, n)| (t.to_string(), n)).collect();
        assert_eq!(asked, expected, "trusted={trusted}");
        // The tools were offered, and the echo came back as untrusted text.
        let second = provider.requests.lock().unwrap()[1].clone();
        let echoed = second.iter().rev().find(|m| m.text.contains("echo: hello")).expect("echo result");
        assert!(echoed.text.contains("untrusted"), "{}", echoed.text);
        hub.stop_all().await;
    }
    assert_eq!(std::fs::read_to_string(dir.path().join("notes.txt")).unwrap(), "buy milk\nbuy milk\n");
}

#[tokio::test]
async fn a_denied_call_never_reaches_the_server() {
    let dir = tempfile::tempdir().unwrap();
    let Some(l) = launch(dir.path(), false) else { return };
    let hub = McpHub::new(None);
    hub.configure(vec![l]).await;
    hub.refresh("fake", &CancellationToken::new()).await.unwrap();
    let provider = Arc::new(MockProvider::scripted(vec![reply("", vec![call("mcp_fake_write_note", json!({ "note": "secret" }))]), reply("Okay, I won't.", vec![])]));
    let host = FakeHost::new(Box::new(|_| Some(Decision::Denied)));
    run_with(hub.clone(), host, provider).await;
    assert!(!dir.path().join("notes.txt").exists());
    hub.stop_all().await;
}
