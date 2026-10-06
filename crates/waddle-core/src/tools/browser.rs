//! Chrome through Waddle's extension: read pages and tabs directly instead of
//! from screenshots, and act on elements by id. Clicks are real mouse clicks at
//! the element's place on screen (the duck walks there), with the page's own
//! `element.click()` as the fallback.

use serde_json::{json, Value};

use crate::llm::{ToolCall, ToolSpec};
use crate::tools::spec;

/// Search pages to go to directly, one step instead of typing into a search box.
pub const SEARCH_ADDRESSES: &str = "Search straight from the address (put the words in with + between them): \
the web https://www.google.com/search?q=<words>, YouTube https://www.youtube.com/results?search_query=<words>, \
a YouTube channel's newest videos https://www.youtube.com/@<handle>/videos, YouTube Music https://music.youtube.com/search?q=<words>, \
GitHub https://github.com/search?q=<words>&type=repositories, Google Maps https://www.google.com/maps/search/<place>, \
Wikipedia https://en.wikipedia.org/w/index.php?search=<words>, Amazon https://www.amazon.com/s?k=<words>.";

pub fn is_browser_tool(name: &str) -> bool {
    name.starts_with("browser_")
}

pub fn specs() -> Vec<ToolSpec> {
    let tab = json!({ "type": "integer", "description": "Tab id from browser_tabs (default: the active tab)" });
    let read = json!({ "type": "string", "enum": ["text", "elements"], "description": "Also read the page afterwards, in the same step" });
    vec![
        spec(
            "browser_tabs",
            "List, switch to, open or close Chrome tabs. To open a page beside another one (split screen, another monitor), open it with new_window, then arrange_window.",
            json!({
                "action": { "type": "string", "enum": ["list", "switch", "open", "close"] },
                "tab": tab,
                "url": { "type": "string", "description": "For open" },
                "new_window": { "type": "boolean", "description": "For open: in a new Chrome window" },
                "read": read
            }),
            &["action"],
        ),
        spec(
            "browser_read",
            "Read a Chrome tab: its title, address and text (mode text), or its buttons, links and fields with ids (mode elements).",
            json!({ "tab": tab, "mode": { "type": "string", "enum": ["text", "elements"] } }),
            &[],
        ),
        spec(
            "browser_navigate",
            &format!("Go to a web address in Chrome (it comes to the front). {SEARCH_ADDRESSES}"),
            json!({ "url": { "type": "string" }, "tab": tab, "read": read }),
            &["url"],
        ),
        spec(
            "browser_click",
            "Click an element from the latest browser_read elements list, by its id (e.g. \"e12\").",
            json!({ "element": { "type": "string" }, "tab": tab, "read": read }),
            &["element"],
        ),
        spec(
            "browser_type",
            "Type text into a field from the latest elements list, replacing what's there. submit: true presses Enter after.",
            json!({ "element": { "type": "string" }, "text": { "type": "string" }, "submit": { "type": "boolean" }, "tab": tab }),
            &["element", "text"],
        ),
    ]
}

pub fn summarize(call: &ToolCall) -> Option<String> {
    let a = &call.arguments;
    let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let short = |t: String| if t.chars().count() > 50 { format!("{}…", t.chars().take(50).collect::<String>()) } else { t };
    Some(match call.name.as_str() {
        "browser_tabs" => match s("action").as_str() {
            "switch" => "Switch Chrome tab".into(),
            "open" => format!("Open {}", short(s("url"))),
            "close" => "Close a Chrome tab".into(),
            _ => "Look at your Chrome tabs".into(),
        },
        "browser_read" => "Read the page".into(),
        "browser_navigate" => format!("Go to {}", short(s("url"))),
        "browser_click" => "Click on the page".into(),
        "browser_type" => format!("Type \"{}\"", short(s("text"))),
        _ => return None,
    })
}

/// Only web pages: no `javascript:`, `file:` or browser-internal addresses. A bare
/// domain gets https.
pub fn safe_url(url: &str) -> anyhow::Result<String> {
    let url = url.trim();
    anyhow::ensure!(!url.is_empty(), "`url` is required");
    let full = if url.contains("://") { url.to_string() } else { format!("https://{url}") };
    let parsed = reqwest::Url::parse(&full).map_err(|e| anyhow::anyhow!("`{url}` isn't a web address ({e})"))?;
    anyhow::ensure!(matches!(parsed.scheme(), "http" | "https"), "only http and https addresses can be opened, not `{}`", parsed.scheme());
    anyhow::ensure!(parsed.host_str().is_some_and(|h| !h.is_empty()), "`{url}` has no website in it");
    Ok(parsed.to_string())
}

/// The model-readable form of a `read` result.
pub fn format_read(v: &Value) -> String {
    let title = v["title"].as_str().unwrap_or("");
    let url = v["url"].as_str().unwrap_or("");
    let mut out = format!("Tab: \"{title}\" ({url})\n");
    if let Some(text) = v["text"].as_str() {
        out.push_str(text);
        if v["truncated"].as_bool() == Some(true) {
            out.push_str("\n[… more below; read again after scrolling, or use elements]");
        }
    }
    if let Some(list) = v["elements"].as_array() {
        if list.is_empty() {
            out.push_str("No buttons, links or fields found.");
        }
        for e in list {
            let id = e["id"].as_str().unwrap_or("?");
            let role = e["role"].as_str().unwrap_or("element");
            let name = e["name"].as_str().unwrap_or("");
            let mut line = format!("[{id}] {role} \"{name}\"");
            if let Some(value) = e["value"].as_str().filter(|v| !v.is_empty()) {
                line.push_str(&format!(" = \"{value}\""));
            }
            if e["offscreen"].as_bool() == Some(true) {
                line.push_str(" (off screen)");
            }
            out.push_str(&line);
            out.push('\n');
        }
    }
    out.trim_end().to_string()
}

pub fn format_tabs(v: &Value) -> String {
    let tabs = v.as_array().cloned().unwrap_or_default();
    if tabs.is_empty() {
        return "No Chrome tabs are open.".into();
    }
    tabs.iter()
        .map(|t| {
            format!(
                "[tab {}]{} \"{}\" ({})",
                t["id"],
                if t["active"].as_bool() == Some(true) { " (active)" } else { "" },
                t["title"].as_str().unwrap_or(""),
                t["url"].as_str().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Where an element is on screen, in logical screen pixels, from what the page
/// reports: its rect in CSS pixels and the window's position and sizes (DIPs).
/// Page zoom is devicePixelRatio over the monitor's scale. The browser frame is
/// assumed even on the left, right and bottom, with the rest (tabs, address bar) on top.
pub fn screen_point(loc: &Value, monitor_scale: f64) -> Option<(f64, f64)> {
    let f = |k: &str| loc[k].as_f64();
    let (x, y, w, h) = (f("x")?, f("y")?, f("w")?, f("h")?);
    let (sx, sy) = (f("screenX")?, f("screenY")?);
    let (ow, oh, iw, ih) = (f("outerWidth")?, f("outerHeight")?, f("innerWidth")?, f("innerHeight")?);
    let zoom = f("dpr").unwrap_or(1.0) / monitor_scale.max(0.5);
    let border = ((ow - iw * zoom) / 2.0).max(0.0);
    let top = (oh - ih * zoom - border).max(0.0);
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    Some((sx + border + (x + w / 2.0) * zoom, sy + top + (y + h / 2.0) * zoom))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_addresses_open() {
        assert_eq!(safe_url("example.com/a?b=1").unwrap(), "https://example.com/a?b=1");
        assert!(safe_url("http://localhost:3000").is_ok());
        for bad in ["javascript:alert(1)", "file:///C:/Windows", "chrome://settings", "", "https://"] {
            assert!(safe_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn element_rects_become_screen_points() {
        // A maximised window at 150% scaling, page zoom 100%: 8 px borders, 80 px of tabs and toolbar.
        let loc = json!({ "x": 100, "y": 50, "w": 40, "h": 20, "screenX": 0, "screenY": 0, "outerWidth": 1296, "outerHeight": 868, "innerWidth": 1280, "innerHeight": 780, "dpr": 1.5 });
        assert_eq!(screen_point(&loc, 1.5), Some((8.0 + 120.0, 80.0 + 60.0)));
        // The same page zoomed to 125%: CSS pixels grow on screen.
        let zoomed = json!({ "x": 100, "y": 50, "w": 40, "h": 20, "screenX": 0, "screenY": 0, "outerWidth": 1296, "outerHeight": 868, "innerWidth": 1024, "innerHeight": 624, "dpr": 1.875 });
        let (x, y) = screen_point(&zoomed, 1.5).unwrap();
        assert!((x - (8.0 + 150.0)).abs() < 0.01 && (y - (80.0 + 75.0)).abs() < 0.01, "{x},{y}");
        assert_eq!(screen_point(&json!({ "x": 1, "y": 1, "w": 0, "h": 0, "screenX": 0, "screenY": 0, "outerWidth": 1, "outerHeight": 1, "innerWidth": 1, "innerHeight": 1 }), 1.0), None);
    }

    #[test]
    fn pages_and_tabs_read_as_short_lists() {
        let read = json!({ "title": "Inbox", "url": "https://mail.example", "elements": [
            { "id": "e1", "role": "button", "name": "Compose" },
            { "id": "e2", "role": "textbox", "name": "Search mail", "value": "invoice", "offscreen": true }
        ]});
        assert_eq!(format_read(&read), "Tab: \"Inbox\" (https://mail.example)\n[e1] button \"Compose\"\n[e2] textbox \"Search mail\" = \"invoice\" (off screen)");
        let tabs = json!([{ "id": 7, "title": "Docs", "url": "https://docs.example", "active": true }]);
        assert_eq!(format_tabs(&tabs), "[tab 7] (active) \"Docs\" (https://docs.example)");
    }
}
