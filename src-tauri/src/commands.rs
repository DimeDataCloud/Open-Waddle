//! Commands the overlay and settings windows call.

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use waddle_core::audit::{AuditRecord, VerifyReport};
use waddle_core::config::{ProviderKind, VoiceBackend};
use waddle_core::google::gmail::MailDraft;
use waddle_core::google::{auth, OAuthClient, SCOPES};
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
pub fn send_message(state: State<'_, AppState>, text: String, selection: Option<bool>) {
    // Focus stays in the chat box for follow-ups; the bridge hands focus back
    // to the user's app right before Waddle types anything.
    match selection.filter(|s| *s).and_then(|_| state.host.take_selection()) {
        Some(sel) => state.session.user_message_with_selection(text, sel),
        None => state.session.user_message(text),
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
    state.session.warm();
}

#[tauri::command]
pub fn halt(state: State<'_, AppState>) -> bool {
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
}

#[tauri::command]
pub async fn google_status(state: State<'_, AppState>) -> CmdResult<GoogleStatus> {
    let google = state.google.read().unwrap().clone();
    let (email, error) = match google {
        Some(g) => match g.mail_profile().await {
            Ok((email, _)) => (Some(email), None),
            Err(e) => (None, Some(format!("{e:#}"))),
        },
        None => (None, None),
    };
    Ok(GoogleStatus {
        client_id: state.settings.read().unwrap().google_client_id.clone(),
        has_secret: state.secrets.get(Secret::GoogleClient).is_some(),
        connected: state.google.read().unwrap().is_some(),
        email,
        error,
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
            Ok(Some(text))
        }
        _ => Err(format!("`{action}` doesn't apply to that nudge")),
    }
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
) -> CmdResult<SettingsView> {
    let mut storage = None;
    if let Some(k) = api_key {
        storage = Some(state.secrets.set(Secret::LlmKey, &k).map_err(err)?);
    }
    if let Some(k) = stt_key {
        state.secrets.set(Secret::SttKey, &k).map_err(err)?;
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
            let rec = tokio::task::spawn_blocking(voice::start).await.map_err(err)?.map_err(err)?;
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
