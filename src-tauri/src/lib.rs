//! Project Waddle desktop app: the Body (transparent overlay), the GUI Hands
//! and Eyes, and the glue that connects them to `waddle-core`.

mod ambient;
mod actuate;
mod bridge;
mod commands;
mod desktop;
mod overlay;
mod presence;
mod secrets;
mod selftest;
mod shortcuts;
#[cfg(windows)]
mod uia;
mod voice;
mod watch;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder};
use waddle_core::audit::AuditLog;
use waddle_core::config::ProviderKind;
use waddle_core::llm::{build_provider, mock::MockProvider, Provider};
use waddle_core::decide::{Decider, Jev};
use waddle_core::facts::FactStore;
use waddle_core::google::style::StyleNote;
use waddle_core::google::{Endpoints, Google, OAuthClient};
use waddle_core::reminders::ReminderStore;
use waddle_core::skills::SkillStore;
use waddle_core::traces::TraceStore;
use waddle_core::tools::fs::Workspace;
use waddle_core::{Session, SessionConfig, Settings};

use crate::bridge::TauriHost;
use crate::overlay::{Geometry, OverlayState};
use crate::secrets::{Secret, Secrets};

pub struct AppState {
    pub session: Arc<Session>,
    pub host: Arc<TauriHost>,
    pub overlay: Arc<OverlayState>,
    pub audit: Arc<AuditLog>,
    pub settings: RwLock<Settings>,
    pub secrets: Secrets,
    pub recording: Mutex<Option<voice::Recording>>,
    pub skills: Arc<SkillStore>,
    pub reminders: Arc<ReminderStore>,
    pub facts: Arc<FactStore>,
    pub decider: RwLock<Option<Arc<dyn Decider>>>,
    pub traces: Arc<TraceStore>,
    /// The signed-in Google account, if any.
    pub google: RwLock<Option<Arc<Google>>>,
    pub style: Arc<StyleNote>,
    /// Cancels a Google sign-in that's waiting for the browser.
    pub google_signin: Mutex<Option<tokio_util::sync::CancellationToken>>,
    /// Meeting, mail and brief nudges: the watcher's memory and what's on screen.
    pub nudges: watch::Shared,
    settings_path: PathBuf,
    workspace: RwLock<Arc<Workspace>>,
    demo: RwLock<bool>,
}

fn default_workspace(app: &AppHandle) -> PathBuf {
    app.path()
        .document_dir()
        .or_else(|_| app.path().home_dir())
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("Waddle")
}

/// Picks the provider for these settings. Without a key for a hosted API,
/// Waddle runs its scripted demo instead of failing on the first message.
pub(crate) fn provider_for(settings: &Settings, secrets: &Secrets) -> (Arc<dyn Provider>, bool) {
    let forced_mock = std::env::var("WADDLE_PROVIDER").map(|v| v == "mock").unwrap_or(false);
    let key = secrets.get(Secret::LlmKey);
    if forced_mock || settings.provider == ProviderKind::Mock || (key.is_none() && commands::needs_key(settings)) {
        return (Arc::new(MockProvider::demo()), true);
    }
    (build_provider(settings, key), false)
}

/// Quick decisions (Jev) need an OpenRouter key; the demo and other endpoints get none.
pub(crate) fn decider_for(settings: &Settings, secrets: &Secrets, demo: bool) -> Option<Arc<dyn Decider>> {
    if demo {
        return None;
    }
    let key = secrets.get(Secret::LlmKey);
    Jev::for_settings(settings, key.as_deref()).map(|j| Arc::new(j) as Arc<dyn Decider>)
}

/// Google's servers, or a fake on this machine for development (`WADDLE_GOOGLE_BASE`).
pub(crate) fn google_endpoints() -> Endpoints {
    match std::env::var("WADDLE_GOOGLE_BASE") {
        Ok(base) if base.starts_with("http://127.0.0.1:") || base.starts_with("http://localhost:") => Endpoints::at(&base),
        _ => Endpoints::google(),
    }
}

/// The Google account, when the user has set a client ID and signed in.
pub(crate) fn google_for(settings: &Settings, secrets: &Secrets) -> Option<Arc<Google>> {
    let id = settings.google_client_id.trim();
    if id.is_empty() {
        return None;
    }
    let refresh = secrets.get(Secret::GoogleRefreshToken)?;
    let client = OAuthClient { id: id.to_string(), secret: secrets.get(Secret::GoogleClient) };
    Some(Arc::new(Google::new(google_endpoints(), client, refresh)))
}

/// Waddle's own source folder, if the user turned self-editing on.
fn self_source_for(settings: &Settings) -> anyhow::Result<Option<Arc<Workspace>>> {
    match &settings.self_source_dir {
        Some(dir) if dir.is_dir() => Ok(Some(Arc::new(Workspace::new(dir)?))),
        Some(dir) => anyhow::bail!("self-editing folder {} does not exist", dir.display()),
        None => Ok(None),
    }
}

impl AppState {
    pub fn workspace(&self) -> Arc<Workspace> {
        self.workspace.read().unwrap().clone()
    }

    pub fn is_demo(&self) -> bool {
        *self.demo.read().unwrap()
    }

    pub fn apply_settings(&self, settings: Settings) -> anyhow::Result<()> {
        let ws_dir = settings.workspace_dir.clone().unwrap_or_else(|| self.workspace().root().to_path_buf());
        let workspace = Arc::new(Workspace::new(ws_dir)?);
        let (provider, demo) = provider_for(&settings, &self.secrets);
        let decider = decider_for(&settings, &self.secrets, demo);
        let self_source = self_source_for(&settings)?;
        let google = google_for(&settings, &self.secrets);
        if let Some(dir) = self.settings_path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&self.settings_path, serde_json::to_string_pretty(&settings)?)?;
        self.session.configure(SessionConfig {
            settings: settings.clone(),
            provider,
            workspace: workspace.clone(),
            skills: Some(self.skills.clone()),
            reminders: Some(self.reminders.clone()),
            facts: Some(self.facts.clone()),
            decider: decider.clone(),
            self_source,
            traces: Some(self.traces.clone()),
            google: google.clone(),
            style: Some(self.style.clone()),
        });
        *self.google.write().unwrap() = google;
        *self.settings.write().unwrap() = settings;
        *self.workspace.write().unwrap() = workspace;
        *self.demo.write().unwrap() = demo;
        *self.decider.write().unwrap() = decider;
        Ok(())
    }
}

/// Starts Waddle at sign-in, or stops doing so. Release builds only: a development
/// build shouldn't register itself to start with the computer.
pub fn sync_autostart(app: &AppHandle, on: bool) {
    if cfg!(debug_assertions) {
        return;
    }
    use tauri_plugin_autostart::ManagerExt;
    let launcher = app.autolaunch();
    if launcher.is_enabled().unwrap_or(!on) == on {
        return;
    }
    let r = if on { launcher.enable() } else { launcher.disable() };
    if let Err(e) = r {
        log::warn!("autostart: {e}");
    }
}

/// Gives the overlay keyboard focus so the chat box can take typing.
pub fn focus_overlay(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("overlay") {
        let _ = w.set_focus();
    }
}

/// Shows (or creates) the settings window. Never call this from a sync
/// command or an event handler: on Windows, creating a WebView2 window there
/// deadlocks the main thread. Use an async command or `open_settings_soon`.
pub fn show_settings(app: &AppHandle) -> tauri::Result<()> {
    if let Some(w) = app.get_webview_window("settings") {
        w.show()?;
        w.set_focus()?;
        return Ok(());
    }
    WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("settings.html".into()))
        .title("Waddle Settings")
        .inner_size(560.0, 720.0)
        .min_inner_size(420.0, 480.0)
        .build()?;
    Ok(())
}

/// Opens the settings window from an event handler, off the main thread.
pub fn open_settings_soon(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = show_settings(&app) {
            log::warn!("could not open settings: {e}");
        }
    });
}

/// Places the overlay over the primary monitor's work area (not the full
/// screen, so Windows doesn't treat it as a full-screen app).
fn place_overlay(app: &AppHandle) -> anyhow::Result<Geometry> {
    let window = app.get_webview_window("overlay").ok_or_else(|| anyhow::anyhow!("overlay window missing"))?;
    let monitor = window
        .primary_monitor()?
        .or(window.current_monitor()?)
        .ok_or_else(|| anyhow::anyhow!("no monitor"))?;
    let work = monitor.work_area();
    let geometry = Geometry {
        origin_x: work.position.x,
        origin_y: work.position.y,
        width: work.size.width,
        height: work.size.height,
        scale: monitor.scale_factor(),
        screen_w: monitor.size().width,
        screen_h: monitor.size().height,
        screen_x: monitor.position().x,
        screen_y: monitor.position().y,
    };
    window.set_position(PhysicalPosition::new(work.position.x, work.position.y))?;
    window.set_size(PhysicalSize::new(work.size.width, work.size.height))?;
    window.show()?;
    Ok(geometry)
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let talk = MenuItem::with_id(app, "talk", "Talk to Waddle", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let wander = MenuItem::with_id(app, "wander", "Pause / resume wandering", true, None::<&str>)?;
    let halt = MenuItem::with_id(app, "halt", "Halt current task", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Waddle", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&talk, &settings, &wander, &halt, &sep, &quit])?;
    let mut tray = TrayIconBuilder::with_id("waddle").tooltip("Project Waddle").menu(&menu);
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.on_menu_event(|app, event| match event.id.as_ref() {
        "talk" => {
            focus_overlay(app);
            let _ = app.emit_to("overlay", "chat:open", serde_json::json!({ "voice": false }));
        }
        "settings" => open_settings_soon(app),
        "wander" => {
            let _ = app.emit_to("overlay", "wander:toggle", ());
        }
        "halt" => {
            if let Some(state) = app.try_state::<AppState>() {
                state.session.halt();
            }
        }
        "quit" => app.exit(0),
        _ => {}
    })
    .build(app)?;
    Ok(())
}

fn setup(app: &mut tauri::App) -> anyhow::Result<()> {
    let handle = app.handle().clone();
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("panic: {info}");
        default_hook(info);
    }));
    log::info!("Waddle {} starting on {} {}", app.package_info().version, std::env::consts::OS, std::env::consts::ARCH);
    let config_dir = handle.path().app_config_dir()?;
    let data_dir = handle.path().app_data_dir()?;
    let settings_path = config_dir.join("settings.json");
    let settings: Settings = std::fs::read_to_string(&settings_path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let secrets = Secrets::new(&config_dir);
    let audit = Arc::new(AuditLog::open(&data_dir.join("audit.sqlite"))?);
    let workspace = Arc::new(Workspace::new(settings.workspace_dir.clone().unwrap_or_else(|| default_workspace(&handle)))?);

    let geometry = place_overlay(&handle)?;
    let overlay = OverlayState::new(geometry);
    let host = TauriHost::new(handle.clone(), overlay.clone());
    let (provider, demo) = provider_for(&settings, &secrets);
    let decider = decider_for(&settings, &secrets, demo);
    let skills = Arc::new(SkillStore::new(data_dir.join("skills"))?);
    let traces = Arc::new(TraceStore::new(data_dir.join("traces")));
    let reminders = Arc::new(ReminderStore::new(data_dir.join("reminders.json")));
    let facts = Arc::new(FactStore::new(data_dir.join("facts.json")));
    let style = Arc::new(StyleNote::new(data_dir.join("style.md")));
    let nudges = watch::load(data_dir.join("nudges.json"));
    sync_autostart(&handle, settings.autostart);
    let google = google_for(&settings, &secrets);
    let self_source = self_source_for(&settings).unwrap_or_else(|e| {
        log::warn!("{e}");
        None
    });
    let tauri::async_runtime::RuntimeHandle::Tokio(rt) = tauri::async_runtime::handle();
    let session = Session::new(
        rt,
        host.clone(),
        audit.clone(),
        SessionConfig {
            settings: settings.clone(),
            provider,
            workspace: workspace.clone(),
            skills: Some(skills.clone()),
            reminders: Some(reminders.clone()),
            facts: Some(facts.clone()),
            decider: decider.clone(),
            self_source,
            traces: Some(traces.clone()),
            google: google.clone(),
            style: Some(style.clone()),
        },
    );

    app.manage(AppState {
        session,
        host: host.clone(),
        overlay: overlay.clone(),
        audit,
        settings: RwLock::new(settings),
        secrets,
        recording: Mutex::default(),
        skills,
        reminders: reminders.clone(),
        facts,
        decider: RwLock::new(decider),
        traces,
        google: RwLock::new(google),
        style,
        google_signin: Mutex::default(),
        nudges: nudges.clone(),
        settings_path,
        workspace: RwLock::new(workspace),
        demo: RwLock::new(demo),
    });

    overlay::spawn_hit_test(handle.clone(), overlay);
    bridge::spawn_reminder_clock(host.clone(), reminders);
    ambient::spawn(handle.clone(), host.clone());
    watch::spawn(handle.clone(), host.clone(), nudges);
    bridge::spawn_window_sampler(host);
    build_tray(&handle)?;
    {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        if let Err(e) = handle.global_shortcut().register(shortcuts::talk_key()) {
            log::warn!("could not register Ctrl+Alt+Space: {e}");
        }
        if let Err(e) = handle.global_shortcut().register(shortcuts::selection_key()) {
            log::warn!("could not register Ctrl+Alt+A: {e}");
        }
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // A log file in the OS log folder: release builds on Windows have no console.
        .plugin(
            tauri_plugin_log::Builder::new()
                .clear_targets()
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir { file_name: Some("waddle".into()) }),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                ])
                .level(log::LevelFilter::Info)
                .max_file_size(2_000_000)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepOne)
                .build(),
        )
        .plugin(tauri_plugin_global_shortcut::Builder::new().with_handler(shortcuts::handle).build())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .setup(|app| {
            setup(app).map_err(|e| {
                eprintln!("Waddle failed to start: {e:#}");
                e.into()
            })
        })
        .invoke_handler(tauri::generate_handler![
            commands::send_message,
            commands::warm_up,
            commands::run_self_test,
            commands::halt,
            commands::answer_approval,
            commands::duck_arrived,
            commands::set_hit_rects,
            commands::set_capture,
            commands::bootstrap,
            commands::get_settings,
            commands::save_settings,
            commands::audit_recent,
            commands::audit_verify,
            commands::open_settings,
            commands::open_workspace,
            commands::clear_memory,
            commands::skills_list,
            commands::skill_forget,
            commands::facts_list,
            commands::fact_add,
            commands::fact_forget,
            commands::open_answer,
            commands::undo_send,
            commands::nudge_action,
            commands::google_status,
            commands::google_connect,
            commands::google_cancel,
            commands::google_disconnect,
            commands::style_get,
            commands::style_set,
            commands::drop_selection,
            commands::rate_task,
            commands::traces_summary,
            commands::export_traces,
            commands::voice_start,
            commands::voice_stop,
            commands::quit,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Waddle");
}
