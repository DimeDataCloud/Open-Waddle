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
        "routine" if matches!(arg("action"), "add" | "") => {
            assess(Tier::Destructive, "sets up a task that runs on its own on a schedule (it acts for you later, asking before changes)")
        }
        "routine" => assess(Tier::NonDestructive, "looks at, pauses or removes routines"),
        "remember" | "forget" => assess(Tier::NonDestructive, "updates the facts Waddle keeps about you (listed in Settings)"),
        "replace_selection" => assess(Tier::ScopedMutation, "replaces the text you selected"),
        "mail_search" | "mail_read" | "contacts_find" | "calendar_events" | "calendar_free" => assess(Tier::NonDestructive, "reads your Google account"),
        "mail_style" => assess(Tier::NonDestructive, "reads some of your sent mail to learn your writing style"),
        "mail_draft" => assess(Tier::ScopedMutation, "saves a draft in Gmail (nothing is sent)"),
        "mail_modify" => assess(Tier::ScopedMutation, "archives, labels or marks an email"),
        "calendar_create" => assess(Tier::ScopedMutation, "adds an event to your calendar (attendees get an invitation)"),
        "calendar_update" => assess(Tier::ScopedMutation, "changes a calendar event (attendees are told)"),
        "calendar_respond" => assess(Tier::ScopedMutation, "answers an invitation"),
        "browser_read" => assess(Tier::Passive, "reads a web page"),
        "browser_tabs" if arg("action") == "close" => assess(Tier::ScopedMutation, "closes a Chrome tab"),
        "browser_tabs" | "browser_navigate" => assess(Tier::NonDestructive, "moves around in Chrome"),
        "browser_click" => assess(Tier::ScopedMutation, "clicks on a web page"),
        "browser_type" => assess(Tier::ScopedMutation, "types into a web page"),
        "mail_send" => assess(Tier::Destructive, "sends an email from your account"),
        "mail_trash" => assess(Tier::Destructive, "moves an email to the bin"),
        "calendar_delete" => assess(Tier::Destructive, "deletes a calendar event (attendees are told)"),
        "copy_to_clipboard" => assess(Tier::ScopedMutation, "replaces what's on the clipboard"),
        "drag" => assess(Tier::ScopedMutation, "drags in another application"),
        "open_app" => assess(Tier::NonDestructive, "opens an application"),
        "arrange_window" => assess(Tier::NonDestructive, "moves or resizes a window"),
        "duck" => assess(Tier::Passive, "moves Waddle itself"),
        "read_file" | "list_dir" => assess(Tier::NonDestructive, "reads the workspace"),
        "find_files" | "read_document" => assess(Tier::NonDestructive, "reads your files"),
        "drive_search" | "drive_read" => assess(Tier::NonDestructive, "reads your Google Drive"),
        "move_file" | "rename_file" => assess(Tier::ScopedMutation, "moves or renames a file in a folder you allowed"),
        "create_document" if ctx.file_exists(arg("path")) => assess(Tier::ScopedMutation, "replaces a file (the old copy goes to the Recycle Bin)"),
        "create_document" => assess(Tier::ScopedMutation, "creates a new document"),
        "delete_file" => assess(Tier::Destructive, "sends a file to the Recycle Bin"),
        "save_skill" | "forget_skill" => assess(Tier::Destructive, "changes Waddle's long-term memory (loaded into every task)"),
        "update_settings" => assess(Tier::Destructive, "changes Waddle's own settings"),
        "delegate" => assess(Tier::NonDestructive, "starts a sub-task; each of its actions is gated on its own"),
        "write_file" if is_self_path(arg("path")) => assess(Tier::Destructive, "edits Waddle's own source code"),
        "write_file" => {
            if ctx.file_exists(arg("path")) {
                assess(Tier::ScopedMutation, "overwrites a file (the old copy goes to the Recycle Bin)")
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

/// Tiers for MCP tools, by fixed rules. A server's own "read-only" hint only
/// lowers the tier for servers the user marked trusted; an untrusted server
/// could say anything about its tools.
pub fn classify_mcp(server: &str, trusted: bool, read_only: bool, destructive: bool) -> Assessment {
    match (trusted, read_only, destructive) {
        (true, true, _) => assess(Tier::NonDestructive, format!("reads through {server} (a trusted MCP server; the tool says it only reads)")),
        (true, false, true) => assess(Tier::Destructive, format!("may change or delete things through {server} (MCP)")),
        (true, false, false) => assess(Tier::ScopedMutation, format!("changes something through {server} (a trusted MCP server)")),
        (false, true, _) => assess(Tier::ScopedMutation, format!("uses {server} (MCP; the tool says it only reads, but the server isn't marked trusted)")),
        (false, false, _) => assess(Tier::Destructive, format!("uses {server}, an MCP server not marked trusted")),
    }
}

/// Paths under `self/` refer to Waddle's own source folder.
pub fn is_self_path(path: &str) -> bool {
    let p = path.trim_start_matches("./");
    p == "self" || p.starts_with("self/") || p.starts_with("self\\")
}

/// A shortcut in one standard spelling: lower case, modifiers first in the order
/// ctrl, alt, shift, meta, and one name per key (`control` → `ctrl`, `win`/`cmd`/`super` → `meta`,
/// `del` → `delete`, `esc` → `escape`). "F4 + Alt" and "alt+f4" are the same shortcut.
pub fn canonical_keys(keys: &str) -> String {
    const MODIFIERS: [&str; 4] = ["ctrl", "alt", "shift", "meta"];
    let mut mods = [false; 4];
    let mut rest: Vec<&str> = vec![];
    let lower = keys.to_ascii_lowercase();
    for part in lower.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        let name = match part {
            "control" | "ctl" | "strg" => "ctrl",
            "option" | "opt" | "altgr" => "alt",
            "win" | "windows" | "cmd" | "command" | "super" | "logo" | "os" => "meta",
            "del" => "delete",
            "esc" => "escape",
            "return" => "enter",
            "ins" => "insert",
            "pgup" => "pageup",
            "pgdn" => "pagedown",
            "arrowup" => "up",
            "arrowdown" => "down",
            "arrowleft" => "left",
            "arrowright" => "right",
            "spacebar" => "space",
            other => other,
        };
        match MODIFIERS.iter().position(|m| *m == name) {
            Some(i) => mods[i] = true,
            None if !rest.contains(&name) => rest.push(name),
            None => {}
        }
    }
    MODIFIERS.iter().zip(mods).filter(|(_, on)| *on).map(|(m, _)| *m).chain(rest).collect::<Vec<_>>().join("+")
}

fn classify_keys(keys: &str) -> Assessment {
    let norm = canonical_keys(keys);
    // In canonical spelling (see `canonical_keys`).
    const DANGEROUS: &[&str] = &[
        "alt+f4", "ctrl+f4", "ctrl+w", "ctrl+shift+w", "ctrl+q", "meta+q", "meta+w",
        "delete", "shift+delete", "ctrl+shift+delete", "ctrl+alt+delete", "meta+r",
        "meta+l", "ctrl+shift+escape",
    ];
    // Keys that only move around (snap or switch windows and tabs, open a new
    // tab, reach the address bar, scroll, zoom, go back) change nothing, so they
    // don't wait for the countdown. Enter, Space and typing still do.
    const MOVES: &[&str] = &[
        "meta+left", "meta+right", "meta+up", "meta+down", "shift+meta+left", "shift+meta+right",
        "alt+tab", "alt+shift+tab", "ctrl+tab", "ctrl+shift+tab", "ctrl+pageup", "ctrl+pagedown",
        "ctrl+t", "ctrl+n", "ctrl+shift+n", "ctrl+l", "alt+d", "f6", "ctrl+f", "f3",
        "alt+left", "alt+right", "f5", "ctrl+r", "f11",
        "ctrl+plus", "ctrl+=", "ctrl+-", "ctrl+minus", "ctrl+0",
        "up", "down", "left", "right", "pageup", "pagedown", "home", "end", "ctrl+home", "ctrl+end",
        "tab", "shift+tab", "escape",
    ];
    if DANGEROUS.contains(&norm.as_str()) {
        assess(Tier::Destructive, format!("`{keys}` can close windows or delete data"))
    } else if MOVES.contains(&norm.as_str()) {
        assess(Tier::NonDestructive, "moves around (switches, snaps, scrolls or opens a tab)")
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
    #[test]
    fn mcp_tiers_follow_trust_not_the_servers_word() {
        let t = |trusted, ro, de| classify_mcp("x", trusted, ro, de).tier;
        assert_eq!(t(true, true, false), Tier::NonDestructive);
        assert_eq!(t(true, false, false), Tier::ScopedMutation);
        assert_eq!(t(true, false, true), Tier::Destructive);
        assert_eq!(t(false, true, false), Tier::ScopedMutation, "an untrusted server's read-only hint only gets a countdown");
        assert_eq!(t(false, false, false), Tier::Destructive);
    }

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
    fn browser_tiers() {
        assert_eq!(tier_of("browser_read", json!({}), false), Tier::Passive);
        assert_eq!(tier_of("browser_tabs", json!({"action":"list"}), false), Tier::NonDestructive);
        assert_eq!(tier_of("browser_tabs", json!({"action":"close"}), false), Tier::ScopedMutation);
        assert_eq!(tier_of("browser_navigate", json!({"url":"x"}), false), Tier::NonDestructive);
        assert_eq!(tier_of("browser_click", json!({}), false), Tier::ScopedMutation);
        assert_eq!(tier_of("browser_type", json!({}), false), Tier::ScopedMutation);
    }

    #[test]
    fn only_sends_and_deletes_need_a_click_in_google() {
        for read in ["mail_search", "mail_read", "contacts_find", "calendar_events", "calendar_free", "mail_style"] {
            assert_eq!(tier_of(read, json!({}), false), Tier::NonDestructive, "{read}");
        }
        for notice in ["mail_draft", "mail_modify", "calendar_create", "calendar_update", "calendar_respond"] {
            assert_eq!(tier_of(notice, json!({}), false), Tier::ScopedMutation, "{notice}");
        }
        for click in ["mail_send", "mail_trash", "calendar_delete"] {
            assert_eq!(tier_of(click, json!({}), false), Tier::Destructive, "{click}");
        }
    }

    #[test]
    fn write_file_escalates_when_overwriting() {
        assert_eq!(tier_of("write_file", json!({"path":"new.txt"}), false), Tier::ScopedMutation);
        assert_eq!(tier_of("write_file", json!({"path":"old.txt"}), true), Tier::ScopedMutation, "the old copy goes to the Recycle Bin");
        assert_eq!(tier_of("write_file", json!({"path":"self/src/lib.rs"}), true), Tier::Destructive, "own code still needs a click");
        assert_eq!(tier_of("delete_file", json!({"path":"x"}), true), Tier::Destructive);
        assert_eq!(tier_of("move_file", json!({}), true), Tier::ScopedMutation);
        assert_eq!(tier_of("find_files", json!({}), false), Tier::NonDestructive);
        assert_eq!(tier_of("drive_read", json!({}), false), Tier::NonDestructive);
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
        for spelled in ["f4+alt", "Control+W", "W+ctrl", "shift+del", "Win + R", "cmd+q", "esc+shift+ctrl", "Ctrl+Alt+Del", "ctrl+F4"] {
            assert_eq!(tier_of("press_keys", json!({ "keys": spelled }), false), Tier::Destructive, "{spelled}");
        }
        assert_eq!(canonical_keys("Shift + Control + Left"), "ctrl+shift+left");
        assert_eq!(canonical_keys("super+ctrl+ctrl+d"), "ctrl+meta+d");
        assert_eq!(tier_of("press_keys", json!({"keys":"ctrl+s"}), false), Tier::ScopedMutation);
    }

    #[test]
    fn keys_that_only_move_around_skip_the_countdown() {
        for k in ["win+right", "Windows + Left", "shift+win+right", "alt+tab", "ctrl+t", "ctrl+n", "ctrl+l", "ctrl+tab", "pagedown", "ArrowDown", "esc", "alt+left", "f5", "ctrl+plus"] {
            assert_eq!(tier_of("press_keys", json!({ "keys": k }), false), Tier::NonDestructive, "{k}");
        }
        // These can send, submit, type, paste, undo or save: they keep the countdown.
        for k in ["enter", "space", "ctrl+enter", "ctrl+v", "ctrl+z", "ctrl+s", "a", "ctrl+a", "ctrl+shift+enter", "shift+enter"] {
            assert_eq!(tier_of("press_keys", json!({ "keys": k }), false), Tier::ScopedMutation, "{k}");
        }
        assert_eq!(tier_of("press_keys", json!({"keys":"ctrl+w"}), false), Tier::Destructive);
        assert_eq!(tier_of("arrange_window", json!({"action":"left_half"}), false), Tier::NonDestructive);
        assert_eq!(tier_of("duck", json!({"trick":"fly_around"}), false), Tier::Passive);
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
