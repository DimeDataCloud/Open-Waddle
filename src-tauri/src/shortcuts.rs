//! Global hotkeys. Escape halts the running task and is registered only while
//! a task runs, so Waddle never steals Escape from other apps when idle.
//! Ctrl+Alt+Space (any time) opens the chat and starts listening.
//! Ctrl+Alt+A opens the chat with the text selected in the front app attached.
//! Ctrl+Alt+H opens (or closes) the conversation history.

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutEvent, ShortcutState};

use crate::AppState;

fn halt_key() -> Shortcut {
    Shortcut::new(None, Code::Escape)
}

pub fn talk_key() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Space)
}

pub fn selection_key() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyA)
}

pub fn history_key() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyH)
}

pub fn set_halt_hotkey(app: &AppHandle, on: bool) {
    let gs = app.global_shortcut();
    let key = halt_key();
    let result = if on {
        if gs.is_registered(key) { Ok(()) } else { gs.register(key) }
    } else if gs.is_registered(key) {
        gs.unregister(key)
    } else {
        Ok(())
    };
    if let Err(e) = result {
        log::warn!("could not {} the Escape halt hotkey: {e}", if on { "register" } else { "release" });
    }
}

pub fn handle(app: &AppHandle, shortcut: &Shortcut, event: ShortcutEvent) {
    if event.state() != ShortcutState::Pressed {
        return;
    }
    let Some(state) = app.try_state::<AppState>() else { return };
    if *shortcut == halt_key() {
        if state.host.halt_hotkey_allowed() && state.session.halt() {
            let _ = app.emit_to("overlay", "agent", waddle_core::AgentEvent::Notice { text: "Stopping!".into() });
        }
    } else if *shortcut == talk_key() {
        crate::focus_overlay(app);
        let _ = app.emit_to("overlay", "chat:open", serde_json::json!({ "voice": true }));
    } else if *shortcut == history_key() {
        crate::focus_overlay(app);
        let _ = app.emit_to("overlay", "history:toggle", ());
    } else if *shortcut == selection_key() {
        let (app, host) = (app.clone(), state.host.clone());
        tauri::async_runtime::spawn(async move {
            // Read the selection while the user's app still has focus, then open the chat.
            let preview = host.capture_selection().await;
            crate::focus_overlay(&app);
            let _ = app.emit_to("overlay", "chat:open", serde_json::json!({ "voice": false, "selection": preview }));
        });
    }
}
