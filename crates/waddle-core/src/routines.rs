//! Routines: tasks that run on their own on a schedule ("every weekday at
//! 8:45, summarise my unread email"). Kept in `routines.json` beside the
//! reminders. The app's clock starts a due routine when Waddle is free; one
//! that came due while the computer was off is marked missed, not run late.
//!
//! A routine runs without the screen (email, calendar, files, connected tools),
//! and every step that would change something waits for the user's click.

use anyhow::{bail, Context};
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;

const MAX_ROUTINES: usize = 20;
const MAX_GOAL: usize = 500;
/// A routine up to this late (the computer was asleep a moment) still runs.
pub const GRACE_MS: i64 = 15 * 60 * 1000;

const DAY_NAMES: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
pub const WEEKDAYS: u8 = 0b0011111;
pub const EVERY_DAY: u8 = 0b1111111;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Routine {
    pub id: String,
    /// What to do, in the user's words.
    pub goal: String,
    /// "HH:MM", local time.
    pub time: String,
    /// Days it repeats on: bit 0 = Monday … bit 6 = Sunday. 0 = once, on `date`.
    pub days: u8,
    /// The day of a one-off routine, "YYYY-MM-DD".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// When it's next due (ms since 1970); None when a one-off is over.
    pub next_ms: Option<i64>,
    #[serde(default)]
    pub paused: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_ms: Option<i64>,
    /// How the last run went, in a few words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_result: Option<String>,
}

impl Routine {
    /// An empty routine, to describe a schedule with.
    pub fn blank() -> Routine {
        Routine { id: String::new(), goal: String::new(), time: String::new(), days: 0, date: None, next_ms: None, paused: false, last_run_ms: None, last_result: None }
    }

    /// "weekdays at 08:45", "Fri at 16:00", "once on Mon 5 Oct at 18:00".
    pub fn schedule(&self) -> String {
        let when = match self.days {
            0 => match self.date.as_deref().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()) {
                Some(d) => format!("once on {}", d.format("%a %-d %b")),
                None => "once".into(),
            },
            EVERY_DAY => "every day".into(),
            WEEKDAYS => "weekdays".into(),
            0b1100000 => "weekends".into(),
            mask => (0..7).filter(|i| mask & (1 << i) != 0).map(|i| title(DAY_NAMES[i])).collect::<Vec<_>>().join(", "),
        };
        format!("{when} at {}", self.time)
    }
}

fn title(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
}

/// Reads "08:45", "8:45", "9am", "7.30 pm".
pub fn parse_time(s: &str) -> Option<NaiveTime> {
    let s = s.trim().to_lowercase().replace(' ', "");
    let (body, pm) = match s.strip_suffix("pm") {
        Some(b) => (b.to_string(), Some(true)),
        None => match s.strip_suffix("am") {
            Some(b) => (b.to_string(), Some(false)),
            None => (s, None),
        },
    };
    let (h, m) = match body.split_once([':', '.']) {
        Some((h, m)) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        None => (body.parse::<u32>().ok()?, 0),
    };
    let h = match pm {
        Some(true) if h < 12 => h + 12,
        Some(false) if h == 12 => 0,
        Some(_) if h > 12 => return None,
        _ => h,
    };
    NaiveTime::from_hms_opt(h, m, 0)
}

/// Days from the tool's `days`: an array of names ("mon", "Tuesday") or one of
/// "daily", "every day", "weekdays", "weekends"; a single name works too.
pub fn parse_days(v: &Value) -> anyhow::Result<u8> {
    let names: Vec<String> = match v {
        Value::Array(a) => a.iter().filter_map(Value::as_str).map(|s| s.trim().to_lowercase()).collect(),
        Value::String(s) => s.split([',', ' ']).map(|p| p.trim().to_lowercase()).filter(|p| !p.is_empty()).collect(),
        Value::Null => return Ok(0),
        _ => bail!("`days` must be a list of day names"),
    };
    let joined = names.join(" ");
    match joined.as_str() {
        "" => return Ok(0),
        "daily" | "every day" | "everyday" | "all" => return Ok(EVERY_DAY),
        "weekdays" | "workdays" | "weekday" => return Ok(WEEKDAYS),
        "weekends" | "weekend" => return Ok(0b1100000),
        _ => {}
    }
    let mut mask = 0u8;
    for n in names {
        let i = DAY_NAMES.iter().position(|d| n.starts_with(d)).ok_or_else(|| anyhow::anyhow!("`{n}` isn't a day name"))?;
        mask |= 1 << i;
    }
    Ok(mask)
}

/// The first time after `after` the routine is due, or None if it never is again.
pub fn next_due<Tz: TimeZone>(time: NaiveTime, days: u8, date: Option<NaiveDate>, after: &DateTime<Tz>) -> Option<DateTime<Tz>> {
    let tz = after.timezone();
    if days == 0 {
        let at = tz.from_local_datetime(&date?.and_time(time)).earliest()?;
        return (at > *after).then_some(at);
    }
    let mut day = after.date_naive();
    for _ in 0..8 {
        let weekday = day.weekday().num_days_from_monday();
        if days & (1 << weekday) != 0 {
            if let Some(at) = tz.from_local_datetime(&day.and_time(time)).earliest() {
                if at > *after {
                    return Some(at);
                }
            }
        }
        day = day.succ_opt()?;
    }
    None
}

/// What the clock should do now.
#[derive(Debug, Clone, PartialEq)]
pub enum Due {
    /// Start this routine.
    Run(Routine),
    /// It came due while Waddle wasn't running; marked missed.
    Missed(Routine),
}

pub struct RoutineStore {
    path: PathBuf,
    items: Mutex<Vec<Routine>>,
}

impl RoutineStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let items = crate::store::load_json(&path);
        Self { path, items: Mutex::new(items) }
    }

    fn save(&self, items: &[Routine]) -> anyhow::Result<()> {
        crate::store::write_atomic(&self.path, serde_json::to_string_pretty(items)?).with_context(|| format!("saving {}", self.path.display()))
    }

    pub fn list(&self) -> Vec<Routine> {
        let mut v = self.items.lock().unwrap().clone();
        v.sort_by_key(|r| (r.paused, r.next_ms.unwrap_or(i64::MAX)));
        v
    }

    pub fn add<Tz: TimeZone>(&self, goal: &str, time: &str, days: u8, date: Option<&str>, now: &DateTime<Tz>) -> anyhow::Result<Routine> {
        let goal = goal.trim();
        anyhow::ensure!(!goal.is_empty(), "a routine needs a `goal`: what to do");
        let t = parse_time(time).ok_or_else(|| anyhow::anyhow!("couldn't read the time `{time}`; use HH:MM"))?;
        let date = match (days, date.map(str::trim).filter(|d| !d.is_empty())) {
            (0, Some(d)) => Some(NaiveDate::parse_from_str(d, "%Y-%m-%d").with_context(|| format!("couldn't read the date `{d}`; use YYYY-MM-DD"))?),
            // Once, with no date: the next time that clock time comes round.
            (0, None) => {
                let today = now.date_naive();
                let at_today = now.timezone().from_local_datetime(&today.and_time(t)).earliest();
                Some(if at_today.is_some_and(|a| a > *now) { today } else { today.succ_opt().unwrap_or(today) })
            }
            (_, _) => None,
        };
        let next = next_due(t, days, date, now).ok_or_else(|| anyhow::anyhow!("that time has already passed"))?;
        let mut items = self.items.lock().unwrap();
        anyhow::ensure!(items.len() < MAX_ROUTINES, "too many routines ({MAX_ROUTINES}); delete some first");
        let mut bytes = [0u8; 3];
        let _ = getrandom::fill(&mut bytes);
        let r = Routine {
            id: hex::encode(bytes),
            goal: goal.chars().take(MAX_GOAL).collect(),
            time: t.format("%H:%M").to_string(),
            days,
            date: date.map(|d| d.format("%Y-%m-%d").to_string()),
            next_ms: Some(next.timestamp_millis()),
            paused: false,
            last_run_ms: None,
            last_result: None,
        };
        items.push(r.clone());
        self.save(&items)?;
        Ok(r)
    }

    fn update<T>(&self, id: &str, f: impl FnOnce(&mut Vec<Routine>, usize) -> T) -> anyhow::Result<T> {
        let mut items = self.items.lock().unwrap();
        let i = items.iter().position(|r| r.id == id.trim()).ok_or_else(|| anyhow::anyhow!("no routine with id `{id}`"))?;
        let out = f(&mut items, i);
        self.save(&items)?;
        Ok(out)
    }

    /// Pauses or resumes. A resumed routine is next due at its next time from now.
    pub fn set_paused<Tz: TimeZone>(&self, id: &str, paused: bool, now: &DateTime<Tz>) -> anyhow::Result<Routine> {
        self.update(id, |items, i| {
            let r = &mut items[i];
            r.paused = paused;
            if !paused {
                r.next_ms = schedule_after(r, now);
            }
            r.clone()
        })
    }

    pub fn delete(&self, id: &str) -> anyhow::Result<Routine> {
        self.update(id, |items, i| items.remove(i))
    }

    /// Routines due by `now`. Ones overdue by more than the grace period are
    /// marked missed and moved to their next time. `free`: Waddle isn't busy,
    /// so a due routine can start (it's moved on too); otherwise it waits.
    pub fn take_due<Tz: TimeZone>(&self, now: &DateTime<Tz>, free: bool) -> Vec<Due> {
        let now_ms = now.timestamp_millis();
        let mut items = self.items.lock().unwrap();
        let mut out = vec![];
        let mut started = false;
        for r in items.iter_mut().filter(|r| !r.paused) {
            let Some(due) = r.next_ms.filter(|d| *d <= now_ms) else { continue };
            if now_ms - due > GRACE_MS {
                r.last_result = Some("Missed: Waddle wasn't running".into());
                r.next_ms = schedule_after(r, now);
                out.push(Due::Missed(r.clone()));
            } else if free && !started {
                // One at a time: the next waits until this one is done.
                started = true;
                r.last_run_ms = Some(now_ms);
                r.next_ms = schedule_after(r, now);
                out.push(Due::Run(r.clone()));
            }
        }
        if !out.is_empty() {
            if let Err(e) = self.save(&items) {
                log::warn!("{e:#}");
            }
        }
        out
    }

    /// Notes how a run went ("Done", "Failed: …").
    pub fn record(&self, id: &str, result: &str) {
        let short: String = result.chars().take(120).collect();
        if let Err(e) = self.update(id, |items, i| items[i].last_result = Some(short)) {
            log::warn!("{e:#}");
        }
    }

    /// The `routine` tool.
    pub fn handle<Tz: TimeZone>(&self, args: &Value, now: DateTime<Tz>) -> anyhow::Result<String>
    where
        Tz::Offset: std::fmt::Display,
    {
        let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
        match s("action").as_str() {
            "list" => {
                let list = self.list();
                if list.is_empty() {
                    return Ok("No routines set.".into());
                }
                let tz = now.timezone();
                let lines: Vec<String> = list
                    .iter()
                    .map(|r| {
                        let next = match (r.paused, r.next_ms) {
                            (true, _) => "paused".to_string(),
                            (false, Some(ms)) => format!("next {}", tz.timestamp_millis_opt(ms).unwrap().format("%a %-d %b %H:%M")),
                            (false, None) => "finished".to_string(),
                        };
                        format!("[{}] {} ({next}): {}", r.id, r.schedule(), r.goal)
                    })
                    .collect();
                Ok(lines.join("\n"))
            }
            "pause" | "resume" => {
                let r = self.set_paused(&s("id"), s("action") == "pause", &now)?;
                Ok(format!("{} the routine \"{}\".", if r.paused { "Paused" } else { "Resumed" }, r.goal))
            }
            "delete" => {
                let r = self.delete(&s("id"))?;
                Ok(format!("Deleted the routine \"{}\".", r.goal))
            }
            "add" | "" => {
                let days = parse_days(args.get("days").unwrap_or(&Value::Null))?;
                let date = s("date");
                let r = self.add(&s("goal"), &s("time"), days, (!date.is_empty()).then_some(date.as_str()), &now)?;
                let next = now.timezone().timestamp_millis_opt(r.next_ms.unwrap_or_default()).unwrap();
                Ok(format!("Routine [{}] set: {}, first on {}. It runs without the screen, and asks before changing anything.", r.id, r.schedule(), next.format("%a %-d %b %H:%M")))
            }
            other => bail!("unknown action `{other}`; use add, list, pause, resume or delete"),
        }
    }
}

fn schedule_after<Tz: TimeZone>(r: &Routine, now: &DateTime<Tz>) -> Option<i64> {
    let t = parse_time(&r.time)?;
    let date = r.date.as_deref().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
    // Nudged a minute on, so a routine started this minute isn't due again at once.
    next_due(t, r.days, date, &(now.clone() + Duration::seconds(1))).map(|d| d.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;
    use serde_json::json;

    fn at(d: u32, h: u32, m: u32) -> DateTime<FixedOffset> {
        // 5 October 2026 is a Monday.
        FixedOffset::east_opt(3600).unwrap().with_ymd_and_hms(2026, 10, d, h, m, 0).unwrap()
    }

    #[test]
    fn schedule_math() {
        let t = NaiveTime::from_hms_opt(8, 45, 0).unwrap();
        assert_eq!(next_due(t, WEEKDAYS, None, &at(5, 8, 0)), Some(at(5, 8, 45)));
        assert_eq!(next_due(t, WEEKDAYS, None, &at(5, 9, 0)), Some(at(6, 8, 45)));
        assert_eq!(next_due(t, WEEKDAYS, None, &at(9, 9, 0)), Some(at(12, 8, 45)), "Friday after 8:45 → Monday");
        assert_eq!(next_due(t, 1 << 4, None, &at(5, 9, 0)), Some(at(9, 8, 45)), "Fridays");
        let d = NaiveDate::from_ymd_opt(2026, 10, 7);
        assert_eq!(next_due(t, 0, d, &at(5, 9, 0)), Some(at(7, 8, 45)));
        assert_eq!(next_due(t, 0, d, &at(8, 9, 0)), None, "a one-off in the past");
    }

    #[test]
    fn days_and_times_read_naturally() {
        assert_eq!(parse_days(&json!(["mon", "Wednesday", "fri"])).unwrap(), 0b10101);
        assert_eq!(parse_days(&json!("weekdays")).unwrap(), WEEKDAYS);
        assert_eq!(parse_days(&json!("every day")).unwrap(), EVERY_DAY);
        assert_eq!(parse_days(&json!("sat, sun")).unwrap(), 0b1100000);
        assert_eq!(parse_days(&Value::Null).unwrap(), 0);
        assert!(parse_days(&json!(["someday"])).is_err());
        assert_eq!(parse_time("9am"), NaiveTime::from_hms_opt(9, 0, 0));
        assert_eq!(parse_time("16:30"), NaiveTime::from_hms_opt(16, 30, 0));
        assert_eq!(parse_time("teatime"), None);
    }

    #[test]
    fn runs_once_when_free_and_misses_what_came_due_while_off() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routines.json");
        let store = RoutineStore::new(&path);
        let msg = store.handle(&json!({ "action": "add", "goal": "Summarise my unread email", "time": "08:45", "days": "weekdays" }), at(5, 8, 0)).unwrap();
        assert!(msg.contains("weekdays at 08:45, first on Mon 5 Oct 08:45"), "{msg}");
        store.handle(&json!({ "goal": "Back up notes", "time": "09:00", "days": ["mon"] }), at(5, 8, 0)).unwrap();

        // Not yet due.
        assert!(store.take_due(&at(5, 8, 44), true).is_empty());
        // Due, but Waddle is busy: it waits.
        assert!(store.take_due(&at(5, 8, 46), false).is_empty());
        let due = store.take_due(&at(5, 8, 47), true);
        assert!(matches!(&due[..], [Due::Run(r)] if r.goal == "Summarise my unread email"));
        assert!(store.take_due(&at(5, 8, 48), true).is_empty(), "moved on to tomorrow");
        let reopened = RoutineStore::new(&path).list();
        let r = reopened.iter().find(|r| r.goal == "Summarise my unread email").unwrap();
        assert_eq!(r.next_ms, Some(at(6, 8, 45).timestamp_millis()));
        assert_eq!(r.last_run_ms, Some(at(5, 8, 47).timestamp_millis()));

        // The computer was off all Monday morning: the 9:00 backup is missed, not run at noon.
        let due = store.take_due(&at(5, 12, 0), true);
        assert!(matches!(&due[..], [Due::Missed(r)] if r.goal == "Back up notes"), "{due:?}");
        let backup = store.list().into_iter().find(|r| r.goal == "Back up notes").unwrap();
        assert_eq!(backup.next_ms, Some(at(12, 9, 0).timestamp_millis()));
        assert_eq!(backup.last_result.as_deref(), Some("Missed: Waddle wasn't running"));
        store.record(&backup.id, "Done");
        assert_eq!(RoutineStore::new(&path).list().iter().find(|r| r.id == backup.id).unwrap().last_result.as_deref(), Some("Done"));
    }

    #[test]
    fn one_offs_pausing_and_listing() {
        let store = RoutineStore::new(tempfile::tempdir().unwrap().path().join("r.json"));
        // "today at 18:00" as a one-off with no date.
        store.handle(&json!({ "goal": "Draft the weekly report", "time": "6pm" }), at(5, 14, 0)).unwrap();
        let r = store.list()[0].clone();
        assert_eq!((r.days, r.date.as_deref()), (0, Some("2026-10-05")));
        assert_eq!(r.schedule(), "once on Mon 5 Oct at 18:00");
        assert!(matches!(&store.take_due(&at(5, 18, 1), true)[..], [Due::Run(_)]));
        assert_eq!(store.list()[0].next_ms, None, "a one-off doesn't come back");

        store.handle(&json!({ "goal": "Water reminder", "time": "10:00", "days": "daily" }), at(5, 14, 0)).unwrap();
        let id = store.list().iter().find(|r| r.days == EVERY_DAY).unwrap().id.clone();
        assert!(store.handle(&json!({ "action": "pause", "id": id }), at(5, 14, 0)).unwrap().starts_with("Paused"));
        assert!(store.take_due(&at(6, 10, 1), true).is_empty(), "paused routines don't run");
        store.handle(&json!({ "action": "resume", "id": id }), at(7, 9, 0)).unwrap();
        assert_eq!(store.list().iter().find(|r| r.id == id).unwrap().next_ms, Some(at(7, 10, 0).timestamp_millis()), "resumed from now, nothing backfilled");

        let list = store.handle(&json!({ "action": "list" }), at(7, 9, 0)).unwrap();
        assert!(list.contains("every day at 10:00 (next Wed 7 Oct 10:00): Water reminder"), "{list}");
        assert!(store.handle(&json!({ "action": "delete", "id": id }), at(7, 9, 0)).unwrap().contains("Water reminder"));
        assert!(store.handle(&json!({ "goal": "x", "time": "08:00", "date": "2020-01-01" }), at(7, 9, 0)).is_err(), "in the past");
        assert!(store.handle(&json!({ "goal": " ", "time": "08:00" }), at(7, 9, 0)).is_err());
    }
}
