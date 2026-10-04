# Project Waddle

Project Waddle is a desktop AI agent you can watch. Waddle is a 16×14 pixel-art duck who lives on a transparent overlay above your windows. It walks along title bars to whatever it's about to touch. It says what it's doing in a speech bubble as it works, and it asks before anything risky. You can talk to it, by typing or by voice, while it's working.

This repo is the MVP foundation. It runs on your own machine. Apart from cheap cloud model calls (optional), it costs nothing.

```
            ┌──────────────────────────── Brain (waddle-core) ───────────────────────────┐
 you ──────►│ Session: one task at a time, but you can keep talking                      │
 (voice/    │   ├─ Planner lane (System 2): streams narration, calls tools, step by step │
  typing)   │   ├─ Quick-reply lane (System 1): answers instantly while the planner works│
            │   └─ "stop"/"wait"/Esc: halts immediately, no model involved               │
            │ Safety: deterministic tiers 0–3 · untrusted-content tags · hash-chained log│
            └───────┬───────────────────────────────┬────────────────────────────────────┘
                    │ GUI actions                    │ files / commands
            ┌───────▼─────── Body + Hands + Eyes (Tauri app) ─┐   ┌─▼─ Workspace folder ─┐
            │ overlay · click-through · window platforms      │   │ read/write/list      │
            │ walk → stand beside target → act (enigo)        │   │ run_command (timeout,│
            │ Windows UI Automation · screenshots (xcap)      │   │ output cap, kill)    │
            └─────────────────────────────────────────────────┘   └──────────────────────┘
```

## What it can do today

- **Live on your desktop.**
  - Waddle walks on the top edges of your windows and the bottom of the screen, rides windows you drag, and falls when its window disappears.
  - It wanders when idle and falls asleep after two quiet minutes.
  - Drag the duck to carry it. Click it to chat. Right-click it for Settings.
- **Real-time conversation.**
  - Replies stream into the bubble as they're generated.
  - Messages you send *during* a task get an instant quick reply, and they change the running task at its next step.
  - Saying or typing "stop", "wait" or "cancel" halts at once.
- **Voice.**
  - Press **Ctrl+Alt+Space** (or the 🎙 button) to talk.
  - On Windows this uses the built-in voice typing: free and live.
  - A Whisper-compatible backend is also available: Groq's free tier, OpenAI, or a local whisper server.
- **Act.**
  - See the screen: window list; on Windows, the buttons and fields of any window via UI Automation; screenshots when needed.
  - Open apps, click, type, press shortcuts, scroll and drag. Waddle walks to the target and stands beside it before each action.
  - Read and write files and run terminal commands in its workspace folder (`Documents\Waddle`).
- **Assist.**
  - "Where's the export button?" or "how do I turn on Night light?": Waddle walks over and circles the spot with a label instead of clicking, so you learn where it is.
  - Reads what you copied ("summarise what I copied", "translate this") and puts results on your clipboard ("copy that address for me").
  - Reminders: "remind me in 20 minutes to stretch", "remind me at 3pm to call Sam". The duck chimes and pops up when one is due, even after a restart, and says so if it came due while Waddle was closed.
- **Improve itself, with your approval.**
  - Save "skills" (lessons it reads back at the start of every task) and change its own settings (model, wandering, safety countdown…).
  - Split big jobs into sub-tasks handled by nested copies of itself, up to 2 levels deep by default.
  - Edit its own source code, if you point it at this repo in Settings.

## Safety model

| Tier | What | What happens |
|---|---|---|
| 0 | Reading the screen, pointing at things | Runs silently |
| 1 | Opening apps, scrolling, reading the workspace or clipboard, reminders, starting sub-tasks | Runs, logged |
| 2 | Clicks, typing, dragging, copying to the clipboard, new files, read-only commands (`dir`, `git status`…) | Shown with a 2 s countdown and a **Cancel** button (or "always ask" in Settings) |
| 3 | Deleting/overwriting, any other command, network, self-changes (skills, settings, own code) | Waddle turns red and **waits for your click** |

- **Tiers are fixed rules, not a model's judgement.** Approval only comes from a click in Waddle's own UI, never from model output.
- **Screen and file contents are treated as data.** Text from the screen, files and command output is wrapped in tags with an unguessable id. The model is told never to follow instructions inside them.
- **Stopping:** Escape (registered only while a task runs), the ■ Stop button, double-clicking the duck, the tray menu, or saying "stop".
- **Activity log:** every action, decision and result goes to a SQLite log that can only be appended to. Each entry is hash-chained to the previous one, so edits are detectable. Settings → Activity log → *Verify integrity*.
- **Honest limits:** commands run in the workspace folder, but this is **not an OS sandbox**. Tier 3 approval is the real gate.

## Quick start (Windows 11 on ARM, e.g. Surface Pro with Snapdragon)

### Option A: install a build

1. Download `Waddle_0.1.0_arm64-setup.exe` from the latest CI run's artifacts (or the file shared with you).
2. It isn't code-signed yet, so Windows SmartScreen will warn you. Click **More info → Run anyway**.
3. Waddle appears at the bottom of your screen in **demo mode** (no AI). Click the duck and type anything to see the full flow.

### Option B: build from source

1. Install **Rust**: <https://rustup.rs> (choose the default `aarch64-pc-windows-msvc` host).
2. Install **Visual Studio 2022 Build Tools** with:
   - "Desktop development with C++"
   - **MSVC ARM64 build tools**
   - a Windows 11 SDK
3. Install **Node.js 22 LTS (ARM64)**: <https://nodejs.org>
4. Then:
   ```powershell
   git clone https://github.com/DimeDataCloud/Waddle; cd Waddle
   npm install
   npm run tauri dev            # run it
   npm run tauri build          # make an installer (src-tauri target\release\bundle\nsis)
   ```

WebView2 ships with Windows 11, so nothing else is needed.

### Give Waddle a brain (pennies, or free)

Right-click the duck → **Settings**:

| Preset | Cost | Notes |
|---|---|---|
| **OpenRouter** (default) | ~$0.001 per task (measured, [docs/MODELS.md](docs/MODELS.md)) | Create a key at <https://openrouter.ai/keys>, add $5 credit, paste the key. Defaults: `qwen/qwen3-vl-8b-instruct` plans; `google/gemini-2.5-flash-lite` gives quick replies. Your PC does almost no work. |
| **Ollama** (local) | Free | Install <https://ollama.com> (0.17.6 or newer), run `ollama pull qwen3.5:4b`. Waddle turns off the model's hidden "thinking", caps it to a third of your CPU cores and unloads it 30 s after each task. On Snapdragon it runs on the CPU only, so it's slow and keeps the machine busy while it thinks. `qwen3.5:2b` is faster; `qwen3.5:9b` is smarter if you have 32 GB of RAM. |
| LM Studio / Foundry Local / custom | Free | Any OpenAI-compatible endpoint. LM Studio (`qwen3.5-4b`) runs Qwen3.5 faster than Ollama does. Foundry Local can use the Snapdragon NPU (text models only). |

API keys are stored in **Windows Credential Manager** (macOS Keychain / Linux Secret Service), not in files. For development, `OPENROUTER_API_KEY` or `WADDLE_API_KEY` env vars also work. `WADDLE_PROVIDER=mock` forces demo mode.

## Try it: on-device checklist

1. The duck appears, drops onto a window's title bar or the taskbar edge, and wanders.
2. Clicks pass through everywhere except on the duck and its bubbles.
3. Drag the duck and drop it: it falls and lands on a window.
4. "Create hello.txt that says hi" → a Tier 2 countdown appears, then `Documents\Waddle\hello.txt` exists.
5. "Delete hello.txt" → the duck turns red and an approval card waits.
6. "Open Notepad and type hello" → Waddle opens Notepad, walks to it, and types.
7. While it's working, say "actually type goodbye instead" → you get an instant reply in blue, and the task adapts.
8. Press **Escape** mid-task → it stops.
9. Settings → Activity log shows each step; *Verify integrity* reports the log intact.

## Something not working?

Right-click the duck → **Settings** → **Diagnostics** → **Run self-test**.
- It checks the display, window tracking, accessibility, screenshots, mouse and keyboard, key storage, the workspace, commands, the activity log, the talk shortcut, the microphone, and the model (one small call).
- It saves `waddle-report-<time>.txt` in your workspace folder. Send that file. It leaves out keys, file contents, window titles and what you've typed.
- Waddle also keeps a log at `%LOCALAPPDATA%\dev.waddle.app\logs\waddle.log` (macOS: `~/Library/Logs/dev.waddle.app/`, Linux: `~/.local/share/dev.waddle.app/logs/`). The report includes its last lines.

## Project layout

```
crates/waddle-core/   Brain, non-GUI Hands and safety (no OS code; fully unit-tested)
  src/session.rs        real-time session: steering, quick replies, halt phrases, memory
  src/agent.rs          planner loop, tier gate, delegation (recursive sub-tasks)
  src/llm/              streaming providers: OpenAI-compatible (SSE), Ollama (NDJSON), mock
  src/safety.rs         deterministic permission tiers
  src/skills.rs         long-term skill memory (self-improvement)
  src/audit.rs          append-only, hash-chained SQLite log
  src/tools/            tool schemas, coordinate conversion, workspace files, commands
src-tauri/            Desktop shell: overlay, click-through, window tracking, input, UIA, voice, tray
src/                  Overlay frontend: sprite renderer, palette maths, physics, pathfinding, UI
bench/                Model benchmark: realistic app screens rendered in Chrome + task checks
training/             Synthetic click data, LoRA fine-tuning and GRPO scripts
docs/ARCHITECTURE.md  How the blueprint maps to this MVP, what's deferred and why
docs/MODELS.md        Which model to use, with benchmark results
docs/TRAINING.md      How to make the model better: harness, data, fine-tuning
```

## Development

```bash
cargo test --workspace      # core + app unit tests, agent/session integration tests
npm test                    # frontend: palette, platforms, physics, pathfinding, behaviour
npm run typecheck
npm run tauri dev
```

Linux build dependencies: `libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev libpipewire-0.3-dev libasound2-dev libclang-dev`.

## Roadmap

- macOS: Accessibility and Screen Recording permission prompts, the macOS accessibility fast path, Apple Silicon build.
- Multiple monitors and mixed display scaling.
- OmniParser as an optional local vision service. Its detector is AGPL-licensed; keep it separate from paid tiers.
- Benchmark Laya (local "System 1" decision model) on real activity-log data for screenshot-needed and task-done decisions.
- Foundry Local NPU mode with text-only accessibility grounding.
- The "Pictionary" defence against injected instructions: render untrusted text as an image instead of passing it as text.
- PTY terminal and a real OS sandbox for commands.
- The other seven characters, several characters at once, and a white-label asset pack loader.
