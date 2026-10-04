//! Real-model checks. Ignored by default (CI has no model). Against a local
//! Ollama server:
//!
//!   ollama pull qwen3.5:4b
//!   cargo test -p waddle-core --test live -- --ignored --nocapture --test-threads=1
//!
//! Against OpenRouter (the default cloud brain; about $0.001 per test):
//!
//!   WADDLE_LIVE=openrouter OPENROUTER_API_KEY=... cargo test -p waddle-core --test live -- --ignored --nocapture
//!
//! WADDLE_LIVE_MODEL and WADDLE_OLLAMA_URL override the model and server.

mod common;

use base64::Engine;
use common::FakeHost;
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio_util::sync::CancellationToken;

use waddle_core::agent::{Agent, AgentDeps, TaskStatus};
use waddle_core::audit::AuditLog;
use waddle_core::config::ProviderKind;
use waddle_core::llm::{build_provider, ChatRequest, ImageData};
use waddle_core::tools::fs::Workspace;
use waddle_core::tools::{ElementInfo, GuiAction, WindowInfo};
use waddle_core::{AgentEvent, Decision, Host, Outcome, Settings};

fn openrouter() -> bool {
    std::env::var("WADDLE_LIVE").is_ok_and(|v| v == "openrouter")
}

fn live_settings() -> Settings {
    if openrouter() {
        let defaults = Settings::default();
        return Settings {
            model: std::env::var("WADDLE_LIVE_MODEL").unwrap_or(defaults.model.clone()),
            tier2_countdown_ms: 20,
            max_steps: 6,
            ..defaults
        };
    }
    let mut s = Settings {
        provider: ProviderKind::Ollama,
        base_url: std::env::var("WADDLE_OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".into()),
        model: std::env::var("WADDLE_LIVE_MODEL").unwrap_or_else(|_| "qwen3.5:4b".into()),
        tier2_countdown_ms: 20,
        max_steps: 6,
        ..Settings::default()
    };
    // Use every core here so the run measures the model, not the cap.
    s.ollama.num_thread = std::thread::available_parallelism().ok().map(|n| n.get() as u32);
    s
}

struct Run {
    outcome: Outcome,
    message: String,
    host: Arc<FakeHost>,
    _dir: tempfile::TempDir,
    workspace: Arc<Workspace>,
}

async fn run(goal: &str, setup: impl FnOnce(&FakeHost)) -> Run {
    run_with(goal, setup, false).await
}

/// `warm`: first send what the app sends while the user types (see `Session::warm`).
async fn run_with(goal: &str, setup: impl FnOnce(&FakeHost), warm: bool) -> Run {
    let dir = tempfile::tempdir().unwrap();
    let workspace = Arc::new(Workspace::new(dir.path().join("ws")).unwrap());
    let host = FakeHost::new(Box::new(|_| Some(Decision::Approved)));
    // A real image even when the task needn't look: the stub image would make Ollama fail the step.
    *host.screenshot.lock().unwrap() = Some(screen_png());
    setup(&host);
    let settings = live_settings();
    let deps = AgentDeps {
        provider: build_provider(&settings, std::env::var("OPENROUTER_API_KEY").ok().filter(|_| openrouter())),
        host: host.clone(),
        audit: Arc::new(AuditLog::open_in_memory().unwrap()),
        workspace: workspace.clone(),
        settings,
        skills: None,
        reminders: None,
        decider: None,
        self_source: None,
    };
    let env = host.env();
    let agent = Agent::new(&deps, "live".into(), CancellationToken::new(), Arc::new(Mutex::new(TaskStatus::default())), &env);
    if warm {
        let started = Instant::now();
        let (messages, tools) = agent.opening(&[]);
        deps.provider.warm(ChatRequest { model: &deps.settings.model, messages: &messages, tools: &tools, temperature: 0.2, max_tokens: 1 }).await;
        println!("\nwarm-up took {:.1}s (hidden while the user types)", started.elapsed().as_secs_f64());
    }
    let (_tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let r = agent.run(goal, &[], &mut rx).await;
    report(goal, &host, &r.outcome, &r.message);
    println!("used: {}", agent.usage());
    Run { outcome: r.outcome, message: r.message, host, _dir: dir, workspace }
}

fn report(goal: &str, host: &FakeHost, outcome: &Outcome, message: &str) {
    println!("\n== {goal}");
    for (t, label) in host.timeline.lock().unwrap().iter() {
        println!("{t:7.1}s  {label}");
    }
    println!("final: {outcome:?} {message}");
}

/// Asks Ollama to drop the model from memory, as happens 30 s after a task.
async fn unload(settings: &Settings) {
    let body = serde_json::json!({ "model": settings.model, "keep_alive": 0 });
    let _ = reqwest::Client::new().post(format!("{}/api/generate", settings.base_url)).json(&body).send().await;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
}

fn clicks(host: &FakeHost) -> Vec<(f64, f64)> {
    host.gui_calls
        .lock()
        .unwrap()
        .iter()
        .filter_map(|c| match c {
            GuiAction::Click { x, y, .. } => Some((*x, *y)),
            _ => None,
        })
        .collect()
}

fn inside((x, y): (f64, f64), (x0, y0, x1, y1): (f64, f64, f64, f64)) -> bool {
    (x0..=x1).contains(&x) && (y0..=y1).contains(&y)
}

fn narration(host: &FakeHost) -> String {
    host.events()
        .into_iter()
        .filter_map(|e| match e {
            AgentEvent::TextDelta { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

/// One focused app window filling the middle of the screen, like `screen_png` draws.
fn desktop(h: &FakeHost) {
    *h.windows.lock().unwrap() = vec![WindowInfo { title: "Report - Editor".into(), app: "editor".into(), x: 120.0, y: 120.0, w: 1200.0, h: 740.0, focused: true }];
    *h.screenshot.lock().unwrap() = Some(screen_png());
}

/// A 1440x960 "screen" with a blue button on the left and a red one at the bottom right.
fn screen_png() -> ImageData {
    let mut img = image::RgbImage::from_pixel(1440, 960, image::Rgb([236, 236, 236]));
    let mut fill = |x0: u32, y0: u32, x1: u32, y1: u32, c: [u8; 3]| {
        for y in y0..y1 {
            for x in x0..x1 {
                img.put_pixel(x, y, image::Rgb(c));
            }
        }
    };
    fill(120, 120, 1320, 860, [255, 255, 255]);
    fill(220, 300, 460, 380, [30, 90, 220]);
    fill(1000, 700, 1240, 780, [220, 30, 30]);
    let mut png = Vec::new();
    img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    ImageData { mime: "image/png".into(), base64: base64::engine::general_purpose::STANDARD.encode(png) }
}

#[tokio::test]
#[ignore = "needs a model: a local Ollama server, or WADDLE_LIVE=openrouter"]
async fn writes_a_file() {
    let r = run("Create a file named hello.txt that contains the text hi", |_| {}).await;
    assert_eq!(r.outcome, Outcome::Done, "{}", r.message);
    let body = std::fs::read_to_string(r.workspace.root().join("hello.txt")).expect("hello.txt was not written");
    assert_eq!(body.trim().to_lowercase(), "hi");
    assert!(!narration(&r.host).contains("<think>"), "thinking leaked into the bubble");
}

#[tokio::test]
#[ignore = "needs a model: a local Ollama server, or WADDLE_LIVE=openrouter"]
async fn clicks_an_accessibility_element() {
    let r = run("In the app that's open, press the Save button", |h| {
        desktop(h);
        let el = |id, role: &str, name: &str, x| ElementInfo { id, role: role.into(), name: name.into(), x, y: 600.0, w: 90.0, h: 32.0 };
        *h.elements.lock().unwrap() = vec![el(1, "button", "Open", 300.0), el(2, "edit", "File name", 420.0), el(3, "button", "Save", 900.0), el(4, "button", "Cancel", 1010.0)];
    })
    .await;
    let calls = r.host.gui_calls.lock().unwrap().clone();
    assert!(calls.contains(&GuiAction::ClickElement { id: 3 }), "expected click_element 3, got {calls:?}");
}

#[tokio::test]
#[ignore = "needs a model: a local Ollama server, or WADDLE_LIVE=openrouter"]
async fn clicks_what_it_sees_in_a_screenshot() {
    let r = run("Look at the screen and click the red button", desktop).await;
    let calls = r.host.gui_calls.lock().unwrap().clone();
    let click = calls.iter().find_map(|c| match c {
        GuiAction::Click { x, y, .. } => Some((*x, *y)),
        _ => None,
    });
    let (x, y) = click.unwrap_or_else(|| panic!("no click, calls: {calls:?}"));
    assert!((1000.0..=1240.0).contains(&x) && (700.0..=780.0).contains(&y), "clicked ({x}, {y}), red button is (1000-1240, 700-780)");
}

#[tokio::test]
#[ignore = "needs a model: a local Ollama server, or WADDLE_LIVE=openrouter"]
async fn two_screenshots_keep_the_cache() {
    // Watch the server log while this runs: the second look_at_screen step should
    // restore the previous checkpoint instead of "erased invalidated context checkpoint".
    let r = run("Look at the screen and click the blue button. Then look at the screen again and click the red button.", desktop).await;
    let clicks = clicks(&r.host);
    assert!(clicks.iter().any(|c| inside(*c, (220.0, 300.0, 460.0, 380.0))), "no click on the blue button: {clicks:?}");
    assert!(clicks.iter().any(|c| inside(*c, (1000.0, 700.0, 1240.0, 780.0))), "no click on the red button: {clicks:?}");
}

/// A browser showing a real web form (a screenshot from the Linux test desktop), with no accessibility list.
fn signup_page(h: &FakeHost) {
    *h.os.lock().unwrap() = "linux".into();
    *h.windows.lock().unwrap() = vec![WindowInfo { title: "Signup form".into(), app: "chromium".into(), x: 150.0, y: 120.0, w: 1100.0, h: 650.0, focused: true }];
    let png = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/signup.png")).unwrap();
    *h.screenshot.lock().unwrap() = Some(ImageData { mime: "image/png".into(), base64: base64::engine::general_purpose::STANDARD.encode(png) });
}

#[tokio::test]
#[ignore = "needs a model: a local Ollama server, or WADDLE_LIVE=openrouter"]
async fn fills_in_a_web_form() {
    let r = run("Type Ada in the name box on the signup page, then press Subscribe", signup_page).await;
    let calls = r.host.gui_calls.lock().unwrap().clone();
    let name_box = (254.0, 336.0, 594.0, 380.0);
    // Either type_text at the box, or a click in the box followed by type_text.
    let typed = calls.iter().enumerate().any(|(i, c)| match c {
        GuiAction::TypeText { text, at } if text.trim() == "Ada" => match at {
            Some(p) => inside(*p, name_box),
            None => calls[..i].iter().rev().find_map(|c| match c {
                GuiAction::Click { x, y, .. } => Some(inside((*x, *y), name_box)),
                _ => None,
            }) == Some(true),
        },
        _ => false,
    });
    assert!(typed, "Ada wasn't typed into the name box: {calls:?}");
    assert!(clicks(&r.host).iter().any(|c| inside(*c, (360.0, 404.0, 530.0, 458.0))), "Subscribe wasn't clicked: {calls:?}");
}

#[tokio::test]
#[ignore = "needs a model: a local Ollama server, or WADDLE_LIVE=openrouter"]
async fn warm_up_hides_the_model_load_and_the_prompt() {
    let settings = live_settings();
    if !settings.is_local() {
        return println!("warm-up only applies to local models");
    }
    // From the first model call to the first tool: what the user waits for after Enter.
    let first_step = |r: &Run| {
        let t = r.host.timeline.lock().unwrap();
        match (t.first(), t.get(1)) {
            (Some(a), Some(b)) => b.0 - a.0,
            _ => f64::NAN,
        }
    };

    unload(&settings).await;
    let cold = run("Create a file named cold.txt that contains the text cold", desktop).await;

    unload(&settings).await;
    let warm = run_with("Create a file named warm.txt that contains the text warm", desktop, true).await;

    println!("\nfirst step after idle: cold {:.1}s, warmed {:.1}s", first_step(&cold), first_step(&warm));
    assert_eq!(warm.outcome, Outcome::Done, "{}", warm.message);
    assert!(first_step(&warm) < first_step(&cold) / 2.0, "warming should at least halve the first step");
}
