//! Self-test results and the report file a user can send when something is
//! wrong on their machine. The desktop app runs the OS checks; the model probe
//! and the report text live here so they are testable.

use serde::Serialize;
use serde_json::json;
use std::future::Future;
use std::time::Instant;

use crate::audit::AuditRecord;
use crate::llm::{ChatRequest, Message, Provider, StreamEvent, ToolSpec};
use crate::Settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pass,
    Warn,
    Fail,
    Skip,
}

impl Status {
    fn mark(self) -> &'static str {
        match self {
            Status::Pass => "PASS",
            Status::Warn => "WARN",
            Status::Fail => "FAIL",
            Status::Skip => "skip",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub name: String,
    pub status: Status,
    pub detail: String,
    pub millis: u64,
}

/// Times one check; an error becomes a failed check with the error as its detail.
pub async fn run_check<F, Fut>(name: &str, check: F) -> Check
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = anyhow::Result<(Status, String)>>,
{
    let started = Instant::now();
    let (status, detail) = check().await.unwrap_or_else(|e| (Status::Fail, format!("{e:#}")));
    Check { name: name.to_string(), status, detail, millis: started.elapsed().as_millis() as u64 }
}

/// The models an Ollama server has pulled, or an error if it isn't running.
pub async fn ollama_models(base_url: &str) -> anyhow::Result<Vec<String>> {
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let resp = reqwest::Client::builder().timeout(std::time::Duration::from_secs(3)).build()?.get(&url).send().await?;
    anyhow::ensure!(resp.status().is_success(), "Ollama answered {}", resp.status());
    let v: serde_json::Value = resp.json().await?;
    Ok(v["models"].as_array().into_iter().flatten().filter_map(|m| m["name"].as_str().map(str::to_string)).collect())
}

/// The model to suggest from what Ollama has: Waddle's tested default if pulled, else the first.
pub fn pick_ollama_model(models: &[String]) -> Option<String> {
    const PREFERRED: [&str; 3] = ["qwen3.5:4b", "qwen3.5:9b", "qwen3:4b"];
    PREFERRED.iter().find(|p| models.iter().any(|m| m == *p)).map(|p| p.to_string()).or_else(|| models.first().cloned())
}

/// One small model call with one tool: does the model answer, and can it call tools?
pub async fn probe_model(provider: &dyn Provider, model: &str) -> anyhow::Result<(Status, String)> {
    let tools = [ToolSpec {
        name: "ping".into(),
        description: "Reports that the assistant is ready.".into(),
        parameters: json!({ "type": "object", "properties": { "word": { "type": "string" } }, "required": ["word"] }),
    }];
    let messages = [
        Message::system("You are being tested. Do exactly what the user asks."),
        Message::user("Call the ping tool with the word \"ready\"."),
    ];
    let req = ChatRequest { model, messages: &messages, tools: &tools, temperature: 0.0, max_tokens: 64, web: None };
    let started = Instant::now();
    let mut first_token = None;
    let mut on_event = |_: StreamEvent| {
        first_token.get_or_insert_with(|| started.elapsed());
    };
    let resp = crate::ledger::scoped(crate::ledger::Purpose::Check, provider.chat(req, &mut on_event)).await?;
    let secs = started.elapsed().as_secs_f64();
    if resp.tool_calls.iter().any(|c| c.name == "ping") {
        Ok((Status::Pass, format!("{model} answered in {secs:.1}s and called a tool")))
    } else {
        let said: String = resp.text.chars().take(80).collect();
        Ok((Status::Warn, format!("{model} answered in {secs:.1}s but didn't call the tool, so actions won't work (it said: {said:?})")))
    }
}

/// Seconds since the Unix epoch as "YYYY-MM-DD HH:MM:SS UTC".
pub fn format_utc(ts_ms: i64) -> String {
    let secs = ts_ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC", rem / 3600, rem % 3600 / 60, rem % 60)
}

pub struct ReportInput<'a> {
    pub app_version: &'a str,
    pub os: &'a str,
    pub generated_ms: i64,
    pub settings: &'a Settings,
    pub has_api_key: bool,
    pub demo: bool,
    pub checks: &'a [Check],
    pub audit: &'a [AuditRecord],
    pub log_tail: &'a str,
}

/// Only the scheme and host of an endpoint: paths and query strings can carry tokens.
fn endpoint_host(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or(host);
    if scheme.is_empty() { host.to_string() } else { format!("{scheme}://{host}") }
}

/// What an activity-log row may show in a report. The safety reason for a tool
/// call is Waddle's own wording and errors help diagnose; the user's messages,
/// Waddle's answers and tool output (file contents, command output) are left out.
fn safe_detail(kind: &str, detail: Option<&str>) -> String {
    let d = detail.unwrap_or("");
    let shown = match kind {
        "tool" | "halt" => d.to_string(),
        "tool_result" if d.starts_with("Error") => d.to_string(),
        "tool_result" | "task_end" => String::new(),
        _ if d.is_empty() => String::new(),
        _ => format!("({} characters, not included)", d.chars().count()),
    };
    shown.chars().take(80).collect()
}

/// The text a user sends when asking for help. Holds no keys, file contents,
/// command arguments, window titles or what the user typed.
pub fn render_report(r: &ReportInput<'_>) -> String {
    let count = |s: Status| r.checks.iter().filter(|c| c.status == s).count();
    let s = r.settings;
    let mut out = format!(
        "Project Waddle diagnostic report\nVersion {} on {}\nGenerated {}\n\nSummary: {} passed, {} warnings, {} failed, {} skipped\n",
        r.app_version,
        r.os,
        format_utc(r.generated_ms),
        count(Status::Pass),
        count(Status::Warn),
        count(Status::Fail),
        count(Status::Skip),
    );

    out.push_str("\n== Self-test\n");
    for c in r.checks {
        out.push_str(&format!("[{}] {:<24} {} ({} ms)\n", c.status.mark(), c.name, c.detail, c.millis));
    }

    out.push_str("\n== Settings (no keys)\n");
    let provider = format!("{:?}", s.provider);
    let lines = [
        ("Brain", if r.demo { format!("demo mode ({provider})") } else { provider }),
        ("Endpoint", endpoint_host(&s.base_url)),
        ("Planner model", s.model.clone()),
        ("Quick-reply model", s.fast_model().to_string()),
        ("API key saved", if r.has_api_key { "yes" } else { "no" }.to_string()),
        ("Coordinates", format!("{:?} (resolved {:?})", s.coord_mode, s.coord_mode())),
        ("Tier 2", format!("{:?}, {} ms countdown", s.tier2_mode, s.tier2_countdown_ms)),
        ("Steps / timeouts", format!("{} steps, task {} s, command {} s", s.max_steps, s.task_timeout_secs, s.command_timeout_secs)),
        (
            "Local model caps",
            format!(
                "threads {}, keep loaded {}, context {}",
                s.ollama.num_thread.map(|n| n.to_string()).unwrap_or_else(|| "auto".into()),
                s.ollama.keep_alive,
                s.ollama.num_ctx
            ),
        ),
        ("Voice", format!("{:?}", s.voice.backend)),
        ("Self-editing", if s.self_source_dir.is_some() { "on" } else { "off" }.to_string()),
    ];
    for (k, v) in lines {
        out.push_str(&format!("{k:<18} {v}\n"));
    }

    out.push_str(&format!("\n== Recent activity (last {}, oldest first)\n", r.audit.len()));
    for a in r.audit.iter().rev() {
        let detail = safe_detail(&a.kind, a.detail.as_deref());
        out.push_str(&format!(
            "{}  {:<12} {:<14} tier {:<2} {:<10} {}\n",
            format_utc(a.ts_ms),
            a.kind,
            a.tool.as_deref().unwrap_or("-"),
            a.tier.map(|t| t.to_string()).unwrap_or_else(|| "-".into()),
            a.decision.as_deref().unwrap_or("-"),
            detail
        ));
    }

    out.push_str("\n== Log (most recent lines)\n");
    out.push_str(if r.log_tail.trim().is_empty() { "(empty)\n" } else { r.log_tail });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::mock::{call, reply, MockProvider};

    #[test]
    fn picks_the_tested_ollama_model() {
        let m = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(pick_ollama_model(&m(&["llama3:8b", "qwen3.5:4b"])).as_deref(), Some("qwen3.5:4b"));
        assert_eq!(pick_ollama_model(&m(&["llama3:8b"])).as_deref(), Some("llama3:8b"));
        assert_eq!(pick_ollama_model(&[]), None);
    }

    #[tokio::test]
    async fn probe_reports_whether_tools_work() {
        let good = MockProvider::scripted(vec![reply("Pinging.", vec![call("ping", json!({"word": "ready"}))])]);
        let (status, detail) = probe_model(&good, "m").await.unwrap();
        assert_eq!(status, Status::Pass, "{detail}");
        let chatty = MockProvider::scripted(vec![reply("Ready!", vec![])]);
        let (status, detail) = probe_model(&chatty, "m").await.unwrap();
        assert_eq!(status, Status::Warn);
        assert!(detail.contains("didn't call the tool"), "{detail}");
    }

    #[tokio::test]
    async fn failing_checks_carry_the_error() {
        let c = run_check("Broken", || async { anyhow::bail!("no monitor found") }).await;
        assert_eq!((c.status, c.detail.as_str()), (Status::Fail, "no monitor found"));
    }

    #[test]
    fn dates_format_in_utc() {
        assert_eq!(format_utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(format_utc(1_791_082_800_000), "2026-10-04 03:00:00 UTC");
        assert_eq!(format_utc(951_782_400_000), "2000-02-29 00:00:00 UTC");
    }

    #[test]
    fn report_leaves_out_secrets() {
        let settings = Settings { base_url: "https://user:hunter2@openrouter.ai/api/v1?key=sk-secret".into(), ..Settings::default() };
        let row = |kind: &str, detail: &str| AuditRecord { kind: kind.into(), detail: Some(detail.into()), ..Default::default() };
        let mut audit = vec![
            row("task_start", "email my landlord about the leak"),
            row("tool_result", "Dear diary, today"),
            row("tool_result", "Error: no monitor found"),
            row("task_end", "I emailed your landlord."),
        ];
        audit.push(AuditRecord {
            kind: "tool".into(),
            tool: Some("write_file".into()),
            args: Some(r#"{"content":"my diary"}"#.into()),
            tier: Some(2),
            decision: Some("approved".into()),
            detail: Some("creates a new file".into()),
            ..Default::default()
        });
        let checks = [Check { name: "Screenshot".into(), status: Status::Pass, detail: "1440x960".into(), millis: 80 }];
        let text = render_report(&ReportInput {
            app_version: "0.1.4",
            os: "windows aarch64",
            generated_ms: 0,
            settings: &settings,
            has_api_key: true,
            demo: false,
            checks: &checks,
            audit: &audit,
            log_tail: "WARN something\n",
        });
        assert!(text.contains("Summary: 1 passed, 0 warnings, 0 failed, 0 skipped"));
        assert!(text.contains("[PASS] Screenshot"));
        assert!(text.contains("https://openrouter.ai\n"), "{text}");
        assert!(text.contains("write_file") && text.contains("creates a new file"));
        assert!(text.contains("Error: no monitor found") && text.contains("(32 characters, not included)"), "{text}");
        for secret in ["hunter2", "sk-secret", "my diary", "landlord", "Dear diary"] {
            assert!(!text.contains(secret), "report leaked {secret}");
        }
    }
}
