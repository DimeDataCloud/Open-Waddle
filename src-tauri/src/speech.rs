//! Reading replies aloud. Windows uses its built-in speech synthesizer (WinRT);
//! macOS uses `say`; Linux uses speech-dispatcher or eSpeak when installed.
//! One thread does the speaking: lines queue up, and Stop cuts them all.
//! What to say comes from `waddle_core::speech`.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use waddle_core::config::VoiceOutSettings;
use waddle_core::speech::Picker;
use waddle_core::AgentEvent;

type Then = Box<dyn FnOnce() + Send>;

enum Cmd {
    Say { text: String, voice: String, rate: f64, then: Option<Then> },
    Stop,
}

pub struct Speech {
    tx: Mutex<Sender<Cmd>>,
    settings: RwLock<VoiceOutSettings>,
    picker: Mutex<Picker>,
    /// A meeting is on until then (ms since 1970), from the calendar check.
    meeting_until: AtomicI64,
    /// The user's last message was spoken, so talk mode may listen again after the answer.
    by_voice: AtomicBool,
    speaking: Arc<AtomicBool>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl Speech {
    pub fn new() -> Arc<Self> {
        let (tx, rx) = mpsc::channel();
        let speaking = Arc::new(AtomicBool::new(false));
        let flag = speaking.clone();
        std::thread::Builder::new().name("waddle-speech".into()).spawn(move || run(rx, flag)).expect("speech thread");
        Arc::new(Self {
            tx: Mutex::new(tx),
            settings: RwLock::default(),
            picker: Mutex::default(),
            meeting_until: AtomicI64::new(0),
            by_voice: AtomicBool::new(false),
            speaking,
        })
    }

    pub fn set_settings(&self, s: VoiceOutSettings) {
        if !s.enabled {
            self.stop();
        }
        *self.settings.write().unwrap() = s;
    }

    pub fn set_meeting_until(&self, ms: i64) {
        self.meeting_until.store(ms, Ordering::SeqCst);
    }

    /// Whether the message being sent was spoken (talk mode answers those).
    pub fn set_by_voice(&self, voice: bool) {
        self.by_voice.store(voice, Ordering::SeqCst);
    }

    pub fn is_speaking(&self) -> bool {
        self.speaking.load(Ordering::SeqCst)
    }

    /// Stops talking now and forgets anything queued.
    pub fn stop(&self) {
        let _ = self.tx.lock().unwrap().send(Cmd::Stop);
    }

    fn send(&self, text: String, then: Option<Then>) {
        let s = self.settings.read().unwrap().clone();
        let _ = self.tx.lock().unwrap().send(Cmd::Say { text, voice: s.voice, rate: s.rate, then });
    }

    /// Reads a sample in the given voice, whatever the settings (the Test button).
    pub fn test(&self, voice: String, rate: f64) {
        self.stop();
        let text = "Hi! I'm Waddle. This is how I'll sound when I read my answers.".to_string();
        let _ = self.tx.lock().unwrap().send(Cmd::Say { text, voice, rate, then: None });
    }

    fn quiet(&self, fullscreen: bool) -> Option<&'static str> {
        waddle_core::speech::quiet_reason(fullscreen, now_ms() < self.meeting_until.load(Ordering::SeqCst))
    }

    /// Follows the agent's events and reads out answers. `listen_again` runs
    /// after an answer to a spoken message, when talk mode is on.
    pub fn observe(&self, event: &AgentEvent, fullscreen: impl FnOnce() -> bool, listen_again: impl FnOnce() + Send + 'static) {
        let settings = self.settings.read().unwrap().clone();
        let Some(text) = self.picker.lock().unwrap().observe(event, &settings) else { return };
        if let Some(why) = self.quiet(fullscreen()) {
            log::info!("not reading the reply aloud: {why}");
            return;
        }
        let then: Option<Then> = (settings.talk_mode && self.by_voice.load(Ordering::SeqCst)).then(|| Box::new(listen_again) as Then);
        self.send(text, then);
    }

    /// Reads out a nudge or reminder, if those are on.
    pub fn announce(&self, text: &str, fullscreen: bool) {
        let s = self.settings.read().unwrap().clone();
        if !(s.enabled && s.nudges) || self.quiet(fullscreen).is_some() {
            return;
        }
        let text = waddle_core::speech::speakable(text);
        if !text.is_empty() {
            self.send(text, None);
        }
    }
}

/// The voices this computer has, by name. Empty where only the default can be used.
pub fn voices() -> Vec<String> {
    #[cfg(windows)]
    return win::voices();
    #[cfg(target_os = "macos")]
    return std::process::Command::new("say")
        .args(["-v", "?"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().filter_map(|l| l.split("  ").next()).map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect())
        .unwrap_or_default();
    #[cfg(all(unix, not(target_os = "macos")))]
    return Vec::new();
}

/// Whether this computer can speak at all.
pub fn available() -> bool {
    #[cfg(windows)]
    return !voices().is_empty();
    #[cfg(not(windows))]
    return program().is_some();
}

trait Voice {
    fn start(&mut self, text: &str, voice: &str, rate: f64) -> anyhow::Result<()>;
    /// Still talking.
    fn busy(&mut self) -> bool;
    fn stop(&mut self);
}

fn run(rx: Receiver<Cmd>, speaking: Arc<AtomicBool>) {
    #[cfg(windows)]
    let mut voice = win::WinVoice::new();
    #[cfg(not(windows))]
    let mut voice = Process::default();
    let mut queue: VecDeque<(String, String, f64, Option<Then>)> = VecDeque::new();
    loop {
        if queue.is_empty() {
            match rx.recv() {
                Ok(Cmd::Say { text, voice, rate, then }) => queue.push_back((text, voice, rate, then)),
                Ok(Cmd::Stop) => continue,
                Err(_) => return,
            }
        }
        let Some((text, name, rate, then)) = queue.pop_front() else { continue };
        if let Err(e) = voice.start(&text, &name, rate) {
            log::warn!("couldn't speak: {e:#}");
            continue;
        }
        speaking.store(true, Ordering::SeqCst);
        let mut finished = false;
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Cmd::Say { text, voice, rate, then }) => queue.push_back((text, voice, rate, then)),
                Ok(Cmd::Stop) => {
                    voice.stop();
                    queue.clear();
                    break;
                }
                Err(RecvTimeoutError::Timeout) => {
                    if !voice.busy() {
                        finished = true;
                        break;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    voice.stop();
                    return;
                }
            }
        }
        speaking.store(false, Ordering::SeqCst);
        if finished && queue.is_empty() {
            if let Some(then) = then {
                then();
            }
        }
    }
}

/// The speech program on macOS and Linux.
#[cfg(not(windows))]
fn program() -> Option<&'static str> {
    let candidates: &[&str] = if cfg!(target_os = "macos") { &["say"] } else { &["spd-say", "espeak-ng", "espeak"] };
    let path = std::env::var_os("PATH")?;
    candidates.iter().copied().find(|name| std::env::split_paths(&path).any(|dir| dir.join(name).is_file()))
}

#[cfg(not(windows))]
#[derive(Default)]
struct Process {
    child: Option<std::process::Child>,
    program: Option<&'static str>,
}

#[cfg(not(windows))]
impl Voice for Process {
    fn start(&mut self, text: &str, voice: &str, rate: f64) -> anyhow::Result<()> {
        let program = program().ok_or_else(|| anyhow::anyhow!("no speech program found (install speech-dispatcher or espeak-ng)"))?;
        let mut cmd = std::process::Command::new(program);
        let rate = rate.clamp(0.5, 2.0);
        match program {
            "spd-say" => {
                // -w waits until it's said, so the child's exit means done.
                cmd.args(["-w", "-r", &(((rate - 1.0) * 100.0).round() as i32).clamp(-100, 100).to_string()]);
            }
            // `say` takes words per minute as -r, eSpeak as -s.
            _ => {
                cmd.args([if program == "say" { "-r" } else { "-s" }, &((175.0 * rate).round() as i32).to_string()]);
                if !voice.is_empty() {
                    cmd.args(["-v", voice]);
                }
            }
        }
        // The text is one argument (never parsed by a shell); a leading space keeps "-5 °C" from reading as an option.
        let text = if text.starts_with('-') { format!(" {text}") } else { text.to_string() };
        cmd.arg(text).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        self.child = Some(cmd.spawn()?);
        self.program = Some(program);
        Ok(())
    }

    fn busy(&mut self) -> bool {
        self.child.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None)))
    }

    fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        if self.program == Some("spd-say") {
            // The speech server keeps talking after its client is gone.
            let _ = std::process::Command::new("spd-say").arg("-S").status();
        }
    }
}

#[cfg(windows)]
mod win {
    use super::Voice;
    use std::time::Instant;
    use windows::core::HSTRING;
    use windows::Media::Core::MediaSource;
    use windows::Media::Playback::{MediaPlaybackState, MediaPlayer};
    use windows::Media::SpeechSynthesis::SpeechSynthesizer;

    fn init() {
        // The speech thread talks to WinRT; any thread may call `voices`.
        unsafe {
            let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
        }
    }

    pub fn voices() -> Vec<String> {
        init();
        let Ok(all) = SpeechSynthesizer::AllVoices() else { return Vec::new() };
        let n = all.Size().unwrap_or(0);
        (0..n).filter_map(|i| all.GetAt(i).ok()).filter_map(|v| v.DisplayName().ok()).map(|n| n.to_string()).collect()
    }

    pub struct WinVoice {
        player: Option<MediaPlayer>,
        started: Instant,
        playing: bool,
    }

    impl WinVoice {
        pub fn new() -> Self {
            init();
            Self { player: None, started: Instant::now(), playing: false }
        }
    }

    impl Voice for WinVoice {
        fn start(&mut self, text: &str, voice: &str, rate: f64) -> anyhow::Result<()> {
            self.stop();
            let synth = SpeechSynthesizer::new()?;
            if !voice.is_empty() {
                let all = SpeechSynthesizer::AllVoices()?;
                for i in 0..all.Size()? {
                    let v = all.GetAt(i)?;
                    if v.DisplayName()?.to_string() == voice {
                        synth.SetVoice(&v)?;
                        break;
                    }
                }
            }
            synth.Options()?.SetSpeakingRate(rate.clamp(0.5, 2.0))?;
            let stream = synth.SynthesizeTextToStreamAsync(&HSTRING::from(text))?.join()?;
            let source = MediaSource::CreateFromStream(&stream, &stream.ContentType()?)?;
            let player = MediaPlayer::new()?;
            player.SetSource(&source)?;
            player.Play()?;
            self.player = Some(player);
            self.started = Instant::now();
            self.playing = false;
            Ok(())
        }

        fn busy(&mut self) -> bool {
            let Some(player) = &self.player else { return false };
            let state = player.PlaybackSession().and_then(|s| s.PlaybackState()).unwrap_or(MediaPlaybackState::None);
            if state == MediaPlaybackState::Playing {
                self.playing = true;
            }
            match state {
                MediaPlaybackState::Playing | MediaPlaybackState::Opening | MediaPlaybackState::Buffering => true,
                // Not started yet: give it a moment before calling it done.
                _ => !self.playing && self.started.elapsed().as_secs() < 3,
            }
        }

        fn stop(&mut self) {
            if let Some(p) = self.player.take() {
                let _ = p.Pause();
                let _ = p.Close();
            }
        }
    }
}
