//! Model benchmark: runs the tasks in bench/tasks.json against one or more
//! OpenRouter models and prints success rate, time and cost per model.
//! Ignored by default (it spends money: about $0.01-0.05 per model).
//!
//!   OPENROUTER_API_KEY=... WADDLE_BENCH_MODELS=qwen/qwen3-vl-8b-instruct,google/gemini-3.1-flash-lite \
//!     cargo test -p waddle-core --test bench -- --ignored --nocapture
//!
//! Optional: WADDLE_BENCH_REPEAT (default 1), WADDLE_BENCH_ONLY=id,id,
//! WADDLE_BENCH_REASONING=off|low|medium|high, WADDLE_BENCH_COORDS=pixels|norm1000
//! (default: Waddle's own choice for the model). Results go to bench/results/.
//!
//! Training data: WADDLE_BENCH_TRACES=<dir> saves every passing run as a rated
//! training trace (rejection sampling: a strong model's successes become
//! examples for a small one). Use task sets other than these for training.
//!
//! Local models: WADDLE_BENCH_PROVIDER=ollama (WADDLE_OLLAMA_URL, default
//! http://localhost:11434) runs the tasks one at a time with a longer timeout.

mod common;

use base64::Engine;
use common::FakeHost;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio_util::sync::CancellationToken;

use waddle_core::agent::{Agent, AgentDeps, TaskStatus};
use waddle_core::audit::AuditLog;
use waddle_core::config::{CoordMode, ProviderKind, Reasoning};
use waddle_core::llm::{build_provider, ChatRequest, ChatResponse, EventSink, ImageData, Provider};
use waddle_core::tools::fs::Workspace;
use waddle_core::tools::{ElementInfo, GuiAction, WindowInfo};
use waddle_core::{Decision, Host, Outcome, Settings};

type Box4 = [f64; 4];

fn bench_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../bench"))
}

struct Suite {
    tasks: Vec<Value>,
    boxes: HashMap<String, HashMap<String, Box4>>,
    windows: HashMap<String, String>,
}

fn load() -> Suite {
    let dir = bench_dir();
    let spec: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("tasks.json")).unwrap()).unwrap();
    let mut boxes: HashMap<String, HashMap<String, Box4>> =
        serde_json::from_str(&std::fs::read_to_string(dir.join("screens/boxes.json")).unwrap()).unwrap();
    let extra: HashMap<String, HashMap<String, Box4>> = serde_json::from_value(spec["extra_boxes"].clone()).unwrap();
    for (screen, b) in extra {
        boxes.entry(screen).or_default().extend(b);
    }
    let windows = serde_json::from_value(spec["windows"].clone()).unwrap();
    let only: Option<Vec<String>> = std::env::var("WADDLE_BENCH_ONLY").ok().map(|v| v.split(',').map(str::to_string).collect());
    let tasks = spec["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| only.as_ref().is_none_or(|o| o.iter().any(|id| t["id"] == id.as_str())))
        .cloned()
        .collect();
    Suite { tasks, boxes, windows }
}

/// Spaces out model calls to stay under a requests-per-minute cap
/// (`WADDLE_BENCH_RPM`; new OpenRouter accounts get 20 per model), and retries
/// once after a 429. Waiting is not counted in a task's time.
struct Paced {
    inner: Arc<dyn Provider>,
    gap: std::time::Duration,
    waited: Arc<Mutex<f64>>,
}

static NEXT_SLOT: tokio::sync::Mutex<Option<Instant>> = tokio::sync::Mutex::const_new(None);

impl Paced {
    async fn slot(&self) {
        let started = Instant::now();
        let mut next = NEXT_SLOT.lock().await;
        let at = next.unwrap_or(started).max(started);
        *next = Some(at + self.gap);
        drop(next);
        tokio::time::sleep_until(at.into()).await;
        *self.waited.lock().unwrap() += started.elapsed().as_secs_f64();
    }
}

#[async_trait::async_trait]
impl Provider for Paced {
    async fn chat(&self, req: ChatRequest<'_>, on_event: EventSink<'_>) -> anyhow::Result<ChatResponse> {
        self.slot().await;
        let req2 = req;
        match self.inner.chat(req, on_event).await {
            Err(e) if e.to_string().contains("429") => {
                let t = Instant::now();
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                *self.waited.lock().unwrap() += t.elapsed().as_secs_f64();
                self.slot().await;
                self.inner.chat(req2, &mut |_| {}).await
            }
            r => r,
        }
    }
}

fn screen(name: &str) -> ImageData {
    let png = std::fs::read(bench_dir().join(format!("screens/{name}.png"))).unwrap();
    ImageData { mime: "image/png".into(), base64: base64::engine::general_purpose::STANDARD.encode(png) }
}

fn inside((x, y): (f64, f64), b: &Box4) -> bool {
    (b[0]..=b[2]).contains(&x) && (b[1]..=b[3]).contains(&y)
}

fn norm_keys(k: &str) -> String {
    k.to_lowercase().replace([' ', '-'], "").replace("control", "ctrl").replace("return", "enter")
}

/// Whether action `i` satisfies one alternative of a step. `typed_at` tracks
/// where the keyboard focus is (the last click or type position).
fn satisfies(alt: &Value, action: &GuiAction, focus: Option<(f64, f64)>, boxes: &HashMap<String, Box4>) -> bool {
    let bx = |k: &str| alt[k].as_str().and_then(|n| boxes.get(n));
    match action {
        GuiAction::Click { x, y, .. } => bx("click").is_some_and(|b| inside((*x, *y), b)),
        GuiAction::ClickElement { id } => alt["element"].as_u64() == Some(*id as u64),
        GuiAction::TypeText { text, at } => {
            if let Some(want) = alt["type"].as_str() {
                let at = at.or(focus);
                let place_ok = bx("into").is_none_or(|b| at.is_some_and(|p| inside(p, b)));
                return text.to_lowercase().contains(&want.to_lowercase()) && place_ok;
            }
            alt["keys"] == "enter" && text.ends_with('\n')
        }
        GuiAction::PressKeys { keys } => alt["keys"].as_str().is_some_and(|k| norm_keys(keys) == norm_keys(k)),
        GuiAction::PointAt { x, y, .. } => bx("point").is_some_and(|b| inside((*x, *y), b)),
        GuiAction::Scroll { dx, dy, at } => {
            let dir_ok = match alt["scroll"].as_str() {
                Some("down") => *dy > 0,
                Some("up") => *dy < 0,
                Some("right") => *dx > 0,
                Some("left") => *dx < 0,
                _ => false,
            };
            dir_ok && bx("at").is_none_or(|b| at.is_none_or(|p| inside(p, b)))
        }
        GuiAction::Drag { from, to } => bx("drag").is_some_and(|b| inside(*from, b)) && bx("to").is_some_and(|b| inside(*to, b)),
        GuiAction::WriteClipboard { text } => alt["clipboard"].as_str().is_some_and(|w| text.to_lowercase().contains(&w.to_lowercase())),
        _ => false,
    }
}

/// Matches the expected steps, in order, against what the agent did.
fn check_steps(expect: &[Value], actions: &[GuiAction], boxes: &HashMap<String, Box4>) -> bool {
    let mut step = 0;
    let mut focus = None;
    for a in actions {
        if step < expect.len() && expect[step].as_array().unwrap().iter().any(|alt| satisfies(alt, a, focus, boxes)) {
            step += 1;
        }
        if let Some(p) = a.target() {
            focus = Some(p);
        }
    }
    step == expect.len()
}

struct TaskResult {
    id: String,
    pass: bool,
    /// Did the right things, but may have kept re-checking the (static) test screen until the step limit.
    hit: bool,
    secs: f64,
    /// Time spent waiting on the model (the rest is looking and tools).
    model_secs: f64,
    cost: f64,
    tokens: u64,
    steps: usize,
    note: String,
}

async fn run_task(model: &str, task: &Value, suite: &Suite, key: &str) -> TaskResult {
    let id = task["id"].as_str().unwrap().to_string();
    let dir = tempfile::tempdir().unwrap();
    let workspace = Arc::new(Workspace::new(dir.path().join("ws")).unwrap());
    let reminders = Arc::new(waddle_core::reminders::ReminderStore::new(dir.path().join("reminders.json")));
    let host = FakeHost::new(Box::new(|_| Some(Decision::Approved)));
    *host.os.lock().unwrap() = "windows".into();
    let screen_name = task["screen"].as_str().unwrap_or("dialog");
    *host.screenshot.lock().unwrap() = Some(screen(screen_name));
    let empty = HashMap::new();
    let boxes = suite.boxes.get(screen_name).unwrap_or(&empty);
    if let Some(title) = suite.windows.get(screen_name) {
        let app = if title.contains("Chrome") { "chrome" } else { title.rsplit(" - ").next().unwrap_or(title) };
        *host.windows.lock().unwrap() = vec![WindowInfo { title: title.clone(), app: app.to_lowercase(), x: 0.0, y: 0.0, w: 1440.0, h: 960.0, focused: true }];
    }
    if let Some(els) = task["elements"].as_array() {
        *host.elements.lock().unwrap() = els
            .iter()
            .map(|e| {
                let b = e[3].as_str().and_then(|n| boxes.get(n)).copied().unwrap_or([300.0, 300.0, 1100.0, 760.0]);
                ElementInfo { id: e[0].as_u64().unwrap() as u32, role: e[1].as_str().unwrap().into(), name: e[2].as_str().unwrap().into(), x: b[0], y: b[1], w: b[2] - b[0], h: b[3] - b[1] }
            })
            .collect();
    }

    let mut settings = Settings { model: model.into(), tier2_countdown_ms: 10, max_steps: 6, ..Settings::default() };
    if local() {
        settings.provider = ProviderKind::Ollama;
        settings.base_url = std::env::var("WADDLE_OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".into());
        settings.ollama.num_thread = std::thread::available_parallelism().ok().map(|n| n.get() as u32);
    }
    match std::env::var("WADDLE_BENCH_REASONING").as_deref() {
        Ok("off") => settings.reasoning = Reasoning::Off,
        Ok("low") => settings.reasoning = Reasoning::Low,
        Ok("medium") => settings.reasoning = Reasoning::Medium,
        Ok("high") => settings.reasoning = Reasoning::High,
        _ => {}
    }
    match std::env::var("WADDLE_BENCH_COORDS").as_deref() {
        Ok("pixels") => settings.coord_mode = CoordMode::Pixels,
        Ok("norm1000") => settings.coord_mode = CoordMode::Norm1000,
        _ => {}
    }
    let waited = Arc::new(Mutex::new(0.0));
    let mut provider = build_provider(&settings, Some(key.to_string()));
    if let Some(rpm) = std::env::var("WADDLE_BENCH_RPM").ok().and_then(|v| v.parse::<f64>().ok()) {
        provider = Arc::new(Paced { inner: provider, gap: std::time::Duration::from_secs_f64(60.0 / rpm), waited: waited.clone() });
    }
    let decider = waddle_core::decide::Jev::for_settings(&settings, Some(key)).map(|j| Arc::new(j) as Arc<dyn waddle_core::decide::Decider>);
    let deps = AgentDeps {
        provider,
        host: host.clone(),
        audit: Arc::new(AuditLog::open_in_memory().unwrap()),
        workspace: workspace.clone(),
        settings,
        skills: None,
        reminders: Some(reminders.clone()),
        facts: None,
        decider,
        self_source: None,
        selection: None,
        google: None,
        style: None,
    };
    let env = host.env();
    let agent = Agent::new(&deps, id.clone(), CancellationToken::new(), Arc::new(Mutex::new(TaskStatus::default())), &env);
    let (_tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let started = Instant::now();
    let goal = task["goal"].as_str().unwrap();
    let limit = if local() { 900 } else { 120 };
    let r = match tokio::time::timeout(std::time::Duration::from_secs(limit), agent.run(goal, &[], &mut rx)).await {
        Ok(r) => r,
        Err(_) => waddle_core::agent::RunResult { outcome: Outcome::TimedOut, message: "timed out".into() },
    };
    let secs = started.elapsed().as_secs_f64() - *waited.lock().unwrap();
    let usage = agent.usage();
    let model_secs = (agent.timing().model_ms() as f64 / 1000.0 - *waited.lock().unwrap()).max(0.0);
    let actions = host.gui_calls.lock().unwrap().clone();
    let steps = host.timeline.lock().unwrap().iter().filter(|(_, l)| l == "model call").count();

    // The fake screen never changes after a click, so a model that checks its work
    // sees "nothing happened" and may retry until the step limit. `hit` forgives that.
    let provider_error = r.message.starts_with("I couldn't reach my brain");
    let finished = (r.outcome == Outcome::Done || r.outcome == Outcome::Failed) && !provider_error;
    let mut pass = true;
    if let Some(expect) = task["expect"].as_array() {
        pass &= check_steps(expect, &actions, boxes);
    }
    if let Some(want) = task["answer"].as_str() {
        pass &= r.message.to_lowercase().contains(want);
    }
    if let Some(want) = task["reminder"].as_str() {
        pass &= reminders.list().iter().any(|r| r.text.to_lowercase().contains(want));
    }
    if task["no_input"] == true {
        pass &= !actions.iter().any(GuiAction::is_blind_input);
    }
    if let Some(f) = task["file"].as_array() {
        let body = std::fs::read_to_string(workspace.root().join(f[0].as_str().unwrap())).unwrap_or_default();
        pass &= body.trim().eq_ignore_ascii_case(f[1].as_str().unwrap());
    }
    let hit = pass && !provider_error && r.outcome != Outcome::TimedOut;
    let pass = pass && finished;
    if let (true, Ok(dir)) = (pass, std::env::var("WADDLE_BENCH_TRACES")) {
        let store = waddle_core::traces::TraceStore::new(dir.into());
        let (messages, tools) = agent.transcript();
        static RUN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = RUN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let tid = format!("{}_{}_{}_{n}", id.replace('-', "_"), model.replace(|c: char| !c.is_ascii_alphanumeric(), "_"), std::process::id());
        let mut meta = waddle_core::traces::TraceMeta::new(&tid, goal, model, r.outcome, &r.message, usage);
        meta.rating = Some(true);
        if let Err(e) = store.save(&meta, &messages, &tools) {
            eprintln!("couldn't save trace: {e:#}");
        }
    }
    let did: Vec<String> = actions
        .iter()
        .map(|a| match a {
            GuiAction::Click { x, y, .. } => format!("click({x:.0},{y:.0})"),
            GuiAction::TypeText { text, at } => format!("type({text:?}{})", at.map(|(x, y)| format!(" @{x:.0},{y:.0}")).unwrap_or_default()),
            GuiAction::PressKeys { keys } => format!("keys({keys})"),
            GuiAction::ClickElement { id } => format!("element({id})"),
            GuiAction::LookAtScreen => "look".into(),
            GuiAction::ListWindows => "windows".into(),
            GuiAction::FindElements { .. } => "elements".into(),
            other => format!("{other:?}"),
        })
        .collect();
    let note = format!("{:?} [{}] {}", r.outcome, did.join(" "), r.message.chars().take(400).collect::<String>().replace('\n', " "));
    TaskResult { id, pass, hit, secs, model_secs, cost: usage.cost.unwrap_or(0.0), tokens: usage.prompt_tokens + usage.completion_tokens, steps, note }
}

fn local() -> bool {
    std::env::var("WADDLE_BENCH_PROVIDER").is_ok_and(|v| v == "ollama")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "spends money: needs OPENROUTER_API_KEY and WADDLE_BENCH_MODELS"]
async fn benchmark_models() {
    let key = if local() { String::new() } else { std::env::var("OPENROUTER_API_KEY").expect("OPENROUTER_API_KEY") };
    let models: Vec<String> = std::env::var("WADDLE_BENCH_MODELS").expect("WADDLE_BENCH_MODELS").split(',').map(|s| s.trim().to_string()).collect();
    let repeat: usize = std::env::var("WADDLE_BENCH_REPEAT").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    let suite = Arc::new(load());
    let tag = std::env::var("WADDLE_BENCH_TAG").unwrap_or_default();
    std::fs::create_dir_all(bench_dir().join("results")).unwrap();
    let mut summary = vec![];
    // New OpenRouter accounts get a low per-model requests-per-minute cap.
    let limit = std::env::var("WADDLE_BENCH_CONCURRENCY").ok().and_then(|v| v.parse().ok()).unwrap_or(64);
    let gate = Arc::new(tokio::sync::Semaphore::new(limit));
    for model in &models {
        // Tasks run concurrently (each has its own fake desktop) to keep the bench quick.
        let mut handles = vec![];
        for _ in 0..repeat {
            for task in suite.tasks.clone() {
                let (model, suite, key, gate) = (model.clone(), suite.clone(), key.clone(), gate.clone());
                let h = tokio::spawn(async move {
                    let _permit = gate.acquire_owned().await.unwrap();
                    run_task(&model, &task, &suite, &key).await
                });
                // A local server works through one request at a time; queueing would only add timeouts.
                if local() {
                    let _ = h.await.map(|r| handles.push(tokio::spawn(async move { r })));
                } else {
                    handles.push(h);
                }
            }
        }
        let mut results = vec![];
        for h in handles {
            results.push(h.await.unwrap());
        }
        println!("\n### {model} {tag}");
        for r in &results {
            println!("{} {:12} {:5.1}s ${:.5} {}", if r.pass { "PASS" } else if r.hit { "HIT " } else { "FAIL" }, r.id, r.secs, r.cost, r.note);
        }
        let n = results.len() as f64;
        let passed = results.iter().filter(|r| r.pass).count();
        let hits = results.iter().filter(|r| r.hit).count();
        let secs = results.iter().map(|r| r.secs).sum::<f64>() / n;
        let model_secs = results.iter().map(|r| r.model_secs).sum::<f64>() / n;
        let cost = results.iter().map(|r| r.cost).sum::<f64>() / n;
        let tokens = results.iter().map(|r| r.tokens).sum::<u64>() as f64 / n;
        let steps = results.iter().map(|r| r.steps).sum::<usize>() as f64 / n;
        let line = format!("{model:45} {tag:8} {passed:3}/{:<3} hit {hits:3} {secs:6.1}s/task (model {model_secs:.1}s) ${:.5}/task {tokens:7.0} tok {steps:4.1} steps", results.len(), cost);
        println!("{line}");
        summary.push(line);
        let file = bench_dir().join(format!("results/{}{}.json", model.replace('/', "__"), if tag.is_empty() { String::new() } else { format!("@{tag}") }));
        let rows: Vec<Value> = results
            .iter()
            .map(|r| json!({ "id": r.id, "pass": r.pass, "hit": r.hit, "secs": r.secs, "model_secs": r.model_secs, "cost": r.cost, "tokens": r.tokens, "steps": r.steps, "note": r.note }))
            .collect();
        std::fs::write(file, serde_json::to_string_pretty(&json!({ "model": model, "tag": tag, "passed": passed, "hits": hits, "runs": results.len(), "secs": secs, "cost": cost, "results": rows })).unwrap()).unwrap();
    }
    println!("\n## Summary");
    for l in summary {
        println!("{l}");
    }
}
