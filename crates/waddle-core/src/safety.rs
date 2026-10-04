//! Deterministic permission tiers. No model output can lower a tier or grant an
//! approval: approvals only arrive from a click in Waddle's own UI.
//!
//! Tier 0: passive perception (screen reading). Runs silently.
//! Tier 1: navigation and read-only access. Runs, logged.
//! Tier 2: scoped changes (new files, clicks, typing, read-only commands). Shown with a cancel window.
//! Tier 3: anything destructive, unknown or outside the workspace. Blocks until the user approves.

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;

use crate::llm::ToolCall;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Tier {
    Passive = 0,
    NonDestructive = 1,
    ScopedMutation = 2,
    Destructive = 3,
}

impl Tier {
    pub fn number(self) -> u8 {
        self as u8
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assessment {
    pub tier: Tier,
    pub reason: String,
}

fn assess(tier: Tier, reason: impl Into<String>) -> Assessment {
    Assessment { tier, reason: reason.into() }
}

/// Facts about the environment the classifier needs.
pub trait SafetyContext {
    /// Whether a workspace-relative path already exists.
    fn file_exists(&self, path: &str) -> bool;
}

pub fn classify(call: &ToolCall, ctx: &dyn SafetyContext) -> Assessment {
    let arg = |k: &str| call.arguments.get(k).and_then(Value::as_str).unwrap_or("");
    match call.name.as_str() {
        "list_windows" | "look_at_screen" | "find_elements" => assess(Tier::Passive, "reads the screen"),
        "point_at" => assess(Tier::Passive, "points at the screen without touching it"),
        "scroll" => assess(Tier::NonDestructive, "scrolls"),
        "read_clipboard" => assess(Tier::NonDestructive, "reads the clipboard"),
        "reminder" => assess(Tier::NonDestructive, "manages reminders"),
        "remember" | "forget" => assess(Tier::NonDestructive, "updates the facts Waddle keeps about you (listed in Settings)"),
        "replace_selection" => assess(Tier::ScopedMutation, "replaces the text you selected"),
        "copy_to_clipboard" => assess(Tier::ScopedMutation, "replaces what's on the clipboard"),
        "drag" => assess(Tier::ScopedMutation, "drags in another application"),
        "open_app" => assess(Tier::NonDestructive, "opens an application"),
        "read_file" | "list_dir" => assess(Tier::NonDestructive, "reads the workspace"),
        "save_skill" | "forget_skill" => assess(Tier::Destructive, "changes Waddle's long-term memory (loaded into every task)"),
        "update_settings" => assess(Tier::Destructive, "changes Waddle's own settings"),
        "delegate" => assess(Tier::NonDestructive, "starts a sub-task; each of its actions is gated on its own"),
        "write_file" if is_self_path(arg("path")) => assess(Tier::Destructive, "edits Waddle's own source code"),
        "write_file" => {
            if ctx.file_exists(arg("path")) {
                assess(Tier::Destructive, "overwrites an existing file")
            } else {
                assess(Tier::ScopedMutation, "creates a new file in the workspace")
            }
        }
        "click" | "click_element" => assess(Tier::ScopedMutation, "clicks in another application"),
        "type_text" => assess(Tier::ScopedMutation, "types into another application"),
        "press_keys" => classify_keys(arg("keys")),
        "run_command" => classify_command(arg("command")),
        other => assess(Tier::Destructive, format!("unknown tool `{other}`")),
    }
}

/// Paths under `self/` refer to Waddle's own source folder.
pub fn is_self_path(path: &str) -> bool {
    let p = path.trim_start_matches("./");
    p == "self" || p.starts_with("self/") || p.starts_with("self\\")
}

fn classify_keys(keys: &str) -> Assessment {
    let norm: String = keys.to_ascii_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    const DANGEROUS: &[&str] = &[
        "alt+f4", "ctrl+w", "ctrl+shift+w", "ctrl+q", "cmd+q", "cmd+w", "meta+q", "meta+w",
        "delete", "shift+delete", "ctrl+shift+delete", "ctrl+alt+delete", "win+r", "meta+r",
        "win+l", "meta+l", "ctrl+shift+esc",
    ];
    if DANGEROUS.contains(&norm.as_str()) {
        assess(Tier::Destructive, format!("`{keys}` can close windows or delete data"))
    } else {
        assess(Tier::ScopedMutation, "presses keys in another application")
    }
}

fn shell_control() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"[;&|<>`$(){}\r\n]").unwrap())
}

fn outside_workspace() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Absolute paths, drive letters, home shortcuts, parent traversal, UNC paths, env-var paths.
    RE.get_or_init(|| Regex::new(r#"(^|[\s"'=])(/|~|\.\.|[A-Za-z]:|\\\\|%)"#).unwrap())
}

/// Programs whose plain invocation only reads state.
const READ_ONLY: &[&str] = &[
    "ls", "dir", "gci", "get-childitem", "cat", "type", "gc", "get-content", "pwd", "get-location",
    "echo", "write-output", "whoami", "hostname", "date", "get-date", "where", "which", "get-command",
    "tree", "head", "tail", "wc", "findstr", "select-string", "sls", "get-process", "ps", "uname",
    "ver", "get-item", "gi", "test-path", "measure-object", "get-filehash", "file", "stat",
];

const GIT_READ_ONLY: &[&str] = &["status", "log", "diff", "show", "branch", "rev-parse", "ls-files"];

/// Words that make a command dangerous even when the program is otherwise harmless.
const DENY_WORDS: &[&str] = &[
    "rm", "del", "erase", "rmdir", "rd", "remove-item", "ri", "format", "mkfs", "dd", "shutdown",
    "restart-computer", "stop-computer", "reg", "regedit", "curl", "wget", "invoke-webrequest", "iwr",
    "invoke-restmethod", "irm", "ssh", "scp", "sftp", "ftp", "nc", "netcat", "sudo", "runas", "chmod",
    "chown", "kill", "taskkill", "stop-process", "env", "env:", "printenv", "set",
    "--delete", "-delete", "--force", "-force", "push", "reset", "clean",
];

pub fn classify_command(command: &str) -> Assessment {
    let cmd = command.trim();
    if cmd.is_empty() {
        return assess(Tier::Destructive, "empty command");
    }
    if shell_control().is_match(cmd) {
        return assess(Tier::Destructive, "uses shell control characters (pipes, redirects, chaining or variables)");
    }
    let lower = cmd.to_ascii_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    if let Some(bad) = words.iter().find(|w| DENY_WORDS.contains(w)) {
        return assess(Tier::Destructive, format!("contains `{bad}`, which can delete data, use the network or reveal secrets"));
    }
    if outside_workspace().is_match(cmd) {
        return assess(Tier::Destructive, "reaches outside the workspace folder");
    }
    let program = words[0].trim_end_matches(".exe");
    let args = &words[1..];
    if args.len() == 1 && matches!(args[0], "--version" | "-v" | "-version" | "--help" | "-h" | "/?") {
        return assess(Tier::ScopedMutation, "prints version or help text");
    }
    if program == "git" {
        return match args.first() {
            Some(sub) if GIT_READ_ONLY.contains(sub) => assess(Tier::ScopedMutation, "read-only git command"),
            _ => assess(Tier::Destructive, "git command that can change history or the network"),
        };
    }
    if READ_ONLY.contains(&program) {
        return assess(Tier::ScopedMutation, "read-only command inside the workspace");
    }
    assess(Tier::Destructive, format!("`{program}` is not on the read-only list"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Ctx(bool);
    impl SafetyContext for Ctx {
        fn file_exists(&self, _: &str) -> bool {
            self.0
        }
    }

    fn tier_of(name: &str, args: serde_json::Value, exists: bool) -> Tier {
        classify(&ToolCall { id: "x".into(), name: name.into(), arguments: args }, &Ctx(exists)).tier
    }

    #[test]
    fn perception_and_navigation_tiers() {
        assert_eq!(tier_of("look_at_screen", json!({}), false), Tier::Passive);
        assert_eq!(tier_of("find_elements", json!({}), false), Tier::Passive);
        assert_eq!(tier_of("open_app", json!({"name":"notepad"}), false), Tier::NonDestructive);
        assert_eq!(tier_of("read_file", json!({"path":"a"}), true), Tier::NonDestructive);
        assert_eq!(tier_of("point_at", json!({"x":1,"y":2}), false), Tier::Passive);
        assert_eq!(tier_of("scroll", json!({"direction":"down"}), false), Tier::NonDestructive);
        assert_eq!(tier_of("reminder", json!({"action":"add"}), false), Tier::NonDestructive);
        assert_eq!(tier_of("drag", json!({}), false), Tier::ScopedMutation);
        assert_eq!(tier_of("copy_to_clipboard", json!({"text":"x"}), false), Tier::ScopedMutation);
        assert_eq!(tier_of("remember", json!({"fact":"x"}), false), Tier::NonDestructive);
        assert_eq!(tier_of("forget", json!({"id":"x"}), false), Tier::NonDestructive);
        assert_eq!(tier_of("replace_selection", json!({"text":"x"}), false), Tier::ScopedMutation);
    }

    #[test]
    fn write_file_escalates_when_overwriting() {
        assert_eq!(tier_of("write_file", json!({"path":"new.txt"}), false), Tier::ScopedMutation);
        assert_eq!(tier_of("write_file", json!({"path":"old.txt"}), true), Tier::Destructive);
    }

    #[test]
    fn self_modification_always_needs_approval() {
        assert_eq!(tier_of("save_skill", json!({"name":"x","instructions":"y"}), false), Tier::Destructive);
        assert_eq!(tier_of("update_settings", json!({"changes":{}}), false), Tier::Destructive);
        assert_eq!(tier_of("write_file", json!({"path":"self/src/main.ts"}), false), Tier::Destructive);
        assert_eq!(tier_of("read_file", json!({"path":"self/src/main.ts"}), true), Tier::NonDestructive);
        assert_eq!(tier_of("delegate", json!({"goal":"x"}), false), Tier::NonDestructive);
    }

    #[test]
    fn unknown_tools_are_destructive() {
        assert_eq!(tier_of("format_disk", json!({}), false), Tier::Destructive);
    }

    #[test]
    fn dangerous_key_combos() {
        assert_eq!(tier_of("press_keys", json!({"keys":"Alt + F4"}), false), Tier::Destructive);
        assert_eq!(tier_of("press_keys", json!({"keys":"ctrl+s"}), false), Tier::ScopedMutation);
    }

    #[test]
    fn read_only_commands_are_tier_two() {
        for c in ["dir", "ls -la", "Get-ChildItem", "cat notes.txt", "git status", "git log --oneline", "python --version", "type notes.txt"] {
            assert_eq!(classify_command(c).tier, Tier::ScopedMutation, "{c}");
        }
    }

    #[test]
    fn risky_commands_are_tier_three() {
        for c in [
            "rm notes.txt",
            "Remove-Item notes.txt",
            "del /f notes.txt",
            "curl http://evil.example",
            "cat ~/.ssh/id_rsa",
            "type C:\\Users\\me\\secrets.txt",
            "cat ../outside.txt",
            "ls | sh",
            "echo hi > notes.txt",
            "echo $env:OPENROUTER_API_KEY",
            "Get-ChildItem env:",
            "git push",
            "git reset --hard",
            "python script.py",
            "npm install",
            "env",
            "",
        ] {
            assert_eq!(classify_command(c).tier, Tier::Destructive, "{c}");
        }
    }
}
