//! Runs terminal commands in the workspace folder with a timeout, an output
//! cap and line-by-line streaming. This confines the working directory; it is
//! not an OS security sandbox. The tier system is what gates risky commands.

use anyhow::Context;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio_util::sync::CancellationToken;

const MAX_OUTPUT: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct CommandResult {
    pub exit_code: Option<i32>,
    pub output: String,
    pub timed_out: bool,
    pub cancelled: bool,
    pub truncated: bool,
}

impl CommandResult {
    pub fn describe(&self) -> String {
        let status = if self.cancelled {
            "cancelled by the user".to_string()
        } else if self.timed_out {
            "stopped: timed out".to_string()
        } else {
            match self.exit_code {
                Some(c) => format!("exit code {c}"),
                None => "terminated".to_string(),
            }
        };
        let mut s = format!("Command finished ({status}).");
        if self.truncated {
            s.push_str(&format!(" Output was cut at {} KB.", MAX_OUTPUT / 1024));
        }
        s
    }
}

fn build(command: &str) -> Command {
    #[cfg(windows)]
    {
        let mut c = Command::new("powershell.exe");
        // UTF-8 output so non-ASCII text survives; CREATE_NO_WINDOW keeps consoles from flashing.
        let script = format!("[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; {command}");
        c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &script]);
        c.creation_flags(0x0800_0000);
        c
    }
    #[cfg(not(windows))]
    {
        let mut c = Command::new("sh");
        c.args(["-c", command]);
        c.process_group(0);
        c
    }
}

/// Kills the command and everything it spawned.
async fn kill_tree(child: &mut Child) {
    if let Some(pid) = child.id() {
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        #[cfg(not(windows))]
        {
            let _ = std::process::Command::new("kill")
                .args(["-9", "--", &format!("-{pid}")])
                .stderr(Stdio::null())
                .status();
        }
    }
    let _ = child.kill().await;
}

pub async fn run_command(
    command: &str,
    cwd: &Path,
    timeout: Duration,
    cancel: &CancellationToken,
    on_line: &mut (dyn FnMut(&str) + Send),
) -> anyhow::Result<CommandResult> {
    let mut child = build(command)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("could not start the shell")?;
    let mut out = BufReader::new(child.stdout.take().unwrap()).split(b'\n');
    let mut err = BufReader::new(child.stderr.take().unwrap()).split(b'\n');
    let (mut out_done, mut err_done) = (false, false);
    let mut output = String::new();
    let mut truncated = false;
    let mut timed_out = false;
    let mut cancelled = false;
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);

    let mut take = |seg: Vec<u8>, output: &mut String, truncated: &mut bool| {
        let line = String::from_utf8_lossy(&seg);
        let line = line.trim_end_matches('\r');
        on_line(line);
        if output.len() + line.len() < MAX_OUTPUT {
            output.push_str(line);
            output.push('\n');
        } else {
            *truncated = true;
        }
    };

    while !(out_done && err_done) {
        tokio::select! {
            _ = cancel.cancelled() => { cancelled = true; break; }
            _ = &mut deadline => { timed_out = true; break; }
            seg = out.next_segment(), if !out_done => match seg {
                Ok(Some(s)) => take(s, &mut output, &mut truncated),
                _ => out_done = true,
            },
            seg = err.next_segment(), if !err_done => match seg {
                Ok(Some(s)) => take(s, &mut output, &mut truncated),
                _ => err_done = true,
            },
        }
    }

    let exit_code = if cancelled || timed_out {
        kill_tree(&mut child).await;
        None
    } else {
        match tokio::time::timeout(Duration::from_secs(5), child.wait()).await {
            Ok(Ok(status)) => status.code(),
            _ => {
                kill_tree(&mut child).await;
                None
            }
        }
    };
    Ok(CommandResult { exit_code, output, timed_out, cancelled, truncated })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::Instant;

    async fn run(cmd: &str, timeout_ms: u64, cancel: &CancellationToken) -> (CommandResult, Vec<String>) {
        let dir = std::env::temp_dir();
        let mut lines = vec![];
        let r = run_command(cmd, &dir, Duration::from_millis(timeout_ms), cancel, &mut |l| lines.push(l.to_string()))
            .await
            .unwrap();
        (r, lines)
    }

    #[tokio::test]
    async fn captures_stdout_stderr_and_exit_code() {
        let (r, lines) = run("echo one; echo two 1>&2; exit 3", 5000, &CancellationToken::new()).await;
        assert_eq!(r.exit_code, Some(3));
        assert!(lines.contains(&"one".to_string()) && lines.contains(&"two".to_string()));
        assert!(r.output.contains("one") && r.output.contains("two"));
    }

    #[tokio::test]
    async fn timeout_kills_the_process_tree() {
        let start = Instant::now();
        let (r, _) = run("sleep 30 & sleep 30; echo never", 300, &CancellationToken::new()).await;
        assert!(r.timed_out);
        assert!(!r.output.contains("never"));
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn cancellation_stops_immediately() {
        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            c2.cancel();
        });
        let start = Instant::now();
        let (r, _) = run("sleep 30", 60_000, &cancel).await;
        assert!(r.cancelled);
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn output_is_capped() {
        let (r, _) = run("yes waddle | head -c 200000", 10_000, &CancellationToken::new()).await;
        assert!(r.truncated);
        assert!(r.output.len() <= MAX_OUTPUT);
    }
}
