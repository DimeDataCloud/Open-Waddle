# Project Waddle

Project Waddle is a desktop AI agent you can watch. Waddle is a 16×14 pixel-art duck who lives on a transparent overlay above your windows. It walks along title bars to whatever it's about to touch. It says what it's doing in a speech bubble as it works, and it asks before anything risky. You can talk to it, by typing or by voice, while it's working.

Version 0.2 makes it a real assistant:
- It works with your Gmail, Google Calendar, Contacts and Drive.
- It taps you on the shoulder before meetings and when important mail arrives.
- It reads and acts on web pages in Chrome through its extension.
- It finds and reads your documents.
- It sorts each message into a quick chat, a cited research answer or a task.

It runs on your own machine. Apart from cheap cloud model calls (optional; a busy day costs a few cents), it costs nothing.

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
  - **Conversation history:** 🕘 in the chat box (or **Ctrl+Alt+H**) opens the last 50 messages beside the duck, with formatting, working links and a copy button on each. Long replies show their start in the bubble and a **More…** button. ↑ in the chat box brings back what you said before; Shift+Enter starts a new line.
- **Voice.**
  - Press **Ctrl+Alt+Space** (or the 🎙 button) to talk.
  - On Windows this uses the built-in voice typing: free and live.
  - A Whisper-compatible backend is also available: Groq's free tier, OpenAI, or a local whisper server.
  - **Spoken replies** (Settings → Voice, off by default): Waddle reads its answers aloud in a Windows voice of your choice, and optionally nudges and reminders. Only answers are read, not every step. It keeps quiet in full screen and during meetings on your calendar, and stops when you open the chat box, talk or press Stop. **Talk mode** listens again for 6 seconds after answering something you said aloud, so you can have a back-and-forth without touching anything.
- **Act.**
  - See the screen: window list; on Windows, the buttons and fields of any window via UI Automation; screenshots when needed.
  - Open apps, click, type, press shortcuts, scroll and drag. Waddle walks to the target and stands beside it before each action.
  - Read and write files and run terminal commands in its workspace folder (`Documents\Waddle`).
- **Answer, research, or act: picked per message.** A tiny decision model sorts each message in about 0.3 s:
  - *Chat* ("thanks!", "what's a haiku?") gets an instant answer without a task. If it needs current facts ("who won last night?"), the answer uses a web search (about $0.007).
  - *Research* ("research how to keep basil alive indoors") gets a short cited summary in the bubble and a **Full answer** button that saves the whole answer, with numbered sources, to `Documents\Waddle\research\` and opens it.
  - *Tasks* start straight away: Waddle looks at the screen while the router decides, so tasks don't wait for it.
- **Gmail, Calendar and Contacts** (connect your Google account in Settings; see [docs/GOOGLE_SETUP.md](docs/GOOGLE_SETUP.md)).
  - "What's my next meeting?" answers in about 3 seconds, straight from your calendar, without touching the screen.
  - "Any important unread email?", "archive the newsletters", "reply to Ana that Thursday works", "find 45 minutes with Sam next week and invite him".
  - Before anything is sent, a card shows the whole email. You can edit it, and after **Send** you have 10 seconds to **Undo**. Binning mail and deleting events also wait for your click.
  - "Learn how I write emails" makes a short style note from about 20 of your sent emails, so drafts sound like you.
  - While it works through Google, the duck pecks at a little laptop.
  - Not connected? Waddle uses Gmail and Calendar in Chrome on screen instead.
- **Taps you on the shoulder** (with Google connected):
  - Five minutes before a meeting, with **Join** (the Meet link) and **Snooze**.
  - When important email arrives, with **Open**, **Reply** and **Mute sender**. A tiny decision model judges importance from only the sender, subject and first line: on 30 test emails it flagged 11 of 12 important ones and none of the 18 that could wait.
  - In full screen (a film, a presentation) the duck shows a small "!" and waits until you're back or five minutes have passed. Meetings always arrive before they start.
  - The first time you're at the computer after 6 am, it offers a **brief**: today's meetings and important unread email, in one short paragraph (about $0.0005).
  - Waddle starts with Windows (switch it off in Settings), so the nudges work without you opening it.
- **Find and read your files.** "Find my train ticket from October and tell me what I paid", "summarise the PDF I downloaded this morning", "what's the total in budget.xlsx?"
  - Waddle searches your Documents, Downloads, Desktop, Pictures, Music, Videos and Google Drive folders by name, then by text inside.
  - It reads PDF, Word, PowerPoint, Excel and CSV files as text, and with Google connected, your Google Drive too (Docs, Sheets and Slides).
  - It makes new Word, Excel, CSV and Markdown files, and moves and renames files, only in its workspace and folders you allow (Settings → Files). Each change shows a 2 s notice.
  - Deleting sends things to the Recycle Bin and waits for your click. Overwriting a file keeps the old copy in the Recycle Bin.
  - App data, SSH keys, password databases, browser profiles and `.env` files are always off limits.
- **Use Chrome properly** (with Waddle's extension: Settings → Chrome → **Set up Chrome extension**).
  - Waddle reads the page's text, or its buttons, links and fields, instead of squinting at screenshots, and acts on them by name. "Fill in this form with my work address", "open the second result", "what does this page say about returns?"
  - Clicks are real mouse clicks: the duck walks to the button and presses it, so pages behave just as they do for you.
  - "Search for…" goes straight to Google's results and reads them.
  - Without the extension, Waddle still drives Chrome from the screen.
- **Routines.** "Every weekday at 8:45, summarise my unread email" or "every Friday at 4pm, list what I worked on this week": Waddle does it on its own at that time and shows the result. Routines work without the screen (email, calendar, files, connected tools), ask before changing anything, and are skipped (not run late) if Waddle wasn't running. Setting one up through chat needs your click; manage them in Settings → Nudges & routines.
- **Use more apps through MCP.** Add the MCP servers of apps you use (GitHub, Notion, your notes, a database…) in Settings → Tools, by typing the command or pasting the JSON block their instructions show, and Waddle can use their tools. Untrusted servers wait for your click; what tools return is treated as untrusted text. See [docs/MCP.md](docs/MCP.md).
- **Work on what you selected.** Select text anywhere and press **Ctrl+Alt+A**: the chat opens with the selection attached ("make this friendlier", "translate to French"). Waddle puts the result on your clipboard or, after a 2 s notice, replaces the selection.
- **Remember what matters.** "Remember that I take my coffee black" or "my manager is Sam" go into a short list of facts (2 KB at most) that every later task can see. Waddle may save useful facts on its own, never from the screen, files or web pages. See and edit the list in Settings → Memory & privacy.
- **Feel alive, cheaply.** Between tasks a tiny decision model picks what the duck does from the app you're in and whether you're idle (never window titles): perch on your window, explore its edges, watch your cursor, nap when you're away, or keep out of the way when you're in full screen. Each decision costs about $0.00002.
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
| 0 | Reading the screen or a web page, pointing at things | Runs silently |
| 1 | Opening apps, going to web pages and switching tabs, scrolling, finding and reading your files and Google Drive, reading the clipboard, reading your mail, calendar and contacts, reminders, remembering facts, starting sub-tasks | Runs, logged |
| 2 | Clicks and typing (in apps or on web pages), closing tabs, dragging, copying to the clipboard, replacing a selection, new files and documents, overwriting (old copy to the Recycle Bin), moving and renaming files, read-only commands (`dir`, `git status`…), mail drafts, archiving and labels, new or changed events, invitations and replies to them | Shown with a 2 s countdown and a **Cancel** button (or "always ask" in Settings) |
| 3 | Deleting (to the Recycle Bin), any other command, network, self-changes (skills, settings, own code), sending email (the card shows the whole message, editable, then 10 s to Undo), binning mail, deleting events | Waddle turns red and **waits for your click** |

- **Tiers are fixed rules, not a model's judgement.** Approval only comes from a click in Waddle's own UI, never from model output.
- **Screen and file contents are treated as data.** Text from the screen, files and command output is wrapped in tags with an unguessable id. The model is told never to follow instructions inside them.
- **Stopping:** Escape (registered only while a task runs), the ■ Stop button, double-clicking the duck, the tray menu, or saying "stop".
- **Activity log:** every action, decision and result goes to a SQLite log that can only be appended to. Each entry is hash-chained to the previous one, so edits are detectable. Settings → Activity log → *Verify integrity*.
- **No training on your data:** OpenRouter requests ask for providers that don't keep or train on prompts (`data_collection: deny`). Settings → Memory & privacy turns this off if a model you want needs it.
- **Honest limits:** commands run in the workspace folder, but this is **not an OS sandbox**. Tier 3 approval is the real gate.

## Quick start (Windows 11 on ARM, e.g. Surface Pro with Snapdragon)

### Option A: install a build

1. Download `Waddle_0.2.0_arm64-setup.exe` from the latest CI run's artifacts (or the file shared with you).
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
| **OpenRouter** (default) | ~$0.0005–0.001 per task (measured, [docs/MODELS.md](docs/MODELS.md)) | Create a key at <https://openrouter.ai/keys>, add $5 credit, paste the key. Defaults: `openai/gpt-6-luna` plans; `google/gemini-2.5-flash-lite` chats and researches; TypeSafe's Jev makes the split-second decisions. Your PC does almost no work. |
| **Ollama** (local) | Free | Install <https://ollama.com> (0.17.6 or newer), run `ollama pull qwen3.5:4b`. Waddle turns off the model's hidden "thinking", caps it to a third of your CPU cores and unloads it 30 s after each task. On Snapdragon it runs on the CPU only, so it's slow and keeps the machine busy while it thinks. `qwen3.5:2b` is faster; `qwen3.5:9b` is smarter if you have 32 GB of RAM. |
| LM Studio / Foundry Local / custom | Free | Any OpenAI-compatible endpoint. LM Studio (`qwen3.5-4b`) runs Qwen3.5 faster than Ollama does. Foundry Local can use the Snapdragon NPU (text models only). |

API keys are stored in **Windows Credential Manager** (macOS Keychain / Linux Secret Service), not in files. For development, `OPENROUTER_API_KEY` or `WADDLE_API_KEY` env vars also work. `WADDLE_PROVIDER=mock` forces demo mode.

### Connect Google and Chrome (optional, 5 minutes each)

- **Google** (Gmail, Calendar, Contacts, Drive): follow [docs/GOOGLE_SETUP.md](docs/GOOGLE_SETUP.md). You create your own OAuth client, paste it into Settings → Google account, and press **Connect**.
- **Chrome:** Settings → Chrome → **Set up Chrome extension**. Then in Chrome open `chrome://extensions`, turn on **Developer mode**, click **Load unpacked**, and pick the folder that opened. See [extension/README.md](extension/README.md).
- **Files:** Settings → Files. Waddle can read your user folders. Add the folders it may also change.

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
10. "Research the best houseplants for low light" → a short summary with sources in brackets, then **Full answer** opens a Markdown file from `Documents\Waddle\research\`.
11. Select a sentence in Notepad, press **Ctrl+Alt+A**, type "make this more formal" → a 2 s notice, then the sentence is replaced.
12. "Remember that my dog is called Biscuit" → Settings → Memory & privacy lists it; a later "what's my dog called?" knows.
13. Connect Google (Settings → Google account, then [docs/GOOGLE_SETUP.md](docs/GOOGLE_SETUP.md)) → "Connected as you@…".
14. "What's my next meeting?" → the answer comes in under 5 seconds, with the Meet link if there is one.
15. "Reply to <someone>'s last email saying thanks" → the send card shows the whole reply; **Edit** a word, **Send**, then press **Undo** within 10 s → nothing is sent. Do it again without Undo → the reply shows up in the thread in Gmail.
16. "Find 30 minutes with <a contact> next week and send an invite with a Meet link" → three free slots inside 9–17, then a 2 s notice, then the event and the invite.
17. Put a meeting with a Meet link in your calendar 6 minutes from now → about 5 minutes before, the duck says "📅 … starts in 5 min." with **Join** (opens Meet) and **Snooze** (back in 2 minutes).
18. Send yourself an email from another account with the subject "Can you call me today?" → within about 2 minutes a ✉️ nudge; **Mute sender** stops further ones (Settings → Nudges lists them).
19. Start a full-screen video, then repeat step 17 → a red "!" over the duck instead of the bubble; leave full screen → the full nudge.
20. Restart Windows → Waddle starts on its own. The next morning, the first time you use the computer → "☀️ Good morning! Want a quick brief of today?"
21. Settings → Chrome → **Set up Chrome extension**, then follow the three steps → "Connected (extension 0.1.12)".
22. On any sign-up or search page: "fill in the search box with rubber ducks and press search" → the duck walks to the field and button on screen and the page reacts as if you'd clicked.
23. "Search for the weather in Paris and tell me tomorrow's forecast" → Google's results open in Chrome and Waddle answers from them.
24. Download any PDF, then ask "what's the PDF I just downloaded about?" → Waddle finds it in Downloads and summarises it, with no approval needed.
25. "Make a spreadsheet of my last three electricity bills" (with the bills in Documents) → a 2 s notice, then an .xlsx in `Documents\Waddle` that opens in Excel.
26. Settings → Files: add a folder to the "create, change" list, then "move the receipts from my workspace into <that folder>" → notices, and the files move. "Delete the old one" → a red card; after you approve, the file is in the Recycle Bin.
27. With Google connected (press **Connect** again after this update, to allow Drive), "what does my Drive say about the budget?" → it finds and reads Docs and Sheets.

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
  src/tools/            tool schemas, coordinate conversion, scoped files, documents, browser tools, commands
  src/google/           Google sign-in (PKCE), Gmail, Calendar, Contacts, Drive
  src/nudges.rs         when to nudge: meetings, important mail, the morning brief
src-tauri/            Desktop shell: overlay, click-through, window tracking, input, UIA, voice, tray
src/                  Overlay frontend: sprite renderer, palette maths, physics, pathfinding, UI
bench/                Model benchmark: realistic app screens rendered in Chrome + task checks
extension/            Waddle for Chrome (MV3): reads pages and acts on elements via native messaging
training/             Synthetic click data, LoRA fine-tuning and GRPO scripts
docs/ARCHITECTURE.md  How the blueprint maps to this MVP, what's deferred and why
docs/MODELS.md        Which model to use, with benchmark results (screen tasks and assistant tasks)
docs/GOOGLE_SETUP.md  Connecting Gmail, Calendar, Contacts and Drive
docs/TRAINING.md      How to make the model better: harness, data, fine-tuning
docs/MCP.md           Adding MCP servers (other apps' tools) and how they're kept safe
```

## Development

```bash
cargo test --workspace      # core + app unit tests, agent/session integration tests
npm test                    # frontend and the Chrome extension's page code (jsdom)
npm run build:extension     # rebuild the extension (its built files are committed and embedded in the app)
npm run typecheck
npm run tauri dev
```

Linux build dependencies: `libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev libpipewire-0.3-dev libasound2-dev libclang-dev`.

## Roadmap

v0.2 (Gmail, Calendar, Chrome, files, nudges, memory) is done. The v0.3 production pass is planned in [docs/ROADMAP.md](docs/ROADMAP.md): reliability fixes, a spending meter and budget, multiple monitors, a conversation panel, spoken replies, MCP tools, routines, a first-run guide and visual effects, released as 0.2.1 → 0.2.9 for on-device testing, then 0.3.0.
