//! What Waddle spends on model calls: a daily ledger by purpose, a monthly
//! budget, and `Metered`, which records every paid call and pauses them once
//! the month's budget is used up.
//!
//! Callers say what a call is for with `scoped` (chat, research, a brief…);
//! anything unscoped counts as part of a task.

use async_trait::async_trait;
use chrono::{DateTime, Datelike, Local, TimeZone};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::llm::{ChatRequest, ChatResponse, EventSink, Provider, Usage};

/// What a model call was for, as shown in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Task,
    Chat,
    Research,
    Quick,
    Brief,
    Decision,
    Check,
}

impl Purpose {
    pub fn key(self) -> &'static str {
        match self {
            Purpose::Task => "tasks",
            Purpose::Chat => "chat",
            Purpose::Research => "research",
            Purpose::Quick => "quick replies",
            Purpose::Brief => "briefs",
            Purpose::Decision => "decisions",
            Purpose::Check => "self-tests",
        }
    }
}

tokio::task_local! {
    static PURPOSE: Purpose;
}

/// Runs `f` with its model calls counted under `purpose`.
pub async fn scoped<F: Future>(purpose: Purpose, f: F) -> F::Output {
    PURPOSE.scope(purpose, f).await
}

fn current() -> Purpose {
    PURPOSE.try_with(|p| *p).unwrap_or(Purpose::Task)
}

/// Calls, cost and tokens for one purpose on one day.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Line {
    pub calls: u64,
    /// US dollars, as the server reported (or estimated for decisions).
    pub cost: f64,
    pub tokens_in: u64,
    pub tokens_out: u64,
}

impl Line {
    fn add(&mut self, other: &Line) {
        self.calls += other.calls;
        self.cost += other.cost;
        self.tokens_in += other.tokens_in;
        self.tokens_out += other.tokens_out;
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Book {
    /// "YYYY-MM-DD" → purpose → line.
    days: BTreeMap<String, BTreeMap<String, Line>>,
    /// The month ("YYYY-MM") the 80% warning was last given for.
    warned_month: Option<String>,
}

/// One day's total, for the 30-day chart.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DayTotal {
    pub day: String,
    pub cost: f64,
}

/// What Settings shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    pub today: f64,
    pub month: f64,
    /// 0 means no limit.
    pub budget: f64,
    pub paused: bool,
    /// This month, by purpose.
    pub by_purpose: BTreeMap<String, Line>,
    /// The last 30 days, oldest first, including days with no spending.
    pub days: Vec<DayTotal>,
}

/// Days of history kept.
const KEEP_DAYS: usize = 400;
/// The warning comes once a month, at this share of the budget.
const WARN_AT: f64 = 0.8;

type Notify = Arc<dyn Fn(String) + Send + Sync>;

pub struct Ledger {
    path: Option<PathBuf>,
    book: Mutex<Book>,
    budget: Mutex<f64>,
    notify: Mutex<Option<Notify>>,
}

fn day_key<Tz: TimeZone>(t: &DateTime<Tz>) -> String {
    format!("{:04}-{:02}-{:02}", t.year(), t.month(), t.day())
}

fn month_key<Tz: TimeZone>(t: &DateTime<Tz>) -> String {
    format!("{:04}-{:02}", t.year(), t.month())
}

fn money(x: f64) -> String {
    match x {
        x if x >= 1.0 => format!("${x:.2}"),
        x if x >= 0.01 => format!("${x:.3}"),
        x if x >= 0.0001 => format!("${x:.4}"),
        x if x > 0.0 => "<$0.0001".to_string(),
        _ => "$0.00".to_string(),
    }
}

impl Ledger {
    /// A ledger kept in `path` (JSON), or only in memory with `None`.
    pub fn new(path: Option<PathBuf>) -> Self {
        let book = path.as_deref().map(crate::store::load_json).unwrap_or_default();
        Self { path, book: Mutex::new(book), budget: Mutex::new(0.0), notify: Mutex::default() }
    }

    /// The monthly budget in dollars; 0 means no limit.
    pub fn set_budget(&self, dollars: f64) {
        *self.budget.lock().unwrap() = if dollars.is_finite() { dollars.max(0.0) } else { 0.0 };
    }

    /// Where the 80% warning goes (the app shows it in the bubble).
    pub fn on_notice(&self, f: impl Fn(String) + Send + Sync + 'static) {
        *self.notify.lock().unwrap() = Some(Arc::new(f));
    }

    fn month_total_in(book: &Book, month: &str) -> f64 {
        book.days.iter().filter(|(d, _)| d.starts_with(month)).flat_map(|(_, p)| p.values()).map(|l| l.cost).sum()
    }

    /// Records one call. Warns once a month when spending passes 80% of the budget.
    pub fn record<Tz: TimeZone>(&self, purpose: Purpose, usage: Usage, now: &DateTime<Tz>) {
        let line = Line { calls: 1, cost: usage.cost.unwrap_or(0.0), tokens_in: usage.prompt_tokens, tokens_out: usage.completion_tokens };
        let budget = *self.budget.lock().unwrap();
        let warning = {
            let mut book = self.book.lock().unwrap();
            book.days.entry(day_key(now)).or_default().entry(purpose.key().to_string()).or_default().add(&line);
            while book.days.len() > KEEP_DAYS {
                let oldest = book.days.keys().next().cloned();
                if let Some(k) = oldest {
                    book.days.remove(&k);
                }
            }
            let month = month_key(now);
            let total = Self::month_total_in(&book, &month);
            let warn = budget > 0.0 && total >= budget * WARN_AT && total < budget && book.warned_month.as_deref() != Some(month.as_str());
            if warn {
                book.warned_month = Some(month);
            }
            if let Some(path) = &self.path {
                if let Ok(text) = serde_json::to_string(&*book) {
                    if let Err(e) = crate::store::write_atomic(path, text) {
                        log::warn!("saving the spending ledger: {e}");
                    }
                }
            }
            warn.then(|| format!("Heads up: I've used {} of this month's {} budget. You can change it in Settings → Brain.", money(total), money(budget)))
        };
        if let Some(w) = warning {
            let notify = self.notify.lock().unwrap().clone();
            if let Some(n) = notify {
                n(w);
            }
        }
    }

    /// Why paid calls are paused, if this month's spending has reached the budget.
    pub fn paused<Tz: TimeZone>(&self, now: &DateTime<Tz>) -> Option<String> {
        let budget = *self.budget.lock().unwrap();
        if budget <= 0.0 {
            return None;
        }
        let total = Self::month_total_in(&self.book.lock().unwrap(), &month_key(now));
        (total >= budget).then(|| {
            format!(
                "I've reached this month's {} budget, so paid model calls are paused until the 1st. You can raise it in Settings → Brain.",
                money(budget)
            )
        })
    }

    pub fn summary<Tz: TimeZone>(&self, now: &DateTime<Tz>) -> Summary {
        let book = self.book.lock().unwrap();
        let today_key = day_key(now);
        let month = month_key(now);
        let sum = |day: &str| book.days.get(day).map(|p| p.values().map(|l| l.cost).sum()).unwrap_or(0.0);
        let mut by_purpose: BTreeMap<String, Line> = BTreeMap::new();
        for (_, purposes) in book.days.iter().filter(|(d, _)| d.starts_with(&month)) {
            for (k, l) in purposes {
                by_purpose.entry(k.clone()).or_default().add(l);
            }
        }
        let days = (0..30)
            .rev()
            .map(|back| {
                let d = day_key(&(now.clone() - chrono::Duration::days(back)));
                DayTotal { cost: sum(&d), day: d }
            })
            .collect();
        let budget = *self.budget.lock().unwrap();
        let month_total = Self::month_total_in(&book, &month);
        Summary { today: sum(&today_key), month: month_total, budget, paused: budget > 0.0 && month_total >= budget, by_purpose, days }
    }
}

/// A hosted provider whose calls are recorded in the ledger and paused over budget.
pub struct Metered {
    inner: Arc<dyn Provider>,
    ledger: Arc<Ledger>,
}

impl Metered {
    pub fn new(inner: Arc<dyn Provider>, ledger: Arc<Ledger>) -> Self {
        Self { inner, ledger }
    }
}

#[async_trait]
impl Provider for Metered {
    async fn chat(&self, req: ChatRequest<'_>, on_event: EventSink<'_>) -> anyhow::Result<ChatResponse> {
        if let Some(why) = self.ledger.paused(&Local::now()) {
            anyhow::bail!(why);
        }
        let resp = self.inner.chat(req, on_event).await?;
        self.ledger.record(current(), resp.usage, &Local::now());
        Ok(resp)
    }

    async fn warm(&self, req: ChatRequest<'_>) {
        self.inner.warm(req).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::mock::{reply, MockProvider};
    use crate::llm::Message;
    use chrono::FixedOffset;

    fn at(y: i32, m: u32, d: u32) -> DateTime<FixedOffset> {
        FixedOffset::east_opt(0).unwrap().with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap()
    }

    fn cost(c: f64) -> Usage {
        Usage { prompt_tokens: 100, completion_tokens: 10, cost: Some(c) }
    }

    #[test]
    fn totals_by_day_month_and_purpose_and_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spending.json");
        let l = Ledger::new(Some(path.clone()));
        l.record(Purpose::Task, cost(0.002), &at(2026, 9, 30));
        l.record(Purpose::Task, cost(0.001), &at(2026, 10, 3));
        l.record(Purpose::Chat, cost(0.0005), &at(2026, 10, 4));
        l.record(Purpose::Decision, Usage { cost: None, ..Usage::default() }, &at(2026, 10, 4));
        let s = Ledger::new(Some(path)).summary(&at(2026, 10, 4));
        assert!((s.today - 0.0005).abs() < 1e-9);
        assert!((s.month - 0.0015).abs() < 1e-9, "September doesn't count: {}", s.month);
        assert_eq!(s.by_purpose["tasks"].calls, 1);
        assert_eq!(s.by_purpose["decisions"].calls, 1);
        assert_eq!(s.days.len(), 30);
        assert_eq!(s.days.last().unwrap().day, "2026-10-04");
        assert!((s.days[25].cost - 0.002).abs() < 1e-9, "{:?}", s.days[25]);
    }

    #[test]
    fn warns_once_at_80_percent_and_pauses_at_the_budget() {
        let l = Ledger::new(None);
        l.set_budget(1.0);
        let said = Arc::new(Mutex::new(vec![]));
        let s2 = said.clone();
        l.on_notice(move |m| s2.lock().unwrap().push(m));
        let now = at(2026, 10, 4);
        l.record(Purpose::Task, cost(0.5), &now);
        assert!(said.lock().unwrap().is_empty());
        l.record(Purpose::Task, cost(0.35), &now);
        l.record(Purpose::Task, cost(0.01), &now);
        assert_eq!(said.lock().unwrap().len(), 1, "{:?}", said.lock().unwrap());
        assert!(said.lock().unwrap()[0].contains("$0.850 of this month's $1.00"));
        assert!(l.paused(&now).is_none());
        l.record(Purpose::Task, cost(0.2), &now);
        assert!(l.paused(&now).unwrap().contains("$1.00 budget"));
        assert!(l.paused(&at(2026, 11, 1)).is_none(), "a new month starts fresh");
        l.set_budget(0.0);
        assert!(l.paused(&now).is_none(), "0 means no limit");
    }

    #[tokio::test]
    async fn metered_calls_are_labelled_and_stop_over_budget() {
        let ledger = Arc::new(Ledger::new(None));
        ledger.set_budget(0.01);
        let mut r = reply("hi", vec![]);
        r.usage = cost(0.006);
        let inner = Arc::new(MockProvider::scripted(vec![r.clone(), r.clone(), r]));
        let p = Metered::new(inner.clone(), ledger.clone());
        let msgs = [Message::user("hi")];
        let req = ChatRequest { model: "m", messages: &msgs, tools: &[], temperature: 0.2, max_tokens: 10, web: None };
        p.chat(req, &mut |_| {}).await.unwrap();
        scoped(Purpose::Chat, p.chat(req, &mut |_| {})).await.unwrap();
        let err = p.chat(req, &mut |_| {}).await.unwrap_err().to_string();
        assert!(err.contains("budget"), "{err}");
        assert_eq!(inner.request_count(), 2, "the paused call never reached the model");
        let s = ledger.summary(&Local::now());
        assert_eq!((s.by_purpose["tasks"].calls, s.by_purpose["chat"].calls), (1, 1));
        assert!(s.paused);
    }
}
