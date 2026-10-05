//! Reminders: "remind me in 20 minutes to stretch". Kept in a small JSON file
//! so they survive a restart; the app checks for due ones and pops them up.

use anyhow::{bail, Context};
use chrono::{DateTime, Duration, Local, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;

const MAX_REMINDERS: usize = 50;
const MAX_TEXT: usize = 200;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reminder {
    pub id: String,
    pub text: String,
    /// Unix milliseconds.
    pub due_ms: i64,
}

pub struct ReminderStore {
    path: PathBuf,
    items: Mutex<Vec<Reminder>>,
    /// Scheduled tasks, kept beside the reminders (`routines.json`).
    pub routines: crate::routines::RoutineStore,
}

impl ReminderStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let items = crate::store::load_json(&path);
        let routines = crate::routines::RoutineStore::new(path.with_file_name("routines.json"));
        Self { path, items: Mutex::new(items), routines }
    }

    fn save(&self, items: &[Reminder]) -> anyhow::Result<()> {
        crate::store::write_atomic(&self.path, serde_json::to_string_pretty(items)?).with_context(|| format!("saving {}", self.path.display()))
    }

    pub fn list(&self) -> Vec<Reminder> {
        let mut v = self.items.lock().unwrap().clone();
        v.sort_by_key(|r| r.due_ms);
        v
    }

    pub fn add(&self, text: &str, due_ms: i64) -> anyhow::Result<Reminder> {
        let text = text.trim();
        if text.is_empty() {
            bail!("a reminder needs `text`");
        }
        let mut items = self.items.lock().unwrap();
        if items.len() >= MAX_REMINDERS {
            bail!("too many reminders ({MAX_REMINDERS}); cancel some first");
        }
        let mut bytes = [0u8; 3];
        let _ = getrandom::fill(&mut bytes);
        let r = Reminder { id: hex::encode(bytes), text: text.chars().take(MAX_TEXT).collect(), due_ms };
        items.push(r.clone());
        self.save(&items)?;
        Ok(r)
    }

    pub fn cancel(&self, id: &str) -> anyhow::Result<Reminder> {
        let mut items = self.items.lock().unwrap();
        let i = items.iter().position(|r| r.id == id.trim()).ok_or_else(|| anyhow::anyhow!("no reminder with id `{id}`"))?;
        let r = items.remove(i);
        self.save(&items)?;
        Ok(r)
    }

    /// Removes and returns every reminder due by `now_ms`.
    pub fn take_due(&self, now_ms: i64) -> Vec<Reminder> {
        let mut items = self.items.lock().unwrap();
        let (due, rest): (Vec<_>, Vec<_>) = items.drain(..).partition(|r| r.due_ms <= now_ms);
        *items = rest;
        if !due.is_empty() {
            if let Err(e) = self.save(&items) {
                log::warn!("{e:#}");
            }
        }
        due
    }

    /// The `reminder` tool.
    pub fn handle<Tz: TimeZone>(&self, args: &Value, now: DateTime<Tz>) -> anyhow::Result<String>
    where
        Tz::Offset: std::fmt::Display,
    {
        let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
        match s("action").as_str() {
            "list" => {
                let list = self.list();
                if list.is_empty() {
                    return Ok("No reminders set.".into());
                }
                let tz = now.timezone();
                let lines: Vec<String> = list
                    .iter()
                    .map(|r| format!("[{}] {} – {}", r.id, describe(&tz.timestamp_millis_opt(r.due_ms).unwrap(), &now), r.text))
                    .collect();
                Ok(lines.join("\n"))
            }
            "cancel" => {
                let r = self.cancel(&s("id"))?;
                Ok(format!("Cancelled the reminder \"{}\".", r.text))
            }
            "add" | "" => {
                let due = due_time(args, &now)?;
                let r = self.add(&s("text"), due.timestamp_millis())?;
                Ok(format!("Reminder [{}] set for {}.", r.id, describe(&due, &now)))
            }
            other => bail!("unknown action `{other}`; use add, list or cancel"),
        }
    }
}

/// When a reminder is due: `in_minutes` from now, or the next `at` (HH:MM, 24h or am/pm).
fn due_time<Tz: TimeZone>(args: &Value, now: &DateTime<Tz>) -> anyhow::Result<DateTime<Tz>> {
    let minutes = match args.get("in_minutes") {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    };
    if let Some(m) = minutes.filter(|m| *m > 0.0) {
        anyhow::ensure!(m <= 60.0 * 24.0 * 30.0, "reminders can be up to 30 days away");
        return Ok(now.clone() + Duration::seconds((m * 60.0).round() as i64));
    }
    let at = args.get("at").and_then(Value::as_str).unwrap_or("").trim().to_lowercase();
    if at.is_empty() {
        bail!("give `in_minutes` or `at` (HH:MM)");
    }
    let t = parse_clock(&at).ok_or_else(|| anyhow::anyhow!("couldn't read the time `{at}`; use HH:MM"))?;
    let tz = now.timezone();
    let mut day = now.date_naive();
    for _ in 0..2 {
        if let Some(due) = tz.from_local_datetime(&day.and_time(t)).earliest() {
            if due > *now {
                return Ok(due);
            }
        }
        day = day.succ_opt().unwrap_or(day);
    }
    bail!("couldn't work out when `{at}` is")
}

fn parse_clock(s: &str) -> Option<NaiveTime> {
    let s = s.replace(' ', "");
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

fn describe<Tz: TimeZone>(due: &DateTime<Tz>, now: &DateTime<Tz>) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let mins = ((due.clone() - now.clone()).num_seconds().max(0) + 59) / 60;
    let when = if mins < 60 {
        format!("in {mins} min")
    } else if mins < 60 * 24 {
        format!("in {}h {:02}m", mins / 60, mins % 60)
    } else {
        format!("in {} days", mins / (60 * 24))
    };
    let day = if due.date_naive() == now.date_naive() { String::new() } else { format!("{} ", due.format("%a %-d %b")) };
    format!("{day}{} ({when})", due.format("%H:%M"))
}

/// The current local time for the task message, e.g. "Sunday 4 October 2026, 14:05".
pub fn now_line() -> String {
    Local::now().format("%A %-d %B %Y, %H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;
    use serde_json::json;

    fn now() -> DateTime<FixedOffset> {
        FixedOffset::east_opt(3600).unwrap().with_ymd_and_hms(2026, 10, 4, 14, 5, 0).unwrap()
    }

    #[test]
    fn adds_lists_takes_and_survives_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("reminders.json");
        let store = ReminderStore::new(&path);
        let msg = store.handle(&json!({"action": "add", "text": "stretch", "in_minutes": 20}), now()).unwrap();
        assert!(msg.contains("14:25 (in 20 min)"), "{msg}");
        let msg = store.handle(&json!({"action": "add", "text": "call mum", "at": "3pm"}), now()).unwrap();
        assert!(msg.contains("15:00 (in 55 min)"), "{msg}");
        let msg = store.handle(&json!({"text": "bins", "at": "08:30"}), now()).unwrap();
        assert!(msg.contains("Mon 5 Oct 08:30"), "times already past today mean tomorrow: {msg}");

        let list = store.handle(&json!({"action": "list"}), now()).unwrap();
        assert!(list.find("stretch").unwrap() < list.find("call mum").unwrap(), "{list}");

        let reopened = ReminderStore::new(&path);
        assert_eq!(reopened.list().len(), 3);
        let due = reopened.take_due((now() + Duration::minutes(21)).timestamp_millis());
        assert_eq!(due.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["stretch"]);
        assert_eq!(ReminderStore::new(&path).list().len(), 2, "taken reminders are gone for good");

        let id = reopened.list()[0].id.clone();
        assert!(reopened.handle(&json!({"action": "cancel", "id": id}), now()).unwrap().contains("call mum"));
        assert!(reopened.handle(&json!({"action": "cancel", "id": "nope"}), now()).is_err());
    }

    #[test]
    fn rejects_bad_times() {
        let store = ReminderStore::new(tempfile::tempdir().unwrap().path().join("r.json"));
        assert!(store.handle(&json!({"action": "add", "text": "x"}), now()).is_err());
        assert!(store.handle(&json!({"action": "add", "text": "x", "at": "teatime"}), now()).is_err());
        assert!(store.handle(&json!({"action": "add", "text": " ", "in_minutes": 5}), now()).is_err());
        assert_eq!(parse_clock("12am"), NaiveTime::from_hms_opt(0, 0, 0));
        assert_eq!(parse_clock("7.30 pm"), NaiveTime::from_hms_opt(19, 30, 0));
        assert_eq!(parse_clock("13pm"), None);
    }
}
