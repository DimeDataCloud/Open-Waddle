//! User-editable settings. API keys are deliberately not stored here; the app
//! keeps them in the OS keychain and passes them in when building providers.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// Any OpenAI-compatible endpoint: OpenRouter, OpenAI, LM Studio, llama.cpp, Foundry Local.
    OpenaiCompat,
    /// Native Ollama API, which exposes the resource controls we need for local mode.
    Ollama,
    /// Scripted provider for demos and tests. Never calls the network.
    Mock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordMode {
    /// Pick from the model name.
    Auto,
    /// x,y are pixels of the latest screenshot (screenshots are taken at logical screen size).
    Pixels,
    /// x,y are normalised 0..1000 across the screen (Qwen-VL, Gemini, Gemma families).
    Norm1000,
}

impl CoordMode {
    pub fn resolve(self, model: &str) -> CoordMode {
        match self {
            CoordMode::Auto => {
                let m = model.to_ascii_lowercase();
                if ["qwen", "gemini", "gemma"].iter().any(|k| m.contains(k)) {
                    CoordMode::Norm1000
                } else {
                    CoordMode::Pixels
                }
            }
            other => other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier2Mode {
    /// Show the action with a short cancel window, then proceed.
    Countdown,
    /// Always wait for an explicit click, like tier 3.
    Ask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceBackend {
    /// The OS dictation feature types into the chat box (Windows voice typing, Win+H).
    System,
    /// Record in-app and send to an OpenAI-compatible /audio/transcriptions endpoint.
    WhisperApi,
    Off,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct OllamaSettings {
    /// CPU threads for inference. None = a third of the cores, leaving room for other work.
    pub num_thread: Option<u32>,
    /// How long the model stays in memory after a request ("30s", "5m", "0").
    pub keep_alive: String,
    pub num_ctx: u32,
}

impl Default for OllamaSettings {
    fn default() -> Self {
        Self { num_thread: None, keep_alive: "30s".into(), num_ctx: 8192 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct VoiceSettings {
    pub backend: VoiceBackend,
    pub base_url: String,
    pub model: String,
    pub language: Option<String>,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            backend: if cfg!(windows) { VoiceBackend::System } else { VoiceBackend::WhisperApi },
            base_url: "https://api.groq.com/openai/v1".into(),
            model: "whisper-large-v3-turbo".into(),
            language: None,
        }
    }
}

/// How much hidden thinking a hosted model may do before answering (OpenRouter's
/// `reasoning.effort`). Thinking makes each step slower and dearer; Waddle's steps
/// are small, so most models do best with it off or low.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Reasoning {
    /// Don't say; the model's own default applies.
    #[default]
    Default,
    Off,
    Low,
    Medium,
    High,
}

impl Reasoning {
    /// The OpenRouter effort name, or None to leave the request unchanged.
    pub fn effort(self) -> Option<&'static str> {
        match self {
            Reasoning::Default => None,
            Reasoning::Off => Some("none"),
            Reasoning::Low => Some("low"),
            Reasoning::Medium => Some("medium"),
            Reasoning::High => Some("high"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CharacterSettings {
    pub id: String,
    /// Base body colour; shading is derived from it at runtime.
    pub color: String,
}

impl Default for CharacterSettings {
    fn default() -> Self {
        Self { id: "waddle".into(), color: "#FFD23F".into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub provider: ProviderKind,
    pub base_url: String,
    /// Planner model (System 2): reasons and calls tools.
    pub model: String,
    /// Quick-reply model (System 1 lane): answers while a task is running. Empty = same as `model`.
    pub fast_model: String,
    pub coord_mode: CoordMode,
    /// Hidden thinking for hosted models (OpenAI-compatible endpoints only).
    pub reasoning: Reasoning,
    /// Start every task with a screenshot (and the window list) attached to the request.
    pub look_first: bool,
    /// Skip that opening screenshot when a quick check (TypeSafe Jev, on OpenRouter)
    /// is confident the task doesn't need the screen: reminders, files, maths, writing.
    pub smart_look: bool,
    /// Describe each step in words. Off: Waddle acts silently (the overlay shows
    /// each action visually) and only speaks to answer, ask or finish.
    pub narrate: bool,
    /// Answer small talk ("thanks!", "tell me a joke") straight away with the fast
    /// model, when a quick check (Jev, on OpenRouter) is sure it needs no computer use.
    pub quick_chat: bool,
    /// Let a quick check (Jev, on OpenRouter) pick what the duck does between tasks,
    /// from the front app's name and how long the user has been idle (never window titles).
    pub ambient_brain: bool,
    /// Save each task (conversation and screenshots) on this computer as training data.
    pub record_traces: bool,
    /// On OpenRouter, use only providers that don't store or train on what Waddle sends.
    pub no_training: bool,
    pub tier2_mode: Tier2Mode,
    pub tier2_countdown_ms: u64,
    pub max_steps: u32,
    pub task_timeout_secs: u64,
    pub command_timeout_secs: u64,
    /// None = Documents/Waddle.
    pub workspace_dir: Option<PathBuf>,
    pub wander: bool,
    /// Folder holding Waddle's own source code. When set, Waddle can read it
    /// (as `self/...`) and edit it with approval. None = self-editing off.
    pub self_source_dir: Option<PathBuf>,
    /// How deep `delegate` may nest sub-tasks. 0 disables delegation.
    pub max_delegation_depth: u32,
    pub character: CharacterSettings,
    pub ollama: OllamaSettings,
    pub voice: VoiceSettings,
    /// OAuth client ID of the user's Google Cloud project (Desktop app type). Empty = Google off.
    pub google_client_id: String,
    /// Hours (local, 24h) that `calendar_free` proposes meetings in, on weekdays.
    pub working_hours: (u32, u32),
    /// After Send on the send card, how long Undo stays available before the email goes.
    pub send_undo_secs: u64,
    /// Paid model calls pause when this month's spending reaches this many US dollars. 0 = no limit.
    pub monthly_budget: f64,
    /// Tap the user on the shoulder five minutes before a meeting (needs Google).
    pub meeting_nudges: bool,
    /// Tell the user about important new email as it arrives (needs Google).
    pub mail_nudges: bool,
    /// Offer a morning brief on the first activity after 06:00 (needs Google).
    pub morning_brief: bool,
    /// Senders never nudged about: addresses, or whole domains as `@example.com`.
    pub muted_senders: Vec<String>,
    /// Start Waddle when the user signs in to Windows.
    pub autostart: bool,
    /// Let Waddle read the user's own folders (Documents, Downloads, Desktop,
    /// Pictures, Music, Videos) and Drive for desktop.
    pub read_user_folders: bool,
    /// More folders Waddle may read in.
    pub read_folders: Vec<PathBuf>,
    /// Folders Waddle may create, change, move and rename files in, besides its workspace.
    pub write_folders: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: ProviderKind::OpenaiCompat,
            base_url: "https://openrouter.ai/api/v1".into(),
            model: "openai/gpt-6-luna".into(),
            fast_model: "google/gemini-2.5-flash-lite".into(),
            coord_mode: CoordMode::Auto,
            reasoning: Reasoning::Default,
            look_first: true,
            smart_look: true,
            narrate: false,
            quick_chat: true,
            ambient_brain: true,
            record_traces: false,
            no_training: true,
            tier2_mode: Tier2Mode::Countdown,
            tier2_countdown_ms: 2000,
            max_steps: 20,
            task_timeout_secs: 300,
            command_timeout_secs: 60,
            workspace_dir: None,
            wander: true,
            self_source_dir: None,
            max_delegation_depth: 2,
            character: CharacterSettings::default(),
            ollama: OllamaSettings::default(),
            voice: VoiceSettings::default(),
            google_client_id: String::new(),
            working_hours: (9, 17),
            send_undo_secs: 10,
            monthly_budget: 5.0,
            meeting_nudges: true,
            mail_nudges: true,
            morning_brief: true,
            muted_senders: vec![],
            autostart: true,
            read_user_folders: true,
            read_folders: vec![],
            write_folders: vec![],
        }
    }
}

impl Settings {
    pub fn fast_model(&self) -> &str {
        if self.fast_model.trim().is_empty() {
            &self.model
        } else {
            &self.fast_model
        }
    }

    pub fn coord_mode(&self) -> CoordMode {
        self.coord_mode.resolve(&self.model)
    }

    /// The model runs on this machine (Ollama, or an OpenAI-compatible server on localhost).
    pub fn is_local(&self) -> bool {
        self.provider == ProviderKind::Ollama || ["://localhost", "://127.0.0.1", "://[::1]"].iter().any(|h| self.base_url.contains(h))
    }

    pub fn is_openrouter(&self) -> bool {
        self.provider == ProviderKind::OpenaiCompat && self.base_url.contains("openrouter.ai")
    }

    /// How many screenshots a task keeps in its conversation before older ones are dropped.
    /// Hosted APIs bill every image on every call, so they keep one. Local servers reuse their
    /// prompt cache only while the conversation is append-only, so they keep as many as fit
    /// (about 1400 tokens each, leaving 4096 for the prompt, tools and steps).
    pub fn image_budget(&self) -> usize {
        if !self.is_local() {
            return 1;
        }
        (self.ollama.num_ctx.saturating_sub(4096) / 1400).clamp(1, 3) as usize
    }
}

/// Settings Waddle may change about itself (with approval). Endpoints, keys,
/// folders and the self-editing switch stay user-only, so a compromised task
/// can't redirect traffic or widen its own reach.
pub const SELF_EDITABLE: &[&str] = &[
    "model",
    "fast_model",
    "coord_mode",
    "reasoning",
    "look_first",
    "smart_look",
    "narrate",
    "quick_chat",
    "ambient_brain",
    "wander",
    "color",
    "tier2_mode",
    "tier2_countdown_ms",
    "max_steps",
    "command_timeout_secs",
    "voice_backend",
];

fn parse_enum<T: serde::de::DeserializeOwned>(key: &str, v: &serde_json::Value) -> anyhow::Result<T> {
    serde_json::from_value(v.clone()).map_err(|e| anyhow::anyhow!("`{key}`: {e}"))
}

/// Applies a flat patch like `{"model": "...", "wander": false}` to a copy of `base`.
/// Returns the new settings and the keys that changed.
pub fn apply_patch(base: &Settings, patch: &serde_json::Value) -> anyhow::Result<(Settings, Vec<String>)> {
    let obj = patch.as_object().ok_or_else(|| anyhow::anyhow!("`changes` must be an object"))?;
    if obj.is_empty() {
        anyhow::bail!("no changes given");
    }
    let mut next = base.clone();
    let mut changed = vec![];
    for (k, v) in obj {
        if !SELF_EDITABLE.contains(&k.as_str()) {
            anyhow::bail!("`{k}` can only be changed by the user in Settings. Editable: {}", SELF_EDITABLE.join(", "));
        }
        let text = || v.as_str().map(str::to_string).ok_or_else(|| anyhow::anyhow!("`{k}` must be a string"));
        let num = |lo: u64, hi: u64| {
            v.as_u64().filter(|n| (lo..=hi).contains(n)).ok_or_else(|| anyhow::anyhow!("`{k}` must be a number from {lo} to {hi}"))
        };
        match k.as_str() {
            "model" => next.model = text()?,
            "fast_model" => next.fast_model = text()?,
            "coord_mode" => next.coord_mode = parse_enum(k, v)?,
            "reasoning" => next.reasoning = parse_enum(k, v)?,
            "look_first" => next.look_first = v.as_bool().ok_or_else(|| anyhow::anyhow!("`look_first` must be true or false"))?,
            "narrate" => next.narrate = v.as_bool().ok_or_else(|| anyhow::anyhow!("`narrate` must be true or false"))?,
            "smart_look" => next.smart_look = v.as_bool().ok_or_else(|| anyhow::anyhow!("`smart_look` must be true or false"))?,
            "quick_chat" => next.quick_chat = v.as_bool().ok_or_else(|| anyhow::anyhow!("`quick_chat` must be true or false"))?,
            "ambient_brain" => next.ambient_brain = v.as_bool().ok_or_else(|| anyhow::anyhow!("`ambient_brain` must be true or false"))?,
            "wander" => next.wander = v.as_bool().ok_or_else(|| anyhow::anyhow!("`wander` must be true or false"))?,
            "color" => next.character.color = text()?,
            "tier2_mode" => next.tier2_mode = parse_enum(k, v)?,
            "tier2_countdown_ms" => next.tier2_countdown_ms = num(500, 10_000)?,
            "max_steps" => next.max_steps = num(1, 50)? as u32,
            "command_timeout_secs" => next.command_timeout_secs = num(5, 600)?,
            "voice_backend" => next.voice.backend = parse_enum(k, v)?,
            _ => unreachable!(),
        }
        changed.push(k.clone());
    }
    Ok((next, changed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patches_only_whitelisted_settings() {
        let base = Settings::default();
        let (next, changed) = apply_patch(&base, &serde_json::json!({"model": "x/y", "wander": false, "tier2_mode": "ask"})).unwrap();
        assert_eq!((next.model.as_str(), next.wander, next.tier2_mode), ("x/y", false, Tier2Mode::Ask));
        assert_eq!(changed.len(), 3);
        for bad in [
            serde_json::json!({"base_url": "https://evil.example"}),
            serde_json::json!({"self_source_dir": "/"}),
            serde_json::json!({"max_steps": 500}),
            serde_json::json!({"tier2_mode": "never"}),
            serde_json::json!({}),
        ] {
            assert!(apply_patch(&base, &bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn local_endpoints_keep_more_screenshots() {
        let cloud = Settings::default();
        assert!(!cloud.is_local());
        assert_eq!(cloud.image_budget(), 1);
        let ollama = Settings { provider: ProviderKind::Ollama, ..Settings::default() };
        assert!(ollama.is_local());
        assert_eq!(ollama.image_budget(), 2, "8192 context fits two screenshots");
        let lm = Settings { base_url: "http://localhost:1234/v1".into(), ..Settings::default() };
        assert!(lm.is_local());
        let mut big = ollama.clone();
        big.ollama.num_ctx = 32768;
        assert_eq!(big.image_budget(), 3);
        big.ollama.num_ctx = 4096;
        assert_eq!(big.image_budget(), 1);
    }

    #[test]
    fn coord_mode_auto_follows_model_family() {
        assert_eq!(CoordMode::Auto.resolve("qwen/qwen3-vl-8b-instruct"), CoordMode::Norm1000);
        assert_eq!(CoordMode::Auto.resolve("google/gemini-2.5-flash-lite"), CoordMode::Norm1000);
        assert_eq!(CoordMode::Auto.resolve("anthropic/claude-haiku-4.5"), CoordMode::Pixels);
        assert_eq!(CoordMode::Pixels.resolve("qwen3-vl:4b"), CoordMode::Pixels);
    }

    #[test]
    fn partial_settings_json_fills_defaults() {
        let s: Settings = serde_json::from_str(r#"{"model":"qwen3-vl:4b","provider":"ollama"}"#).unwrap();
        assert_eq!(s.provider, ProviderKind::Ollama);
        assert_eq!(s.max_steps, 20);
        assert_eq!(s.fast_model(), "google/gemini-2.5-flash-lite");
        let s2 = Settings { fast_model: String::new(), ..s };
        assert_eq!(s2.fast_model(), "qwen3-vl:4b");
    }
}
