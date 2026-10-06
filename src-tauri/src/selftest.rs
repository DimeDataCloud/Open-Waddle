//! "Run self-test" in Settings: checks each part of Waddle on this machine and
//! writes a report the user can send when something doesn't work. The report
//! leaves out keys, file contents, command arguments and window titles.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};
use tokio_util::sync::CancellationToken;
use waddle_core::diagnostics::{self, run_check, Check, ReportInput, Status};
use waddle_core::tools::shell;

use crate::secrets::{Secret, Secrets};
use crate::{actuate, desktop, shortcuts, voice, AppState};

const LOG_TAIL_LINES: usize = 150;
const AUDIT_ENTRIES: usize = 60;

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> anyhow::Result<T> {
    tokio::task::spawn_blocking(f).await?
}

pub fn log_file(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_log_dir().ok().map(|d| d.join("waddle.log"))
}

fn log_tail(app: &AppHandle) -> String {
    let Some(text) = log_file(app).and_then(|p| std::fs::read_to_string(p).ok()) else { return String::new() };
    let lines: Vec<&str> = text.lines().collect();
    let mut tail = lines[lines.len().saturating_sub(LOG_TAIL_LINES)..].join("\n");
    tail.push('\n');
    tail
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub async fn run(app: &AppHandle) -> Vec<Check> {
    let state = app.state::<AppState>();
    let settings = state.settings.read().unwrap().clone();
    let mut checks = vec![];

    let g = state.overlay.geometry();
    let monitors = app.available_monitors().map(|m| m.len()).unwrap_or(0);
    checks.push(
        run_check("Display", || async move {
            let others = match monitors {
                0 | 1 => String::new(),
                n => format!("; {n} monitors{}", if g.primary { "" } else { ", on a secondary one" }),
            };
            Ok((
                Status::Pass,
                format!("{}x{} work area at {:.0}% scale (screen {}x{}){others}", g.width, g.height, g.scale * 100.0, g.screen_w, g.screen_h),
            ))
        })
        .await,
    );

    checks.push(
        run_check("Window tracking", || async {
            let windows = blocking(|| Ok(desktop::list_windows())).await?;
            let mut apps: Vec<String> = windows.iter().map(|w| w.app.clone()).filter(|a| !a.is_empty()).collect();
            apps.dedup();
            apps.truncate(6);
            Ok(match windows.len() {
                0 => (Status::Warn, "no other windows found, so Waddle can only walk on the taskbar".into()),
                n => (Status::Pass, format!("{n} windows ({})", apps.join(", "))),
            })
        })
        .await,
    );

    checks.push(
        run_check("Accessibility", || async {
            #[cfg(windows)]
            {
                let (app_name, n) = state.host.probe_accessibility().await?;
                Ok(match n {
                    0 => (Status::Warn, format!("{app_name} exposes no controls; Waddle will use screenshots there")),
                    n => (Status::Pass, format!("{n} controls readable in {app_name}")),
                })
            }
            #[cfg(not(windows))]
            Ok((Status::Skip, "the accessibility fast path is Windows-only for now; screenshots are used instead".to_string()))
        })
        .await,
    );

    let (lw, lh) = g.screen_logical();
    let origin = (g.screen_x, g.screen_y);
    checks.push(
        run_check("Screenshot", || async move {
            let (image, w, h) = blocking(move || actuate::screenshot(lw.round() as u32, lh.round() as u32, origin)).await?;
            Ok((Status::Pass, format!("{w}x{h}, {} KB", image.base64.len() * 3 / 4 / 1024)))
        })
        .await,
    );

    checks.push(
        run_check("Mouse and keyboard", || async {
            let (x, y) = blocking(actuate::probe_input).await?;
            Ok((Status::Pass, format!("input simulation ready (cursor at {x}, {y})")))
        })
        .await,
    );

    let has_key = state.secrets.get(Secret::LlmKey).is_some();
    checks.push(
        run_check("Key storage", || async {
            Ok(match blocking(Secrets::probe_keychain).await {
                Ok(()) => (Status::Pass, "OS keychain works".into()),
                Err(e) => (Status::Warn, format!("keychain unavailable ({e:#}); keys are kept in a private file instead")),
            })
        })
        .await,
    );

    let workspace = state.workspace();
    checks.push(
        run_check("Workspace", || async {
            let probe = ".waddle-selftest.txt";
            workspace.write_file(probe, "ok")?;
            let back = workspace.read_file(probe)?;
            let _ = std::fs::remove_file(workspace.root().join(probe));
            anyhow::ensure!(back.contains("ok"), "read back something else");
            Ok((Status::Pass, format!("{} is writable", workspace.root().display())))
        })
        .await,
    );

    let root = state.workspace().root().to_path_buf();
    checks.push(
        run_check("Commands", || async move {
            let r = shell::run_command("echo waddle-ok", &root, Duration::from_secs(15), &CancellationToken::new(), &mut |_| {}).await?;
            anyhow::ensure!(r.output.contains("waddle-ok"), "unexpected output: {}", r.output.trim());
            Ok((Status::Pass, if cfg!(windows) { "PowerShell runs" } else { "the shell runs" }.to_string()))
        })
        .await,
    );

    let audit = state.audit.clone();
    checks.push(
        run_check("Activity log", || async move {
            let report = blocking(move || audit.verify()).await?;
            Ok(if report.ok {
                (Status::Pass, format!("{} entries, chain intact", report.entries))
            } else {
                (Status::Fail, format!("entry {} was altered", report.first_bad_id.unwrap_or_default()))
            })
        })
        .await,
    );

    checks.push(
        run_check("Talk shortcut", || async {
            use tauri_plugin_global_shortcut::GlobalShortcutExt;
            Ok(if app.global_shortcut().is_registered(shortcuts::talk_key()) {
                (Status::Pass, "Ctrl+Alt+Space is registered".into())
            } else {
                (Status::Warn, "Ctrl+Alt+Space is taken by another app; click the duck to talk instead".into())
            })
        })
        .await,
    );

    let backend = settings.voice.backend;
    checks.push(
        run_check("Microphone", || async move {
            if backend == waddle_core::config::VoiceBackend::Off {
                return Ok((Status::Skip, "voice input is off".into()));
            }
            Ok(match blocking(voice::probe).await {
                Ok(name) => (Status::Pass, name),
                Err(e) => (Status::Warn, format!("{e:#}; typing still works")),
            })
        })
        .await,
    );

    let servers: Vec<String> = settings.mcp_servers.iter().filter(|s| s.enabled).map(|s| s.name.clone()).collect();
    let hub = state.mcp.clone();
    checks.push(
        run_check("Tools (MCP)", || async move {
            if servers.is_empty() {
                return Ok((Status::Skip, "no MCP servers added".into()));
            }
            let mut ok = vec![];
            let mut bad = vec![];
            for name in &servers {
                match hub.refresh(name, &tokio_util::sync::CancellationToken::new()).await {
                    Ok(tools) => ok.push(format!("{name}: {} tools", tools.len())),
                    Err(e) => bad.push(format!("{name}: {e:#}")),
                }
            }
            Ok(if bad.is_empty() { (Status::Pass, ok.join("; ")) } else { (Status::Warn, [bad, ok].concat().join("; ")) })
        })
        .await,
    );

    let routines = state.reminders.routines.list();
    checks.push(
        run_check("Routines", || async move {
            if routines.is_empty() {
                return Ok((Status::Skip, "none set up".into()));
            }
            let paused = routines.iter().filter(|r| r.paused).count();
            let missed = routines.iter().filter(|r| r.last_result.as_deref().is_some_and(|l| !l.starts_with("Done"))).count();
            let detail = format!("{} set up, {paused} paused; {missed} didn't finish last time", routines.len());
            Ok((if missed > 0 { Status::Warn } else { Status::Pass }, detail))
        })
        .await,
    );

    let speak = settings.voice_out.enabled;
    checks.push(
        run_check("Spoken replies", || async move {
            let voices = blocking(|| Ok(crate::speech::voices())).await?;
            let can = !voices.is_empty() || blocking(|| Ok(crate::speech::available())).await?;
            Ok(match (can, speak) {
                (true, on) => {
                    let which = if voices.is_empty() { "the default voice".to_string() } else { format!("{} voices ({})", voices.len(), voices.iter().take(3).cloned().collect::<Vec<_>>().join(", ")) };
                    (Status::Pass, format!("{which}{}", if on { "" } else { "; reading aloud is off" }))
                }
                (false, true) => (Status::Warn, "no voice to read with; answers stay on screen".into()),
                (false, false) => (Status::Skip, "no voice installed; reading aloud is off".into()),
            })
        })
        .await,
    );

    let spend = state.ledger.summary(&chrono::Local::now());
    checks.push(
        run_check("Spending", || async move {
            use waddle_core::ledger::money;
            if spend.budget <= 0.0 {
                return Ok((Status::Pass, format!("{} this month; no monthly limit set", money(spend.month))));
            }
            let detail = format!("{} of the {} monthly budget used ({} today)", money(spend.month), money(spend.budget), money(spend.today));
            Ok(if spend.paused {
                (Status::Warn, format!("{detail}; paid model calls are paused until the 1st"))
            } else if spend.month >= spend.budget * 0.8 {
                (Status::Warn, format!("{detail}; close to the limit"))
            } else {
                (Status::Pass, detail)
            })
        })
        .await,
    );

    let (provider, demo) = crate::provider_for(&settings, &state.secrets, &state.ledger);
    let model = settings.model.clone();
    let fast_check = (!demo && settings.has_fast_endpoint()).then(|| (provider.clone(), settings.fast_model().to_string(), state.secrets.get(Secret::FastKey).is_some() || settings.fast_is_local()));
    checks.push(
        run_check("Model", || async move {
            if demo {
                let why = if crate::commands::needs_key(&settings) && !has_key { "no API key saved" } else { "Demo preset" };
                return Ok((Status::Skip, format!("demo mode ({why}); add a key or pick a local model in Settings")));
            }
            diagnostics::probe_model(provider.as_ref(), &model).await
        })
        .await,
    );
    if let Some((provider, fast_model, has_key)) = fast_check {
        checks.push(
            run_check("Quick-reply service", || async move {
                if !has_key {
                    return Ok((Status::Warn, "no key saved for it, so the planner's service answers quick replies".into()));
                }
                diagnostics::probe_model(provider.as_ref(), &fast_model).await
            })
            .await,
        );
    }

    for c in &checks {
        match c.status {
            Status::Fail => log::warn!("self-test {}: FAIL {}", c.name, c.detail),
            Status::Warn => log::info!("self-test {}: WARN {}", c.name, c.detail),
            _ => {}
        }
    }
    checks
}

/// Runs the self-test and writes the report into the workspace folder.
pub async fn report(app: &AppHandle) -> anyhow::Result<(Vec<Check>, PathBuf)> {
    let checks = run(app).await;
    let state = app.state::<AppState>();
    let settings = state.settings.read().unwrap().clone();
    let audit = state.audit.recent(AUDIT_ENTRIES).unwrap_or_default();
    let generated_ms = now_ms();
    let text = diagnostics::render_report(&ReportInput {
        app_version: &app.package_info().version.to_string(),
        os: &format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
        generated_ms,
        settings: &settings,
        has_api_key: state.secrets.get(Secret::LlmKey).is_some(),
        demo: state.is_demo(),
        checks: &checks,
        audit: &audit,
        log_tail: &log_tail(app),
    });
    let stamp: String = diagnostics::format_utc(generated_ms).chars().filter(|c| c.is_ascii_digit()).take(14).collect();
    let path = state.workspace().root().join(format!("waddle-report-{stamp}.txt"));
    std::fs::write(&path, text)?;
    log::info!("self-test report written to {}", path.display());
    Ok((checks, path))
}
