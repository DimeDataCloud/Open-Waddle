//! Google Calendar (the user's primary calendar): list, create, update, respond,
//! delete, and free-slot finding inside working hours.

use anyhow::bail;
use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike, Weekday};
use serde_json::{json, Value};

use super::Google;

#[derive(Debug, Clone, PartialEq)]
pub enum When {
    At(DateTime<FixedOffset>),
    /// All-day events.
    Day(NaiveDate),
}

impl When {
    fn parse(v: &Value) -> Option<When> {
        if let Some(dt) = v["dateTime"].as_str() {
            return DateTime::parse_from_rfc3339(dt).ok().map(When::At);
        }
        v["date"].as_str().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()).map(When::Day)
    }

    /// The moment this starts, in `tz` (all-day events start at local midnight).
    pub fn instant<Tz: TimeZone>(&self, tz: &Tz) -> DateTime<Tz> {
        match self {
            When::At(dt) => dt.with_timezone(tz),
            When::Day(d) => tz
                .from_local_datetime(&d.and_time(NaiveTime::MIN))
                .earliest()
                .unwrap_or_else(|| tz.from_utc_datetime(&d.and_time(NaiveTime::MIN))),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attendee {
    pub email: String,
    pub name: String,
    pub response: String,
    pub is_self: bool,
    pub organizer: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub id: String,
    pub title: String,
    pub start: When,
    pub end: When,
    pub location: String,
    pub description: String,
    pub attendees: Vec<Attendee>,
    pub meet: Option<String>,
    pub link: String,
    /// Marked "free" in Google Calendar: doesn't block time.
    pub transparent: bool,
    pub cancelled: bool,
}

impl Event {
    pub fn parse(v: &Value) -> Option<Event> {
        let attendees = v["attendees"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|p| Attendee {
                        email: p["email"].as_str().unwrap_or("").into(),
                        name: p["displayName"].as_str().unwrap_or("").into(),
                        response: p["responseStatus"].as_str().unwrap_or("needsAction").into(),
                        is_self: p["self"].as_bool().unwrap_or(false),
                        organizer: p["organizer"].as_bool().unwrap_or(false),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let meet = v["hangoutLink"].as_str().map(str::to_string).or_else(|| {
            v["conferenceData"]["entryPoints"]
                .as_array()?
                .iter()
                .find(|e| e["entryPointType"].as_str() == Some("video"))
                .and_then(|e| e["uri"].as_str().map(str::to_string))
        });
        Some(Event {
            id: v["id"].as_str()?.into(),
            title: v["summary"].as_str().unwrap_or("(no title)").into(),
            start: When::parse(&v["start"])?,
            end: When::parse(&v["end"])?,
            location: v["location"].as_str().unwrap_or("").into(),
            description: v["description"].as_str().map(super::html_to_text).unwrap_or_default(),
            attendees,
            meet,
            link: v["htmlLink"].as_str().unwrap_or("").into(),
            transparent: v["transparency"].as_str() == Some("transparent"),
            cancelled: v["status"].as_str() == Some("cancelled"),
        })
    }

    /// The user's own answer to the invitation, if they're an attendee.
    pub fn my_response(&self) -> Option<&str> {
        self.attendees.iter().find(|a| a.is_self).map(|a| a.response.as_str())
    }

    /// Whether this blocks the user's time.
    pub fn busy(&self) -> bool {
        !self.transparent && !self.cancelled && self.my_response() != Some("declined") && !matches!(self.start, When::Day(_))
    }
}

/// Reads a time the model gives: RFC 3339, `YYYY-MM-DDTHH:MM`, `YYYY-MM-DD HH:MM`,
/// `YYYY-MM-DD` (midnight), `now`, `today` or `tomorrow`. Local forms are in `now`'s time zone.
pub fn parse_when<Tz: TimeZone>(s: &str, now: &DateTime<Tz>) -> anyhow::Result<DateTime<Tz>> {
    let s = s.trim();
    let tz = now.timezone();
    let midnight = |d: NaiveDate| tz.from_local_datetime(&d.and_time(NaiveTime::MIN)).earliest();
    let found = match s.to_lowercase().as_str() {
        "" => bail!("a time is required"),
        "now" => Some(now.clone()),
        "today" => midnight(now.date_naive()),
        "tomorrow" => midnight(now.date_naive() + Duration::days(1)),
        _ => {
            if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                Some(dt.with_timezone(&tz))
            } else if let Some(naive) = ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"]
                .iter()
                .find_map(|f| NaiveDateTime::parse_from_str(s, f).ok())
            {
                tz.from_local_datetime(&naive).earliest()
            } else if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                midnight(d)
            } else {
                None
            }
        }
    };
    found.ok_or_else(|| anyhow::anyhow!("couldn't read the time `{s}`; use YYYY-MM-DDTHH:MM"))
}

/// Free slots of `minutes` between `from` and `to`, inside working hours
/// (`hours`, e.g. 9..17) on weekdays, avoiding `busy`. Up to `n`, preferring
/// one per day, earliest first; starts are rounded up to the quarter hour.
pub fn free_slots<Tz: TimeZone>(
    busy: &[(DateTime<Tz>, DateTime<Tz>)],
    from: &DateTime<Tz>,
    to: &DateTime<Tz>,
    minutes: i64,
    hours: (u32, u32),
    n: usize,
) -> Vec<(DateTime<Tz>, DateTime<Tz>)> {
    let tz = from.timezone();
    let len = Duration::minutes(minutes.max(5));
    let mut busy: Vec<_> = busy.to_vec();
    busy.sort_by_key(|b| b.0.clone());
    let mut all = vec![];
    let mut day = from.date_naive();
    while day <= to.date_naive() && all.len() < 200 {
        let at = |h: u32| tz.from_local_datetime(&day.and_hms_opt(h.min(23), 0, 0).unwrap()).earliest();
        if !matches!(day.weekday(), Weekday::Sat | Weekday::Sun) {
            if let (Some(open), Some(close)) = (at(hours.0), if hours.1 >= 24 { at(23).map(|d| d + Duration::hours(1)) } else { at(hours.1) }) {
                let start = open.max(from.clone());
                let close = close.min(to.clone());
                let mut cursor = start.clone();
                for (b0, b1) in busy.iter().filter(|(b0, b1)| *b1 > start && *b0 < close) {
                    if *b1 <= cursor {
                        continue;
                    }
                    push_slots(&mut all, round_up(&cursor), b0.clone(), len);
                    cursor = b1.clone();
                }
                push_slots(&mut all, round_up(&cursor), close, len);
            }
        }
        day += Duration::days(1);
    }
    // One per day first, then fill with the rest, keeping time order.
    let mut picked: Vec<(DateTime<Tz>, DateTime<Tz>)> = vec![];
    for s in &all {
        if picked.len() < n && !picked.iter().any(|p| p.0.date_naive() == s.0.date_naive()) {
            picked.push(s.clone());
        }
    }
    for s in &all {
        if picked.len() < n && !picked.contains(s) {
            picked.push(s.clone());
        }
    }
    picked.sort_by_key(|s| s.0.clone());
    picked
}

/// Candidate slots in one gap: at its start, then every hour (or every `len`, if longer).
fn push_slots<Tz: TimeZone>(out: &mut Vec<(DateTime<Tz>, DateTime<Tz>)>, mut start: DateTime<Tz>, gap_end: DateTime<Tz>, len: Duration) {
    let step = len.max(Duration::hours(1));
    while start.clone() + len <= gap_end && out.len() < 200 {
        out.push((start.clone(), start.clone() + len));
        start += step;
    }
}

fn round_up<Tz: TimeZone>(t: &DateTime<Tz>) -> DateTime<Tz> {
    let extra = (15 - t.minute() % 15) % 15;
    let t = t.clone() - Duration::seconds(t.second() as i64) - Duration::nanoseconds(t.nanosecond() as i64);
    if extra == 0 {
        t
    } else {
        t + Duration::minutes(extra as i64)
    }
}

/// What `calendar_create` sends.
#[derive(Debug, Clone, Default)]
pub struct NewEvent {
    pub title: String,
    pub start: String,
    pub end: String,
    pub attendees: Vec<String>,
    pub location: String,
    pub description: String,
    pub meet: bool,
}

impl Google {
    fn cal(&self, path: &str) -> String {
        format!("{}/calendars/primary/{path}", self.endpoints.calendar)
    }

    pub async fn calendar_events(&self, from: &str, to: &str, query: &str, max: u32) -> anyhow::Result<Vec<Event>> {
        let mut q = vec![
            ("timeMin", from.to_string()),
            ("timeMax", to.to_string()),
            ("singleEvents", "true".into()),
            ("orderBy", "startTime".into()),
            ("maxResults", max.clamp(1, 100).to_string()),
        ];
        if !query.trim().is_empty() {
            q.push(("q", query.trim().to_string()));
        }
        let r = self.get(&self.cal("events"), &q).await?;
        Ok(r["items"].as_array().into_iter().flatten().filter_map(Event::parse).filter(|e| !e.cancelled).collect())
    }

    pub async fn calendar_get(&self, id: &str) -> anyhow::Result<Event> {
        let r = self.get(&self.cal(&format!("events/{}", id.trim())), &[]).await?;
        Event::parse(&r).ok_or_else(|| anyhow::anyhow!("Google sent an event Waddle couldn't read"))
    }

    pub async fn calendar_create(&self, e: &NewEvent) -> anyhow::Result<Event> {
        anyhow::ensure!(!e.title.trim().is_empty(), "`title` is required");
        let mut body = json!({
            "summary": e.title.trim(),
            "start": { "dateTime": e.start },
            "end": { "dateTime": e.end },
        });
        if !e.location.trim().is_empty() {
            body["location"] = json!(e.location.trim());
        }
        if !e.description.trim().is_empty() {
            body["description"] = json!(e.description.trim());
        }
        if !e.attendees.is_empty() {
            body["attendees"] = json!(e.attendees.iter().map(|a| json!({ "email": a.trim() })).collect::<Vec<_>>());
        }
        if e.meet {
            let mut id = [0u8; 8];
            let _ = getrandom::fill(&mut id);
            body["conferenceData"] = json!({ "createRequest": { "requestId": hex::encode(id), "conferenceSolutionKey": { "type": "hangoutsMeet" } } });
        }
        let r = self.post(&self.cal("events"), &[("conferenceDataVersion", "1".into()), ("sendUpdates", "all".into())], &body).await?;
        Event::parse(&r).ok_or_else(|| anyhow::anyhow!("Google sent an event Waddle couldn't read"))
    }

    pub async fn calendar_update(&self, id: &str, patch: &Value) -> anyhow::Result<Event> {
        let r = self
            .call(reqwest::Method::PATCH, &self.cal(&format!("events/{}", id.trim())), &[("sendUpdates", "all".into())], Some(patch))
            .await?;
        Event::parse(&r).ok_or_else(|| anyhow::anyhow!("Google sent an event Waddle couldn't read"))
    }

    /// Accepts, declines or tentatively accepts an invitation.
    pub async fn calendar_respond(&self, id: &str, response: &str) -> anyhow::Result<Event> {
        let response = match response.trim().to_lowercase().as_str() {
            "yes" | "accept" | "accepted" => "accepted",
            "no" | "decline" | "declined" => "declined",
            "maybe" | "tentative" => "tentative",
            other => bail!("unknown response `{other}`; use accepted, declined or tentative"),
        };
        let raw = self.get(&self.cal(&format!("events/{}", id.trim())), &[]).await?;
        let mut attendees = raw["attendees"].as_array().cloned().unwrap_or_default();
        let me = attendees.iter_mut().find(|a| a["self"].as_bool() == Some(true)).ok_or_else(|| anyhow::anyhow!("you aren't invited to that event, so there's nothing to answer"))?;
        me["responseStatus"] = json!(response);
        self.calendar_update(id, &json!({ "attendees": attendees })).await
    }

    pub async fn calendar_delete(&self, id: &str) -> anyhow::Result<()> {
        anyhow::ensure!(!id.trim().is_empty(), "`id` is required");
        self.call(reqwest::Method::DELETE, &self.cal(&format!("events/{}", id.trim())), &[("sendUpdates", "all".into())], None).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tz() -> FixedOffset {
        FixedOffset::east_opt(3600).unwrap()
    }

    fn at(d: u32, h: u32, m: u32) -> DateTime<FixedOffset> {
        tz().with_ymd_and_hms(2026, 10, d, h, m, 0).unwrap()
    }

    #[test]
    fn reads_the_time_forms_models_use() {
        let now = at(5, 14, 7); // Monday
        assert_eq!(parse_when("2026-10-06T09:30", &now).unwrap(), at(6, 9, 30));
        assert_eq!(parse_when("2026-10-06 09:30", &now).unwrap(), at(6, 9, 30));
        assert_eq!(parse_when("2026-10-06T08:30:00Z", &now).unwrap(), at(6, 9, 30));
        assert_eq!(parse_when("tomorrow", &now).unwrap(), at(6, 0, 0));
        assert_eq!(parse_when("2026-10-07", &now).unwrap(), at(7, 0, 0));
        assert!(parse_when("teatime", &now).is_err());
    }

    #[test]
    fn free_slots_respect_working_hours_events_and_weekends() {
        // Monday 5 Oct, 10:07. Busy 11:00-12:00 and 13:00-16:30 today; Tuesday morning blocked.
        let now = at(5, 10, 7);
        let busy = vec![(at(5, 11, 0), at(5, 12, 0)), (at(5, 13, 0), at(5, 16, 30)), (at(6, 9, 0), at(6, 12, 0))];
        let slots = free_slots(&busy, &now, &at(9, 23, 0), 60, (9, 17), 3);
        let starts: Vec<_> = slots.iter().map(|s| s.0).collect();
        // Today 10:15 doesn't fit an hour before 11:00; 12:00-13:00 does. One per day first.
        assert_eq!(starts, vec![at(5, 12, 0), at(6, 12, 0), at(7, 9, 0)]);
        assert!(slots.iter().all(|(s, e)| *e - *s == Duration::minutes(60)));

        // Friday afternoon to Monday: the weekend is skipped, and one free day still gives choices an hour apart.
        let slots = free_slots(&[], &at(9, 16, 50), &at(12, 23, 0), 30, (9, 17), 2);
        assert_eq!(slots.iter().map(|s| s.0).collect::<Vec<_>>(), vec![at(12, 9, 0), at(12, 10, 0)]);
    }

    #[test]
    fn parses_events_with_meet_links_and_responses() {
        let v = json!({
            "id": "e1", "summary": "Standup", "status": "confirmed",
            "start": { "dateTime": "2026-10-05T10:00:00+01:00" }, "end": { "dateTime": "2026-10-05T10:15:00+01:00" },
            "attendees": [{ "email": "me@x.com", "self": true, "responseStatus": "declined" }, { "email": "sam@x.com", "responseStatus": "accepted" }],
            "conferenceData": { "entryPoints": [{ "entryPointType": "video", "uri": "https://meet.google.com/abc-defg-hij" }] }
        });
        let e = Event::parse(&v).unwrap();
        assert_eq!(e.meet.as_deref(), Some("https://meet.google.com/abc-defg-hij"));
        assert_eq!(e.my_response(), Some("declined"));
        assert!(!e.busy(), "declined meetings don't block time");
    }
}
