//! Google Workspace: sign-in (OAuth with PKCE and a loopback redirect), Gmail,
//! Calendar and Contacts. Every base URL is injectable, so the tests run the
//! real client against a local fake server.

pub mod auth;
pub mod calendar;
pub mod gmail;
pub mod people;
pub mod style;
pub mod tools;

use anyhow::bail;
use reqwest::Method;
use serde_json::Value;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

/// Scopes asked for at sign-in. Drive (read-only) joins them in milestone 5.
pub const SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/gmail.modify",
    "https://www.googleapis.com/auth/calendar.events",
    "https://www.googleapis.com/auth/contacts.readonly",
    "https://www.googleapis.com/auth/contacts.other.readonly",
];

#[derive(Debug, Clone, PartialEq)]
pub struct Endpoints {
    pub auth: String,
    pub token: String,
    pub revoke: String,
    /// Gmail, up to and including `users/me`.
    pub gmail: String,
    pub calendar: String,
    pub people: String,
}

impl Endpoints {
    pub fn google() -> Self {
        Self {
            auth: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token: "https://oauth2.googleapis.com/token".into(),
            revoke: "https://oauth2.googleapis.com/revoke".into(),
            gmail: "https://gmail.googleapis.com/gmail/v1/users/me".into(),
            calendar: "https://www.googleapis.com/calendar/v3".into(),
            people: "https://people.googleapis.com/v1".into(),
        }
    }

    /// Everything under one base URL: the fake server in tests.
    pub fn at(base: &str) -> Self {
        let base = base.trim_end_matches('/');
        Self {
            auth: format!("{base}/o/oauth2/v2/auth"),
            token: format!("{base}/token"),
            revoke: format!("{base}/revoke"),
            gmail: format!("{base}/gmail/v1/users/me"),
            calendar: format!("{base}/calendar/v3"),
            people: format!("{base}/v1"),
        }
    }
}

/// The OAuth client the user created in their Google Cloud project (Desktop app type).
/// Google calls the Desktop client's secret a secret, but it can't be kept secret in an
/// installed app; PKCE is what protects the sign-in.
#[derive(Debug, Clone, PartialEq)]
pub struct OAuthClient {
    pub id: String,
    pub secret: Option<String>,
}

/// A signed-in Google account.
pub struct Google {
    http: reqwest::Client,
    pub endpoints: Endpoints,
    client: OAuthClient,
    refresh_token: String,
    access: tokio::sync::Mutex<Option<(String, Instant)>>,
    /// People API search needs one empty "warm-up" query per session before it finds anyone.
    contacts_warm: AtomicBool,
}

pub(crate) fn http_client() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(20)).build().unwrap_or_default()
}

impl Google {
    pub fn new(endpoints: Endpoints, client: OAuthClient, refresh_token: String) -> Self {
        Self {
            http: http_client(),
            endpoints,
            client,
            refresh_token,
            access: tokio::sync::Mutex::new(None),
            contacts_warm: AtomicBool::new(false),
        }
    }

    async fn token(&self) -> anyhow::Result<String> {
        let mut access = self.access.lock().await;
        if let Some((token, expires)) = access.as_ref() {
            if Instant::now() + Duration::from_secs(60) < *expires {
                return Ok(token.clone());
            }
        }
        let fresh = auth::refresh(&self.http, &self.endpoints, &self.client, &self.refresh_token).await?;
        *access = Some((fresh.access_token.clone(), Instant::now() + Duration::from_secs(fresh.expires_in)));
        Ok(fresh.access_token)
    }

    /// One API call with a fresh access token; a 401 refreshes the token and retries once.
    pub(crate) async fn call(&self, method: Method, url: &str, query: &[(&str, String)], body: Option<&Value>) -> anyhow::Result<Value> {
        for attempt in 0..2 {
            let token = self.token().await?;
            let mut req = self.http.request(method.clone(), url).bearer_auth(&token).query(query);
            if let Some(b) = body {
                req = req.json(b);
            }
            let resp = req.send().await?;
            let status = resp.status();
            if status == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
                self.access.lock().await.take();
                continue;
            }
            let text = resp.text().await?;
            if !status.is_success() {
                bail!("{}", api_error(status.as_u16(), &text));
            }
            if text.trim().is_empty() {
                return Ok(Value::Null);
            }
            return Ok(serde_json::from_str(&text)?);
        }
        bail!("Google rejected the sign-in; reconnect Google in Settings")
    }

    pub(crate) async fn get(&self, url: &str, query: &[(&str, String)]) -> anyhow::Result<Value> {
        self.call(Method::GET, url, query, None).await
    }

    pub(crate) async fn post(&self, url: &str, query: &[(&str, String)], body: &Value) -> anyhow::Result<Value> {
        self.call(Method::POST, url, query, Some(body)).await
    }
}

/// Google's error JSON, cut down to something the model (and the user) can act on.
fn api_error(status: u16, body: &str) -> String {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let message = parsed
        .as_ref()
        .and_then(|v| v["error"]["message"].as_str().or_else(|| v["error_description"].as_str()).or_else(|| v["error"].as_str()))
        .unwrap_or(body)
        .chars()
        .take(300)
        .collect::<String>();
    match status {
        403 if message.to_lowercase().contains("scope") => {
            format!("Google said 403 ({message}). Reconnect Google in Settings to grant access.")
        }
        404 => format!("Google said 404: not found ({message})"),
        _ => format!("Google said {status}: {message}"),
    }
}

/// Plain text from an HTML email body: drops scripts, styles and tags, keeps line breaks.
pub fn html_to_text(html: &str) -> String {
    use regex::Regex;
    use std::sync::OnceLock;
    static BLOCKS: OnceLock<Regex> = OnceLock::new();
    static BREAKS: OnceLock<Regex> = OnceLock::new();
    static TAGS: OnceLock<Regex> = OnceLock::new();
    static BLANKS: OnceLock<Regex> = OnceLock::new();
    let blocks = BLOCKS.get_or_init(|| Regex::new(r"(?is)<(script|style|head)\b.*?</(script|style|head)\s*>").unwrap());
    let breaks = BREAKS.get_or_init(|| Regex::new(r"(?i)<br\s*/?>|</(p|div|li|tr|h[1-6])\s*>").unwrap());
    let tags = TAGS.get_or_init(|| Regex::new(r"(?s)<[^>]*>").unwrap());
    let blanks = BLANKS.get_or_init(|| Regex::new(r"\n[ \t]*(\n[ \t]*)+").unwrap());
    let s = blocks.replace_all(html, "");
    let s = breaks.replace_all(&s, "\n");
    let s = tags.replace_all(&s, "");
    let s = s
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'");
    blanks.replace_all(&s, "\n\n").trim().to_string()
}

/// Cuts text to `max` characters, marking the cut.
pub(crate) fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    format!("{}…", s.chars().take(max).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_bodies_become_readable_text() {
        let html = "<html><head><style>p{color:red}</style></head><body><p>Hi Sam,</p><p>See you at 3&nbsp;pm &amp; bring <b>snacks</b>.<br>Thanks</p><script>alert(1)</script></body></html>";
        assert_eq!(html_to_text(html), "Hi Sam,\nSee you at 3 pm & bring snacks.\nThanks");
    }

    #[test]
    fn api_errors_are_short_and_actionable() {
        let body = r#"{"error":{"code":403,"message":"Request had insufficient authentication scopes.","status":"PERMISSION_DENIED"}}"#;
        assert!(api_error(403, body).contains("Reconnect Google"));
        assert_eq!(api_error(500, "oops"), "Google said 500: oops");
    }
}
