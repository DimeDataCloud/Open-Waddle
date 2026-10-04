//! Commands the overlay and settings windows call.

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use waddle_core::audit::{AuditRecord, VerifyReport};
use waddle_core::config::{ProviderKind, VoiceBackend};
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
pub fn send_message(state: State<'_, AppState>, text: String) {
    // Focus stays in the chat box for follow-ups; the bridge hands focus back
    // to the user's app right before Waddle types anything.
    state.session.user_message(text);
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
pub fn answer_approval(state: State<'_, AppState>, id: String, approved: bool) {
    state.host.answer_approval(&id, approved);
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
    #[cfg(windows)]
    let r = std::process::Command::new("explorer").arg(&path).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(&path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(&path).spawn();
    r.map(|_| ()).map_err(err)
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
