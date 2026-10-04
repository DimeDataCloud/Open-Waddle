//! Tool schemas shown to the model, typed GUI actions for the host, and
//! formatting of results back into model-readable text.
//!
//! Coordinates: the host works in logical screen pixels. The model may use
//! pixels or a 0..1000 grid (see `CoordMode`); conversion happens only here.

pub mod fs;
pub mod shell;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::CoordMode;
use crate::llm::{ImageData, ToolCall, ToolSpec};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Mouse, keyboard, screenshots and window listing are available.
    pub gui: bool,
    /// The accessibility fast path (`find_elements` / `click_element`) is available.
    pub accessibility: bool,
    /// Waddle's own source folder is reachable as `self/...`.
    pub self_edit: bool,
    /// `delegate` may start a nested sub-task.
    pub delegation: bool,
    /// Long-term skill memory and self-settings are available.
    pub self_improve: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coords {
    pub mode: CoordMode,
    pub screen_w: f64,
    pub screen_h: f64,
}

impl Coords {
    /// Model coordinates → logical screen pixels.
    pub fn to_screen(&self, x: f64, y: f64) -> (f64, f64) {
        match self.mode {
            CoordMode::Norm1000 => {
                // Small models sometimes answer in 0-1 fractions instead of 0-1000.
                // Nothing real sits within a pixel of the top-left corner, so read those as fractions.
                let fractions = (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y) && (x.fract() != 0.0 || y.fract() != 0.0);
                let scale = if fractions { 1.0 } else { 1000.0 };
                (x / scale * self.screen_w, y / scale * self.screen_h)
            }
            _ => (x, y),
        }
    }

    /// Logical screen pixels → model coordinates, rounded for display.
    pub fn from_screen(&self, x: f64, y: f64) -> (i64, i64) {
        match self.mode {
            CoordMode::Norm1000 => (
                (x / self.screen_w.max(1.0) * 1000.0).round() as i64,
                (y / self.screen_h.max(1.0) * 1000.0).round() as i64,
            ),
            _ => (x.round() as i64, y.round() as i64),
        }
    }

    pub fn describe(&self) -> String {
        match self.mode {
            CoordMode::Norm1000 => "Coordinates are normalised 0-1000 on both axes: (0,0) is the top-left of the screen and (1000,1000) the bottom-right.".into(),
            _ => format!(
                "Coordinates are pixels: (0,0) is the top-left of the screen, ({:.0},{:.0}) the bottom-right. Screenshots are taken at exactly this size.",
                self.screen_w, self.screen_h
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    Left,
    Right,
}

/// A GUI action in logical screen pixels, ready for the host to perform.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum GuiAction {
    ListWindows,
    LookAtScreen,
    FindElements { window: Option<String> },
    OpenApp { name: String },
    Click { x: f64, y: f64, button: MouseButton, double: bool },
    ClickElement { id: u32 },
    TypeText { text: String, at: Option<(f64, f64)> },
    PressKeys { keys: String },
    /// Play mode: a frozen copy of the screen becomes a playground to blast.
    /// `target` names a window to aim at; `autoplay` lets the duck play by itself.
    Play { autoplay: bool, target: Option<String>, weapon: Option<String> },
}

impl GuiAction {
    /// Input that goes wherever focus happens to be (as opposed to an element Waddle found).
    pub fn is_blind_input(&self) -> bool {
        matches!(self, GuiAction::Click { .. } | GuiAction::TypeText { .. } | GuiAction::PressKeys { .. })
    }

    /// Actions that show Waddle what's on the desktop (or put a known app in front).
    pub fn perceives(&self) -> bool {
        matches!(self, GuiAction::ListWindows | GuiAction::LookAtScreen | GuiAction::FindElements { .. } | GuiAction::OpenApp { .. })
    }

    /// Where Waddle should walk before acting, if anywhere.
    pub fn target(&self) -> Option<(f64, f64)> {
        match self {
            GuiAction::Click { x, y, .. } => Some((*x, *y)),
            GuiAction::TypeText { at, .. } => *at,
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowInfo {
    pub title: String,
    pub app: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub focused: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ElementInfo {
    pub id: u32,
    pub role: String,
    pub name: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GuiResult {
    Windows(Vec<WindowInfo>),
    Screenshot { image: ImageData, width: u32, height: u32 },
    Elements { window: String, elements: Vec<ElementInfo> },
    Done(String),
}

/// What a tool hands back to the agent loop.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome {
    pub text: String,
    pub image: Option<ImageData>,
    /// Set when the text came from outside Waddle (screen, files, command output).
    pub untrusted_source: Option<&'static str>,
}

impl ToolOutcome {
    pub fn trusted(text: impl Into<String>) -> Self {
        Self { text: text.into(), image: None, untrusted_source: None }
    }
    pub fn untrusted(source: &'static str, text: impl Into<String>) -> Self {
        Self { text: text.into(), image: None, untrusted_source: Some(source) }
    }
}

fn spec(name: &str, description: &str, properties: Value, required: &[&str]) -> ToolSpec {
    let mut parameters = json!({ "type": "object", "properties": properties });
    if !required.is_empty() {
        parameters["required"] = json!(required);
    }
    ToolSpec { name: name.into(), description: description.into(), parameters }
}

pub fn specs(caps: Capabilities, coords: &Coords) -> Vec<ToolSpec> {
    let unit = match coords.mode {
        CoordMode::Norm1000 => "0-1000 normalised",
        _ => "screen pixels",
    };
    let mut v = vec![];
    if caps.gui {
        v.push(spec("list_windows", "List visible windows, front to back, with titles, apps and bounds.", json!({}), &[]));
        if caps.accessibility {
            v.push(spec(
                "find_elements",
                "List the buttons, fields, links and menus of the front window, or of the window whose title contains `window`.",
                json!({ "window": { "type": "string" } }),
                &[],
            ));
            v.push(spec(
                "click_element",
                "Click an element from the latest find_elements list.",
                json!({ "id": { "type": "integer" } }),
                &["id"],
            ));
        }
        v.push(spec("look_at_screen", "Take a screenshot.", json!({}), &[]));
        v.push(spec("open_app", "Launch an app by name, e.g. \"notepad\".", json!({ "name": { "type": "string" } }), &["name"]));
        v.push(spec(
            "click",
            &format!("Click a point ({unit})."),
            json!({
                "x": { "type": "number" }, "y": { "type": "number" },
                "button": { "type": "string", "enum": ["left", "right"] },
                "double": { "type": "boolean" }
            }),
            &["x", "y"],
        ));
        v.push(spec(
            "type_text",
            &format!("Type into the focused field, or click x,y ({unit}) first if given."),
            json!({ "text": { "type": "string" }, "x": { "type": "number" }, "y": { "type": "number" } }),
            &["text"],
        ));
        v.push(spec(
            "play",
            "Start play mode: a harmless copy of the screen becomes a playground that gets blasted apart with silly weapons \
(pea shooter, egg bazooka, laser eyes...). Nothing real is touched and Esc ends it. Use it when the user wants to play, \
is bored or stressed, or asks you to wreck, smash or blow up their screen or a window. It ends your task: say something fun first.",
            json!({
                "autoplay": { "type": "boolean", "description": "true = you play by yourself while the user watches" },
                "target": { "type": "string", "description": "title of a window to aim at" },
                "weapon": { "type": "string", "enum": ["pea", "crumbs", "feathers", "egg", "laser", "flame", "quack"] }
            }),
            &[],
        ));
        v.push(spec(
            "press_keys",
            "Press a key or shortcut, e.g. \"enter\" or \"ctrl+s\".",
            json!({ "keys": { "type": "string" } }),
            &["keys"],
        ));
    }
    // Paths are relative to the workspace (the system prompt says so); only self/ needs explaining.
    let path = if caps.self_edit {
        json!({ "type": "string", "description": "Relative to the workspace, or start with self/ for your own source code" })
    } else {
        json!({ "type": "string" })
    };
    let mut command_props = json!({ "command": { "type": "string" } });
    if caps.self_edit {
        command_props["cwd"] = json!({ "type": "string", "enum": ["workspace", "self"], "description": "Default workspace; self is your source folder" });
    }
    v.push(spec(
        "run_command",
        "Run a terminal command (PowerShell on Windows, sh elsewhere) and get its output.",
        command_props,
        &["command"],
    ));
    v.push(spec("read_file", "Read a text file.", json!({ "path": path }), &["path"]));
    v.push(spec(
        "write_file",
        "Create or overwrite a text file.",
        json!({ "path": path, "content": { "type": "string" } }),
        &["path", "content"],
    ));
    v.push(spec("list_dir", "List a folder (default: the workspace).", json!({ "path": path }), &[]));
    if caps.delegation {
        v.push(spec(
            "delegate",
            "Hand an independent part of a big job to a fresh copy of yourself and get its result. It knows only what you pass it.",
            json!({ "goal": { "type": "string" }, "context": { "type": "string" } }),
            &["goal"],
        ));
    }
    if caps.self_improve {
        v.push(spec(
            "save_skill",
            "Save short instructions for doing something well next time (e.g. which app or command worked). Loaded into every future task.",
            json!({ "name": { "type": "string" }, "instructions": { "type": "string" } }),
            &["name", "instructions"],
        ));
        v.push(spec("forget_skill", "Remove a saved skill.", json!({ "name": { "type": "string" } }), &["name"]));
        v.push(spec(
            "update_settings",
            &format!(
                "Change your own settings from the next task on. Keys: {}.",
                crate::config::SELF_EDITABLE.join(", ")
            ),
            json!({ "changes": { "type": "object", "description": "e.g. {\"model\": \"...\", \"wander\": false}" } }),
            &["changes"],
        ));
    }
    v
}

pub fn is_gui_tool(name: &str) -> bool {
    matches!(
        name,
        "list_windows" | "look_at_screen" | "find_elements" | "open_app" | "click" | "click_element" | "type_text" | "press_keys" | "play"
    )
}

fn num(args: &Value, k: &str) -> Option<f64> {
    match args.get(k)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn text(args: &Value, k: &str) -> Option<String> {
    args.get(k).and_then(Value::as_str).map(str::to_string)
}

/// Parses a GUI tool call into a host action, converting coordinates.
pub fn parse_gui_action(call: &ToolCall, coords: &Coords) -> Result<GuiAction, String> {
    let a = &call.arguments;
    let point = |required: bool| -> Result<Option<(f64, f64)>, String> {
        match (num(a, "x"), num(a, "y")) {
            (Some(x), Some(y)) => Ok(Some(coords.to_screen(x, y))),
            (None, None) if !required => Ok(None),
            _ => Err("both x and y are required numbers".into()),
        }
    };
    Ok(match call.name.as_str() {
        "list_windows" => GuiAction::ListWindows,
        "look_at_screen" => GuiAction::LookAtScreen,
        "find_elements" => GuiAction::FindElements { window: text(a, "window").filter(|s| !s.trim().is_empty()) },
        "open_app" => GuiAction::OpenApp { name: text(a, "name").filter(|s| !s.trim().is_empty()).ok_or("`name` is required")? },
        "click" => {
            let (x, y) = point(true)?.unwrap();
            let button = if text(a, "button").as_deref() == Some("right") { MouseButton::Right } else { MouseButton::Left };
            let double = a.get("double").and_then(Value::as_bool).unwrap_or(false);
            GuiAction::Click { x, y, button, double }
        }
        "click_element" => GuiAction::ClickElement { id: num(a, "id").ok_or("`id` is required")? as u32 },
        "type_text" => GuiAction::TypeText { text: text(a, "text").ok_or("`text` is required")?, at: point(false)? },
        "press_keys" => GuiAction::PressKeys { keys: text(a, "keys").filter(|s| !s.trim().is_empty()).ok_or("`keys` is required")? },
        "play" => GuiAction::Play {
            autoplay: a.get("autoplay").and_then(Value::as_bool).unwrap_or(false),
            target: text(a, "target").filter(|s| !s.trim().is_empty()),
            weapon: text(a, "weapon").filter(|s| !s.trim().is_empty()),
        },
        other => return Err(format!("`{other}` is not a GUI tool")),
    })
}

/// Turns a host result into what the model sees.
pub fn format_gui_result(result: GuiResult, coords: &Coords) -> ToolOutcome {
    match result {
        GuiResult::Windows(ws) => {
            if ws.is_empty() {
                return ToolOutcome::trusted("No visible windows.");
            }
            let lines: Vec<String> = ws
                .iter()
                .enumerate()
                .map(|(i, w)| {
                    let (x, y) = coords.from_screen(w.x, w.y);
                    let (x2, y2) = coords.from_screen(w.x + w.w, w.y + w.h);
                    format!(
                        "{}. \"{}\" [{}] from ({x},{y}) to ({x2},{y2}){}",
                        i + 1,
                        w.title,
                        w.app,
                        if w.focused { " (focused)" } else { "" }
                    )
                })
                .collect();
            ToolOutcome::untrusted("window_list", format!("Visible windows, front to back:\n{}", lines.join("\n")))
        }
        GuiResult::Elements { window, elements } => {
            if elements.is_empty() {
                return ToolOutcome::trusted(format!(
                    "No accessible elements found in \"{window}\". Use look_at_screen instead."
                ));
            }
            let lines: Vec<String> = elements
                .iter()
                .map(|e| {
                    let (cx, cy) = coords.from_screen(e.x + e.w / 2.0, e.y + e.h / 2.0);
                    format!("[{}] {} \"{}\" at ({cx},{cy})", e.id, e.role, e.name)
                })
                .collect();
            ToolOutcome::untrusted("screen_elements", format!("Elements in \"{window}\":\n{}", lines.join("\n")))
        }
        GuiResult::Screenshot { image, width, height } => ToolOutcome {
            text: format!("Screenshot taken ({width}x{height}); it is attached in the next message."),
            image: Some(image),
            untrusted_source: None,
        },
        GuiResult::Done(t) => ToolOutcome::trusted(t),
    }
}

/// One-line, human-readable description of a call for the speech bubble and approval card.
pub fn summarize(call: &ToolCall) -> String {
    let a = &call.arguments;
    let s = |k: &str| text(a, k).unwrap_or_default();
    let short = |t: String| if t.chars().count() > 60 { format!("{}…", t.chars().take(60).collect::<String>()) } else { t };
    match call.name.as_str() {
        "list_windows" => "Look at the open windows".into(),
        "look_at_screen" => "Take a screenshot".into(),
        "find_elements" => "Read the buttons and fields on screen".into(),
        "open_app" => format!("Open {}", s("name")),
        // The duck walks to the spot, so the model's raw coordinates would only confuse.
        "click" => match (s("button").as_str(), a.get("double").and_then(Value::as_bool).unwrap_or(false)) {
            ("right", _) => "Right-click here".into(),
            (_, true) => "Double-click here".into(),
            _ => "Click here".into(),
        },
        "click_element" => format!("Click element {}", num(a, "id").unwrap_or(0.0)),
        "type_text" => format!("Type \"{}\"", short(s("text"))),
        "press_keys" => format!("Press {}", s("keys")),
        "play" => if s("target").is_empty() { "Play time!".into() } else { format!("Play time: aiming at {}", short(s("target"))) },
        "run_command" => format!("Run `{}`", short(s("command"))),
        "read_file" => format!("Read {}", s("path")),
        "write_file" => format!("Write {}", s("path")),
        "delegate" => format!("Delegate: {}", short(s("goal"))),
        "save_skill" => format!("Learn skill \"{}\"", s("name")),
        "forget_skill" => format!("Forget skill \"{}\"", s("name")),
        "update_settings" => format!("Change my settings: {}", a.get("changes").map(|c| c.to_string()).unwrap_or_default()),
        "list_dir" => format!("List {}", if s("path").is_empty() { "the workspace".into() } else { s("path") }),
        other => format!("Use {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coords(mode: CoordMode) -> Coords {
        Coords { mode, screen_w: 1440.0, screen_h: 960.0 }
    }

    fn call(name: &str, args: Value) -> ToolCall {
        ToolCall { id: "1".into(), name: name.into(), arguments: args }
    }

    #[test]
    fn norm1000_converts_both_ways() {
        let c = coords(CoordMode::Norm1000);
        assert_eq!(c.to_screen(500.0, 500.0), (720.0, 480.0));
        assert_eq!(c.from_screen(720.0, 480.0), (500, 500));
        assert_eq!(c.to_screen(0.5, 0.25), (720.0, 240.0), "0-1 fractions are read as fractions");
        assert_eq!(c.to_screen(1.0, 0.0), (1.44, 0.0), "whole numbers stay on the 0-1000 grid");
        let p = coords(CoordMode::Pixels);
        assert_eq!(p.to_screen(10.0, 20.0), (10.0, 20.0));
    }

    #[test]
    fn parses_play() {
        let c = coords(CoordMode::Pixels);
        let a = parse_gui_action(&call("play", json!({"autoplay": true, "target": "Chrome", "weapon": "egg"})), &c).unwrap();
        assert_eq!(a, GuiAction::Play { autoplay: true, target: Some("Chrome".into()), weapon: Some("egg".into()) });
        let a = parse_gui_action(&call("play", json!({"target": " "})), &c).unwrap();
        assert_eq!(a, GuiAction::Play { autoplay: false, target: None, weapon: None });
        assert!(!a.is_blind_input(), "play needs no look first: it only touches a copy");
    }

    #[test]
    fn parses_click_with_string_numbers() {
        let a = parse_gui_action(&call("click", json!({"x":"250","y":100,"button":"right"})), &coords(CoordMode::Norm1000)).unwrap();
        assert_eq!(a, GuiAction::Click { x: 360.0, y: 96.0, button: MouseButton::Right, double: false });
        assert_eq!(a.target(), Some((360.0, 96.0)));
    }

    #[test]
    fn rejects_incomplete_arguments() {
        let c = coords(CoordMode::Pixels);
        assert!(parse_gui_action(&call("click", json!({"x":1})), &c).is_err());
        assert!(parse_gui_action(&call("open_app", json!({})), &c).is_err());
        assert!(parse_gui_action(&call("type_text", json!({"text":"hi","x":5})), &c).is_err());
        assert_eq!(parse_gui_action(&call("type_text", json!({"text":"hi"})), &c).unwrap().target(), None);
    }

    #[test]
    fn specs_respect_capabilities() {
        let c = coords(CoordMode::Pixels);
        let names = |caps| specs(caps, &c).into_iter().map(|s| s.name).collect::<Vec<_>>();
        let headless = names(Capabilities::default());
        assert!(headless.contains(&"run_command".to_string()) && !headless.contains(&"click".to_string()));
        let full = names(Capabilities { gui: true, accessibility: true, self_edit: true, delegation: true, self_improve: true });
        assert!(full.contains(&"delegate".to_string()) && full.contains(&"save_skill".to_string()));
        assert!(full.contains(&"find_elements".to_string()) && full.contains(&"click".to_string()));
        let no_a11y = names(Capabilities { gui: true, ..Default::default() });
        assert!(!no_a11y.contains(&"find_elements".to_string()));
    }

    #[test]
    fn window_and_element_lists_are_untrusted_and_use_model_coords() {
        let c = coords(CoordMode::Norm1000);
        let out = format_gui_result(
            GuiResult::Elements {
                window: "Notepad".into(),
                elements: vec![ElementInfo { id: 3, role: "button".into(), name: "Save".into(), x: 700.0, y: 470.0, w: 40.0, h: 20.0 }],
            },
            &c,
        );
        assert_eq!(out.untrusted_source, Some("screen_elements"));
        assert!(out.text.contains("[3] button \"Save\" at (500,500)"), "{}", out.text);
    }
}
