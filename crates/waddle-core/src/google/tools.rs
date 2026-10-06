//! The Gmail, Calendar and Contacts tools: schemas, handlers and the text the
//! model gets back. Tiers live in `safety.rs`; `mail_send` (send card and Undo)
//! and `mail_style` (a model call) are run by the agent.

use chrono::{DateTime, Duration, TimeZone};
use serde_json::{json, Value};

use super::calendar::{free_slots, parse_when, Event, NewEvent, When};
use super::gmail::MailDraft;
use super::Google;
use crate::llm::{ToolCall, ToolSpec};
use crate::tools::{spec, ToolOutcome};

pub fn is_google_tool(name: &str) -> bool {
    name.starts_with("mail_") || name.starts_with("calendar_") || name.starts_with("drive_") || name == "contacts_find"
}

/// Reads, run several at a time.
pub fn is_parallel_read(name: &str) -> bool {
    matches!(name, "mail_search" | "mail_read" | "calendar_events" | "calendar_free" | "contacts_find" | "drive_search" | "drive_read")
}

pub fn specs() -> Vec<ToolSpec> {
    let draft_props = json!({
        "to": { "type": "string", "description": "Email addresses, comma-separated. For a reply, leave empty to answer the sender" },
        "cc": { "type": "string" },
        "subject": { "type": "string", "description": "For a reply, leave empty for \"Re: <original>\"" },
        "body": { "type": "string", "description": "Plain text, signed the way the user signs" },
        "reply_to": { "type": "string", "description": "Id of the message this replies to (keeps it in the thread)" }
    });
    let mut send_props = draft_props.clone();
    send_props["draft_id"] = json!({ "type": "string", "description": "Send a saved draft instead of giving the message" });
    vec![
        spec(
            "mail_search",
            "Search Gmail with Gmail's search syntax, e.g. \"is:unread newer_than:2d\", \"from:sam invoice\", \"in:inbox\". Newest first.",
            json!({ "query": { "type": "string" }, "max": { "type": "integer", "description": "Default 10, at most 25" } }),
            &["query"],
        ),
        spec("mail_read", "Read one email by id (from mail_search).", json!({ "id": { "type": "string" } }), &["id"]),
        spec("mail_draft", "Save a draft in Gmail without sending it.", draft_props, &["body"]),
        spec(
            "mail_send",
            "Send an email or a saved draft. The user sees the full message and has to click Send, so only use this when they asked you to send.",
            send_props,
            &[],
        ),
        spec(
            "mail_modify",
            "Archive, mark read or unread, star, or label an email.",
            json!({
                "id": { "type": "string" },
                "archive": { "type": "boolean" },
                "read": { "type": "boolean" },
                "star": { "type": "boolean" },
                "add_labels": { "type": "array", "items": { "type": "string" } },
                "remove_labels": { "type": "array", "items": { "type": "string" } }
            }),
            &["id"],
        ),
        spec("mail_trash", "Move an email to the bin (the user has to approve).", json!({ "id": { "type": "string" } }), &["id"]),
        spec(
            "mail_style",
            "learn: read about 20 of the user's sent emails once and save a short note on how they write (used for every draft). show: see the note.",
            json!({ "action": { "type": "string", "enum": ["learn", "show"] } }),
            &["action"],
        ),
        spec(
            "calendar_events",
            "List events in the user's calendar between from and to (local times like 2026-10-05T09:00; default: the next 7 days), optionally matching a text query.",
            json!({ "from": { "type": "string" }, "to": { "type": "string" }, "query": { "type": "string" } }),
            &[],
        ),
        spec(
            "calendar_create",
            "Create an event. Attendees get an invitation. meet: true adds a Google Meet link.",
            json!({
                "title": { "type": "string" },
                "start": { "type": "string", "description": "Local time, e.g. 2026-10-05T14:00" },
                "end": { "type": "string", "description": "Default: 30 minutes after start" },
                "attendees": { "type": "array", "items": { "type": "string" }, "description": "Email addresses" },
                "location": { "type": "string" },
                "description": { "type": "string" },
                "meet": { "type": "boolean" }
            }),
            &["title", "start"],
        ),
        spec(
            "calendar_update",
            "Change an event: give only what changes. Attendees are told.",
            json!({
                "id": { "type": "string" },
                "title": { "type": "string" },
                "start": { "type": "string" },
                "end": { "type": "string" },
                "location": { "type": "string" },
                "description": { "type": "string" },
                "add_attendees": { "type": "array", "items": { "type": "string" } }
            }),
            &["id"],
        ),
        spec(
            "calendar_respond",
            "Answer an invitation.",
            json!({ "id": { "type": "string" }, "response": { "type": "string", "enum": ["accepted", "declined", "tentative"] } }),
            &["id", "response"],
        ),
        spec("calendar_delete", "Delete an event (the user has to approve; attendees are told).", json!({ "id": { "type": "string" } }), &["id"]),
        spec(
            "calendar_free",
            "Find free times in the user's own calendar, within their working hours on weekdays. Returns the best 3.",
            json!({
                "minutes": { "type": "integer", "description": "Length of the meeting (default 30)" },
                "from": { "type": "string", "description": "Default: now" },
                "to": { "type": "string", "description": "Default: 7 days after from" }
            }),
            &[],
        ),
        spec(
            "drive_search",
            "Find files in the user's Google Drive by name or text inside.",
            json!({ "query": { "type": "string" }, "max": { "type": "integer", "description": "Default 10" } }),
            &["query"],
        ),
        spec("drive_read", "Read a Google Drive file as text (Docs, Sheets as tables, Slides, PDF, Word, Excel).", json!({ "id": { "type": "string" } }), &["id"]),
        spec(
            "contacts_find",
            "Look up people in the user's Google Contacts (and people they've emailed) by name; returns names and email addresses.",
            json!({ "name": { "type": "string" } }),
            &["name"],
        ),
    ]
}

pub fn summarize(call: &ToolCall) -> Option<String> {
    let a = &call.arguments;
    let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let short = |t: String| if t.chars().count() > 50 { format!("{}…", t.chars().take(50).collect::<String>()) } else { t };
    Some(match call.name.as_str() {
        "mail_search" => format!("Search mail: {}", short(s("query"))),
        "mail_read" => "Read an email".into(),
        "mail_draft" => format!("Save a draft{}", if s("to").is_empty() { String::new() } else { format!(" to {}", short(s("to"))) }),
        "mail_send" => format!("Send an email{}", if s("to").is_empty() { String::new() } else { format!(" to {}", short(s("to"))) }),
        "mail_modify" => "Tidy an email".into(),
        "mail_trash" => "Move an email to the bin".into(),
        "mail_style" => "Learn how you write emails".into(),
        "calendar_events" => "Check your calendar".into(),
        "calendar_create" => format!("Add \"{}\" to your calendar", short(s("title"))),
        "calendar_update" => "Change a calendar event".into(),
        "calendar_respond" => format!("Answer an invitation: {}", s("response")),
        "calendar_delete" => "Delete a calendar event".into(),
        "calendar_free" => "Find a free time".into(),
        "contacts_find" => format!("Look up {}", short(s("name"))),
        "drive_search" => format!("Search Drive: {}", short(s("query"))),
        "drive_read" => "Read a Drive file".into(),
        _ => return None,
    })
}

fn day_time<Tz: TimeZone>(t: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    t.format("%a %-d %b %H:%M").to_string()
}

fn mail_time<Tz: TimeZone>(ms: i64, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    tz.timestamp_millis_opt(ms).single().map(|t| day_time(&t)).unwrap_or_default()
}

/// One line per event, in the user's time zone.
pub fn event_line<Tz: TimeZone>(e: &Event, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let when = match (&e.start, &e.end) {
        (When::Day(d), _) => format!("{} (all day)", d.format("%a %-d %b")),
        (start, end) => {
            let (s, en) = (start.instant(tz), end.instant(tz));
            let end_fmt = if s.date_naive() == en.date_naive() { en.format("%H:%M").to_string() } else { day_time(&en) };
            format!("{}–{end_fmt}", day_time(&s))
        }
    };
    let mut line = format!("[{}] {when} \"{}\"", e.id, e.title);
    if let Some(m) = &e.meet {
        line.push_str(&format!(" · Meet {m}"));
    }
    if !e.location.is_empty() {
        line.push_str(&format!(" · at {}", e.location));
    }
    let others: Vec<String> = e
        .attendees
        .iter()
        .filter(|a| !a.is_self)
        .take(8)
        .map(|a| {
            let who = if a.name.is_empty() { a.email.clone() } else { format!("{} <{}>", a.name, a.email) };
            if a.response == "needsAction" { who } else { format!("{who} ({})", a.response) }
        })
        .collect();
    if !others.is_empty() {
        line.push_str(&format!(" · with {}", others.join(", ")));
    }
    if let Some(r) = e.my_response() {
        line.push_str(&format!(" · you: {r}"));
    }
    if !e.description.is_empty() {
        line.push_str(&format!("\n    {}", super::clip(&e.description.replace('\n', " "), 200)));
    }
    line
}

fn strings(v: &Value, k: &str) -> Vec<String> {
    match v.get(k) {
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(|s| s.trim().to_string())).filter(|s| !s.is_empty()).collect(),
        Some(Value::String(s)) => s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
        _ => vec![],
    }
}

/// The message a `mail_draft` / `mail_send` call describes.
pub fn draft_from(args: &Value) -> MailDraft {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    MailDraft { to: s("to"), cc: s("cc"), subject: s("subject"), body: s("body"), reply_to: Some(s("reply_to")).filter(|r| !r.trim().is_empty()) }
}

/// What the approval card shows for an action on an existing email or event, looked
/// up from Google so the card says what will really be affected.
pub async fn target_detail<Tz: TimeZone>(g: &Google, call: &ToolCall, tz: &Tz) -> Option<String>
where
    Tz::Offset: std::fmt::Display,
{
    let id = call.arguments.get("id").and_then(Value::as_str)?;
    match call.name.as_str() {
        "mail_trash" | "mail_modify" => {
            let m = g.mail_read(id).await.ok()?;
            Some(format!("From: {}\nSubject: {}\nDate: {}\n\n{}", m.from, m.subject, mail_time(m.internal_ms, tz), super::clip(&m.body, 300)))
        }
        "calendar_delete" | "calendar_update" | "calendar_respond" => g.calendar_get(id).await.ok().map(|e| event_line(&e, tz)),
        _ => None,
    }
}

/// Runs every Google tool except `mail_send` and `mail_style`.
pub async fn run<Tz: TimeZone>(g: &Google, call: &ToolCall, now: DateTime<Tz>, hours: (u32, u32)) -> anyhow::Result<ToolOutcome>
where
    Tz::Offset: std::fmt::Display,
{
    let a = &call.arguments;
    let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let n = |k: &str| a.get(k).and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())));
    let b = |k: &str| a.get(k).and_then(Value::as_bool);
    let tz = now.timezone();
    let time = |k: &str| -> anyhow::Result<Option<DateTime<Tz>>> {
        let v = s(k);
        if v.is_empty() {
            Ok(None)
        } else {
            parse_when(&v, &now).map(Some)
        }
    };
    match call.name.as_str() {
        "mail_search" => {
            let list = g.mail_search(&s("query"), n("max").unwrap_or(10) as u32).await?;
            if list.is_empty() {
                let q = s("query");
                // Gmail matches words literally: "newsletter" doesn't find a digest from Medium.
                let words = q.split_whitespace().any(|w| !w.contains(':'));
                let hint = if words { " Gmail matches words exactly. To find a kind of email (newsletters, promotions, receipts), search \"in:inbox\" and judge by sender and subject." } else { "" };
                return Ok(ToolOutcome::trusted(format!("No emails match \"{q}\".{hint}")));
            }
            let lines: Vec<String> = list
                .iter()
                .map(|m| {
                    format!(
                        "[{}] {} · {} · \"{}\"{}\n    {}",
                        m.id,
                        mail_time(m.internal_ms, &tz),
                        m.from,
                        m.subject,
                        if m.unread { " (unread)" } else { "" },
                        super::clip(&m.snippet, 160)
                    )
                })
                .collect();
            Ok(ToolOutcome::untrusted("email_list", lines.join("\n")))
        }
        "mail_read" => {
            let m = g.mail_read(&s("id")).await?;
            let mut head = format!("From: {}\nTo: {}\n", m.from, m.to);
            if !m.cc.is_empty() {
                head.push_str(&format!("Cc: {}\n", m.cc));
            }
            head.push_str(&format!("Date: {}\nSubject: {}\n\n{}", mail_time(m.internal_ms, &tz), m.subject, m.body));
            Ok(ToolOutcome::untrusted("email", head))
        }
        "mail_draft" => {
            let (d, t) = g.mail_prepare(draft_from(a)).await?;
            let id = g.mail_draft(&d, &t).await?;
            Ok(ToolOutcome::trusted(format!("Saved draft [{id}] to {}: \"{}\". It's in Gmail's Drafts; mail_send with draft_id sends it.", d.to, d.subject)))
        }
        "mail_modify" => {
            g.mail_modify(&s("id"), b("archive"), b("read"), b("star"), &strings(a, "add_labels"), &strings(a, "remove_labels")).await?;
            Ok(ToolOutcome::trusted("Done."))
        }
        "mail_trash" => {
            g.mail_trash(&s("id")).await?;
            Ok(ToolOutcome::trusted("Moved to the bin (it can be restored from Gmail's Bin for 30 days)."))
        }
        "calendar_events" => {
            let from = time("from")?.unwrap_or_else(|| now.clone());
            let to = time("to")?.unwrap_or_else(|| from.clone() + if s("from").is_empty() { Duration::days(7) } else { Duration::days(1) });
            let events = g.calendar_events(&from.to_rfc3339(), &to.to_rfc3339(), &s("query"), 50).await?;
            if events.is_empty() {
                return Ok(ToolOutcome::trusted(format!("No events between {} and {}.", day_time(&from), day_time(&to))));
            }
            let lines: Vec<String> = events.iter().map(|e| event_line(e, &tz)).collect();
            Ok(ToolOutcome::untrusted("calendar", format!("Now: {}\n{}", day_time(&now), lines.join("\n"))))
        }
        "calendar_create" => {
            let start = time("start")?.ok_or_else(|| anyhow::anyhow!("`start` is required"))?;
            let end = time("end")?.unwrap_or_else(|| start.clone() + Duration::minutes(30));
            anyhow::ensure!(end > start, "`end` must be after `start`");
            let e = g
                .calendar_create(&NewEvent {
                    title: s("title"),
                    start: start.to_rfc3339(),
                    end: end.to_rfc3339(),
                    attendees: strings(a, "attendees"),
                    location: s("location"),
                    description: s("description"),
                    meet: b("meet").unwrap_or(false),
                })
                .await?;
            Ok(ToolOutcome::trusted(format!("Created: {}", event_line(&e, &tz))))
        }
        "calendar_update" => {
            let id = s("id");
            let mut patch = json!({});
            for (k, field) in [("title", "summary"), ("location", "location"), ("description", "description")] {
                if !s(k).is_empty() {
                    patch[field] = json!(s(k));
                }
            }
            let start = time("start")?;
            let mut end = time("end")?;
            if let (Some(st), None) = (&start, &end) {
                // Moving an event keeps its length.
                let old = g.calendar_get(&id).await?;
                end = Some(st.clone() + (old.end.instant(&tz) - old.start.instant(&tz)));
            }
            if let Some(st) = start {
                patch["start"] = json!({ "dateTime": st.to_rfc3339() });
            }
            if let Some(en) = end {
                patch["end"] = json!({ "dateTime": en.to_rfc3339() });
            }
            let add = strings(a, "add_attendees");
            if !add.is_empty() {
                let old = g.get(&format!("{}/calendars/primary/events/{}", g.endpoints.calendar, id.trim()), &[]).await?;
                let mut list = old["attendees"].as_array().cloned().unwrap_or_default();
                for email in add {
                    if !list.iter().any(|x| x["email"].as_str().is_some_and(|e| e.eq_ignore_ascii_case(&email))) {
                        list.push(json!({ "email": email }));
                    }
                }
                patch["attendees"] = json!(list);
            }
            anyhow::ensure!(patch.as_object().is_some_and(|o| !o.is_empty()), "nothing to change");
            let e = g.calendar_update(&id, &patch).await?;
            Ok(ToolOutcome::trusted(format!("Updated: {}", event_line(&e, &tz))))
        }
        "calendar_respond" => {
            let e = g.calendar_respond(&s("id"), &s("response")).await?;
            Ok(ToolOutcome::trusted(format!("Answered: {}", event_line(&e, &tz))))
        }
        "calendar_delete" => {
            g.calendar_delete(&s("id")).await?;
            Ok(ToolOutcome::trusted("Deleted the event."))
        }
        "calendar_free" => {
            let minutes = n("minutes").unwrap_or(30).clamp(5, 8 * 60) as i64;
            let from = time("from")?.map(|f| f.max(now.clone())).unwrap_or_else(|| now.clone());
            let to = time("to")?.unwrap_or_else(|| from.clone() + Duration::days(7));
            let events = g.calendar_events(&from.to_rfc3339(), &to.to_rfc3339(), "", 100).await?;
            let busy: Vec<_> = events.iter().filter(|e| e.busy()).map(|e| (e.start.instant(&tz), e.end.instant(&tz))).collect();
            let slots = free_slots(&busy, &from, &to, minutes, hours, 3);
            if slots.is_empty() {
                return Ok(ToolOutcome::trusted(format!(
                    "No free {minutes}-minute slot between {} and {} within working hours ({}:00-{}:00, weekdays).",
                    day_time(&from),
                    day_time(&to),
                    hours.0,
                    hours.1
                )));
            }
            let lines: Vec<String> = slots.iter().map(|(st, en)| format!("{}–{}", day_time(st), en.format("%H:%M"))).collect();
            Ok(ToolOutcome::trusted(format!("Free {minutes}-minute slots (working hours {}:00-{}:00):\n{}", hours.0, hours.1, lines.join("\n"))))
        }
        "drive_search" => {
            let files = g.drive_search(&s("query"), n("max").unwrap_or(10) as u32).await?;
            if files.is_empty() {
                return Ok(ToolOutcome::trusted(format!("Nothing in Drive matches \"{}\".", s("query"))));
            }
            let lines: Vec<String> = files
                .iter()
                .map(|f| format!("[{}] {} \"{}\" (modified {})", f.id, super::drive::kind(&f.mime), f.name, f.modified.get(..10).unwrap_or(&f.modified)))
                .collect();
            Ok(ToolOutcome::untrusted("drive_files", lines.join("\n")))
        }
        "drive_read" => {
            let (f, text) = g.drive_read(&s("id")).await?;
            Ok(ToolOutcome::untrusted("drive_file", format!("{} \"{}\" ({})\n\n{text}", super::drive::kind(&f.mime), f.name, f.link)))
        }
        "contacts_find" => {
            let found = g.contacts_find(&s("name")).await?;
            if found.is_empty() {
                return Ok(ToolOutcome::trusted(format!("No contacts match \"{}\". Ask the user for the email address.", s("name"))));
            }
            let lines: Vec<String> = found.iter().map(|c| if c.name.is_empty() { c.email.clone() } else { format!("{} <{}>", c.name, c.email) }).collect();
            let note = if found.len() > 1 { "\n(More than one match: if it isn't clear which person the user means, ask.)" } else { "" };
            Ok(ToolOutcome::untrusted("contacts", format!("{}{note}", lines.join("\n"))))
        }
        other => anyhow::bail!("`{other}` isn't run here"),
    }
}
