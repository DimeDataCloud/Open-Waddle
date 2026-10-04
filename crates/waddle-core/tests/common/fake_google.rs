//! A small fake of Google's OAuth, Gmail, Calendar and People APIs, served over
//! real HTTP on 127.0.0.1 so the real client code runs end to end.

use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use waddle_core::google::{Endpoints, Google, OAuthClient};

#[derive(Debug, Clone)]
pub struct Req {
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub body: String,
}

impl Req {
    pub fn q(&self, k: &str) -> Option<&str> {
        self.query.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }
}

#[derive(Default)]
pub struct State {
    pub requests: Vec<Req>,
    pub messages: Vec<Value>,
    pub labels: Vec<Value>,
    pub drafts: Vec<(String, Value)>,
    /// Raw messages sent (RFC 2822 text, decoded).
    pub sent: Vec<String>,
    pub trashed: Vec<String>,
    pub events: Vec<Value>,
    pub deleted_events: Vec<String>,
    /// (name, email, saved?) — saved contacts vs "other contacts".
    pub contacts: Vec<(String, String, bool)>,
    pub history_pages: Vec<Value>,
    pub access_token: String,
    pub refreshes: u32,
    /// PKCE challenge of the sign-in in progress.
    pub challenge: Option<String>,
    /// The next API call gets a 401, as if the access token had expired.
    pub expire_next: bool,
}

pub struct FakeGoogle {
    pub base: String,
    pub state: Arc<Mutex<State>>,
}

fn b64url(s: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s)
}

/// RFC 2822 text from Gmail's base64url `raw`, with a base64 body decoded.
pub fn decode_raw(raw: &str) -> String {
    let bytes = base64::engine::general_purpose::URL_SAFE.decode(raw).or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(raw)).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let body = if head.contains("Content-Transfer-Encoding: base64") {
        String::from_utf8(base64::engine::general_purpose::STANDARD.decode(body.replace("\r\n", "")).unwrap()).unwrap()
    } else {
        body.to_string()
    };
    format!("{head}\r\n\r\n{body}")
}

/// A Gmail message (format=full) from raw text, as drafts.get returns it.
fn message_from_raw(id: &str, raw: &str, thread: Option<&str>) -> Value {
    let text = decode_raw(raw);
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    let headers: Vec<Value> = head.lines().filter_map(|l| l.split_once(": ")).map(|(n, v)| json!({ "name": n, "value": v })).collect();
    json!({
        "id": id, "threadId": thread.unwrap_or(id), "labelIds": ["DRAFT"],
        "payload": { "mimeType": "text/plain", "headers": headers, "body": { "data": b64url(body) } }
    })
}

impl FakeGoogle {
    pub async fn start() -> FakeGoogle {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(State {
            access_token: "at-0".into(),
            labels: vec![json!({ "id": "INBOX", "name": "INBOX" }), json!({ "id": "Label_7", "name": "Receipts" })],
            ..Default::default()
        }));
        let st = state.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { break };
                let st = st.clone();
                tokio::spawn(async move {
                    let mut buf = vec![];
                    let mut chunk = [0u8; 8192];
                    let head_end = loop {
                        let n = sock.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break i + 4;
                        }
                    };
                    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                    let len: usize = head
                        .lines()
                        .find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("content-length")).map(|(_, v)| v.trim().parse().unwrap_or(0)))
                        .unwrap_or(0);
                    while buf.len() < head_end + len {
                        let n = sock.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                    }
                    let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
                    let mut first = head.lines().next().unwrap_or("").split_whitespace();
                    let method = first.next().unwrap_or("").to_string();
                    let target = first.next().unwrap_or("/").to_string();
                    let url = reqwest::Url::parse(&format!("http://fake{target}")).unwrap();
                    let auth = head
                        .lines()
                        .find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("authorization")).map(|(_, v)| v.trim().to_string()))
                        .unwrap_or_default();
                    let req = Req {
                        method,
                        path: url.path().to_string(),
                        query: url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect(),
                        body,
                    };
                    let (status, out) = route(&st, req, &auth);
                    let text = if out.is_null() { String::new() } else { out.to_string() };
                    let reply = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                        text.len()
                    );
                    let _ = sock.write_all(reply.as_bytes()).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        FakeGoogle { base, state }
    }

    pub fn endpoints(&self) -> Endpoints {
        Endpoints::at(&self.base)
    }

    pub fn client() -> OAuthClient {
        OAuthClient { id: "test-client".into(), secret: Some("test-secret".into()) }
    }

    /// A signed-in account (refresh token rt-1).
    pub fn google(&self) -> Arc<Google> {
        Arc::new(Google::new(self.endpoints(), Self::client(), "rt-1".into()))
    }

    pub fn add_message(&self, id: &str, from: &str, subject: &str, body: &str, labels: &[&str], internal_ms: i64) {
        self.state.lock().unwrap().messages.push(json!({
            "id": id, "threadId": format!("t-{id}"), "labelIds": labels, "snippet": body.chars().take(80).collect::<String>(),
            "internalDate": internal_ms.to_string(),
            "payload": {
                "mimeType": "multipart/alternative",
                "headers": [
                    { "name": "From", "value": from }, { "name": "To", "value": "me@example.com" },
                    { "name": "Subject", "value": subject }, { "name": "Message-ID", "value": format!("<{id}@mail.example.com>") }
                ],
                "parts": [{ "mimeType": "text/plain", "body": { "data": b64url(body) } }]
            }
        }));
    }

    pub fn add_event(&self, event: Value) {
        self.state.lock().unwrap().events.push(event);
    }

    pub fn add_contact(&self, name: &str, email: &str, saved: bool) {
        self.state.lock().unwrap().contacts.push((name.into(), email.into(), saved));
    }

    pub fn requests(&self, path_part: &str) -> Vec<Req> {
        self.state.lock().unwrap().requests.iter().filter(|r| r.path.contains(path_part)).cloned().collect()
    }

    pub fn sent(&self) -> Vec<String> {
        self.state.lock().unwrap().sent.clone()
    }
}

fn form(body: &str) -> Vec<(String, String)> {
    reqwest::Url::parse(&format!("http://x/?{body}")).unwrap().query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect()
}

fn in_range(event: &Value, min: Option<&str>, max: Option<&str>) -> bool {
    let start = event["start"]["dateTime"].as_str().and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
    let end = event["end"]["dateTime"].as_str().and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
    let parse = |s: Option<&str>| s.and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok());
    match (start, end) {
        (Some(s), Some(e)) => parse(max).is_none_or(|m| s < m) && parse(min).is_none_or(|m| e > m),
        _ => true,
    }
}

fn route(st: &Mutex<State>, req: Req, auth: &str) -> (&'static str, Value) {
    let mut s = st.lock().unwrap();
    s.requests.push(req.clone());
    let p = req.path.as_str();
    if p == "/token" {
        let f = form(&req.body);
        let get = |k: &str| f.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default();
        if get("client_id") != "test-client" {
            return ("401 Unauthorized", json!({ "error": "invalid_client" }));
        }
        return match get("grant_type").as_str() {
            "authorization_code" => {
                let verifier = get("code_verifier");
                let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
                if get("code") != "good-code" || s.challenge.as_deref() != Some(challenge.as_str()) {
                    return ("400 Bad Request", json!({ "error": "invalid_grant", "error_description": "Bad code or verifier" }));
                }
                s.access_token = "at-1".into();
                ("200 OK", json!({ "access_token": "at-1", "expires_in": 3599, "refresh_token": "rt-1", "scope": "gmail.modify" }))
            }
            "refresh_token" if get("refresh_token") == "rt-1" => {
                s.refreshes += 1;
                s.access_token = format!("at-r{}", s.refreshes);
                ("200 OK", json!({ "access_token": s.access_token, "expires_in": 3599 }))
            }
            _ => ("400 Bad Request", json!({ "error": "invalid_grant", "error_description": "Token has been expired or revoked." })),
        };
    }
    if p == "/revoke" {
        return ("200 OK", json!({}));
    }
    if s.expire_next {
        s.expire_next = false;
        return ("401 Unauthorized", json!({ "error": { "code": 401, "message": "Invalid Credentials" } }));
    }
    if auth != format!("Bearer {}", s.access_token) {
        return ("401 Unauthorized", json!({ "error": { "code": 401, "message": "Invalid Credentials" } }));
    }
    let body: Value = serde_json::from_str(&req.body).unwrap_or(Value::Null);
    let parts: Vec<&str> = p.trim_start_matches('/').split('/').collect();
    match (req.method.as_str(), parts.as_slice()) {
        ("GET", ["gmail", "v1", "users", "me", "messages"]) => {
            let q = req.q("q").unwrap_or("").to_lowercase();
            let max: usize = req.q("maxResults").and_then(|m| m.parse().ok()).unwrap_or(10);
            let words: Vec<&str> = q.split_whitespace().filter(|w| !w.contains(':')).collect();
            let hits: Vec<Value> = s
                .messages
                .iter()
                .filter(|m| {
                    let labels = m["labelIds"].to_string();
                    (!q.contains("in:sent") || labels.contains("SENT"))
                        && (!q.contains("is:unread") || labels.contains("UNREAD"))
                        && (q.contains("in:sent") || !labels.contains("SENT"))
                        && words.iter().all(|w| m.to_string().to_lowercase().contains(w))
                })
                .take(max)
                .map(|m| json!({ "id": m["id"], "threadId": m["threadId"] }))
                .collect();
            ("200 OK", json!({ "messages": hits }))
        }
        ("GET", ["gmail", "v1", "users", "me", "messages", id]) => match s.messages.iter().find(|m| m["id"] == *id) {
            Some(m) => ("200 OK", m.clone()),
            None => ("404 Not Found", json!({ "error": { "code": 404, "message": "Requested entity was not found." } })),
        },
        ("POST", ["gmail", "v1", "users", "me", "messages", "send"]) => {
            s.sent.push(decode_raw(body["raw"].as_str().unwrap_or("")));
            ("200 OK", json!({ "id": format!("sent-{}", s.sent.len()), "threadId": body["threadId"] }))
        }
        ("POST", ["gmail", "v1", "users", "me", "messages", id, "modify"]) => {
            let id = id.to_string();
            if let Some(m) = s.messages.iter_mut().find(|m| m["id"] == id.as_str()) {
                let mut labels: Vec<String> = m["labelIds"].as_array().unwrap().iter().map(|l| l.as_str().unwrap().to_string()).collect();
                labels.retain(|l| !body["removeLabelIds"].as_array().unwrap().iter().any(|r| r == l.as_str()));
                for a in body["addLabelIds"].as_array().unwrap() {
                    labels.push(a.as_str().unwrap().into());
                }
                m["labelIds"] = json!(labels);
            }
            ("200 OK", json!({ "id": id }))
        }
        ("POST", ["gmail", "v1", "users", "me", "messages", id, "trash"]) => {
            s.trashed.push(id.to_string());
            ("200 OK", json!({ "id": id }))
        }
        ("POST", ["gmail", "v1", "users", "me", "drafts"]) => {
            let id = format!("d-{}", s.drafts.len() + 1);
            let thread = body["message"]["threadId"].as_str().map(str::to_string);
            let m = message_from_raw(&id, body["message"]["raw"].as_str().unwrap_or(""), thread.as_deref());
            s.drafts.push((id.clone(), json!({ "raw": body["message"]["raw"], "message": m })));
            ("200 OK", json!({ "id": id, "message": { "id": id } }))
        }
        ("GET", ["gmail", "v1", "users", "me", "drafts", id]) => match s.drafts.iter().find(|(d, _)| d == id) {
            Some((d, v)) => ("200 OK", json!({ "id": d, "message": v["message"] })),
            None => ("404 Not Found", json!({ "error": { "code": 404, "message": "Not found" } })),
        },
        ("PUT", ["gmail", "v1", "users", "me", "drafts", id]) => {
            let id = id.to_string();
            let thread = body["message"]["threadId"].as_str().map(str::to_string);
            let m = message_from_raw(&id, body["message"]["raw"].as_str().unwrap_or(""), thread.as_deref());
            if let Some(d) = s.drafts.iter_mut().find(|(d, _)| *d == id) {
                d.1 = json!({ "raw": body["message"]["raw"], "message": m });
            }
            ("200 OK", json!({ "id": id }))
        }
        ("POST", ["gmail", "v1", "users", "me", "drafts", "send"]) => {
            let id = body["id"].as_str().unwrap_or("").to_string();
            let Some(i) = s.drafts.iter().position(|(d, _)| *d == id) else {
                return ("404 Not Found", json!({ "error": { "code": 404, "message": "Not found" } }));
            };
            let (_, d) = s.drafts.remove(i);
            s.sent.push(decode_raw(d["raw"].as_str().unwrap_or("")));
            ("200 OK", json!({ "id": format!("sent-{}", s.sent.len()) }))
        }
        ("GET", ["gmail", "v1", "users", "me", "labels"]) => ("200 OK", json!({ "labels": s.labels })),
        ("GET", ["gmail", "v1", "users", "me", "profile"]) => ("200 OK", json!({ "emailAddress": "me@example.com", "historyId": "100" })),
        ("GET", ["gmail", "v1", "users", "me", "history"]) => {
            let page: usize = req.q("pageToken").and_then(|t| t.trim_start_matches('p').parse().ok()).unwrap_or(0);
            ("200 OK", s.history_pages.get(page).cloned().unwrap_or(json!({ "historyId": req.q("startHistoryId") })))
        }
        ("GET", ["calendar", "v3", "calendars", "primary", "events"]) => {
            let q = req.q("q").unwrap_or("").to_lowercase();
            let mut hits: Vec<Value> = s
                .events
                .iter()
                .filter(|e| in_range(e, req.q("timeMin"), req.q("timeMax")) && e["summary"].as_str().unwrap_or("").to_lowercase().contains(&q))
                .cloned()
                .collect();
            hits.sort_by_key(|e| e["start"]["dateTime"].as_str().unwrap_or("").to_string());
            ("200 OK", json!({ "items": hits }))
        }
        ("GET", ["calendar", "v3", "calendars", "primary", "events", id]) => match s.events.iter().find(|e| e["id"] == *id) {
            Some(e) => ("200 OK", e.clone()),
            None => ("404 Not Found", json!({ "error": { "code": 404, "message": "Not Found" } })),
        },
        ("POST", ["calendar", "v3", "calendars", "primary", "events"]) => {
            let mut e = body.clone();
            e["id"] = json!(format!("ev-{}", s.events.len() + 1));
            e["status"] = json!("confirmed");
            if e.get("conferenceData").is_some() {
                e["hangoutLink"] = json!("https://meet.google.com/new-meet-link");
            }
            s.events.push(e.clone());
            ("200 OK", e)
        }
        ("PATCH", ["calendar", "v3", "calendars", "primary", "events", id]) => {
            let id = id.to_string();
            let Some(e) = s.events.iter_mut().find(|e| e["id"] == id.as_str()) else {
                return ("404 Not Found", json!({ "error": { "code": 404, "message": "Not Found" } }));
            };
            for (k, v) in body.as_object().unwrap() {
                e[k] = v.clone();
            }
            ("200 OK", e.clone())
        }
        ("DELETE", ["calendar", "v3", "calendars", "primary", "events", id]) => {
            let id = id.to_string();
            s.events.retain(|e| e["id"] != id.as_str());
            s.deleted_events.push(id);
            ("204 No Content", Value::Null)
        }
        ("GET", ["v1", "people:searchContacts"]) | ("GET", ["v1", "otherContacts:search"]) => {
            let saved = p.contains("people:searchContacts");
            let q = req.q("query").unwrap_or("").to_lowercase();
            if q.is_empty() {
                return ("200 OK", json!({}));
            }
            let results: Vec<Value> = s
                .contacts
                .iter()
                .filter(|(n, e, sv)| *sv == saved && (n.to_lowercase().contains(&q) || e.contains(&q)))
                .map(|(n, e, _)| json!({ "person": { "names": [{ "displayName": n }], "emailAddresses": [{ "value": e }] } }))
                .collect();
            ("200 OK", json!({ "results": results }))
        }
        _ => ("404 Not Found", json!({ "error": { "code": 404, "message": format!("no fake for {} {p}", req.method) } })),
    }
}
