//! Watches the user's Google calendar and inbox while they're connected, and
//! taps them on the shoulder: five minutes before a meeting, when important
//! mail arrives (scored by Jev from sender, subject and first line only), and
//! with a morning brief offer. The timing rules live in `waddle_core::nudges`.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use waddle_core::nudges::{self, Nudge, NudgeState, Stage, Topic};

use crate::bridge::TauriHost;
use crate::presence::Presence;
use crate::AppState;

const TICK: Duration = Duration::from_secs(15);
const CALENDAR_EVERY: Duration = Duration::from_secs(120);
const MAIL_EVERY: Duration = Duration::from_secs(90);
/// At most this many new emails are scored per check.
const MAX_NEW_MAIL: usize = 5;

/// The watcher's memory and the nudges on screen, shared with `nudge_action`.
pub struct Nudges {
    pub state: NudgeState,
    /// Nudges the overlay is showing, by id, so their buttons can act.
    pub shown: HashMap<String, Topic>,
}

pub type Shared = Arc<Mutex<Nudges>>;

pub fn load(path: std::path::PathBuf) -> Shared {
    Arc::new(Mutex::new(Nudges { state: NudgeState::load(path), shown: HashMap::new() }))
}

fn sender_name(from: &str) -> String {
    let name = from.split('<').next().unwrap_or("").trim().trim_matches('"');
    if name.is_empty() { nudges::address(from) } else { name.to_string() }
}

/// What the overlay shows for a nudge: text and buttons.
pub fn payload(n: &Nudge, now_ms: i64) -> Value {
    let action = |id: &str, label: &str| json!({ "id": id, "label": label });
    let (kind, text, actions) = match &n.topic {
        Topic::Meeting { title, start_ms, join, .. } => {
            let mins = ((start_ms - now_ms) as f64 / 60_000.0).ceil() as i64;
            let text = if mins <= 0 { format!("📅 \"{title}\" is starting now.") } else { format!("📅 \"{title}\" starts in {mins} min.") };
            let mut actions = vec![];
            if join.is_some() {
                actions.push(action("join", "Join"));
            }
            actions.push(action("snooze", "Snooze"));
            ("meeting", text, actions)
        }
        Topic::Mail { from, subject, .. } => (
            "mail",
            format!("✉️ {} wrote: \"{}\"", sender_name(from), subject.chars().take(80).collect::<String>()),
            vec![action("open", "Open"), action("reply", "Reply"), action("mute", "Mute sender")],
        ),
        Topic::Brief => {
            use chrono::{TimeZone, Timelike};
            let hour = chrono::Local.timestamp_millis_opt(now_ms).single().map(|t| t.hour()).unwrap_or(9);
            let hello = match hour {
                0..=11 => "☀️ Good morning!",
                12..=17 => "☀️ Good afternoon!",
                _ => "🌙 Good evening!",
            };
            ("brief", format!("{hello} Want a quick brief of today?"), vec![action("brief", "Show brief"), action("dismiss", "Not now")])
        }
    };
    json!({ "id": n.id, "kind": kind, "stage": n.stage, "text": text, "actions": actions })
}

pub fn spawn(app: AppHandle, host: Arc<TauriHost>, shared: Shared) {
    tauri::async_runtime::spawn(async move {
        let mut presence = Presence::new();
        let mut last_calendar: Option<Instant> = None;
        let mut last_mail: Option<Instant> = None;
        let mut warned_no_jev = false;
        loop {
            tokio::time::sleep(TICK).await;
            let Some(state) = app.try_state::<AppState>() else { continue };
            let settings = state.settings.read().unwrap().clone();
            let google = state.google.read().unwrap().clone();
            let decider = state.decider.read().unwrap().clone();
            let seen = presence.sample(&app, &host);
            let now = chrono::Local::now();
            let now_ms = now.timestamp_millis();

            if let Some(g) = &google {
                if settings.meeting_nudges && last_calendar.is_none_or(|t| t.elapsed() >= CALENDAR_EVERY) {
                    last_calendar = Some(Instant::now());
                    let to = now + chrono::Duration::minutes(10);
                    match g.calendar_events(&now.to_rfc3339(), &to.to_rfc3339(), "", 20).await {
                        Ok(events) => {
                            let mut n = shared.lock().unwrap();
                            for e in nudges::announceable(&events) {
                                let start = e.start.instant(&chrono::Local).timestamp_millis();
                                if n.state.add_meeting(&e.id, &e.title, start, e.meet.clone(), now_ms) {
                                    log::info!("meeting nudge queued");
                                }
                            }
                        }
                        Err(e) => log::warn!("calendar check: {e:#}"),
                    }
                }
                if settings.mail_nudges && last_mail.is_none_or(|t| t.elapsed() >= MAIL_EVERY) {
                    last_mail = Some(Instant::now());
                    match &decider {
                        Some(d) => check_mail(g, d.as_ref(), &shared, &settings.muted_senders, now_ms).await,
                        None if !warned_no_jev => {
                            warned_no_jev = true;
                            log::info!("mail nudges need the importance check (Jev on OpenRouter); skipping");
                        }
                        None => {}
                    }
                }
                if settings.morning_brief {
                    let active = seen.idle_secs < 60 && !seen.fullscreen && !host.is_busy();
                    shared.lock().unwrap().state.offer_brief(&now, active);
                }
            }

            let due = {
                let mut n = shared.lock().unwrap();
                let due = n.state.step(now_ms, seen.fullscreen);
                for d in &due {
                    n.shown.insert(d.id.clone(), d.topic.clone());
                }
                if n.shown.len() > 50 {
                    n.shown.clear();
                }
                n.state.save();
                due
            };
            for d in due {
                log::info!("nudge: {:?} {}", d.stage, match d.topic { Topic::Meeting { .. } => "meeting", Topic::Mail { .. } => "mail", Topic::Brief => "brief" });
                host.emit_overlay("nudge", payload(&d, now_ms));
                if d.stage == Stage::Full {
                    host.emit_overlay("nudge:chime", ());
                }
            }
        }
    });
}

/// New inbox mail since the last check, scored for importance; important ones are queued.
async fn check_mail(g: &waddle_core::google::Google, decider: &dyn waddle_core::decide::Decider, shared: &Shared, muted: &[String], now_ms: i64) {
    let start = shared.lock().unwrap().state.history_id.clone();
    let Some(start) = start else {
        // First run: start from now; older mail isn't news.
        match g.mail_profile().await {
            Ok((_, history)) => shared.lock().unwrap().state.history_id = Some(history),
            Err(e) => log::warn!("gmail profile: {e:#}"),
        }
        return;
    };
    let (ids, latest) = match g.mail_history(&start).await {
        Ok(r) => r,
        Err(e) => {
            log::warn!("gmail history: {e:#}");
            if e.to_string().contains("404") {
                // The history id is too old to replay; start again from now.
                shared.lock().unwrap().state.history_id = None;
            }
            return;
        }
    };
    let fresh: Vec<String> = {
        let mut n = shared.lock().unwrap();
        n.state.history_id = Some(latest);
        ids.into_iter().filter(|id| n.state.first_sight(id)).take(MAX_NEW_MAIL).collect()
    };
    if fresh.is_empty() {
        return;
    }
    let mails = match g.mail_summaries(&fresh).await {
        Ok(m) => m,
        Err(e) => {
            log::warn!("gmail summaries: {e:#}");
            return;
        }
    };
    for m in mails {
        let inbox_unread = m.unread && m.labels.iter().any(|l| l == "INBOX");
        if !inbox_unread || nudges::is_muted(&m.from, muted) {
            continue;
        }
        let Some(score) = decider.mail_importance(&m.from, &m.subject, &m.snippet).await else { continue };
        if score >= nudges::IMPORTANT_ABOVE {
            shared.lock().unwrap().state.add_mail(&m.id, &m.thread_id, &m.from, &m.subject, now_ms);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nudges_read_naturally() {
        let meeting = Nudge { id: "n1".into(), stage: Stage::Full, topic: Topic::Meeting { event_id: "e".into(), title: "Standup".into(), start_ms: 10 * 60_000, join: Some("https://meet.google.com/x".into()) } };
        let p = payload(&meeting, 6 * 60_000);
        assert_eq!(p["text"], "📅 \"Standup\" starts in 4 min.");
        assert_eq!(p["actions"][0]["id"], "join");
        let mail = Nudge { id: "n2".into(), stage: Stage::Small, topic: Topic::Mail { id: "m".into(), thread_id: "t".into(), from: "\"Ana Lee\" <ana@x.com>".into(), subject: "Contract".into() } };
        let p = payload(&mail, 0);
        assert_eq!((p["text"].as_str().unwrap(), p["stage"].as_str().unwrap()), ("✉️ Ana Lee wrote: \"Contract\"", "small"));
        assert_eq!(sender_name("bob@x.com"), "bob@x.com");
    }
}
