//! Test lab: which models, with which coordinate system, click Waddle's screens best for
//! the least money. Each probe is one request (a screenshot and "Click X"), offered
//! Waddle's own click tool and read by Waddle's own parser, so an answer counts exactly as
//! it would in the app. Ignored by default.
//!
//!   node bench/lab/probes.mjs      # once: screens and probes (bench/lab/probes.json)
//!   GEMINI_API_KEY=... OPENROUTER_API_KEY=... \
//!   WADDLE_LAB_MODELS=gemini-3.5-flash-lite,gemma-4-31b-it,qwen/qwen3.7-flash \
//!     cargo test -p waddle-core --test lab -- --ignored --nocapture
//!
//! Cheap by design:
//! - Gemini and Gemma names without "google/" run on Google's free key, and OpenRouter's
//!   ":free" models cost nothing.
//! - Every answer is kept in bench/lab/results.jsonl and never asked for again.
//! - A screening round (WADDLE_LAB_SCREEN probes, default 10) drops a model and coordinate
//!   system that hits under WADDLE_LAB_KEEP (default 0.5) of them. Only each model's better
//!   system goes on to the other probes (WADDLE_LAB_ALL_MODES=1 keeps both).
//! - Paid calls stop once this run has spent WADDLE_LAB_BUDGET dollars (default 0.05), and
//!   WADDLE_LAB_MAX caps the probes a model and coordinate system get (default: all).
//!
//! WADDLE_LAB_MODES=pixels,norm1000 (default both), WADDLE_LAB_RPM (default 14 on Google,
//! 18 on OpenRouter). The report, built from every answer kept so far, goes to
//! bench/lab/REPORT.md. It includes a fitted calibration: the straight-line map (fitted by
//! medians) from what a model said to where the target was, which shows the coordinate space
//! it really answers in.

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use waddle_core::config::CoordMode;
use waddle_core::llm::{build_provider, is_google_name, ChatRequest, ImageData, Message, Provider};
use waddle_core::tools::{parse_gui_action, specs, Capabilities, Coords};
use waddle_core::Settings;

const GOOGLE: &str = "https://generativelanguage.googleapis.com/v1beta/openai/";
/// Targets smaller than this (in square pixels) count as small: icons, toggles, short words.
const SMALL: f64 = 1200.0;

#[derive(Deserialize)]
struct Probes {
    screen: Size,
    probes: Vec<Probe>,
}

#[derive(Deserialize, Clone, Copy)]
struct Size {
    w: f64,
    h: f64,
}

#[derive(Deserialize, Clone)]
struct Probe {
    id: String,
    screen: String,
    kind: String,
    instruction: String,
    boxes: Vec<[f64; 4]>,
}

impl Probe {
    fn hits(&self, (x, y): (f64, f64)) -> bool {
        self.boxes.iter().any(|b| x >= b[0] && x <= b[2] && y >= b[1] && y <= b[3])
    }

    /// How far a click landed from the nearest right box (0 inside it).
    fn miss_by(&self, (x, y): (f64, f64)) -> f64 {
        self.boxes
            .iter()
            .map(|b| {
                let dx = (b[0] - x).max(0.0).max(x - b[2]);
                let dy = (b[1] - y).max(0.0).max(y - b[3]);
                (dx * dx + dy * dy).sqrt()
            })
            .fold(f64::INFINITY, f64::min)
    }

    fn center(&self) -> (f64, f64) {
        let b = self.boxes[0];
        ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0)
    }

    fn small(&self) -> bool {
        let b = self.boxes[0];
        (b[2] - b[0]) * (b[3] - b[1]) < SMALL
    }
}

/// One answer, as kept in results.jsonl.
#[derive(Serialize, Deserialize, Clone)]
struct Answer {
    model: String,
    mode: String,
    probe: String,
    /// The x and y the model gave, before any conversion.
    raw: Option<(f64, f64)>,
    /// Where Waddle would click, in screen pixels.
    at: Option<(f64, f64)>,
    /// The click's arguments as they came, so a better parser can score old answers again.
    #[serde(default)]
    args: Option<Value>,
    error: Option<String>,
    secs: f64,
    cost: f64,
    tokens: u64,
    when: String,
}

fn lab_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../bench/lab"))
}

fn mode_name(m: CoordMode) -> &'static str {
    match m {
        CoordMode::Norm1000 => "norm1000",
        _ => "pixels",
    }
}

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// A stable shuffle, so the screening round mixes screens, sizes and kinds the same way every run.
fn order_key(id: &str) -> u64 {
    id.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

/// The service a model runs on, and whether it costs nothing there.
fn provider_for(model: &str) -> Option<(Arc<dyn Provider>, bool)> {
    if is_google_name(model) {
        let key = std::env::var("GEMINI_API_KEY").ok()?;
        let settings = Settings { base_url: GOOGLE.into(), model: model.into(), ..Settings::default() };
        return Some((build_provider(&settings, Some(key)), true));
    }
    let key = std::env::var("OPENROUTER_API_KEY").ok()?;
    let settings = Settings { model: model.into(), ..Settings::default() };
    Some((build_provider(&settings, Some(key)), model.ends_with(":free")))
}

/// Shared by every model's run: what this run has spent on paid calls, and the results file.
struct Lab {
    probes: Vec<Probe>,
    size: Size,
    images: BTreeMap<String, ImageData>,
    done: Mutex<HashSet<(String, String, String)>>,
    out: Mutex<std::fs::File>,
    spent: Mutex<f64>,
    budget: f64,
}

impl Lab {
    fn keep(&self, a: &Answer) {
        let mut f = self.out.lock().unwrap();
        let _ = writeln!(f, "{}", serde_json::to_string(a).unwrap());
        self.done.lock().unwrap().insert((a.model.clone(), a.mode.clone(), a.probe.clone()));
    }

    fn asked(&self, model: &str, mode: &str, probe: &str) -> bool {
        self.done.lock().unwrap().contains(&(model.to_string(), mode.to_string(), probe.to_string()))
    }

    fn over_budget(&self) -> bool {
        *self.spent.lock().unwrap() >= self.budget
    }
}

async fn ask(provider: &dyn Provider, model: &str, mode: CoordMode, probe: &Probe, lab: &Lab) -> Answer {
    let coords = Coords { mode, screen_w: lab.size.w, screen_h: lab.size.h };
    let tools: Vec<_> = specs(Capabilities { gui: true, ..Capabilities::default() }, &coords).into_iter().filter(|t| t.name == "click").collect();
    let system = format!(
        "You operate a computer for the user. The image is a screenshot of the whole screen. Do what the user asks by calling the click tool once. {}",
        coords.describe()
    );
    let messages = vec![Message::system(system), Message::user_with_image(probe.instruction.clone(), lab.images[&probe.screen].clone())];
    let started = Instant::now();
    let req = ChatRequest { model, messages: &messages, tools: &tools, temperature: 0.2, max_tokens: 1024, web: None };
    let reply = tokio::time::timeout(Duration::from_secs(90), provider.chat(req, &mut |_| {})).await;
    let mut a = Answer {
        model: model.into(),
        mode: mode_name(mode).into(),
        probe: probe.id.clone(),
        raw: None,
        at: None,
        args: None,
        error: None,
        secs: started.elapsed().as_secs_f64(),
        cost: 0.0,
        tokens: 0,
        when: chrono::Local::now().to_rfc3339(),
    };
    match reply {
        Err(_) => a.error = Some("timed out".into()),
        Ok(Err(e)) => a.error = Some(format!("{e:#}").chars().take(300).collect()),
        Ok(Ok(r)) => {
            a.cost = r.usage.cost.unwrap_or(0.0);
            a.tokens = r.usage.prompt_tokens + r.usage.completion_tokens;
            match r.tool_calls.iter().find(|c| c.name == "click") {
                None => a.error = Some(format!("no click: {}", r.text.chars().take(160).collect::<String>())),
                Some(call) => {
                    a.args = Some(call.arguments.clone());
                    read_click(&mut a, &coords);
                }
            }
        }
    }
    a
}

/// Where the click lands and what the model said, read from its arguments by Waddle's parser.
fn read_click(a: &mut Answer, coords: &Coords) {
    let Some(args) = a.args.clone() else { return };
    let call = waddle_core::llm::ToolCall { id: String::new(), name: "click".into(), arguments: args, echo: None };
    a.error = None;
    match parse_gui_action(&call, coords) {
        Ok(action) => {
            a.at = action.target();
            // What the model said, before Waddle's conversion.
            a.raw = a.at.map(|(x, y)| match coords.mode {
                CoordMode::Norm1000 => (x / coords.screen_w * 1000.0, y / coords.screen_h * 1000.0),
                _ => (x, y),
            });
        }
        Err(e) => {
            a.at = None;
            a.raw = None;
            a.error = Some(e);
        }
    }
}

/// Runs one model: the screening round in every coordinate system, then the rest of the
/// probes in the systems that earned it.
async fn run_model(model: String, modes: Vec<CoordMode>, lab: Arc<Lab>) -> String {
    let Some((provider, free)) = provider_for(&model) else { return format!("{model}: no key for its service") };
    let rpm: f64 = env_or("WADDLE_LAB_RPM", if is_google_name(&model) { 14.0 } else { 18.0 });
    let gap = Duration::from_secs_f64(60.0 / rpm);
    let screen_n: usize = env_or("WADDLE_LAB_SCREEN", 10);
    let keep: f64 = env_or("WADDLE_LAB_KEEP", 0.5);
    let all_modes = std::env::var("WADDLE_LAB_ALL_MODES").is_ok_and(|v| v == "1");
    let max: usize = env_or("WADDLE_LAB_MAX", lab.probes.len());
    let probes = &lab.probes[..max.min(lab.probes.len())];
    let (screening, rest) = probes.split_at(screen_n.min(probes.len()));
    let mut failures = 0;
    let mut asked = 0;
    let mut last = Instant::now() - gap;
    let mut note = String::new();

    // Each step asks what isn't kept yet; returns false when the model should stop.
    macro_rules! step {
        ($mode:expr, $probe:expr) => {{
            let mode_s = mode_name($mode);
            if !lab.asked(&model, mode_s, &$probe.id) {
                if !free && lab.over_budget() {
                    note = "stopped at the budget".into();
                    false
                } else {
                    let wait = gap.saturating_sub(last.elapsed());
                    tokio::time::sleep(wait).await;
                    last = Instant::now();
                    let a = ask(provider.as_ref(), &model, $mode, $probe, &lab).await;
                    *lab.spent.lock().unwrap() += a.cost;
                    asked += 1;
                    failures = if a.error.is_some() && a.raw.is_none() { failures + 1 } else { 0 };
                    let err = a.error.clone();
                    lab.keep(&a);
                    if failures >= 3 {
                        note = format!("stopped after 3 failures in a row: {}", err.unwrap_or_default());
                        false
                    } else {
                        true
                    }
                }
            } else {
                true
            }
        }};
    }

    'screen: for &mode in &modes {
        for p in screening {
            if !step!(mode, p) {
                break 'screen;
            }
        }
    }
    if note.is_empty() {
        let size = lab.size;
        let answers: Vec<Answer> = kept_answers()
            .into_iter()
            .filter(|a| a.model == model)
            .map(|mut a| {
                let mode = if a.mode == "norm1000" { CoordMode::Norm1000 } else { CoordMode::Pixels };
                read_click(&mut a, &Coords { mode, screen_w: size.w, screen_h: size.h });
                a
            })
            .collect();
        let rate = |mode: CoordMode| {
            let n = screening.iter().filter(|p| answers.iter().any(|a| a.model == model && a.mode == mode_name(mode) && a.probe == p.id && a.at.is_some_and(|at| p.hits(at)))).count();
            n as f64 / screening.len().max(1) as f64
        };
        let mut ranked: Vec<(CoordMode, f64)> = modes.iter().map(|&m| (m, rate(m))).collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        let going: Vec<CoordMode> = ranked.iter().enumerate().filter(|(i, (_, r))| *r >= keep && (all_modes || *i == 0)).map(|(_, (m, _))| *m).collect();
        let screened: Vec<String> = ranked.iter().map(|(m, r)| format!("{} {:.0}%", mode_name(*m), r * 100.0)).collect();
        note = format!("screening {}", screened.join(", "));
        'rest: for &mode in &going {
            for p in rest {
                if !step!(mode, p) {
                    break 'rest;
                }
            }
        }
        if going.is_empty() {
            note.push_str("; dropped");
        }
    }
    format!("{model}: {asked} asked; {note}")
}

fn kept_answers() -> Vec<Answer> {
    std::fs::read_to_string(lab_dir().join("results.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// `truth = a * said + b` for one axis, fitted by medians (Theil-Sen) so a few wild misses
/// don't drag the line.
fn fit(pairs: &[(f64, f64)]) -> Option<(f64, f64)> {
    if pairs.len() < 3 {
        return None;
    }
    let mut slopes = vec![];
    for (i, (x1, y1)) in pairs.iter().enumerate() {
        for (x2, y2) in &pairs[i + 1..] {
            if (x2 - x1).abs() > 1e-9 {
                slopes.push((y2 - y1) / (x2 - x1));
            }
        }
    }
    if slopes.is_empty() {
        return None;
    }
    let a = median(slopes);
    let b = median(pairs.iter().map(|(x, y)| y - a * x).collect());
    Some((a, b))
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// The report, from every answer kept so far for the probes that exist now.
fn report(probes: &[Probe], size: Size) -> String {
    let by_id: BTreeMap<&str, &Probe> = probes.iter().map(|p| (p.id.as_str(), p)).collect();
    let mut groups: BTreeMap<(String, String), Vec<(Answer, &Probe)>> = BTreeMap::new();
    for mut a in kept_answers() {
        // Score with today's parser where the arguments were kept.
        let mode = if a.mode == "norm1000" { CoordMode::Norm1000 } else { CoordMode::Pixels };
        read_click(&mut a, &Coords { mode, screen_w: size.w, screen_h: size.h });
        if let Some(p) = by_id.get(a.probe.as_str()) {
            groups.entry((a.model.clone(), a.mode.clone())).or_default().push((a, p));
        }
    }
    struct Row {
        model: String,
        mode: String,
        n: usize,
        hit: f64,
        small: String,
        task: String,
        miss: f64,
        cal: String,
        scale: String,
        secs: f64,
        per_k: f64,
        errors: usize,
    }
    let mut rows = vec![];
    for ((model, mode), list) in &groups {
        let n = list.len();
        let hits = |f: &dyn Fn(&Probe) -> bool| {
            let sub: Vec<_> = list.iter().filter(|(_, p)| f(p)).collect();
            let h = sub.iter().filter(|(a, p)| a.at.is_some_and(|at| p.hits(at))).count();
            (h, sub.len())
        };
        let (h, _) = hits(&|_| true);
        let pct = |(h, t): (usize, usize)| if t == 0 { "–".to_string() } else { format!("{:.0}% of {t}", h as f64 * 100.0 / t as f64) };
        let misses: Vec<f64> = list.iter().filter_map(|(a, p)| a.at.map(|at| p.miss_by(at))).collect();
        // What the model said, read as each known coordinate space: the space with the most
        // hits is the one it really answers in, whatever it was asked for.
        let said: Vec<(&Probe, (f64, f64))> = list.iter().filter_map(|(a, p)| a.raw.map(|r| (*p, r))).collect();
        let reading = |to: &dyn Fn((f64, f64)) -> (f64, f64)| said.iter().filter(|(p, r)| p.hits(to(*r))).count();
        let as_pixels = reading(&|r| r);
        let as_grid = reading(&|(x, y)| (x / 1000.0 * size.w, y / 1000.0 * size.h));
        let cal = if said.is_empty() {
            "–".to_string()
        } else if as_grid > as_pixels {
            format!("0–1000 grid {:.0}%", as_grid as f64 * 100.0 / n as f64)
        } else {
            format!("pixels {:.0}%", as_pixels as f64 * 100.0 / n as f64)
        };
        let scale = if said.len() >= 8 {
            match (fit(&said.iter().map(|(p, r)| (r.0, p.center().0)).collect::<Vec<_>>()), fit(&said.iter().map(|(p, r)| (r.1, p.center().1)).collect::<Vec<_>>())) {
                (Some((ax, bx)), Some((ay, by))) => format!("x×{ax:.2}{bx:+.0}, y×{ay:.2}{by:+.0}"),
                _ => "–".into(),
            }
        } else {
            "–".into()
        };
        let cost: f64 = list.iter().map(|(a, _)| a.cost).sum();
        rows.push(Row {
            model: model.clone(),
            mode: mode.clone(),
            n,
            hit: h as f64 / n as f64,
            small: pct(hits(&|p| p.small())),
            task: pct(hits(&|p| p.kind == "task")),
            miss: median(misses),
            cal,
            scale,
            secs: list.iter().map(|(a, _)| a.secs).sum::<f64>() / n as f64,
            per_k: cost / n as f64 * 1000.0,
            errors: list.iter().filter(|(a, _)| a.at.is_none()).count(),
        });
    }
    rows.sort_by(|a, b| b.hit.total_cmp(&a.hit).then(a.per_k.total_cmp(&b.per_k)));
    let mut out = String::from(
        "# Click lab report\n\nWritten by `crates/waddle-core/tests/lab.rs` from every answer in `results.jsonl` (see that file's header for how to run it). \
         One probe is one screenshot of a bench screen (1440×960) and one instruction; a hit is a click inside the target.\n\n\
         - **Best reading:** the model's answers read as pixels and as a 0–1000 grid, whatever it was asked for; the better one is the space it really answers in. Above **Hits** means Waddle should ask it for that space.\n\
         - **Fitted map:** a line through what it said and where the targets were, fitted by medians (pixels: ×1.00+0; a 0–1000 grid: x×1.44, y×0.96).\n\
         - **Per 1,000 clicks:** what OpenRouter charged; Google's free key and `:free` models are $0.\n\n\
         | Model | Coordinates | Probes | Hits | Small targets | Task goals | Median miss | Best reading | Fitted map | Time | Per 1,000 clicks | No click |\n\
         |---|---|---|---|---|---|---|---|---|---|---|---|\n",
    );
    for r in rows {
        out.push_str(&format!(
            "| `{}` | {} | {} | **{:.0}%** | {} | {} | {:.0} px | {} | {} | {:.1} s | ${:.3} | {} |\n",
            r.model,
            r.mode,
            r.n,
            r.hit * 100.0,
            r.small,
            r.task,
            r.miss,
            r.cal,
            r.scale,
            r.secs,
            r.per_k,
            r.errors
        ));
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "calls models: needs WADDLE_LAB_MODELS and GEMINI_API_KEY and/or OPENROUTER_API_KEY"]
async fn click_lab() {
    let dir = lab_dir();
    let spec: Probes = serde_json::from_str(&std::fs::read_to_string(dir.join("probes.json")).expect("run node bench/lab/probes.mjs first")).unwrap();
    let mut probes = spec.probes;
    probes.sort_by_key(|p| order_key(&p.id));
    let mut images = BTreeMap::new();
    for p in &probes {
        if !images.contains_key(&p.screen) {
            let png = std::fs::read(dir.join(format!("screens/{}.png", p.screen))).unwrap();
            images.insert(p.screen.clone(), ImageData { mime: "image/png".into(), base64: base64::engine::general_purpose::STANDARD.encode(png) });
        }
    }
    let models: Vec<String> = std::env::var("WADDLE_LAB_MODELS").unwrap_or_default().split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    let modes: Vec<CoordMode> = std::env::var("WADDLE_LAB_MODES")
        .unwrap_or_else(|_| "pixels,norm1000".into())
        .split(',')
        .map(|m| if m.trim() == "norm1000" { CoordMode::Norm1000 } else { CoordMode::Pixels })
        .collect();
    let done = kept_answers().into_iter().map(|a| (a.model, a.mode, a.probe)).collect();
    let out = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("results.jsonl")).unwrap();
    let lab = Arc::new(Lab { probes: probes.clone(), size: spec.screen, images, done: Mutex::new(done), out: Mutex::new(out), spent: Mutex::new(0.0), budget: env_or("WADDLE_LAB_BUDGET", 0.05) });

    let runs: Vec<_> = models.into_iter().map(|m| tokio::spawn(run_model(m, modes.clone(), lab.clone()))).collect();
    for r in runs {
        println!("{}", r.await.unwrap());
    }
    println!("this run spent ${:.4} on paid calls (budget ${:.2})", *lab.spent.lock().unwrap(), lab.budget);
    let text = report(&probes, spec.screen);
    std::fs::write(dir.join("REPORT.md"), &text).unwrap();
    println!("\n{text}");
}

#[test]
fn the_lab_scores_and_fits_answers() {
    let p = Probe { id: "a".into(), screen: "s".into(), kind: "text".into(), instruction: "Click".into(), boxes: vec![[100.0, 100.0, 140.0, 120.0]] };
    assert!(p.hits((120.0, 110.0)) && !p.hits((99.0, 110.0)));
    assert_eq!(p.miss_by((100.0, 130.0)), 10.0);
    assert!(p.small());
    // A model answering on a 0-1000 grid while asked for pixels: the fit finds the grid.
    let mut pairs: Vec<(f64, f64)> = [100.0, 400.0, 900.0, 1200.0].iter().map(|&x: &f64| (x / 1440.0 * 1000.0, x)).collect();
    // One wild miss doesn't move the line.
    pairs.push((900.0, 50.0));
    let (a, b) = fit(&pairs).unwrap();
    assert!((a - 1.44).abs() < 0.001 && b.abs() < 0.01, "{a} {b}");
    assert_eq!(fit(&[(1.0, 1.0)]), None);
    assert_eq!(median(vec![3.0, 1.0, 2.0]), 2.0);
    let _ = Value::Null;
}
