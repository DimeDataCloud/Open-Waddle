//! Commands the overlay and settings windows call.

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use waddle_core::audit::{AuditRecord, VerifyReport};
use waddle_core::config::{ProviderKind, VoiceBackend};
use waddle_core::google::gmail::MailDraft;
use waddle_core::google::{auth, OAuthClient, SCOPES};
use waddle_core::history::Who;
use waddle_core::Settings;

use crate::bridge::Platform;
use crate::overlay::Rect;
use crate::secrets::Secret;
use crate::{actuate, voice, AppState};

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize)]
pub struct Bootstrap {
    pub color: String,
    pub wander: bool,
    pub platform: &'static str,
    pub demo: bool,
    pub voice_backend: VoiceBackend,
    pub windows: Vec<Platform>,
    /// Problems found while starting (an unreadable settings file), said once.
    pub notices: Vec<String>,
}

#[derive(Serialize)]
pub struct SettingsView {
    pub settings: Settings,
    pub has_api_key: bool,
    pub has_stt_key: bool,
    pub workspace: String,
    pub demo: bool,
    pub key_storage: Option<&'static str>,
}

fn view(state: &AppState, key_storage: Option<&'static str>) -> SettingsView {
    SettingsView {
        settings: state.settings.read().unwrap().clone(),
        has_api_key: state.secrets.get(Secret::LlmKey).is_some(),
        has_stt_key: state.secrets.get(Secret::SttKey).is_some(),
        workspace: state.workspace().root().display().to_string(),
        demo: state.is_demo(),
        key_storage,
    }
}

#[tauri::command]
pub fn send_message(state: State<'_, AppState>, text: String, selection: Option<bool>, voice: Option<bool>) {
    // A new message cuts off whatever Waddle was saying.
    state.host.speech.stop();
    state.host.speech.set_by_voice(voice.unwrap_or(false));
    // Focus stays in the chat box for follow-ups; the bridge hands focus back
    // to the user's app right before Waddle types anything.
    match selection.filter(|s| *s).and_then(|_| state.host.take_selection()) {
        Some(sel) => {
            let said = if text.trim().is_empty() { "Help with this".to_string() } else { text.clone() };
            state.host.record(Who::You, &format!("{said} 📎"));
            state.session.user_message_with_selection(text, sel)
        }
        None => {
            state.host.record(Who::You, &text);
            state.session.user_message(text)
        }
    }
}

/// The welcome's "Test" button: one tiny model call with these settings and key
/// (or the saved key). Returns what happened, in plain words.
#[tauri::command]
pub async fn test_key(state: State<'_, AppState>, settings: Settings, key: Option<String>) -> CmdResult<String> {
    let key = key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).or_else(|| state.secrets.get(Secret::LlmKey));
    if needs_key(&settings) && key.is_none() {
        return Err("Paste your key first.".into());
    }
    let provider = waddle_core::llm::build_provider(&settings, key);
    let provider: std::sync::Arc<dyn waddle_core::llm::Provider> = if settings.is_local() { provider } else { std::sync::Arc::new(waddle_core::ledger::Metered::new(provider, state.ledger.clone())) };
    match waddle_core::diagnostics::probe_model(provider.as_ref(), &settings.model).await {
        Ok((_, detail)) => Ok(format!("It works: {detail}.")),
        Err(e) => Err(format!("{e:#}")),
    }
}

/// The models of an Ollama server on this computer.
#[tauri::command]
pub async fn detect_ollama(base_url: Option<String>) -> CmdResult<(Vec<String>, Option<String>)> {
    let base = base_url.unwrap_or_else(|| "http://localhost:11434".into());
    let models = waddle_core::diagnostics::ollama_models(&base).await.map_err(|_| "Ollama isn't running on this computer. Install it from ollama.com, start it, then run: ollama pull qwen3.5:4b".to_string())?;
    let pick = waddle_core::diagnostics::pick_ollama_model(&models);
    Ok((models, pick))
}

/// The voices that can read replies aloud (empty: only the system's default).
#[tauri::command]
pub async fn speech_voices() -> CmdResult<Vec<String>> {
    tauri::async_runtime::spawn_blocking(crate::speech::voices).await.map_err(err)
}

/// Reads a sample sentence in this voice and speed.
#[tauri::command]
pub fn speech_test(state: State<'_, AppState>, voice: String, rate: f64) -> CmdResult<()> {
    if !crate::speech::available() {
        return Err(if cfg!(windows) {
            "Windows has no voices installed. Add one in Settings → Time & language → Speech.".into()
        } else {
            "No speech program found. Install speech-dispatcher or espeak-ng.".into()
        });
    }
    state.host.speech.test(voice, rate);
    Ok(())
}

#[derive(Serialize)]
pub struct RoutineView {
    pub id: String,
    pub goal: String,
    pub schedule: String,
    /// "Tue 6 Oct 08:45", "paused" or "finished".
    pub next: String,
    pub paused: bool,
    pub last_result: Option<String>,
}

fn routine_views(state: &AppState) -> Vec<RoutineView> {
    use chrono::TimeZone;
    state
        .reminders
        .routines
        .list()
        .into_iter()
        .map(|r| RoutineView {
            next: match (r.paused, r.next_ms) {
                (true, _) => "paused".into(),
                (false, Some(ms)) => chrono::Local.timestamp_millis_opt(ms).single().map(|t| t.format("%a %-d %b %H:%M").to_string()).unwrap_or_default(),
                (false, None) => "finished".into(),
            },
            schedule: r.schedule(),
            id: r.id,
            goal: r.goal,
            paused: r.paused,
            last_result: r.last_result,
        })
        .collect()
}

#[tauri::command]
pub fn routines_list(state: State<'_, AppState>) -> Vec<RoutineView> {
    routine_views(&state)
}

/// Adds a routine from Settings (the user's own, so no approval card).
#[tauri::command]
pub fn routine_add(state: State<'_, AppState>, goal: String, time: String, days: Vec<String>) -> CmdResult<Vec<RoutineView>> {
    let mask = waddle_core::routines::parse_days(&serde_json::json!(days)).map_err(err)?;
    state.reminders.routines.add(&goal, &time, mask, None, &chrono::Local::now()).map_err(err)?;
    Ok(routine_views(&state))
}

#[tauri::command]
pub fn routine_pause(state: State<'_, AppState>, id: String, paused: bool) -> CmdResult<Vec<RoutineView>> {
    state.reminders.routines.set_paused(&id, paused, &chrono::Local::now()).map_err(err)?;
    Ok(routine_views(&state))
}

#[tauri::command]
pub fn routine_delete(state: State<'_, AppState>, id: String) -> CmdResult<Vec<RoutineView>> {
    state.reminders.routines.delete(&id).map_err(err)?;
    Ok(routine_views(&state))
}

#[derive(Serialize)]
pub struct McpStatus {
    pub name: String,
    pub tools: Vec<String>,
    pub problem: Option<String>,
    /// Variables that have a saved value.
    pub saved_env: Vec<String>,
}

/// Each MCP server's tools (as last listed) and any problem starting it.
#[tauri::command]
pub fn mcp_status(state: State<'_, AppState>) -> Vec<McpStatus> {
    let settings = state.settings.read().unwrap().clone();
    let env = state.secrets.mcp_env();
    settings
        .mcp_servers
        .iter()
        .map(|s| McpStatus {
            name: s.name.clone(),
            tools: state.mcp.tools_of(&s.name).into_iter().map(|t| t.name).collect(),
            problem: state.mcp.problem(&s.name),
            saved_env: env.get(&s.name).map(|m| m.keys().cloned().collect()).unwrap_or_default(),
        })
        .collect()
}

/// Starts a server once and lists its tools (Settings → Test). Values typed but
/// not saved yet are used along with saved ones.
#[tauri::command]
pub async fn mcp_test(state: State<'_, AppState>, server: waddle_core::config::McpServerSettings, env: std::collections::BTreeMap<String, String>) -> CmdResult<Vec<String>> {
    waddle_core::config::check_mcp_servers(std::slice::from_ref(&server)).map_err(err)?;
    let saved = state.secrets.mcp_env().remove(&server.name).unwrap_or_default();
    let values: Vec<(String, String)> = server
        .env_keys
        .iter()
        .filter_map(|k| env.get(k).filter(|v| !v.is_empty()).or_else(|| saved.get(k)).map(|v| (k.clone(), v.clone())))
        .collect();
    let launch = waddle_core::mcp::Launch { name: server.name, command: server.command, args: server.args, env: values, trusted: server.trusted };
    let tools = waddle_core::mcp::probe(&launch).await.map_err(|e| format!("{e:#}"))?;
    Ok(tools.into_iter().map(|t| if t.read_only { format!("{} (reads only)", t.name) } else { t.name }).collect())
}

/// The conversation so far, for the history drawer.
#[tauri::command]
pub fn history_list(state: State<'_, AppState>) -> Vec<waddle_core::history::Entry> {
    state.host.history.list()
}

/// A link in a reply. Only web links open; anything else in a reply is just text.
#[tauri::command]
pub fn open_link(url: String) -> CmdResult<()> {
    match tauri::Url::parse(url.trim()) {
        Ok(u) if matches!(u.scheme(), "http" | "https") => crate::desktop::open_url(u.as_str()).map_err(err),
        _ => Err("Only web links (http or https) open from a reply.".into()),
    }
}

/// The user removed the selection chip from the chat box.
#[tauri::command]
pub fn drop_selection(state: State<'_, AppState>) {
    let _ = state.host.take_selection();
}

/// Saves a research answer as Markdown in the workspace and opens it.
#[tauri::command]
pub async fn open_answer(state: State<'_, AppState>, id: String) -> CmdResult<String> {
    state.session.open_answer(&id).map(|p| p.display().to_string()).map_err(err)
}

#[tauri::command]
pub async fn facts_list(state: State<'_, AppState>) -> CmdResult<Vec<waddle_core::facts::Fact>> {
    Ok(state.facts.list())
}

#[tauri::command]
pub async fn fact_add(state: State<'_, AppState>, text: String) -> CmdResult<String> {
    state.facts.add(&text).map_err(err)
}

#[tauri::command]
pub async fn fact_forget(state: State<'_, AppState>, id: String) -> CmdResult<String> {
    state.facts.forget(&id).map_err(err)
}

#[tauri::command]
pub fn warm_up(state: State<'_, AppState>) {
    // The chat box opened: the user is about to speak, so Waddle stops.
    state.host.speech.stop();
    state.session.warm();
}

#[tauri::command]
pub fn halt(state: State<'_, AppState>) -> bool {
    state.host.speech.stop();
    state.session.halt()
}

#[tauri::command]
pub fn answer_approval(state: State<'_, AppState>, id: String, approved: bool, draft: Option<MailDraft>) {
    state.host.answer_approval(&id, approved, draft);
}

/// Undo on the bar that follows Send: the email doesn't go.
#[tauri::command]
pub fn undo_send(state: State<'_, AppState>, id: String) {
    state.host.undo_send(&id);
}

#[derive(Serialize)]
pub struct GoogleStatus {
    pub client_id: String,
    pub has_secret: bool,
    pub connected: bool,
    pub email: Option<String>,
    pub error: Option<String>,
    /// Google refused the saved sign-in; Connect signs in again.
    pub expired: bool,
}

#[tauri::command]
pub async fn google_status(state: State<'_, AppState>) -> CmdResult<GoogleStatus> {
    let google = state.google.read().unwrap().clone();
    let (email, error, expired) = match google {
        Some(g) => match g.mail_profile().await {
            Ok((email, _)) => (Some(email), None, false),
            Err(e) => (None, Some(format!("{e}")), g.is_signed_out()),
        },
        None => (None, None, false),
    };
    Ok(GoogleStatus {
        client_id: state.settings.read().unwrap().google_client_id.clone(),
        has_secret: state.secrets.get(Secret::GoogleClient).is_some(),
        connected: state.google.read().unwrap().is_some(),
        email,
        error,
        expired,
    })
}

/// Signs in to Google in the browser (PKCE, loopback redirect) and keeps the refresh token in the keychain.
#[tauri::command]
pub async fn google_connect(state: State<'_, AppState>, client_id: String, client_secret: Option<String>) -> CmdResult<GoogleStatus> {
    let client_id = client_id.trim().to_string();
    if client_id.is_empty() {
        return Err("Paste the OAuth client ID from your Google Cloud project first (see the setup guide).".into());
    }
    if let Some(secret) = client_secret.filter(|s| !s.trim().is_empty()) {
        state.secrets.set(Secret::GoogleClient, &secret).map_err(err)?;
    }
    let client = OAuthClient { id: client_id.clone(), secret: state.secrets.get(Secret::GoogleClient) };
    let cancel = tokio_util::sync::CancellationToken::new();
    if let Some(old) = state.google_signin.lock().unwrap().replace(cancel.clone()) {
        old.cancel();
    }
    let open = |url: &str| {
        if let Err(e) = crate::desktop::open_url(url) {
            log::warn!("{e:#}");
        }
    };
    let tokens = auth::sign_in(&crate::google_endpoints(), &client, SCOPES, open, &cancel, std::time::Duration::from_secs(300)).await.map_err(|e| format!("{e:#}"))?;
    state.google_signin.lock().unwrap().take();
    state.secrets.set(Secret::GoogleRefreshToken, tokens.refresh_token.as_deref().unwrap_or("")).map_err(err)?;
    let mut settings = state.settings.read().unwrap().clone();
    settings.google_client_id = client_id;
    state.apply_settings(settings).map_err(err)?;
    log::info!("connected to Google");
    google_status(state).await
}

#[tauri::command]
pub fn google_cancel(state: State<'_, AppState>) {
    if let Some(c) = state.google_signin.lock().unwrap().take() {
        c.cancel();
    }
}

/// Signs out: Google forgets the grant and the refresh token leaves the keychain.
#[tauri::command]
pub async fn google_disconnect(state: State<'_, AppState>) -> CmdResult<GoogleStatus> {
    if let Some(token) = state.secrets.get(Secret::GoogleRefreshToken) {
        auth::revoke(&crate::google_endpoints(), &token).await;
    }
    state.secrets.set(Secret::GoogleRefreshToken, "").map_err(err)?;
    let settings = state.settings.read().unwrap().clone();
    state.apply_settings(settings).map_err(err)?;
    google_status(state).await
}

/// A button on a nudge. Returns text to show in the bubble, if any.
#[tauri::command]
pub async fn nudge_action(state: State<'_, AppState>, id: String, action: String) -> CmdResult<Option<String>> {
    use waddle_core::nudges::{self, Topic};
    let topic = state.nudges.lock().unwrap().shown.get(&id).cloned();
    let Some(topic) = topic else { return Ok(None) };
    if action == "dismiss" {
        state.nudges.lock().unwrap().shown.remove(&id);
        return Ok(None);
    }
    match (action.as_str(), &topic) {
        ("join", Topic::Meeting { join: Some(url), .. }) => {
            crate::desktop::open_url(url).map_err(err)?;
            Ok(None)
        }
        ("snooze", Topic::Meeting { .. }) => {
            let now = chrono::Local::now().timestamp_millis();
            let ok = state.nudges.lock().unwrap().state.snooze(topic.clone(), now);
            Ok(Some(if ok { "I'll remind you again in 2 minutes.".into() } else { "It's about to start!".into() }))
        }
        ("open", Topic::Mail { thread_id, .. }) => {
            crate::desktop::open_url(&format!("https://mail.google.com/mail/u/0/#all/{thread_id}")).map_err(err)?;
            Ok(None)
        }
        ("reply", Topic::Mail { id, .. }) => {
            // Only the id: the sender and subject are untrusted and mustn't become the user's words.
            state.session.user_message(format!("Help me reply to the email with id {id}: read it, then write the reply on the send card. If what to say isn't clear, ask me first."));
            Ok(None)
        }
        ("mute", Topic::Mail { from, .. }) => {
            let addr = nudges::address(from);
            let mut settings = state.settings.read().unwrap().clone();
            if !settings.muted_senders.iter().any(|m| m.eq_ignore_ascii_case(&addr)) {
                settings.muted_senders.push(addr.clone());
                state.apply_settings(settings).map_err(err)?;
            }
            Ok(Some(format!("Okay, no more nudges about mail from {addr}. (Settings → Nudges to undo.)")))
        }
        ("brief", Topic::Brief) => {
            let google = state.google.read().unwrap().clone().ok_or("Google isn't connected")?;
            let (provider, model) = state.session.fast();
            let decider = state.decider.read().unwrap().clone();
            let text = nudges::compose_brief(&google, provider.as_ref(), &model, decider.as_deref(), chrono::Local::now()).await.map_err(|e| format!("{e:#}"))?;
            state.host.record(Who::Waddle, &text);
            Ok(Some(text))
        }
        _ => Err(format!("`{action}` doesn't apply to that nudge")),
    }
}

#[derive(Serialize)]
pub struct BrowserStatus {
    pub connected: bool,
    pub version: Option<String>,
    pub folder: Option<String>,
}

#[tauri::command]
pub async fn browser_status(state: State<'_, AppState>) -> CmdResult<BrowserStatus> {
    let folder = state.data_dir.join("extension");
    Ok(BrowserStatus {
        connected: state.host.browser.connected(),
        version: state.host.browser.version(),
        folder: folder.exists().then(|| folder.display().to_string()),
    })
}

/// Puts the extension in a folder Chrome can load it from, registers the relay,
/// and opens the folder. Chrome needs one manual step: Load unpacked.
#[tauri::command]
pub async fn browser_setup(state: State<'_, AppState>) -> CmdResult<BrowserStatus> {
    let folder = crate::browser::install_extension(&state.data_dir).map_err(err)?;
    crate::browser::register_host(&state.data_dir).map_err(err)?;
    if let Err(e) = crate::desktop::open_path(&folder) {
        log::warn!("{e:#}");
    }
    browser_status(state).await
}

/// What Waddle has spent: today, this month by purpose, the last 30 days, and the budget.
#[tauri::command]
pub fn spending(state: State<'_, AppState>) -> waddle_core::ledger::Summary {
    state.ledger.summary(&chrono::Local::now())
}

#[tauri::command]
pub async fn style_get(state: State<'_, AppState>) -> CmdResult<String> {
    Ok(state.style.get().unwrap_or_default())
}

#[tauri::command]
pub async fn style_set(state: State<'_, AppState>, text: String) -> CmdResult<()> {
    state.style.set(&text).map_err(err)
}

#[tauri::command]
pub fn duck_arrived(state: State<'_, AppState>, id: String) {
    state.host.duck_arrived(&id);
}

#[tauri::command]
pub fn set_hit_rects(state: State<'_, AppState>, rects: Vec<Rect>) {
    state.overlay.set_rects(rects);
}

#[tauri::command]
pub fn set_capture(state: State<'_, AppState>, on: bool) {
    state.overlay.set_capture(on);
}

#[tauri::command]
pub fn bootstrap(state: State<'_, AppState>) -> Bootstrap {
    let s = state.settings.read().unwrap();
    Bootstrap {
        color: s.character.color.clone(),
        wander: s.wander,
        platform: if cfg!(windows) { "windows" } else if cfg!(target_os = "macos") { "macos" } else { "linux" },
        demo: state.is_demo(),
        voice_backend: s.voice.backend,
        windows: state.host.platforms(),
        notices: std::mem::take(&mut *state.startup_notices.lock().unwrap()),
    }
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> CmdResult<SettingsView> {
    Ok(view(&state, None))
}

#[tauri::command]
pub async fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
    api_key: Option<String>,
    stt_key: Option<String>,
    mcp_env: Option<crate::secrets::McpEnv>,
) -> CmdResult<SettingsView> {
    waddle_core::config::check_mcp_servers(&settings.mcp_servers).map_err(err)?;
    let mut storage = None;
    if let Some(k) = api_key {
        storage = Some(state.secrets.set(Secret::LlmKey, &k).map_err(err)?);
    }
    if let Some(k) = stt_key {
        state.secrets.set(Secret::SttKey, &k).map_err(err)?;
    }
    // New MCP values replace old ones; values of servers or variables that are gone are dropped.
    let mut env = state.secrets.mcp_env();
    for (server, values) in mcp_env.unwrap_or_default() {
        let slot = env.entry(server).or_default();
        for (k, v) in values {
            if !v.is_empty() {
                slot.insert(k, v);
            }
        }
    }
    env.retain(|server, values| match settings.mcp_servers.iter().find(|s| &s.name == server) {
        Some(s) => {
            values.retain(|k, _| s.env_keys.contains(k));
            !values.is_empty()
        }
        None => false,
    });
    if env != state.secrets.mcp_env() {
        state.secrets.set_mcp_env(&env).map_err(err)?;
    }
    state.apply_settings(settings).map_err(err)?;
    let s = state.settings.read().unwrap().clone();
    crate::sync_autostart(&app, s.autostart);
    let _ = app.emit_to("overlay", "settings", serde_json::json!({ "color": s.character.color, "wander": s.wander, "demo": state.is_demo() }));
    Ok(view(&state, storage))
}

#[tauri::command]
pub async fn audit_recent(state: State<'_, AppState>, limit: usize) -> CmdResult<Vec<AuditRecord>> {
    state.audit.recent(limit.min(500)).map_err(err)
}

#[tauri::command]
pub async fn audit_verify(state: State<'_, AppState>) -> CmdResult<VerifyReport> {
    state.audit.verify().map_err(err)
}

/// Async on purpose: creating a window from a sync command deadlocks WebView2
/// on Windows (white window, frozen overlay).
#[tauri::command]
pub async fn open_settings(app: AppHandle) -> CmdResult<()> {
    crate::show_settings(&app).map_err(err)
}

#[tauri::command]
pub async fn open_workspace(state: State<'_, AppState>) -> CmdResult<()> {
    let path = state.workspace().root().to_path_buf();
    crate::desktop::open_path(&path).map_err(err)
}

#[tauri::command]
pub fn clear_memory(state: State<'_, AppState>) {
    state.session.clear_memory();
    state.host.history.clear();
    state.host.emit_overlay("history:changed", ());
}

#[derive(Serialize)]
pub struct SkillView {
    pub name: String,
    pub body: String,
}

#[tauri::command]
pub async fn skills_list(state: State<'_, AppState>) -> CmdResult<Vec<SkillView>> {
    Ok(state.skills.list().into_iter().map(|s| SkillView { name: s.name, body: s.body }).collect())
}

#[tauri::command]
pub async fn skill_forget(state: State<'_, AppState>, name: String) -> CmdResult<String> {
    state.skills.forget(&name).map_err(err)
}

/// Starts listening. Returns "system" (OS dictation types into the focused chat box) or "recording".
#[tauri::command]
pub async fn voice_start(state: State<'_, AppState>) -> CmdResult<&'static str> {
    state.host.speech.stop();
    let backend = state.settings.read().unwrap().voice.backend;
    match backend {
        VoiceBackend::Off => Err("Voice input is off. Turn it on in Settings.".into()),
        VoiceBackend::System => {
            // Windows voice typing (Win+H) types live into whatever has focus: our chat box.
            if !cfg!(windows) {
                return Err("System dictation can't be started automatically here. Use your OS dictation shortcut, or pick the Whisper backend in Settings.".into());
            }
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            tokio::task::spawn_blocking(|| actuate::press_combo(&[enigo::Key::Meta, enigo::Key::Unicode('h')]))
                .await
                .map_err(err)?
                .map_err(err)?;
            Ok("system")
        }
        VoiceBackend::WhisperApi => {
            let rec = tokio::task::spawn_blocking(voice::start).await.map_err(err)?.map_err(|e| voice::plain(&e))?;
            *state.recording.lock().unwrap() = Some(rec);
            Ok("recording")
        }
    }
}

/// Stops listening. For the Whisper backend, returns the transcript.
#[tauri::command]
pub async fn voice_stop(state: State<'_, AppState>) -> CmdResult<Option<String>> {
    let rec = state.recording.lock().unwrap().take();
    let Some(rec) = rec else {
        if state.settings.read().unwrap().voice.backend == VoiceBackend::System && cfg!(windows) {
            let _ = tokio::task::spawn_blocking(|| actuate::press_combo(&[enigo::Key::Meta, enigo::Key::Unicode('h')])).await;
        }
        return Ok(None);
    };
    let pcm = tokio::task::spawn_blocking(move || rec.finish()).await.map_err(err)?;
    if pcm.len() < (voice::target_rate() / 4) as usize {
        return Ok(None);
    }
    let wav = waddle_core::stt::wav_from_pcm16(&pcm, voice::target_rate());
    let vs = state.settings.read().unwrap().voice.clone();
    let key = state.secrets.get(Secret::SttKey);
    let text = waddle_core::stt::transcribe(&vs.base_url, key.as_deref(), &vs.model, vs.language.as_deref(), wav)
        .await
        .map_err(err)?;
    Ok(if text.is_empty() { None } else { Some(text) })
}

#[derive(Serialize)]
pub struct SelfTestView {
    pub checks: Vec<waddle_core::diagnostics::Check>,
    pub report: String,
}

/// Runs every check (including one small model call) and saves the report in the workspace.
#[tauri::command]
pub async fn run_self_test(app: AppHandle) -> CmdResult<SelfTestView> {
    let (checks, path) = crate::selftest::report(&app).await.map_err(err)?;
    Ok(SelfTestView { checks, report: path.display().to_string() })
}

#[tauri::command]
pub fn quit(app: AppHandle) {
    app.exit(0);
}

/// Whether the current settings can run without a key we don't have.
pub fn needs_key(settings: &Settings) -> bool {
    settings.provider == ProviderKind::OpenaiCompat
        && ["openrouter.ai", "api.openai.com", "api.groq.com"].iter().any(|h| settings.base_url.contains(h))
}

/// The user's 👍/👎 on a saved task.
#[tauri::command]
pub async fn rate_task(state: State<'_, AppState>, task_id: String, good: bool) -> CmdResult<()> {
    state.traces.rate(&task_id, good).map_err(err)
}

#[derive(Serialize)]
pub struct TracesSummary {
    pub folder: String,
    pub total: usize,
    pub good: usize,
    pub bad: usize,
}

#[tauri::command]
pub async fn traces_summary(state: State<'_, AppState>) -> CmdResult<TracesSummary> {
    let list = state.traces.list();
    Ok(TracesSummary {
        folder: state.traces.dir().display().to_string(),
        total: list.len(),
        good: list.iter().filter(|m| m.rating == Some(true)).count(),
        bad: list.iter().filter(|m| m.rating == Some(false)).count(),
    })
}

/// Writes train.jsonl (good tasks as fine-tuning examples) into the workspace and returns its path.
#[tauri::command]
pub async fn export_traces(state: State<'_, AppState>) -> CmdResult<String> {
    let out = state.workspace().root().join("training");
    let (file, n) = state.traces.export(&out, true).map_err(err)?;
    Ok(format!("{} ({n} examples)", file.display()))
}
