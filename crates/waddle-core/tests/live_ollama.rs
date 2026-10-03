//! Real-model checks against a local Ollama server. Ignored by default (CI has
//! no model). Run with:
//!
//!   ollama pull qwen3.5:4b
//!   cargo test -p waddle-core --test live_ollama -- --ignored --nocapture --test-threads=1
//!
//! WADDLE_LIVE_MODEL and WADDLE_OLLAMA_URL override the model and server.

mod common;

use base64::Engine;
use common::FakeHost;
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

use waddle_core::agent::{Agent, AgentDeps, TaskStatus};
use waddle_core::audit::AuditLog;
use waddle_core::config::ProviderKind;
use waddle_core::llm::{build_provider, ImageData};
use waddle_core::tools::fs::Workspace;
use waddle_core::tools::{ElementInfo, GuiAction};
use waddle_core::{AgentEvent, Decision, Host, Outcome, Settings};

fn live_settings() -> Settings {
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
    let dir = tempfile::tempdir().unwrap();
    let workspace = Arc::new(Workspace::new(dir.path().join("ws")).unwrap());
    let host = FakeHost::new(Box::new(|_| Some(Decision::Approved)));
    setup(&host);
    let settings = live_settings();
    let deps = AgentDeps {
        provider: build_provider(&settings, None),
        host: host.clone(),
        audit: Arc::new(AuditLog::open_in_memory().unwrap()),
        workspace: workspace.clone(),
        settings,
        skills: None,
        self_source: None,
    };
    let env = host.env();
    let agent = Agent::new(&deps, "live".into(), CancellationToken::new(), Arc::new(Mutex::new(TaskStatus::default())), &env);
    let (_tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let r = agent.run(goal, &[], &mut rx).await;
    println!("\n== {goal}");
    for (t, label) in host.timeline.lock().unwrap().iter() {
        println!("{t:7.1}s  {label}");
    }
    println!("final: {:?} {}", r.outcome, r.message);
    Run { outcome: r.outcome, message: r.message, host, _dir: dir, workspace }
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
#[ignore = "needs a local Ollama server with the model pulled"]
async fn writes_a_file() {
    let r = run("Create a file named hello.txt that contains the text hi", |_| {}).await;
    assert_eq!(r.outcome, Outcome::Done, "{}", r.message);
    let body = std::fs::read_to_string(r.workspace.root().join("hello.txt")).expect("hello.txt was not written");
    assert_eq!(body.trim().to_lowercase(), "hi");
    assert!(!narration(&r.host).contains("<think>"), "thinking leaked into the bubble");
}

#[tokio::test]
#[ignore = "needs a local Ollama server with the model pulled"]
async fn clicks_an_accessibility_element() {
    let r = run("In the app that's open, press the Save button", |h| {
        let el = |id, role: &str, name: &str, x| ElementInfo { id, role: role.into(), name: name.into(), x, y: 600.0, w: 90.0, h: 32.0 };
        *h.elements.lock().unwrap() = vec![el(1, "button", "Open", 300.0), el(2, "edit", "File name", 420.0), el(3, "button", "Save", 900.0), el(4, "button", "Cancel", 1010.0)];
    })
    .await;
    let calls = r.host.gui_calls.lock().unwrap().clone();
    assert!(calls.contains(&GuiAction::ClickElement { id: 3 }), "expected click_element 3, got {calls:?}");
}

#[tokio::test]
#[ignore = "needs a local Ollama server with the model pulled"]
async fn clicks_what_it_sees_in_a_screenshot() {
    let r = run("Look at the screen and click the red button", |h| *h.screenshot.lock().unwrap() = Some(screen_png())).await;
    let calls = r.host.gui_calls.lock().unwrap().clone();
    let click = calls.iter().find_map(|c| match c {
        GuiAction::Click { x, y, .. } => Some((*x, *y)),
        _ => None,
    });
    let (x, y) = click.unwrap_or_else(|| panic!("no click, calls: {calls:?}"));
    assert!((1000.0..=1240.0).contains(&x) && (700.0..=780.0).contains(&y), "clicked ({x}, {y}), red button is (1000-1240, 700-780)");
}
