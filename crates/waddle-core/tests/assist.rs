//! Assistant benchmark (text only): mail, calendar, contacts, files, documents,
//! memory and Chrome tasks against a fake Google, a temporary folder and a fake
//! Chrome extension. Checks look at what actually happened (emails sent, events
//! made, files written), not just the reply. Ignored by default: it spends money
//! (about $0.005-0.03 per model per pass).
//!
//!   OPENROUTER_API_KEY=... WADDLE_BENCH_MODELS=openai/gpt-6-luna,qwen/qwen3-vl-8b-instruct \
//!     cargo test -p waddle-core --test assist -- --ignored --nocapture
//!
//! Optional: WADDLE_BENCH_REPEAT (default 1), WADDLE_BENCH_ONLY=id,id,
//! WADDLE_BENCH_CONCURRENCY (default 3). Results go to bench/results/<model>@assist.json.

mod common;

use common::fake_google::FakeGoogle;
use common::FakeHost;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio_util::sync::CancellationToken;

use waddle_core::agent::{Agent, AgentDeps};
use waddle_core::audit::AuditLog;
use waddle_core::facts::FactStore;
use waddle_core::google::style::StyleNote;
use waddle_core::llm::{build_provider, ChatRequest, ChatResponse, EventSink, Provider};
use waddle_core::tools::docs;
use waddle_core::tools::fs::Workspace;
use waddle_core::{Decision, Settings};

/// Waits and tries again after a rate limit (new OpenRouter accounts get 20 requests a minute per model).
struct Retry(Arc<dyn Provider>);

#[async_trait::async_trait]
impl Provider for Retry {
    async fn chat(&self, req: ChatRequest<'_>, on_event: EventSink<'_>) -> anyhow::Result<ChatResponse> {
        for attempt in 0..4 {
            match self.0.chat(req, on_event).await {
                Err(e) if attempt < 3 && e.to_string().contains("429") => tokio::time::sleep(std::time::Duration::from_secs(15)).await,
                r => return r,
            }
        }
        unreachable!()
    }
}

struct World {
    fg: FakeGoogle,
    host: Arc<FakeHost>,
    dir: tempfile::TempDir,
    facts: Arc<FactStore>,
}

fn local(offset: chrono::Duration) -> String {
    (chrono::Local::now() + offset).to_rfc3339()
}

fn tiny_pdf(lines: &[&str]) -> Vec<u8> {
    let stream: String = lines.iter().enumerate().map(|(i, l)| format!("BT /F1 14 Tf 20 {} Td ({l}) Tj ET\n", 200 - i * 24)).collect();
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 240] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{stream}endstream", stream.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
    ];
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![];
    for (i, o) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for off in offsets {
        pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    pdf
}

async fn world() -> World {
    let fg = FakeGoogle::start().await;
    let now = chrono::Utc::now().timestamp_millis();
    fg.add_message("m1", "Ana Lee <ana@example.com>", "Coffee on Thursday?", "Are you free for coffee on Thursday at 3?", &["INBOX", "UNREAD"], now - 3_600_000);
    fg.add_message("m2", "Priya Shah <priya@client.example>", "Urgent: invoice needed today", "Finance closes the payment run at 4pm, can you send the invoice before then?", &["INBOX", "UNREAD"], now - 1_800_000);
    fg.add_message("m3", "Medium Daily Digest <noreply@medium.com>", "10 Python tricks you didn't know", "Today's highlights from writers you follow", &["INBOX", "UNREAD"], now - 7_200_000);
    fg.add_message("m4", "Shop Example <deals@shop.example>", "48 hours only: 30% off everything", "Our biggest sale of the season starts now", &["INBOX", "UNREAD"], now - 5_400_000);
    fg.add_event(json!({ "id": "past", "summary": "Standup", "status": "confirmed", "start": { "dateTime": local(chrono::Duration::hours(-3)) }, "end": { "dateTime": local(chrono::Duration::hours(-2)) } }));
    fg.add_event(json!({
        "id": "e1", "summary": "Design review", "status": "confirmed",
        "start": { "dateTime": local(chrono::Duration::minutes(70)) }, "end": { "dateTime": local(chrono::Duration::minutes(100)) },
        "hangoutLink": "https://meet.google.com/abc-defg-hij",
        "attendees": [{ "email": "me@example.com", "self": true, "responseStatus": "accepted" }, { "email": "ana@example.com", "displayName": "Ana Lee", "responseStatus": "accepted" }]
    }));
    fg.add_contact("Sam Lee", "sam@example.com", true);
    fg.add_contact("Ana Lee", "ana@example.com", true);

    let dir = tempfile::tempdir().unwrap();
    let docs_dir = dir.path().join("Documents");
    std::fs::create_dir_all(docs_dir.join("Home")).unwrap();
    std::fs::write(docs_dir.join("Home").join("Lease 2026.pdf"), tiny_pdf(&["Tenancy agreement", "Flat 4, 12 River Street", "Monthly rent: GBP 1,250"])).unwrap();
    std::fs::write(docs_dir.join("budget.xlsx"), docs::make_xlsx("Category,Amount\nRent,1250\nGroceries,320\nTravel,90\n").unwrap()).unwrap();

    let host = FakeHost::new(Box::new(|req| Some(if req.tool == "delete_file" { Decision::Denied } else { Decision::Approved })));
    *host.gui.lock().unwrap() = false;
    {
        let mut r = host.browser_replies.lock().unwrap();
        r.insert("read".into(), json!({ "title": "Sign up - Duck Shop", "url": "https://shop.example/signup", "elements": [
            { "id": "e1", "role": "textbox", "name": "Email address" },
            { "id": "e2", "role": "checkbox", "name": "Send me the newsletter", "value": "unchecked" },
            { "id": "e3", "role": "button", "name": "Create account" }
        ]}));
        r.insert("read:text".into(), json!({ "title": "louvre opening hours - Google Search", "url": "https://www.google.com/search?q=louvre+opening+hours",
            "text": "Musée du Louvre\nOpening hours: 9 am – 6 pm (Wednesday and Friday until 9 pm). Closed on Tuesdays.\nTickets: €22, free for under-18s." }));
        r.insert("type".into(), json!(true));
        r.insert("click".into(), json!(true));
        r.insert("locate".into(), json!({ "screen_x": 300.0, "screen_y": 200.0 }));
        r.insert("navigate".into(), json!({ "title": "Search results", "url": "https://www.google.com/search?q=x" }));
        r.insert("tabs".into(), json!([{ "id": 7, "title": "Sign up - Duck Shop", "url": "https://shop.example/signup", "active": true }]));
    }
    let facts = Arc::new(FactStore::new(dir.path().join("facts.json")));
    World { fg, host, dir, facts }
}

type Check = fn(&World, &str) -> Result<(), String>;

fn need(ok: bool, why: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(why.to_string()) }
}

fn sent(w: &World) -> Vec<String> {
    w.fg.sent()
}

fn labels(w: &World, id: &str) -> String {
    w.fg.state.lock().unwrap().messages.iter().find(|m| m["id"] == id).map(|m| m["labelIds"].to_string()).unwrap_or_default()
}

fn created_events(w: &World) -> Vec<Value> {
    w.fg.requests("/events").into_iter().filter(|r| r.method == "POST").filter_map(|r| serde_json::from_str(&r.body).ok()).collect()
}

fn browser(w: &World, cmd: &str) -> Vec<Value> {
    w.host.browser_calls.lock().unwrap().iter().filter(|(c, _)| c == cmd).map(|(_, a)| a.clone()).collect()
}

const TASKS: &[(&str, &str, Check)] = &[
    ("next_meeting", "What's my next meeting?", |_, m| {
        need(m.to_lowercase().contains("design review") && !m.contains("Standup"), "names the design review")
    }),
    ("important_mail", "Do I have any important unread email? Just tell me who it's from.", |_, m| need(m.contains("Priya"), "mentions Priya's urgent email")),
    ("reply", "Reply to Ana's email saying Thursday at 3 works for me.", |w, _| {
        let s = sent(w);
        need(s.len() == 1, "exactly one email sent")?;
        need(s[0].contains("ana@example.com") && s[0].contains("In-Reply-To: <m1@"), "a threaded reply to Ana")?;
        need(s[0].to_lowercase().contains("thursday"), "says Thursday")
    }),
    ("archive", "Archive the newsletter and the promotion in my inbox.", |w, _| {
        need(!labels(w, "m3").contains("INBOX") && !labels(w, "m4").contains("INBOX"), "both archived")?;
        need(labels(w, "m1").contains("INBOX") && labels(w, "m2").contains("INBOX"), "the real emails stay")
    }),
    ("book", "Book 30 minutes with Sam Lee tomorrow at 2pm, with a Meet link.", |w, _| {
        let events = created_events(w);
        need(events.len() == 1, "one event created")?;
        let e = &events[0];
        need(e["attendees"].to_string().contains("sam@example.com"), "invites Sam")?;
        need(e["start"]["dateTime"].as_str().unwrap_or("").contains("T14:00"), "at 2pm")?;
        need(e.get("conferenceData").is_some(), "with a Meet link")
    }),
    ("lease", "Find my lease PDF and tell me the monthly rent.", |_, m| need(m.contains("1,250") || m.contains("1250"), "says 1,250")),
    ("budget", "How much is budgeted for groceries in the budget spreadsheet in my documents?", |_, m| need(m.contains("320"), "says 320")),
    ("signup", "In Chrome, put test@example.com in the email field and click Create account.", |w, _| {
        need(browser(w, "type").iter().any(|a| a["element"] == "e1" && a["text"] == "test@example.com"), "types the email into e1")?;
        need(browser(w, "locate").iter().chain(browser(w, "click").iter()).any(|a| a["element"] == "e3"), "clicks Create account (e3)")
    }),
    ("search", "Search the web for the Louvre's opening hours.", |w, m| {
        need(m.contains('9') && (m.contains('6') || m.contains("18")), "answers with the hours from the results")?;
        let nav = browser(w, "navigate");
        need(nav.iter().any(|a| { let u = a["url"].as_str().unwrap_or("").to_lowercase(); u.contains("google.com/search") && u.contains("louvre") }), "opens a Google search for the Louvre")?;
        need(!browser(w, "read").is_empty(), "reads the results")
    }),
    ("remember", "Remember that my manager is Priya Shah.", |w, _| need(w.facts.list().iter().any(|f| f.text.contains("Priya")), "saved the fact")),
    ("spreadsheet", "Make a spreadsheet expenses.xlsx in your workspace with columns Item and Cost: coffee 3.50, lunch 12.", |w, _| {
        let path = w.dir.path().join("ws").join("expenses.xlsx");
        let text = docs::read_document(&path, None).map_err(|e| format!("no expenses.xlsx: {e}"))?;
        need(text.to_lowercase().contains("lunch") && text.contains("12"), "has the rows")
    }),
    ("delete_guard", "Delete the lease PDF.", |w, _| {
        let asked = w.host.approvals.lock().unwrap().iter().any(|a| a.tool == "delete_file" && a.tier == 3);
        need(asked, "asks before deleting")?;
        need(w.dir.path().join("Documents/Home/Lease 2026.pdf").exists(), "nothing deleted when the user says no")
    }),
];

struct Result1 {
    id: &'static str,
    pass: bool,
    note: String,
    secs: f64,
    cost: f64,
    steps: usize,
}

async fn run_task(model: &str, key: &str, (id, goal, check): &(&'static str, &'static str, Check)) -> Result1 {
    let w = world().await;
    let settings = Settings {
        model: model.into(),
        look_first: false,
        smart_look: false,
        tier2_countdown_ms: 1,
        max_steps: 12,
        no_training: true,
        ..Settings::default()
    };
    let workspace = Workspace::new(w.dir.path().join("ws")).unwrap().with_roots(&[w.dir.path().join("Documents")], &[]);
    let deps = AgentDeps {
        provider: Arc::new(Retry(build_provider(&settings, Some(key.to_string())))),
        host: w.host.clone(),
        audit: Arc::new(AuditLog::open_in_memory().unwrap()),
        workspace: Arc::new(workspace),
        settings,
        skills: None,
        reminders: None,
        facts: Some(w.facts.clone()),
        decider: None,
        self_source: None,
        selection: None,
        google: Some(w.fg.google()),
        style: Some(Arc::new(StyleNote::new(w.dir.path().join("style.md")))),
        mcp: None,
    };
    let env = waddle_core::Host::env(w.host.as_ref());
    let agent = Agent::new(&deps, format!("bench-{id}"), CancellationToken::new(), Arc::default(), &env);
    let (_tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let started = Instant::now();
    let r = tokio::time::timeout(std::time::Duration::from_secs(120), agent.run(goal, &[], &mut rx)).await;
    let secs = started.elapsed().as_secs_f64();
    let usage = agent.usage();
    let steps = agent.timing().steps.len();
    let (pass, note) = match r {
        Err(_) => (false, "timed out".to_string()),
        Ok(r) if r.outcome == waddle_core::Outcome::StepLimit => (false, "ran out of steps".to_string()),
        Ok(r) if r.message.contains("couldn't reach my brain") => (false, format!("provider error: {}", r.message.chars().take(100).collect::<String>())),
        Ok(r) => match check(&w, &r.message) {
            Ok(()) => (true, r.message.chars().take(120).collect()),
            Err(why) => (false, format!("{why} — {:?}: {}", r.outcome, r.message.chars().take(120).collect::<String>())),
        },
    };
    Result1 { id, pass, note, secs, cost: usage.cost.unwrap_or(0.0), steps }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "spends money: needs OPENROUTER_API_KEY and WADDLE_BENCH_MODELS"]
async fn assistant_tasks() {
    let key = std::env::var("OPENROUTER_API_KEY").expect("OPENROUTER_API_KEY");
    let models: Vec<String> = std::env::var("WADDLE_BENCH_MODELS").expect("WADDLE_BENCH_MODELS").split(',').map(|s| s.trim().to_string()).collect();
    let repeat: usize = std::env::var("WADDLE_BENCH_REPEAT").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    let only: Option<Vec<String>> = std::env::var("WADDLE_BENCH_ONLY").ok().map(|v| v.split(',').map(str::to_string).collect());
    let limit = Arc::new(tokio::sync::Semaphore::new(std::env::var("WADDLE_BENCH_CONCURRENCY").ok().and_then(|v| v.parse().ok()).unwrap_or(3)));
    let tasks: Vec<_> = TASKS.iter().filter(|t| only.as_ref().is_none_or(|o| o.iter().any(|id| id == t.0))).collect();
    for model in &models {
        let runs = futures_util::future::join_all((0..repeat).flat_map(|_| tasks.iter()).map(|t| {
            let (key, model, limit) = (key.clone(), model.clone(), limit.clone());
            async move {
                let _permit = limit.acquire().await.unwrap();
                run_task(&model, &key, t).await
            }
        }))
        .await;
        let passed = runs.iter().filter(|r| r.pass).count();
        let secs: f64 = runs.iter().map(|r| r.secs).sum::<f64>() / runs.len() as f64;
        let cost: f64 = runs.iter().map(|r| r.cost).sum::<f64>() / runs.len() as f64;
        for r in &runs {
            println!("{} {:<15} {:>5.1}s {} steps ${:.5}  {}", if r.pass { "✓" } else { "✗" }, r.id, r.secs, r.steps, r.cost, r.note.replace('\n', " "));
        }
        println!("== {model}: {passed}/{} passed, {secs:.1} s and ${cost:.5} per task", runs.len());
        let out = json!({
            "model": model, "passed": passed, "runs": runs.len(), "secs": secs, "cost": cost,
            "results": runs.iter().map(|r| json!({ "id": r.id, "pass": r.pass, "note": r.note, "secs": r.secs, "cost": r.cost, "steps": r.steps })).collect::<Vec<_>>()
        });
        // A partial run (WADDLE_BENCH_ONLY) mustn't overwrite the full results.
        let suffix = if only.is_some() { "assist-partial" } else { "assist" };
        let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../bench/results")).join(format!("{}@{suffix}.json", model.replace('/', "__")));
        let _ = std::fs::write(path, serde_json::to_string_pretty(&out).unwrap());
    }
}
