//! Files beyond the workspace: reading anywhere the user allowed, writing only
//! where they allowed, documents as text, and deletes through the Recycle Bin
//! with a click.

mod common;

use common::FakeHost;
use serde_json::json;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use waddle_core::agent::{Agent, AgentDeps};
use waddle_core::audit::AuditLog;
use waddle_core::llm::mock::{call, reply, MockProvider};
use waddle_core::llm::Role;
use waddle_core::tools::docs;
use waddle_core::tools::fs::Workspace;
use waddle_core::{Decision, Host, Settings};

struct Fixture {
    dir: tempfile::TempDir,
    host: Arc<FakeHost>,
    provider: Arc<MockProvider>,
    deps: AgentDeps,
}

/// The script gets the temporary root, so it can use absolute paths into it.
fn fixture(script: impl FnOnce(&std::path::Path) -> Vec<waddle_core::llm::ChatResponse>) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let docs_dir = dir.path().join("Documents");
    let sorted = dir.path().join("Sorted");
    std::fs::create_dir_all(docs_dir.join("Taxes")).unwrap();
    std::fs::create_dir_all(&sorted).unwrap();
    std::fs::write(docs_dir.join("Taxes").join("Receipt 2026.docx"), docs::make_docx("Receipt\nTotal paid: £42.10").unwrap()).unwrap();
    std::fs::write(docs_dir.join("budget.csv"), "Item,Cost\nRent,900\n").unwrap();
    let host = FakeHost::new(Box::new(|_| Some(Decision::Approved)));
    let provider = Arc::new(MockProvider::scripted(script(dir.path())));
    let workspace = Workspace::new(dir.path().join("ws")).unwrap().with_roots(&[docs_dir], &[sorted]);
    let deps = AgentDeps {
        provider: provider.clone(),
        host: host.clone(),
        audit: Arc::new(AuditLog::open_in_memory().unwrap()),
        workspace: Arc::new(workspace),
        settings: Settings { model: "test-model".into(), tier2_countdown_ms: 20, look_first: false, ..Settings::default() },
        skills: None,
        reminders: None,
        facts: None,
        decider: None,
        self_source: None,
        selection: None,
        google: None,
        style: None,
        mcp: None,
        recalled: None,
    };
    Fixture { dir, host, provider, deps }
}

async fn run(f: &Fixture) {
    let env = f.host.env();
    let agent = Agent::new(&f.deps, "t".into(), CancellationToken::new(), Arc::default(), &env);
    let (_tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    agent.run("goal", &[], &mut rx).await;
}

fn tool_results(f: &Fixture) -> Vec<String> {
    let reqs = f.provider.requests.lock().unwrap();
    reqs.last().map(|m| m.iter().filter(|m| m.role == Role::Tool).map(|m| m.text.clone()).collect()).unwrap_or_default()
}

#[tokio::test]
async fn finds_and_reads_documents_in_the_users_folders() {
    let f = fixture(|root| {
        let receipt = root.join("Documents/Taxes/Receipt 2026.docx").display().to_string();
        vec![
            reply("", vec![call("find_files", json!({ "query": "receipt" }))]),
            reply("", vec![call("read_document", json!({ "path": receipt }))]),
            reply("You paid £42.10.", vec![]),
        ]
    });
    run(&f).await;
    let results = tool_results(&f);
    assert!(results[0].starts_with("<untrusted source=\"file_listing\"") && results[0].contains("Receipt 2026.docx"), "{}", results[0]);
    assert!(results[1].starts_with("<untrusted source=\"document\"") && results[1].contains("Total paid: £42.10"), "{}", results[1]);
    assert!(f.host.approvals.lock().unwrap().is_empty(), "reading needs no approval");
    let system = f.provider.requests.lock().unwrap()[0][0].text.clone();
    assert!(system.contains("you can read anywhere in") && system.contains("Documents"), "{system}");
}

#[tokio::test]
async fn writes_go_only_where_allowed_and_deletes_need_a_click() {
    let f = fixture(|root| {
        let (docs_dir, sorted) = (root.join("Documents"), root.join("Sorted"));
        vec![
            reply("", vec![call("create_document", json!({ "path": docs_dir.join("new.xlsx").display().to_string(), "kind": "xlsx", "content": "a,b" }))]),
            reply("", vec![call("create_document", json!({ "path": "summary.xlsx", "kind": "xlsx", "content": "Item,Cost\nRent,900" }))]),
            reply("", vec![call("move_file", json!({ "from": "summary.xlsx", "to": sorted.display().to_string() }))]),
            reply("", vec![call("delete_file", json!({ "path": sorted.join("summary.xlsx").display().to_string() }))]),
            reply("Tidied.", vec![]),
        ]
    });
    let sorted = f.dir.path().join("Sorted");
    run(&f).await;
    let results = tool_results(&f);
    assert!(results[0].contains("outside the folders Waddle may change"), "read folders aren't writable: {}", results[0]);
    assert!(results[1].starts_with("Created"), "{}", results[1]);
    assert!(results[2].starts_with("Moved"), "{}", results[2]);
    assert!(results[3].contains("Recycle Bin"), "{}", results[3]);
    assert!(!sorted.join("summary.xlsx").exists());
    let tiers: Vec<(String, u8)> = f.host.approvals.lock().unwrap().iter().map(|a| (a.tool.clone(), a.tier)).collect();
    assert_eq!(
        tiers,
        [("create_document".into(), 2), ("create_document".into(), 2), ("move_file".into(), 2), ("delete_file".into(), 3)],
        "only the delete waits for a click"
    );
}
