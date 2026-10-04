//! Quick decisions around the main model, from a "System One" decision model:
//! one pass, a probability back, ~0.3 s and ~$0.00002 each. Today that's
//! TypeSafe's Jev on OpenRouter (`/api/alpha/decisions`). Every answer is
//! optional: on any error or delay, callers fall back to the careful path.

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

use crate::config::{ProviderKind, Settings};

/// What the duck does between tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DuckIntent {
    /// Sit on the window the user is working in.
    Perch,
    /// Walk along the front window's edges, peeking at it.
    Explore,
    /// Stay near the cursor and look at it.
    Watch,
    Nap,
    /// Retreat to a corner and keep still (focus, full screen, presenting).
    GiveSpace,
    Wander,
}

impl DuckIntent {
    const ALL: [(DuckIntent, &'static str, &'static str); 6] = [
        (DuckIntent::Perch, "perch", "Sit on top of the window the user is working in, keeping them company"),
        (DuckIntent::Explore, "explore", "Walk along the edges of the front window and peek at it"),
        (DuckIntent::Watch, "watch", "Stay nearby and look toward where the user is pointing"),
        (DuckIntent::Nap, "nap", "The user has been away for a while: curl up and sleep"),
        (DuckIntent::GiveSpace, "give_space", "The user is focused, presenting, gaming or watching full screen: retreat to a corner and keep still"),
        (DuckIntent::Wander, "wander", "Roam around the desktop"),
    ];

    fn from_label(label: &str) -> Option<Self> {
        Self::ALL.iter().find(|(_, l, _)| *l == label).map(|(i, _, _)| *i)
    }
}

/// What the user is up to, for the duck's next move. App names only, never window titles.
#[derive(Debug, Clone, PartialEq)]
pub struct Situation {
    pub app: String,
    pub fullscreen: bool,
    pub idle_secs: u64,
    /// Seconds since the front app changed.
    pub app_secs: u64,
    pub previous_app: Option<String>,
    pub local_time: String,
}

impl Situation {
    pub fn describe(&self) -> String {
        let idle = match self.idle_secs {
            0..=5 => "just now".to_string(),
            s if s < 120 => format!("{s} seconds ago"),
            s => format!("{} minutes ago", s / 60),
        };
        let switched = match &self.previous_app {
            Some(prev) if self.app_secs < 15 => format!(" The user switched to it from {prev} {} seconds ago.", self.app_secs),
            _ => String::new(),
        };
        format!(
            "Front app: {}, {}full screen.{switched} The user last used the mouse or keyboard {idle}. Time: {}.",
            if self.app.is_empty() { "the desktop" } else { &self.app },
            if self.fullscreen { "" } else { "not " },
            self.local_time
        )
    }
}

#[async_trait]
pub trait Decider: Send + Sync {
    /// Probability that a task needs the screen.
    async fn needs_screen(&self, _goal: &str) -> Option<f64> {
        None
    }
    /// Probability that a message is only conversation (no computer use needed).
    async fn chat_only(&self, _message: &str) -> Option<f64> {
        None
    }
    /// The duck's next idle behaviour, with its probability.
    async fn duck_intent(&self, _situation: &Situation) -> Option<(DuckIntent, f64)> {
        None
    }
}

pub struct Jev {
    http: reqwest::Client,
    url: String,
    key: String,
}

impl Jev {
    /// Jev is reachable with an OpenRouter key; other endpoints get no decider.
    pub fn for_settings(settings: &Settings, key: Option<&str>) -> Option<Self> {
        let origin = settings.base_url.trim_end_matches('/').strip_suffix("/api/v1")?;
        if settings.provider != ProviderKind::OpenaiCompat || !origin.contains("openrouter.ai") {
            return None;
        }
        let key = key.filter(|k| !k.trim().is_empty())?.to_string();
        let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(2)).build().ok()?;
        Some(Self { http, url: format!("{origin}/api/alpha/decisions"), key })
    }

    async fn ask(&self, state: String, question: Value) -> Option<Value> {
        let body = json!({ "model": "typesafe/jev-1.13", "state": state, "questions": { "q": question } });
        let send = self.http.post(&self.url).bearer_auth(&self.key).json(&body).send();
        let res = tokio::time::timeout(Duration::from_secs(2), send).await.ok()?.ok()?;
        if !res.status().is_success() {
            log::debug!("decision failed: {}", res.status());
            return None;
        }
        let v: Value = tokio::time::timeout(Duration::from_secs(1), res.json()).await.ok()?.ok()?;
        Some(v["answers"]["q"].clone())
    }

    async fn noul(&self, state: String, instructions: &str, yes: &str, no: &str) -> Option<f64> {
        let q = json!({ "type": "noul", "instructions": instructions, "criteria": { "true": yes, "false": no } });
        self.ask(state, q).await?["noul"].as_f64()
    }

    async fn choice(&self, state: String, instructions: &str, options: &[(&str, &str)]) -> Option<(String, HashMap<String, f64>)> {
        let criteria: serde_json::Map<String, Value> = options.iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
        let a = self.ask(state, json!({ "type": "choice", "instructions": instructions, "criteria": criteria })).await?;
        let probs = a["probabilities"].as_object()?.iter().filter_map(|(k, v)| Some((k.clone(), v.as_f64()?))).collect();
        Some((a["choice"].as_str()?.to_string(), probs))
    }
}

#[async_trait]
impl Decider for Jev {
    async fn needs_screen(&self, goal: &str) -> Option<f64> {
        let p = self
            .noul(
                format!("User request to a desktop assistant: {goal}"),
                "Doing this request needs looking at or acting on what is on the user's screen",
                "It refers to something visible (this, it, an open app, page, email, button, selection) or needs clicking, typing, scrolling or reading the screen",
                "It can be done without the screen: files, commands, reminders, the clipboard, maths, writing, or general knowledge",
            )
            .await;
        log::info!("screen check: {p:?}");
        p
    }

    async fn chat_only(&self, message: &str) -> Option<f64> {
        let (_, probs) = self
            .choice(
                format!("Message: {message}"),
                "Does this message to a desktop assistant duck need it to do something on the computer, or is it just conversation it can answer from general knowledge",
                &[
                    ("task", "Needs the computer: the screen, apps, files, clipboard, reminders, the time, settings, or showing where something is"),
                    ("chat", "Small talk, thanks, praise, questions about the duck itself, jokes, writing or maths it can answer directly"),
                ],
            )
            .await?;
        let p = probs.get("chat").copied();
        log::info!("chat check: {p:?}");
        p
    }

    async fn duck_intent(&self, situation: &Situation) -> Option<(DuckIntent, f64)> {
        let options: Vec<(&str, &str)> = DuckIntent::ALL.iter().map(|(_, l, d)| (*l, *d)).collect();
        let (label, probs) = self
            .choice(situation.describe(), "What should a desktop pet duck do next so it is charming but never in the way", &options)
            .await?;
        let p = probs.get(&label).copied().unwrap_or(0.0);
        DuckIntent::from_label(&label).map(|i| (i, p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jev_only_for_openrouter_with_a_key() {
        let s = Settings { base_url: "https://openrouter.ai/api/v1".into(), ..Settings::default() };
        assert!(Jev::for_settings(&s, Some("k")).is_some());
        assert!(Jev::for_settings(&s, None).is_none());
        let local = Settings { base_url: "http://localhost:1234/v1".into(), ..Settings::default() };
        assert!(Jev::for_settings(&local, Some("k")).is_none());
        assert_eq!(Jev::for_settings(&s, Some("k")).unwrap().url, "https://openrouter.ai/api/alpha/decisions");
    }

    #[test]
    fn situations_name_apps_not_titles() {
        let s = Situation { app: "chrome".into(), fullscreen: true, idle_secs: 300, app_secs: 5, previous_app: Some("outlook".into()), local_time: "Sunday 14:05".into() };
        assert_eq!(
            s.describe(),
            "Front app: chrome, full screen. The user switched to it from outlook 5 seconds ago. The user last used the mouse or keyboard 5 minutes ago. Time: Sunday 14:05."
        );
        assert_eq!(DuckIntent::from_label("give_space"), Some(DuckIntent::GiveSpace));
    }
}
