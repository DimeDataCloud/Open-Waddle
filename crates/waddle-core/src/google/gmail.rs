//! Gmail: search, read, draft, send, label, trash, and new-mail history.

use anyhow::{bail, Context};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::Google;

/// A message the user is about to send: what the send card shows and lets them edit.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MailDraft {
    pub to: String,
    pub cc: String,
    pub subject: String,
    pub body: String,
    /// Gmail id of the message this replies to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
}

/// Headers that keep a reply in its thread.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Threading {
    pub thread_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MailSummary {
    pub id: String,
    pub thread_id: String,
    pub from: String,
    pub subject: String,
    /// Unix milliseconds.
    pub internal_ms: i64,
    pub snippet: String,
    pub unread: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mail {
    pub id: String,
    pub thread_id: String,
    pub from: String,
    pub reply_to: String,
    pub to: String,
    pub cc: String,
    pub subject: String,
    pub internal_ms: i64,
    pub body: String,
    pub message_id: String,
    pub references: String,
    pub labels: Vec<String>,
}

/// Longest body handed to the model.
const MAX_BODY: usize = 6000;

fn header(payload: &Value, name: &str) -> String {
    payload["headers"]
        .as_array()
        .and_then(|hs| hs.iter().find(|h| h["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name))))
        .and_then(|h| h["value"].as_str())
        .unwrap_or("")
        .to_string()
}

fn decode(data: &str) -> String {
    let cleaned: String = data.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cleaned.trim_end_matches('='))
        .unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// The first part with this MIME type, searching nested multiparts.
fn find_part<'a>(part: &'a Value, mime: &str) -> Option<&'a Value> {
    if part["mimeType"].as_str() == Some(mime) && part["body"]["data"].is_string() {
        return Some(part);
    }
    part["parts"].as_array()?.iter().find_map(|p| find_part(p, mime))
}

/// The readable text of a message: text/plain if there is one, else text/html stripped.
pub fn body_text(payload: &Value) -> String {
    if let Some(p) = find_part(payload, "text/plain") {
        return decode(p["body"]["data"].as_str().unwrap_or("")).replace("\r\n", "\n").trim().to_string();
    }
    if let Some(p) = find_part(payload, "text/html") {
        return super::html_to_text(&decode(p["body"]["data"].as_str().unwrap_or("")));
    }
    String::new()
}

fn summary(m: &Value) -> MailSummary {
    let payload = &m["payload"];
    MailSummary {
        id: m["id"].as_str().unwrap_or("").into(),
        thread_id: m["threadId"].as_str().unwrap_or("").into(),
        from: header(payload, "From"),
        subject: header(payload, "Subject"),
        internal_ms: m["internalDate"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0),
        snippet: decode_entities(m["snippet"].as_str().unwrap_or("")),
        unread: labels(m).iter().any(|l| l == "UNREAD"),
    }
}

fn labels(m: &Value) -> Vec<String> {
    m["labelIds"].as_array().map(|a| a.iter().filter_map(|l| l.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

fn decode_entities(s: &str) -> String {
    s.replace("&#39;", "'").replace("&quot;", "\"").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">")
}

fn mail(m: &Value) -> Mail {
    let payload = &m["payload"];
    let mut body = body_text(payload);
    if body.chars().count() > MAX_BODY {
        body = format!("{}\n[… cut, {} characters in all]", body.chars().take(MAX_BODY).collect::<String>(), body.chars().count());
    }
    Mail {
        id: m["id"].as_str().unwrap_or("").into(),
        thread_id: m["threadId"].as_str().unwrap_or("").into(),
        from: header(payload, "From"),
        reply_to: header(payload, "Reply-To"),
        to: header(payload, "To"),
        cc: header(payload, "Cc"),
        subject: header(payload, "Subject"),
        internal_ms: m["internalDate"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0),
        body,
        message_id: header(payload, "Message-ID"),
        references: header(payload, "References"),
        labels: labels(m),
    }
}

/// Header values can't carry line breaks: that's how extra headers get smuggled in.
fn one_line(s: &str) -> String {
    s.chars().map(|c| if c == '\r' || c == '\n' { ' ' } else { c }).collect::<String>().trim().to_string()
}

fn encoded_word(s: &str) -> String {
    if s.is_ascii() {
        s.to_string()
    } else {
        format!("=?UTF-8?B?{}?=", base64::engine::general_purpose::STANDARD.encode(s))
    }
}

/// `Ana Müller <ana@example.com>, bob@example.com` with non-ASCII names encoded.
fn address_list(s: &str) -> String {
    one_line(s)
        .split(',')
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(|a| match a.rsplit_once('<') {
            Some((name, addr)) if !name.trim().is_empty() => {
                let name = name.trim().trim_matches('"');
                let name = if name.is_ascii() { format!("\"{}\"", name.replace('"', "")) } else { encoded_word(name) };
                format!("{name} <{}", addr.trim())
            }
            _ => a.to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The base64url `raw` field Gmail takes: an RFC 2822 message with a UTF-8 text body.
pub fn build_raw(d: &MailDraft, t: &Threading) -> String {
    let mut msg = String::new();
    msg.push_str(&format!("To: {}\r\n", address_list(&d.to)));
    if !d.cc.trim().is_empty() {
        msg.push_str(&format!("Cc: {}\r\n", address_list(&d.cc)));
    }
    msg.push_str(&format!("Subject: {}\r\n", encoded_word(&one_line(&d.subject))));
    if let Some(r) = t.in_reply_to.as_deref().filter(|r| !r.is_empty()) {
        msg.push_str(&format!("In-Reply-To: {}\r\n", one_line(r)));
        let refs = t.references.as_deref().map(one_line).filter(|s| !s.is_empty());
        msg.push_str(&format!("References: {}\r\n", refs.map(|s| format!("{s} {r}")).unwrap_or_else(|| one_line(r))));
    }
    msg.push_str("MIME-Version: 1.0\r\nContent-Type: text/plain; charset=\"UTF-8\"\r\nContent-Transfer-Encoding: base64\r\n\r\n");
    let body = base64::engine::general_purpose::STANDARD.encode(d.body.replace("\r\n", "\n").replace('\n', "\r\n"));
    for chunk in body.as_bytes().chunks(76) {
        msg.push_str(std::str::from_utf8(chunk).unwrap_or(""));
        msg.push_str("\r\n");
    }
    base64::engine::general_purpose::URL_SAFE.encode(msg)
}

fn subject_re(s: &str) -> String {
    if s.to_lowercase().starts_with("re:") {
        s.to_string()
    } else {
        format!("Re: {s}")
    }
}

impl Google {
    fn gmail(&self, path: &str) -> String {
        format!("{}/{path}", self.endpoints.gmail)
    }

    pub async fn mail_search(&self, query: &str, max: u32) -> anyhow::Result<Vec<MailSummary>> {
        let list = self.get(&self.gmail("messages"), &[("q", query.to_string()), ("maxResults", max.clamp(1, 25).to_string())]).await?;
        let ids: Vec<String> = list["messages"].as_array().map(|a| a.iter().filter_map(|m| m["id"].as_str().map(str::to_string)).collect()).unwrap_or_default();
        let query: [(&str, String); 4] = [
            ("format", "metadata".into()),
            ("metadataHeaders", "From".into()),
            ("metadataHeaders", "Subject".into()),
            ("metadataHeaders", "Date".into()),
        ];
        let urls: Vec<String> = ids.iter().map(|id| self.gmail(&format!("messages/{id}"))).collect();
        let fetches = urls.iter().map(|url| self.get(url, &query));
        let mut out = vec![];
        for m in futures_util::future::join_all(fetches).await {
            out.push(summary(&m?));
        }
        Ok(out)
    }

    pub async fn mail_read(&self, id: &str) -> anyhow::Result<Mail> {
        anyhow::ensure!(!id.trim().is_empty(), "`id` is required (from mail_search)");
        let m = self.get(&self.gmail(&format!("messages/{}", id.trim())), &[("format", "full".into())]).await?;
        Ok(mail(&m))
    }

    /// Fills in a reply's recipient and subject from the original, and its thread headers.
    pub async fn mail_prepare(&self, mut d: MailDraft) -> anyhow::Result<(MailDraft, Threading)> {
        let Some(orig_id) = d.reply_to.clone().filter(|r| !r.trim().is_empty()) else {
            anyhow::ensure!(!d.to.trim().is_empty(), "`to` is required (an email address; use contacts_find for names)");
            return Ok((d, Threading::default()));
        };
        let orig = self.mail_read(&orig_id).await.context("reading the message to reply to")?;
        if d.to.trim().is_empty() {
            d.to = if orig.reply_to.is_empty() { orig.from.clone() } else { orig.reply_to.clone() };
        }
        if d.subject.trim().is_empty() {
            d.subject = subject_re(&orig.subject);
        }
        let t = Threading {
            thread_id: Some(orig.thread_id.clone()).filter(|s| !s.is_empty()),
            in_reply_to: Some(orig.message_id.clone()).filter(|s| !s.is_empty()),
            references: Some(orig.references.clone()).filter(|s| !s.is_empty()),
        };
        Ok((d, t))
    }

    fn message_body(raw: String, t: &Threading) -> Value {
        let mut m = json!({ "raw": raw });
        if let Some(tid) = &t.thread_id {
            m["threadId"] = json!(tid);
        }
        m
    }

    /// Saves a draft in Gmail; returns its id.
    pub async fn mail_draft(&self, d: &MailDraft, t: &Threading) -> anyhow::Result<String> {
        let r = self.post(&self.gmail("drafts"), &[], &json!({ "message": Self::message_body(build_raw(d, t), t) })).await?;
        Ok(r["id"].as_str().unwrap_or("").to_string())
    }

    /// A saved draft, as the send card shows it.
    pub async fn mail_get_draft(&self, draft_id: &str) -> anyhow::Result<(MailDraft, Threading)> {
        let r = self.get(&self.gmail(&format!("drafts/{}", draft_id.trim())), &[("format", "full".into())]).await?;
        let m = mail(&r["message"]);
        let d = MailDraft { to: m.to, cc: m.cc, subject: m.subject, body: m.body, reply_to: None };
        let payload = &r["message"]["payload"];
        let t = Threading {
            thread_id: Some(m.thread_id).filter(|s| !s.is_empty()),
            in_reply_to: Some(header(payload, "In-Reply-To")).filter(|s| !s.is_empty()),
            references: Some(header(payload, "References"))
                .filter(|s| !s.is_empty())
                // build_raw appends In-Reply-To to References again.
                .map(|r| r.trim_end_matches(&header(payload, "In-Reply-To")).trim().to_string())
                .filter(|s| !s.is_empty()),
        };
        Ok((d, t))
    }

    /// Sends a new message; returns the sent message's id.
    pub async fn mail_send(&self, d: &MailDraft, t: &Threading) -> anyhow::Result<String> {
        let r = self.post(&self.gmail("messages/send"), &[], &Self::message_body(build_raw(d, t), t)).await?;
        Ok(r["id"].as_str().unwrap_or("").to_string())
    }

    /// Sends a saved draft, first saving the user's edits to it if they made any.
    pub async fn mail_send_draft(&self, draft_id: &str, edited: Option<(&MailDraft, &Threading)>) -> anyhow::Result<String> {
        if let Some((d, t)) = edited {
            self.call(
                reqwest::Method::PUT,
                &self.gmail(&format!("drafts/{}", draft_id.trim())),
                &[],
                Some(&json!({ "id": draft_id.trim(), "message": Self::message_body(build_raw(d, t), t) })),
            )
            .await?;
        }
        let r = self.post(&self.gmail("drafts/send"), &[], &json!({ "id": draft_id.trim() })).await?;
        Ok(r["id"].as_str().unwrap_or("").to_string())
    }

    async fn label_ids(&self, names: &[String]) -> anyhow::Result<Vec<String>> {
        if names.is_empty() {
            return Ok(vec![]);
        }
        let all = self.get(&self.gmail("labels"), &[]).await?;
        let all = all["labels"].as_array().cloned().unwrap_or_default();
        names
            .iter()
            .map(|n| {
                all.iter()
                    .find(|l| l["name"].as_str().is_some_and(|x| x.eq_ignore_ascii_case(n.trim())) || l["id"].as_str() == Some(n.trim()))
                    .and_then(|l| l["id"].as_str().map(str::to_string))
                    .ok_or_else(|| anyhow::anyhow!("no Gmail label called \"{n}\""))
            })
            .collect()
    }

    pub async fn mail_modify(&self, id: &str, archive: Option<bool>, read: Option<bool>, star: Option<bool>, add: &[String], remove: &[String]) -> anyhow::Result<()> {
        let mut add_ids = self.label_ids(add).await?;
        let mut remove_ids = self.label_ids(remove).await?;
        let mut flag = |on: Option<bool>, label: &str, when_on_add: bool| match on {
            Some(v) if v == when_on_add => add_ids.push(label.into()),
            Some(_) => remove_ids.push(label.into()),
            None => {}
        };
        flag(archive, "INBOX", false);
        flag(read, "UNREAD", false);
        flag(star, "STARRED", true);
        if add_ids.is_empty() && remove_ids.is_empty() {
            bail!("nothing to change: give archive, read, star or labels");
        }
        self.post(&self.gmail(&format!("messages/{}/modify", id.trim())), &[], &json!({ "addLabelIds": add_ids, "removeLabelIds": remove_ids })).await?;
        Ok(())
    }

    pub async fn mail_trash(&self, id: &str) -> anyhow::Result<()> {
        anyhow::ensure!(!id.trim().is_empty(), "`id` is required");
        self.post(&self.gmail(&format!("messages/{}/trash", id.trim())), &[], &json!({})).await?;
        Ok(())
    }

    /// The account's address and current history id (the starting point for `mail_history`).
    pub async fn mail_profile(&self) -> anyhow::Result<(String, String)> {
        let p = self.get(&self.gmail("profile"), &[]).await?;
        let history = match &p["historyId"] {
            Value::String(s) => s.clone(),
            v => v.to_string(),
        };
        Ok((p["emailAddress"].as_str().unwrap_or("").to_string(), history))
    }

    /// Ids of messages added to the inbox since `start`, and the history id to use next time.
    pub async fn mail_history(&self, start: &str) -> anyhow::Result<(Vec<String>, String)> {
        let mut ids: Vec<String> = vec![];
        let mut latest = start.to_string();
        let mut page: Option<String> = None;
        for _ in 0..10 {
            let mut q = vec![("startHistoryId", start.to_string()), ("historyTypes", "messageAdded".into()), ("labelId", "INBOX".into())];
            if let Some(p) = &page {
                q.push(("pageToken", p.clone()));
            }
            let r = self.get(&self.gmail("history"), &q).await?;
            for h in r["history"].as_array().into_iter().flatten() {
                for added in h["messagesAdded"].as_array().into_iter().flatten() {
                    if let Some(id) = added["message"]["id"].as_str() {
                        if !ids.iter().any(|x| x == id) {
                            ids.push(id.to_string());
                        }
                    }
                }
            }
            if let Some(h) = r["historyId"].as_str() {
                latest = h.to_string();
            }
            page = r["nextPageToken"].as_str().map(str::to_string);
            if page.is_none() {
                break;
            }
        }
        Ok((ids, latest))
    }

    /// The user's own words from recent sent mail (quotes and signatures trimmed), for learning their style.
    pub async fn mail_sent_samples(&self, n: u32) -> anyhow::Result<Vec<String>> {
        let list = self.mail_search("in:sent", n).await?;
        let reads = list.iter().map(|m| self.mail_read(&m.id));
        let mut out = vec![];
        for m in futures_util::future::join_all(reads).await.into_iter().flatten() {
            let own: Vec<&str> = m
                .body
                .lines()
                .take_while(|l| !(l.starts_with("On ") && l.trim_end().ends_with("wrote:")) && !l.starts_with("-----Original"))
                .filter(|l| !l.starts_with('>'))
                .collect();
            let text = own.join("\n").trim().to_string();
            if !text.is_empty() {
                out.push(super::clip(&text, 1200));
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unraw(raw: &str) -> String {
        String::from_utf8(base64::engine::general_purpose::URL_SAFE.decode(raw).unwrap()).unwrap()
    }

    #[test]
    fn raw_messages_thread_and_cannot_smuggle_headers() {
        let d = MailDraft {
            to: "Ana Müller <ana@example.com>, bob@example.com".into(),
            cc: String::new(),
            subject: "Lunch?\r\nBcc: evil@example.com".into(),
            body: "Hi Ana,\nSee you at 1.\n– Sam".into(),
            reply_to: None,
        };
        let t = Threading { thread_id: Some("t1".into()), in_reply_to: Some("<m1@mail>".into()), references: Some("<m0@mail>".into()) };
        let msg = unraw(&build_raw(&d, &t));
        let (head, body) = msg.split_once("\r\n\r\n").unwrap();
        assert!(head.contains("To: =?UTF-8?B?"), "{head}");
        assert!(head.contains(", bob@example.com"));
        assert!(!head.lines().any(|l| l.starts_with("Bcc:")), "line breaks in a subject can't add headers: {head}");
        assert!(head.contains("In-Reply-To: <m1@mail>\r\nReferences: <m0@mail> <m1@mail>"), "{head}");
        let text = base64::engine::general_purpose::STANDARD.decode(body.replace("\r\n", "")).unwrap();
        assert_eq!(String::from_utf8(text).unwrap(), "Hi Ana,\r\nSee you at 1.\r\n– Sam");
    }

    #[test]
    fn bodies_prefer_plain_text_and_fall_back_to_html() {
        let enc = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
        let both = json!({ "mimeType": "multipart/alternative", "parts": [
            { "mimeType": "text/html", "body": { "data": enc("<p>Hello <b>there</b></p>") } },
            { "mimeType": "text/plain", "body": { "data": enc("Hello there\r\n") } }
        ]});
        assert_eq!(body_text(&both), "Hello there");
        let html_only = json!({ "mimeType": "multipart/mixed", "parts": [{ "mimeType": "multipart/alternative", "parts": [
            { "mimeType": "text/html", "body": { "data": enc("<div>Only <i>HTML</i></div>") } }
        ]}]});
        assert_eq!(body_text(&html_only), "Only HTML");
    }
}
